use crate::identity::{self, KeyState};
use client_core::{LanConfig, LanEvent, LanState, TrustStore, Visibility};
use ic_plugin_api::{IC_OK, IC_SETTING_PLAIN, IC_SETTING_SECRET};
use std::collections::HashMap;
use std::io;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tokio::sync::watch;

pub const DEVICES_KEY: &str = "lan_devices";
pub const SET_ASIDE_KEY: &str = "lan_devices_bad";
pub const TRUSTED_CONNECT_KEY: &str = "lan_trusted_connect";
pub const BROWSE_KEY: &str = "lan_browse";
pub const SHARE_TERMINAL_KEY: &str = "share_terminal";
pub const CLOSED_ON_US: &str = "the device closed the connection";

#[cfg(not(target_os = "android"))]
const KIND: &str = "desktop";
#[cfg(target_os = "android")]
const KIND: &str = "phone";

// Never stored: every run starts hidden, whatever the last one was.
static VISIBLE: AtomicBool = AtomicBool::new(false);
static TRUSTED_CONNECT: AtomicBool = AtomicBool::new(true);
static BROWSE: AtomicBool = AtomicBool::new(true);

pub fn visibility_of(visible: bool, trusted_connect: bool) -> Visibility {
    match (visible, trusted_connect) {
        (true, _) => Visibility::Open,
        (false, true) => Visibility::TrustedOnly,
        (false, false) => Visibility::Off,
    }
}

pub fn multicast_wanted(lan_running: bool, browse: bool, visibility: Visibility) -> bool {
    lan_running && (browse || visibility != Visibility::Off)
}

pub fn set_visible(on: bool) {
    VISIBLE.store(on, Ordering::SeqCst);
}

pub fn visible() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

pub fn visibility() -> Visibility {
    visibility_of(visible(), TRUSTED_CONNECT.load(Ordering::SeqCst))
}

pub fn browsing() -> bool {
    BROWSE.load(Ordering::SeqCst)
}

pub fn set_browsing(on: bool) {
    BROWSE.store(on, Ordering::SeqCst);
}

pub fn trusted_connect() -> bool {
    TRUSTED_CONNECT.load(Ordering::SeqCst)
}

pub fn set_trusted_connect(on: bool) {
    TRUSTED_CONNECT.store(on, Ordering::SeqCst);
}

pub fn shares_terminal() -> bool {
    !cfg!(target_os = "android") && crate::read_setting(SHARE_TERMINAL_KEY) == "1"
}

fn switched_on(key: &str) -> bool {
    crate::read_setting(key) != "0"
}

pub fn launch() -> Option<LanConfig> {
    TRUSTED_CONNECT.store(switched_on(TRUSTED_CONNECT_KEY), Ordering::SeqCst);
    BROWSE.store(switched_on(BROWSE_KEY), Ordering::SeqCst);
    if identity::key_state()? == KeyState::Locked {
        return None;
    }
    Some(LanConfig {
        name: identity::device_name(),
        os: std::env::consts::OS.to_string(),
        kind: KIND.to_string(),
        visibility: visibility(),
        browse: browsing(),
        port: p2p_node::lan::net::PORT,
        resources: identity::lan_resources(),
        trust: Arc::new(SettingsTrustStore::new()),
        test_mode: false,
    })
}

pub struct SettingsTrustStore {
    generation: u64,
    // Read on the caller's thread, so a starting network asks the host for nothing.
    read: Mutex<Option<String>>,
}

impl SettingsTrustStore {
    pub fn new() -> SettingsTrustStore {
        SettingsTrustStore {
            generation: crate::net::generation(),
            read: Mutex::new(Some(crate::read_setting(DEVICES_KEY))),
        }
    }

    fn still_current(&self) -> io::Result<()> {
        if self.generation == crate::net::generation() {
            return Ok(());
        }
        Err(io::Error::other("the network was restarted; its successor keeps the book"))
    }
}

impl Default for SettingsTrustStore {
    fn default() -> Self {
        SettingsTrustStore::new()
    }
}

fn kept(code: c_int) -> io::Result<()> {
    if code == IC_OK {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "the application refused to keep it (code {code})"
    )))
}

