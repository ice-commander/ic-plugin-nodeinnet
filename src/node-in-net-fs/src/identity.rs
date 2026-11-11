use nodeinnet_protocol::NodeInfo;
use std::sync::OnceLock;

pub const DEVICE_KEY: &str = "device_id";
pub const DEVICE_NAME_KEY: &str = "device_name";
pub const PRIVATE_KEY: &str = "private_key";
pub const PUBLIC_KEY: &str = "public_key";
pub const KEYRING_PROBE_KEY: &str = "keyring_probe";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Stored,
    Ephemeral,
    Locked,
}

struct Keypair {
    state: KeyState,
    private: String,
    public: String,
}

fn keypair() -> Option<&'static Keypair> {
    static HELD: OnceLock<Keypair> = OnceLock::new();
    if crate::host().is_null() {
        return None;
    }
    Some(HELD.get_or_init(resolved))
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
    named(&crate::read_setting(DEVICE_NAME_KEY))
        .or_else(platform_name)
        .unwrap_or_else(|| "Ice Commander".to_string())
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

pub fn announcing(device_id: &str) -> NodeInfo {
    NodeInfo {
        id: device_id.to_string(),
        name: device_name(),
        os: std::env::consts::OS.to_string(),
        version: crate::plugin_version().to_string(),
        app_type: "gui".to_string(),
        build_type: "plugin".to_string(),
        public_key: public_key(),
        resources: Vec::new(),
        is_online: true,
        last_used: 0,
        is_temporary: false,
    }
}
