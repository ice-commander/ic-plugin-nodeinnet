use nodeinnet_protocol::{NodeInfo, ResourceType, SharedResource};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEVICE_KEY: &str = "device_id";
pub const DEVICE_NAME_KEY: &str = "device_name";
pub const PRIVATE_KEY: &str = "private_key";
pub const PUBLIC_KEY: &str = "public_key";
pub const KEYRING_PROBE_KEY: &str = "keyring_probe";

const QUICKLY: Duration = Duration::from_secs(2);
const QUICKLY_FOR: Duration = Duration::from_secs(2 * 60);
const SLOWLY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Stored,
    Ephemeral,
    Locked,
}

#[derive(Clone)]
struct Keypair {
    state: KeyState,
    private: String,
    public: String,
}

fn held() -> MutexGuard<'static, Option<Keypair>> {
    static HELD: OnceLock<Mutex<Option<Keypair>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn keypair() -> Option<Keypair> {
    if crate::host().is_null() {
        return None;
    }
    Some(held().get_or_insert_with(resolved).clone())
}

fn resolved() -> Keypair {
    let mut private = crate::read_setting(PRIVATE_KEY);
    let public = crate::read_setting(PUBLIC_KEY);
    if private.is_empty() && !public.is_empty() {
        if !keyring_open() {
            let (private, public) = nodeinnet_protocol::crypto::generate_ed25519_keypair();
            return Keypair {
                state: KeyState::Locked,
                private,
                public,
            };
        }
        private = crate::read_setting(PRIVATE_KEY);
    }
    if pairs(&private, &public) {
        return Keypair {
            state: KeyState::Stored,
            private,
            public,
        };
    }
    minted()
}

pub fn resolve_again() -> bool {
    if crate::host().is_null() {
        return false;
    }
    let mut held = held();
    let next = match held.as_ref().map(|current| current.state) {
        None | Some(KeyState::Stored) => return false,
        Some(KeyState::Locked) => {
            if crate::read_setting(PRIVATE_KEY).is_empty() {
                return false;
            }
            resolved()
        }
        Some(KeyState::Ephemeral) => {
            let private = crate::read_setting(PRIVATE_KEY);
            let public = crate::read_setting(PUBLIC_KEY);
            if pairs(&private, &public) {
                Keypair {
                    state: KeyState::Stored,
                    private,
                    public,
                }
            } else if keyring_open() {
                minted()
            } else {
                return false;
            }
        }
    };
    if next.state != KeyState::Stored {
        return false;
    }
    *held = Some(next);
    true
}

fn keyring_open() -> bool {
    let open = crate::write_setting(
        KEYRING_PROBE_KEY,
        Some("1"),
        ic_plugin_api::IC_SETTING_SECRET,
    ) == ic_plugin_api::IC_OK;
    if open {
        let _ = crate::write_setting(KEYRING_PROBE_KEY, None, ic_plugin_api::IC_SETTING_PLAIN);
    }
    open
}

fn pairs(private: &str, public: &str) -> bool {
    let probe = b"node-in-net key pair";
    nodeinnet_protocol::crypto::sign_message(private, probe).is_ok_and(|signed| {
        nodeinnet_protocol::crypto::verify_message(&signed, public, probe).is_ok()
    })
}

fn minted() -> Keypair {
    let (private, public) = nodeinnet_protocol::crypto::generate_ed25519_keypair();
    let state = if crate::write_setting(
        PRIVATE_KEY,
        Some(&private),
        ic_plugin_api::IC_SETTING_SECRET,
    ) == ic_plugin_api::IC_OK
        && crate::write_setting(PUBLIC_KEY, Some(&public), ic_plugin_api::IC_SETTING_PLAIN)
            == ic_plugin_api::IC_OK
    {
        KeyState::Stored
    } else {
        KeyState::Ephemeral
    };
    Keypair {
        state,
        private,
        public,
    }
}

pub fn key_state() -> Option<KeyState> {
    keypair().map(|held| held.state)
}

pub fn private_key() -> String {
    keypair()
        .map(|held| held.private.clone())
        .unwrap_or_default()
}

pub fn public_key() -> String {
    keypair()
        .map(|held| held.public.clone())
        .unwrap_or_default()
}

pub fn pair() -> Option<(String, String)> {
    keypair().map(|held| (held.private, held.public))
}

pub fn forget_device_key() {
    for key in [PRIVATE_KEY, PUBLIC_KEY, crate::lan::DEVICES_KEY] {
        let _ = crate::write_setting(key, None, ic_plugin_api::IC_SETTING_PLAIN);
    }
    *held() = None;
}

#[derive(Default)]
struct Watch {
    stop: bool,
    now: bool,
}

