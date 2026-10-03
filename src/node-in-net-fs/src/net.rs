use client_core::{AppEventHandler, NetCmd, P2pFailure, P2pPeerState, WsState};
use nodeinnet_protocol::{MemoryPeerStore, NodeInfo, P2pMessage};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Peers are reached only through the signalling server; tests/no_lan_discovery.rs pins this.
const LOCAL_DISCOVERY: bool = false;

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

fn commands() -> &'static Mutex<Option<tokio::sync::mpsc::Sender<NetCmd>>> {
    static HELD: OnceLock<Mutex<Option<tokio::sync::mpsc::Sender<NetCmd>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
}

pub fn sender() -> Option<tokio::sync::mpsc::Sender<NetCmd>> {
    match commands().lock() {
        Ok(held) => held.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn terminals_of(nodes: &[NodeInfo]) -> std::collections::HashMap<String, String> {
    nodes
        .iter()
        .filter_map(|node| {
            let shared = node
                .resources
                .iter()
                .find(|held| held.resource_type == nodeinnet_protocol::ResourceType::Terminal)?;
            Some((node.id.clone(), shared.id.clone()))
        })
        .collect()
}

struct Told {
    changed: Arc<dyn Fn() + Send + Sync>,
}

#[async_trait::async_trait]
impl AppEventHandler for Told {
    async fn on_log(&self, msg: String) {
        if msg.starts_with(['❌', '🚨', '⚠', '🚧']) {
            crate::note(&msg);
        }
    }

    async fn on_connected(&self) {
        settle(Seen {
            connected: true,
            ..snapshot()
        });
        (self.changed)();
    }

    async fn on_disconnected(&self) {
        settle(Seen {
            connected: false,
            ..snapshot()
        });
        (self.changed)();
    }

    async fn on_update_nodes(&self, nodes: Vec<NodeInfo>) {
        crate::note(&format!("socket: {} node(s) listed", nodes.len()));
        crate::p2p_rpc::set_peer_terminals(terminals_of(&nodes));
        settle(Seen {
            nodes,
            connected: true,
            problem: String::new(),
        });
        (self.changed)();
    }

    async fn on_download_complete(&self, _path: PathBuf) {}

    async fn on_ws_state_changed(&self, state: WsState) {
        if state == WsState::Disconnected {
            settle(Seen {
                connected: false,
                ..snapshot()
            });
            (self.changed)();
        }
    }

    async fn on_peer_failed(&self, peer_id: String, failure: P2pFailure) {
        crate::note(&format!("peer {peer_id}: {failure}"));
    }

    async fn on_peer_state_changed(&self, peer_id: String, state: P2pPeerState) {
        crate::note(&format!("peer {peer_id}: {state:?}"));
    }

    async fn on_p2p_message(&self, msg: P2pMessage) {
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
}

pub fn open(url: String, me: NodeInfo, private_key: String, changed: Arc<dyn Fn() + Send + Sync>) {
    p2p_handlers::install(
        p2p_handlers::Capabilities::FILESYSTEM,
        p2p_handlers::HostSettings::default(),
    );

    let (net_tx, net_rx) = tokio::sync::mpsc::channel::<NetCmd>(256);
    client_core::network::start_network_thread(
        net_rx,
        net_tx.clone(),
        Arc::new(Told { changed }),
        me.clone(),
        private_key,
        Arc::new(MemoryPeerStore::default()),
        LOCAL_DISCOVERY,
    );
    // No runtime on this thread, so try_send rather than an await.
    if let Err(why) = net_tx.try_send(NetCmd::Connect(url, me.announced(), None)) {
        crate::note(&format!("could not ask the transport to connect: {why}"));
    }
    match commands().lock() {
        Ok(mut held) => *held = Some(net_tx),
        Err(poisoned) => *poisoned.into_inner() = Some(net_tx),
    }
}

pub fn close() {
    let held = match commands().lock() {
        Ok(mut held) => held.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(tx) = held {
        let _ = tx.try_send(NetCmd::Disconnect);
    }
    settle(Seen::default());
}
