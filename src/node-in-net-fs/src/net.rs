use client_core::{
    AppEventHandler, LanCmd, LanConfig, LanEvent, NetCmd, P2pFailure, P2pPeerState, PairPrompt,
    PairReply, WsState,
};
use nodeinnet_protocol::{MemoryPeerStore, NodeInfo, P2pMessage, SharedResource};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{OwnedPermit, Sender};
use tokio::sync::watch;

pub const STOP_BUDGET: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub nodes: Vec<NodeInfo>,
    pub connected: bool,
    pub problem: String,
}

fn seen() -> &'static Mutex<Seen> {
    static HELD: OnceLock<Mutex<Seen>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Seen::default()))
}

pub fn snapshot() -> Seen {
    match seen().lock() {
        Ok(held) => held.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn settle(next: Seen) {
    match seen().lock() {
        Ok(mut held) => *held = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

pub fn socket_url(ws_url: &str, access_token: &str, session_id: &str) -> String {
    format!("{ws_url}?token={access_token}&session_id={session_id}")
}

struct Running {
    thread: JoinHandle<()>,
    // Reserved ahead: an upload can keep every other slot taken for minutes.
    shutdown: Option<OwnedPermit<NetCmd>>,
    disconnect: Option<OwnedPermit<NetCmd>>,
    public_key: String,
}

#[derive(Default)]
struct Life {
    running: Option<Running>,
    stopping: Vec<JoinHandle<()>>,
    account: Option<String>,
}

fn life() -> MutexGuard<'static, Life> {
    static HELD: OnceLock<Mutex<Life>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Life::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn current() -> MutexGuard<'static, Option<Sender<NetCmd>>> {
    static HELD: OnceLock<Mutex<Option<Sender<NetCmd>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static GENERATION: AtomicU64 = AtomicU64::new(0);
static LAN_UP: AtomicBool = AtomicBool::new(false);
static ATTACHED: AtomicBool = AtomicBool::new(false);
static RESTARTING: AtomicBool = AtomicBool::new(false);

fn halt_signal() -> &'static watch::Sender<bool> {
    static HELD: OnceLock<watch::Sender<bool>> = OnceLock::new();
    HELD.get_or_init(|| watch::channel(false).0)
}

pub(crate) fn halt() {
    halt_signal().send_replace(true);
}

pub(crate) fn unhalt() {
    halt_signal().send_replace(false);
}

fn halted() -> bool {
    *halt_signal().borrow()
}

async fn until_halted<T>(work: impl Future<Output = T>) -> Option<T> {
    let mut halt = halt_signal().subscribe();
    tokio::select! {
        done = work => Some(done),
        _ = halt.wait_for(|on| *on) => None,
    }
}

fn waited<T>(work: impl Future<Output = T>) -> Option<T> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .ok()?
        .block_on(until_halted(work))
}

pub fn sender() -> Option<Sender<NetCmd>> {
    current().clone()
}

/// Tests only.
pub fn adopt_sender(tx: Sender<NetCmd>) {
    *current() = Some(tx);
}

pub fn attached() -> bool {
    ATTACHED.load(Ordering::SeqCst)
}

pub fn running() -> bool {
    current().is_some()
}

pub fn lan_running() -> bool {
    LAN_UP.load(Ordering::SeqCst)
}

pub fn restarting() -> bool {
    RESTARTING.load(Ordering::SeqCst)
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

fn install() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        use p2p_handlers::Capabilities;
        let served = if cfg!(target_os = "android") {
            Capabilities::FILESYSTEM
        } else {
            Capabilities::FILESYSTEM | Capabilities::TERMINAL
        };
        p2p_handlers::install(served, p2p_handlers::HostSettings::default());
    });
}

pub fn start(lan: Option<LanConfig>) {
    started(&mut life(), lan);
}

fn started(life: &mut Life, lan: Option<LanConfig>) -> bool {
    if halted() || life.running.is_some() {
        return false;
    }
    let (done, left): (Vec<_>, Vec<_>) = std::mem::take(&mut life.stopping)
        .into_iter()
        .partition(JoinHandle::is_finished);
    for thread in done {
        let _ = thread.join();
    }
    life.stopping = left;
    if !life.stopping.is_empty() {
        crate::note("the last network thread has not stopped yet, so no other is started");
        return false;
    }
    let Some((private_key, public_key)) = crate::identity::pair() else {
        return false;
    };
    install();
    let me = NodeInfo {
        public_key: public_key.clone(),
        ..crate::identity::announcing(&crate::identity::device_id())
    };
    let (net_tx, net_rx) = tokio::sync::mpsc::channel::<NetCmd>(256);
    let Ok(shutdown) = net_tx.clone().try_reserve_owned() else {
        return false;
    };
    let with_lan = lan.is_some();
    let thread = client_core::network::start_network_thread(
        net_rx,
        net_tx.clone(),
        Arc::new(Told {
            generation: generation(),
        }),
        me,
        private_key,
        Arc::new(MemoryPeerStore::default()),
        lan,
    );
    LAN_UP.store(with_lan, Ordering::SeqCst);
    *current() = Some(net_tx);
    life.running = Some(Running {
        thread,
        shutdown: Some(shutdown),
        disconnect: None,
        public_key,
    });
    crate::note(if with_lan {
        "the network is up, the local network with it"
    } else {
        "the network is up, without the local network"
    });
    crate::notify::mark(crate::notify::MULTICAST);
    true
}