fn watch() -> &'static (Mutex<Watch>, Condvar) {
    static HELD: OnceLock<(Mutex<Watch>, Condvar)> = OnceLock::new();
    HELD.get_or_init(|| (Mutex::new(Watch::default()), Condvar::new()))
}

fn watching() -> MutexGuard<'static, Watch> {
    watch()
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn watcher() -> MutexGuard<'static, Option<JoinHandle<()>>> {
    static HELD: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn watch_keyring() {
    let mut running = watcher();
    if running.as_ref().is_some_and(|held| !held.is_finished()) {
        return;
    }
    *watching() = Watch::default();
    *running = std::thread::Builder::new()
        .name("nin-keyring".to_string())
        .spawn(keep_watching)
        .ok();
}

pub fn check_keyring_now() {
    watching().now = true;
    watch().1.notify_one();
}

pub fn stop_watching() {
    let running = watcher().take();
    watching().stop = true;
    watch().1.notify_one();
    if let Some(running) = running {
        let _ = running.join();
    }
}

fn keep_watching() {
    let started = Instant::now();
    loop {
        let pause = if started.elapsed() < QUICKLY_FOR {
            QUICKLY
        } else {
            SLOWLY
        };
        let until = Instant::now() + pause;
        {
            let mut held = watching();
            while !held.stop && !held.now && Instant::now() < until {
                let left = until.saturating_duration_since(Instant::now());
                held = watch()
                    .1
                    .wait_timeout(held, left)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
            if held.stop {
                return;
            }
            held.now = false;
        }
        if key_state() == Some(KeyState::Stored) {
            return;
        }
        if resolve_again() {
            crate::note("the device key can be read now: the network starts again with it");
            crate::net::restart();
            return;
        }
    }
}

pub fn device_id() -> String {
    let held = crate::read_setting(DEVICE_KEY);
    if !held.is_empty() {
        return held;
    }
    let minted = minted_id();
    let _ = crate::write_setting(DEVICE_KEY, Some(&minted), ic_plugin_api::IC_SETTING_PLAIN);
    minted
}

fn minted_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|held| held.as_nanos())
        .unwrap_or_default();
    format!("{now:032x}")
}

pub fn device_name() -> String {
    named(&crate::read_setting(DEVICE_NAME_KEY)).unwrap_or_else(default_device_name)
}

pub fn default_device_name() -> String {
    platform_name().unwrap_or_else(|| "Ice Commander".to_string())
}

fn named(held: &str) -> Option<String> {
    let held = held.trim();
    (!held.is_empty()).then(|| held.to_string())
}

fn host_name() -> Option<String> {
    hostname::get()
        .ok()
        .and_then(|held| named(&held.to_string_lossy()))
}

#[cfg(not(target_os = "android"))]
fn platform_name() -> Option<String> {
    host_name()
}

#[cfg(target_os = "android")]
fn platform_name() -> Option<String> {
    product_model().or_else(host_name)
}

#[cfg(target_os = "android")]
fn product_model() -> Option<String> {
    let mut value = [0 as libc::c_char; libc::PROP_VALUE_MAX as usize];
    let len =
        unsafe { libc::__system_property_get(c"ro.product.model".as_ptr(), value.as_mut_ptr()) };
    let bytes: Vec<u8> = value
        .iter()
        .take(len.max(0) as usize)
        .map(|&held| held as u8)
        .collect();
    named(&String::from_utf8_lossy(&bytes))
}

fn shared_folders(device_id: &str) -> Option<SharedResource> {
    let shares = crate::shares::parse(&crate::read_setting(crate::shares::SHARES_KEY));
    if shares.is_empty() {
        return None;
    }
    Some(SharedResource {
        id: format!("fs-{device_id}"),
        name: "Files".to_string(),
        resource_type: ResourceType::Filesystem,
        config: Some(serde_json::json!({ "shares": crate::shares::listed(&shares) }).to_string()),
        is_active: true,
        session_token: None,
    })
}

pub fn announcing(device_id: &str) -> NodeInfo {
    NodeInfo {
        id: device_id.to_string(),
        name: device_name(),
        os: std::env::consts::OS.to_string(),
        version: crate::plugin_version().to_string(),
        app_type: "gui".to_string(),
        build_type: "plugin".to_string(),
        public_key: public_key(),
        resources: shared_folders(device_id).into_iter().collect(),
        is_online: true,
        last_used: 0,
        is_temporary: false,
    }
}

pub fn lan_resources() -> Vec<SharedResource> {
    let device_id = device_id();
    let mut resources: Vec<SharedResource> = shared_folders(&device_id).into_iter().collect();
    if crate::lan::shares_terminal() {
        resources.push(SharedResource {
            id: format!("terminal-{device_id}"),
            name: "Shell".to_string(),
            resource_type: ResourceType::Terminal,
            config: None,
            is_active: true,
            session_token: None,
        });
    }
    resources
}