impl TrustStore for SettingsTrustStore {
    fn load(&self) -> io::Result<Option<String>> {
        let doc = match self
            .read
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            Some(doc) => doc,
            None => crate::read_setting(DEVICES_KEY),
        };
        Ok((!doc.is_empty()).then_some(doc))
    }

    fn save(&self, doc: &str) -> io::Result<()> {
        self.still_current()?;
        kept(crate::write_setting(
            DEVICES_KEY,
            Some(doc),
            IC_SETTING_SECRET,
        ))
    }

    fn set_aside(&self) -> io::Result<()> {
        self.still_current()?;
        let doc = crate::read_setting(DEVICES_KEY);
        kept(crate::write_setting(
            SET_ASIDE_KEY,
            Some(&doc),
            IC_SETTING_SECRET,
        ))?;
        kept(crate::write_setting(DEVICES_KEY, None, IC_SETTING_PLAIN))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Heard {
    pub state: Option<LanState>,
    pub problem: String,
}

fn heard_so_far() -> MutexGuard<'static, Heard> {
    static HELD: OnceLock<Mutex<Heard>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Heard::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn snapshot() -> Heard {
    heard_so_far().clone()
}

pub fn heard(event: LanEvent) {
    match event {
        LanEvent::State(state) => {
            heard_so_far().state = Some(state);
            crate::net::refresh_terminals();
            crate::notify::mark(crate::notify::SETTINGS | crate::notify::DRIVES);
        }
        LanEvent::Failed(problem) | LanEvent::NotSaved(problem) => {
            crate::note(&problem);
            heard_so_far().problem = problem;
            crate::notify::mark(crate::notify::SETTINGS);
        }
        LanEvent::PairWaiting { id, code } => crate::pairing::comparing(id, code),
        LanEvent::PairDone { id, outcome } => crate::pairing::done(id, outcome),
        LanEvent::PromptClosed { request } => crate::pairing::prompt_closed(&request),
        LanEvent::OpenFailed { id, reason } => could_not_open(id, reason),
        _ => {}
    }
}

pub fn forget() {
    *heard_so_far() = Heard::default();
}

#[derive(Default)]
struct OpenFailures {
    count: u64,
    last: HashMap<String, (u64, String)>,
}

fn open_failures_so_far() -> MutexGuard<'static, OpenFailures> {
    static HELD: OnceLock<Mutex<OpenFailures>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(OpenFailures::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_failures_told() -> &'static watch::Sender<u64> {
    static HELD: OnceLock<watch::Sender<u64>> = OnceLock::new();
    HELD.get_or_init(|| watch::channel(0).0)
}

pub fn open_failures() -> u64 {
    open_failures_so_far().count
}

fn could_not_open(id: String, reason: String) {
    let reason = if reason == CLOSED_ON_US {
        crate::i18n::tr("nodeinnet.lan_forgotten")
    } else {
        reason
    };
    crate::note(&format!("could not open {id}: {reason}"));
    let count = {
        let mut held = open_failures_so_far();
        held.count += 1;
        let count = held.count;
        held.last.insert(id, (count, reason));
        count
    };
    open_failures_told().send_replace(count);
}

fn failed_after(id: &str, counted: u64) -> Option<String> {
    open_failures_so_far()
        .last
        .get(id)
        .filter(|(count, _)| *count > counted)
        .map(|(_, reason)| reason.clone())
}

pub async fn open_failure_after(id: &str, counted: u64) -> String {
    let mut told = open_failures_told().subscribe();
    loop {
        if let Some(reason) = failed_after(id, counted) {
            return reason;
        }
        if told.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

fn sent() -> MutexGuard<'static, Option<bool>> {
    static HELD: OnceLock<Mutex<Option<bool>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn multicast() {
    let wanted = multicast_wanted(crate::net::lan_running(), browsing(), visibility());
    let mut sent = sent();
    match *sent {
        Some(held) if held == wanted => {}
        None if !wanted => *sent = Some(false),
        _ => {
            crate::platform_call("multicast", if wanted { "1" } else { "0" });
            *sent = Some(wanted);
        }
    }
}