pub fn stop(budget: Duration) {
    stopped(&mut life(), budget);
}

fn stopped(life: &mut Life, budget: Duration) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    *current() = None;
    LAN_UP.store(false, Ordering::SeqCst);
    if let Some(mut running) = life.running.take() {
        drop(running.disconnect.take());
        if let Some(shutdown) = running.shutdown.take() {
            shutdown.send(NetCmd::Shutdown);
        }
        life.stopping.push(running.thread);
    }
    let deadline = Instant::now() + budget;
    while life.stopping.iter().any(|thread| !thread.is_finished()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let (done, left): (Vec<_>, Vec<_>) = std::mem::take(&mut life.stopping)
        .into_iter()
        .partition(JoinHandle::is_finished);
    for thread in done {
        let _ = thread.join();
    }
    if !left.is_empty() && !budget.is_zero() {
        crate::note(&format!(
            "the network thread did not stop within {} s; it is left to finish",
            budget.as_secs()
        ));
    }
    life.stopping = left;
    settle(Seen::default());
    crate::p2p_rpc::set_peer_terminals(HashMap::new());
    crate::lan::forget();
    crate::pairing::forget();
    crate::notify::mark(crate::notify::MULTICAST | crate::notify::DRIVES);
}

pub fn restart() {
    restart_after(|| {});
}

/// `between` runs once the old thread has finished and before the next one starts.
pub fn restart_after(between: impl FnOnce()) {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    RESTARTING.store(true, Ordering::SeqCst);
    stopped(&mut life(), Duration::ZERO);
    while !halted() && life().stopping.iter().any(|thread| !thread.is_finished()) {
        std::thread::sleep(Duration::from_millis(50));
    }
    if !halted() {
        between();
        let lan = if crate::draws_with_gtk() {
            crate::lan::launch()
        } else {
            None
        };
        let mut life = life();
        if started(&mut life, lan) {
            if let Some(url) = life.account.clone() {
                reconnected(&mut life, url);
            }
        }
    }
    RESTARTING.store(false, Ordering::SeqCst);
    crate::notify::mark(crate::notify::SETTINGS);
}

fn reconnected(life: &mut Life, url: String) {
    let (Some(running), Some(tx)) = (life.running.as_mut(), current().clone()) else {
        return;
    };
    let me = NodeInfo {
        public_key: running.public_key.clone(),
        ..crate::identity::announcing(&crate::identity::device_id())
    };
    running.disconnect = tx.clone().try_reserve_owned().ok();
    if let Err(why) = tx.try_send(NetCmd::Connect(url, me, None)) {
        crate::note(&format!("could not ask the transport to connect again: {why}"));
    }
}

/// Blocks until there is room in the queue: call it from a plugin thread.
pub fn connect(url: String, me: NodeInfo) {
    let (tx, reserve, me, began) = {
        let mut life = life();
        life.account = Some(url.clone());
        ATTACHED.store(true, Ordering::SeqCst);
        let (Some(running), Some(tx)) = (life.running.as_ref(), current().clone()) else {
            return;
        };
        let me = NodeInfo {
            public_key: running.public_key.clone(),
            ..me
        };
        (tx, running.disconnect.is_none(), me, generation())
    };
    if reserve {
        let Some(Ok(permit)) = waited(tx.clone().reserve_owned()) else {
            return;
        };
        let mut life = life();
        match life.running.as_mut() {
            Some(running) if generation() == began && running.disconnect.is_none() => {
                running.disconnect = Some(permit);
            }
            _ => return,
        }
    }
    if let Some(Err(why)) = waited(tx.send(NetCmd::Connect(url, me, None))) {
        crate::note(&format!("could not ask the transport to connect: {why}"));
    }
}

pub fn disconnect() {
    let mut life = life();
    life.account = None;
    ATTACHED.store(false, Ordering::SeqCst);
    if let Some(permit) = life
        .running
        .as_mut()
        .and_then(|running| running.disconnect.take())
    {
        permit.send(NetCmd::Disconnect);
    }
    drop(life);
    settle(Seen::default());
    refresh_terminals();
    crate::notify::mark(crate::notify::DRIVES);
}

struct Control {
    orders: mpsc::Sender<NetCmd>,
    thread: JoinHandle<()>,
}

fn control() -> MutexGuard<'static, Option<Control>> {
    static HELD: OnceLock<Mutex<Option<Control>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns at once; the commands reach the network in order, however full its queue.
