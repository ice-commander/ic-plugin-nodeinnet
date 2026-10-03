use nodeinnet_protocol::NodeInfo;

pub const DEVICE_KEY: &str = "device_id";
pub const DEVICE_NAME_KEY: &str = "device_name";
pub const PRIVATE_KEY: &str = "private_key";
pub const PUBLIC_KEY: &str = "public_key";

fn keypair() -> (String, String) {
    let private = crate::read_setting(PRIVATE_KEY);
    let public = crate::read_setting(PUBLIC_KEY);
    if !private.is_empty() && !public.is_empty() {
        return (private, public);
    }
    let (private, public) = nodeinnet_protocol::crypto::generate_ed25519_keypair();
    crate::write_setting(
        PRIVATE_KEY,
        Some(&private),
        ic_plugin_api::IC_SETTING_SECRET,
    );
    crate::write_setting(PUBLIC_KEY, Some(&public), ic_plugin_api::IC_SETTING_PLAIN);
    (private, public)
}

pub fn private_key() -> String {
    keypair().0
}

pub fn public_key() -> String {
    keypair().1
}

pub fn device_id() -> String {
    let held = crate::read_setting(DEVICE_KEY);
    if !held.is_empty() {
        return held;
    }
    let minted = minted_id();
    crate::write_setting(DEVICE_KEY, Some(&minted), ic_plugin_api::IC_SETTING_PLAIN);
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
    let held = crate::read_setting(DEVICE_NAME_KEY);
    if !held.is_empty() {
        return held;
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|held| !held.trim().is_empty())
        .unwrap_or_else(|| "Ice Commander".to_string())
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