pub fn queue(cmd: NetCmd) {
    let mut held = control();
    if held.as_ref().is_none_or(|control| control.thread.is_finished()) {
        let (orders, taken) = mpsc::channel();
        let Ok(thread) = std::thread::Builder::new()
            .name("nin-control".to_string())
            .spawn(move || pass_on(taken))
        else {
            return;
        };
        *held = Some(Control { orders, thread });
    }
    if let Some(control) = held.as_ref() {
        let _ = control.orders.send(cmd);
    }
}

pub fn lan(cmd: LanCmd) {
    queue(NetCmd::Lan(cmd));
}

pub fn stop_control() {
    let held = control().take();
    if let Some(control) = held {
        drop(control.orders);
        let _ = control.thread.join();
    }
}

fn pass_on(taken: mpsc::Receiver<NetCmd>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return;
    };
    while let Ok(mut cmd) = taken.recv() {
        loop {
            if halted() {
                return;
            }
            let Some(tx) = sender() else {
                if !restarting() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            match runtime.block_on(until_halted(tx.send(cmd))) {
                None => return,
                Some(Ok(())) => break,
                Some(Err(back)) => {
                    cmd = back.0;
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

fn terminal_of(resources: &[SharedResource]) -> Option<String> {
    resources
        .iter()
        .find(|held| held.resource_type == nodeinnet_protocol::ResourceType::Terminal)
        .map(|held| held.id.clone())
}

pub(crate) fn refresh_terminals() {
    let account = snapshot()
        .nodes
        .into_iter()
        .filter_map(|node| Some((node.id, terminal_of(&node.resources)?)));
    let lan = crate::lan::snapshot()
        .state
        .map(|state| state.devices)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|device| Some((device.id, terminal_of(&device.resources)?)));
    crate::p2p_rpc::set_peer_terminals(account.chain(lan).collect());
}

/// Tests only.
pub fn told() -> Arc<dyn AppEventHandler> {
    Arc::new(Told {
        generation: generation(),
    })
}

struct Told {
    generation: u64,
}

impl Told {
    fn current(&self) -> bool {
        self.generation == generation()
    }
}

#[async_trait::async_trait]
impl AppEventHandler for Told {
    async fn on_log(&self, msg: String) {
        if msg.starts_with(['❌', '🚨', '⚠', '🚧']) {
            crate::note(&msg);
        }
    }

    async fn on_connected(&self) {
        if !self.current() {
            return;
        }
        settle(Seen {
            connected: true,
            ..snapshot()
        });
        crate::notify::mark(crate::notify::ACCOUNT | crate::notify::DRIVES);
    }

    async fn on_disconnected(&self) {
        if !self.current() {
            return;
        }
        settle(Seen {
            connected: false,
            ..snapshot()
        });
        crate::notify::mark(crate::notify::ACCOUNT | crate::notify::DRIVES);
    }

    async fn on_update_nodes(&self, nodes: Vec<NodeInfo>) {
        if !self.current() {
            return;
        }
        crate::note(&format!("socket: {} node(s) listed", nodes.len()));
        settle(Seen {
            nodes,
            connected: true,
            problem: String::new(),
        });
        refresh_terminals();
        crate::notify::mark(crate::notify::ACCOUNT | crate::notify::DRIVES);
    }

    async fn on_download_complete(&self, _path: PathBuf) {}

    async fn on_ws_state_changed(&self, state: WsState) {
        if state == WsState::Disconnected && self.current() {
            settle(Seen {
                connected: false,
                ..snapshot()
            });
            crate::notify::mark(crate::notify::ACCOUNT | crate::notify::DRIVES);
        }
    }

    async fn on_peer_failed(&self, peer_id: String, failure: P2pFailure) {
        crate::note(&format!("peer {peer_id}: {failure}"));
    }

    async fn on_peer_state_changed(&self, peer_id: String, state: P2pPeerState) {
        crate::note(&format!("peer {peer_id}: {state:?}"));
    }

    async fn on_p2p_message(&self, msg: P2pMessage) {
        if !self.current() {
            return;
        }
        // Terminal output carries no request id, so it cannot go through `pending`.
        if let P2pMessage::TerminalOutput { resource_id, data } = &msg {
            crate::p2p_shell::feed_output(resource_id, data.clone());
            return;
        }
        crate::pending::feed_response(msg);
    }

    async fn on_p2p_connected(&self, peer_id: String) {
        crate::note(&format!("peer {peer_id} connected"));
    }

    async fn on_p2p_disconnected(&self, peer_id: String) {
        crate::note(&format!("peer {peer_id} disconnected"));
    }

    async fn on_pair_request(&self, prompt: PairPrompt, reply: PairReply) {
        if self.current() {
            crate::pairing::incoming(prompt, reply);
        }
    }

    async fn on_lan_event(&self, event: LanEvent) {
        if self.current() {
            crate::lan::heard(event);
        }
    }
}
