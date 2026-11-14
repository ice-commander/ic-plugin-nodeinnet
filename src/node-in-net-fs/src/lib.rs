use client_core::{Grants, LanCmd, NetCmd, PairDecision};
use ic_plugin_api::{
    check_host, needs_platform_call, HostCheck, IcBytes, IcFsVTable, IcHost, IcViewClosedFn,
    IcViewDescribeFn, IcViewEventFn, IcViewVTable, IC_ABI_VERSION, IC_ERR_HOST_TOO_OLD,
    IC_ERR_HOST_UNKNOWN, IC_ERR_INIT_FAILED, IC_OK, IC_SETTING_PLAIN, IC_SETTING_SECRET,
    IC_SIDE_RIGHT,
};
use serde_json::Value;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

pub mod account;
pub mod auth;
pub mod directory;
pub mod drives;
pub mod error;
pub mod i18n;
pub mod identity;
pub mod kind;
pub mod lan;
pub mod mount;
pub mod net;
pub mod notify;
pub mod p2p_rpc;
pub mod p2p_shell;
pub mod pairing;
pub mod pending;
pub mod pinned;
pub mod shares;
pub mod sources;
pub mod view;

use account::{account_from, Phase, Session};
use directory::{Directory, Failure};
use identity::KeyState;
use view::{Ask, KIND_ID, VIEW_ID};

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../version.rs"));

ic_plugin_api::declare_about!(
    "ic-node-in-net-fs",
    "Node In Net",
    plugin_version!(),
    "Browses a peer over the Node In Net protocol"
);

pub const ID: &str = "ic-node-in-net-fs";

pub fn plugin_version() -> &'static str {
    plugin_version!()
}
pub const HEADER_ID: &str = "nodeinnet.peers";
pub const VISIBLE_ID: &str = "nodeinnet.visible";
pub(crate) const CONNECTED: &str = include_str!("../assets/connect.svg");
pub(crate) const DISCONNECTED: &str = include_str!("../assets/disconnect.svg");
const EYE: &str = include_str!("../assets/eye.svg");
const EYE_OFF: &str = include_str!("../assets/eye-off.svg");
const LOGIN_PICTURE: &[u8] = include_bytes!("../assets/login.svg");

static HOST: AtomicUsize = AtomicUsize::new(0);
static GTK: AtomicBool = AtomicBool::new(false);
static RENEWING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ANSWER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn session() -> &'static Mutex<Session> {
    static HELD: OnceLock<Mutex<Session>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Session::signed_out()))
}

pub fn snapshot() -> Session {
    match session().lock() {
        Ok(held) => held.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn replace(next: Session) {
    match session().lock() {
        Ok(mut held) => *held = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

fn directory() -> &'static Mutex<Box<dyn Directory>> {
    static HELD: OnceLock<Mutex<Box<dyn Directory>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Box::new(directory::Net::new())))
}

/// Tests only.
pub fn set_directory(other: Box<dyn Directory>) {
    match directory().lock() {
        Ok(mut held) => *held = other,
        Err(poisoned) => *poisoned.into_inner() = other,
    }
}

pub(crate) fn note(said: &str) {
    let host = host();
    if host.is_null() {
        return;
    }
    if let Ok(line) = CString::new(format!("node-in-net: {said}")) {
        unsafe { ((*host).log_warn)(line.as_ptr()) };
    }
}

fn host() -> *const IcHost {
    HOST.load(Ordering::Relaxed) as *const IcHost
}

pub fn draws_with_gtk() -> bool {
    GTK.load(Ordering::Relaxed)
}

pub(crate) fn read_setting(key: &str) -> String {
    let host = host();
    if host.is_null() {
        return String::new();
    }
    let (Ok(id), Ok(key)) = (CString::new(ID), CString::new(key)) else {
        return String::new();
    };
    let bytes = unsafe { ((*host).settings_read)(id.as_ptr(), key.as_ptr()) };
    if bytes.data.is_null() || bytes.len == 0 {
        return String::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len as usize) };
    String::from_utf8_lossy(raw).to_string()
}

pub fn write_setting(key: &str, value: Option<&str>, flags: u32) -> c_int {
    let host = host();
    if host.is_null() {
        return IC_ERR_INIT_FAILED;
    }
    let (Ok(id), Ok(key)) = (CString::new(ID), CString::new(key)) else {
        return IC_ERR_INIT_FAILED;
    };
    match value {
        Some(text) => unsafe {
            ((*host).settings_write)(
                id.as_ptr(),
                key.as_ptr(),
                text.as_ptr(),
                text.len() as u64,
                flags,
            )
        },
        None => unsafe {
            ((*host).settings_write)(id.as_ptr(), key.as_ptr(), std::ptr::null(), 0, flags)
        },
    }
}

pub(crate) fn platform_call(name: &str, value: &str) -> c_int {
    let host = host();
    if host.is_null() {
        return IC_ERR_INIT_FAILED;
    }
    let (Ok(id), Ok(name), Ok(value)) = (CString::new(ID), CString::new(name), CString::new(value))
    else {
        return IC_ERR_INIT_FAILED;
    };
    unsafe { ((*host).platform_call)(id.as_ptr(), name.as_ptr(), value.as_ptr()) }
}

fn store(session: &Session) {
    let _ = write_setting(
        shares::SHARES_KEY,
        Some(&shares::encode(&session.shares)),
        IC_SETTING_PLAIN,
    );
    if session.has_account() {
        let _ = write_setting(
            account::TOKEN_KEY,
            Some(&session.account.token),
            IC_SETTING_SECRET,
        );
        let _ = write_setting(
            account::LOGIN_KEY,
            Some(&session.account.login),
            IC_SETTING_PLAIN,
        );
        let _ = write_setting(
            account::PREMIUM_KEY,
            Some(if session.account.premium { "1" } else { "0" }),
            IC_SETTING_PLAIN,
        );
    } else {
        for key in [account::TOKEN_KEY, account::LOGIN_KEY, account::PREMIUM_KEY] {
            let _ = write_setting(key, None, IC_SETTING_PLAIN);
        }
    }
}

pub fn stored_session() -> Session {
    let mut held = Session::resumed(
        &read_setting(account::TOKEN_KEY),
        &read_setting(account::LOGIN_KEY),
        read_setting(account::PREMIUM_KEY) == "1",
    );
    held.shares = shares::parse(&read_setting(shares::SHARES_KEY));
    held.device_id = identity::device_id();
    held
}

fn flag(on: bool) -> &'static str {
    if on {
        "1"
    } else {
        "0"
    }
}

fn shares_changed() {
    net::queue(NetCmd::ReloadResources(
        identity::announcing(&identity::device_id()).resources,
    ));
    if draws_with_gtk() {
        net::lan(LanCmd::SetResources(identity::lan_resources()));
    }
}

pub fn renewing() -> bool {
    RENEWING.load(Ordering::SeqCst)
}

fn renewal() -> &'static Mutex<Option<JoinHandle<()>>> {
    static HELD: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
}

fn renew_device_key() {
    let mut held = renewal().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if held.as_ref().is_some_and(|running| !running.is_finished()) {
        return;
    }
    if let Some(done) = held.take() {
        let _ = done.join();
    }
    RENEWING.store(true, Ordering::SeqCst);
    *held = std::thread::Builder::new()
        .name("nin-renew".to_string())
        .spawn(|| {
            net::restart_after(identity::forget_device_key);
            if identity::key_state() == Some(KeyState::Ephemeral) {
                identity::watch_keyring();
            }
            RENEWING.store(false, Ordering::SeqCst);
            notify::mark(notify::SETTINGS);
        })
        .ok();
    if held.is_none() {
        RENEWING.store(false, Ordering::SeqCst);
    }
}

fn on_this_device(ask: Ask) {
    match ask {
        Ask::SetDeviceName(name) => {
            let named = (!name.is_empty()).then_some(name.as_str());
            let _ = write_setting(identity::DEVICE_NAME_KEY, named, IC_SETTING_PLAIN);
            net::queue(NetCmd::UpdateName(identity::device_name()));
        }
        Ask::Browse(on) => {
            let _ = write_setting(lan::BROWSE_KEY, Some(flag(on)), IC_SETTING_PLAIN);
            lan::set_browsing(on);
            net::lan(LanCmd::Browse(on));
            notify::mark(notify::MULTICAST | notify::SETTINGS);
        }
        Ask::Visible(on) => {
            lan::set_visible(on);
            net::lan(LanCmd::SetVisibility(lan::visibility()));
            notify::mark(notify::HEADER | notify::MULTICAST | notify::SETTINGS);
        }
        Ask::TrustedConnect(on) => {
            let _ = write_setting(lan::TRUSTED_CONNECT_KEY, Some(flag(on)), IC_SETTING_PLAIN);
            lan::set_trusted_connect(on);
            net::lan(LanCmd::SetVisibility(lan::visibility()));
            notify::mark(notify::HEADER | notify::MULTICAST | notify::SETTINGS);
        }
        Ask::ShareTerminal(on) => {
            let _ = write_setting(lan::SHARE_TERMINAL_KEY, Some(flag(on)), IC_SETTING_PLAIN);
            net::lan(LanCmd::SetResources(identity::lan_resources()));
            notify::mark(notify::SETTINGS);
        }
        Ask::Grant { id, terminal } => net::lan(LanCmd::SetGrants(id, Grants { terminal })),
        Ask::Forget(id) => net::lan(LanCmd::Forget(id)),
        Ask::Unblock(id) => net::lan(LanCmd::Unblock(id)),
        Ask::ResumePairing => net::lan(LanCmd::ResumePairing),
        Ask::AllowWaiting(request) => {
            pairing::answer_waiting(&request, PairDecision::Allow { remember: true });
        }
        Ask::DenyWaiting(request) => {
            pairing::answer_waiting(&request, PairDecision::Deny);
        }
        Ask::NewDeviceKey => renew_device_key(),
        Ask::SharesNoted => {
            let _ = write_setting(shares::NOTED_KEY, Some("1"), IC_SETTING_PLAIN);
        }
        Ask::SignIn { .. }
        | Ask::Check
        | Ask::SignOut
        | Ask::AddShare { .. }
        | Ask::RemoveShare { .. } => {}
    }
}

/// Blocks: call it from a plugin thread, never from the host's loop.
pub fn carry_out(ask: Ask) {
    let held = snapshot();
    match &ask {
        Ask::AddShare { name, path } => {
            let added = shares::add(&held.shares, name, path);
            let changed = added.is_ok();
            let settled = match added {
                Ok(next) => Session {
                    shares: next,
                    problem: String::new(),
                    ..held
                },
                Err(why) => Session {
                    problem: why,
                    ..held
                },
            };
            store(&settled);
            replace(settled);
            if changed {
                shares_changed();
            }
            return;
        }
        Ask::RemoveShare { name } => {
            let left = shares::remove(&held.shares, name);
            let changed = left.len() != held.shares.len();
            let settled = Session {
                shares: left,
                problem: String::new(),
                ..held
            };
            store(&settled);
            replace(settled);
            if changed {
                shares_changed();
            }
            return;
        }
        Ask::SignIn { .. } | Ask::Check | Ask::SignOut => {}
        _ => {
            on_this_device(ask);
            return;
        }
    }
    let outcome = {
        let directory = match directory().lock() {
            Ok(held) => held,
            Err(poisoned) => poisoned.into_inner(),
        };
        match &ask {
            Ask::SignIn { login, password } => directory
                .sign_in(login, password)
                .and_then(|answer| account_from(&answer, login).map_err(Failure::Unanswered)),
            Ask::Check => directory.check(&held.account.token).and_then(|answer| {
                account_from(&answer, &held.account.login).map_err(Failure::Unanswered)
            }),
            Ask::SignOut => {
                let _ = directory.sign_out(&held.account.token);
                Err(Failure::Unanswered(String::new()))
            }
            _ => unreachable!("settled above"),
        }
    };

    // Shares belong to the device, not the account: they survive sign-out and a refused token.
    let kept = held.shares.clone();
    let settled = match (&ask, outcome) {
        (Ask::SignOut, _) => Session {
            shares: kept,
            ..Session::signed_out()
        },
        (_, Ok(account)) => Session {
            phase: Phase::SignedIn,
            account,
            shares: kept,
            device_id: identity::device_id(),
            problem: String::new(),
        },
        (Ask::Check, Err(failure)) if !refuses_the_token(&failure) => Session {
            phase: Phase::Offline,
            problem: in_words(&failure),
            ..held
        },
        (_, Err(failure)) => Session {
            problem: in_words(&failure),
            shares: kept,
            ..Session::signed_out()
        },
    };
    store(&settled);
    if settled.is_signed_in() {
        attach(&settled.account, &identity::device_id());
    } else {
        detach();
    }
    replace(settled);
    notify::mark(notify::ACCOUNT);
}

/// Only a 4xx is about the token; a server that is down says nothing either way.
fn refuses_the_token(failure: &Failure) -> bool {
    matches!(failure, Failure::Refused(400..=499, _))
}

fn in_words(failure: &Failure) -> String {
    match failure {
        Failure::Refused(401 | 403, _) => i18n::tr("nodeinnet.invalid_credentials"),
        Failure::Refused(400..=499, said) => said.clone(),
        Failure::Refused(_, said) | Failure::Unanswered(said) => {
            i18n::trf("nodeinnet.connection_error", &[("error", said)])
        }
    }
}

fn working(what: &'static str) {
    let mut held = snapshot();
    held.phase = Phase::Working(what);
    held.problem = String::new();
    replace(held);
}

fn attach(account: &account::Account, device_id: &str) {
    if account.ws_url.is_empty() || account.access_token.is_empty() {
        note("no socket: the directory gave neither a ws url nor an access token");
        return;
    }
    let url = net::socket_url(&account.ws_url, &account.access_token, device_id);
    note(&format!(
        "opening the signalling socket at {} as {device_id}",
        account.ws_url
    ));
    if !draws_with_gtk() {
        net::start(None);
    }
    net::connect(url, identity::announcing(device_id));
}

fn detach() {
    net::disconnect();
}

fn in_flight() -> &'static Mutex<Option<std::thread::JoinHandle<()>>> {
    static HELD: OnceLock<Mutex<Option<std::thread::JoinHandle<()>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
}

fn off_the_loop(ask: Ask) {
    if !ask.needs_the_network() {
        carry_out(ask);
        return;
    }
    working(match ask {
        Ask::SignIn { .. } => "signing in",
        Ask::Check => "checking",
        _ => "signing out",
    });
    notify::mark(notify::ACCOUNT);
    let started = std::thread::spawn(move || carry_out(ask));
    match in_flight().lock() {
        Ok(mut held) => *held = Some(started),
        Err(poisoned) => *poisoned.into_inner() = Some(started),
    }
}

/// Shutdown only: a request must not outlive the host that takes its answer.
pub fn settle() {
    let held = match in_flight().lock() {
        Ok(mut held) => held.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(handle) = held {
        let _ = handle.join();
    }
    let renewing = renewal()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(handle) = renewing {
        let _ = handle.join();
    }
}

fn answer_with(source: &str) -> IcBytes {
    ANSWER.with(|slot| {
        *slot.borrow_mut() = source.as_bytes().to_vec();
        let held = slot.borrow();
        IcBytes {
            data: held.as_ptr(),
            len: held.len() as u64,
        }
    })
}

extern "C" fn describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    identity::check_keyring_now();
    answer_with(&view::document(&snapshot()).to_string())
}

extern "C" fn kind_describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&kind::document().to_string())
}

extern "C" fn kind_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    if event.is_null() || len == 0 {
        return answer_with("{}");
    }
    let raw = unsafe { std::slice::from_raw_parts(event, len as usize) };
    let parsed: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    answer_with(&kind::reply_for(&parsed).to_string())
}

fn event_of(event: *const u8, len: u64) -> Option<Value> {
    if event.is_null() || len == 0 {
        return None;
    }
    let raw = unsafe { std::slice::from_raw_parts(event, len as usize) };
    Some(serde_json::from_slice(raw).unwrap_or(Value::Null))
}

fn answered(event: *const u8, len: u64) -> Value {
    let Some(parsed) = event_of(event, len) else {
        return serde_json::json!({});
    };
    let (reply, ask) = view::reply_for(&parsed, &snapshot());
    if let Some(ask) = ask {
        off_the_loop(ask);
    }
    reply
}

extern "C" fn on_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&answered(event, len).to_string())
}

extern "C" fn closed(_instance: u64, _user_data: *mut c_void) {
    view::closed();
}

extern "C" fn pair_describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&pairing::pair_document().to_string())
}

extern "C" fn pair_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    let reply = event_of(event, len).map_or_else(
        || serde_json::json!({}),
        |parsed| pairing::pair_reply(&parsed),
    );
    answer_with(&reply.to_string())
}

extern "C" fn pair_closed(_instance: u64, _user_data: *mut c_void) {
    pairing::pair_closed();
}

extern "C" fn pairing_describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&pairing::pairing_document().to_string())
}

extern "C" fn pairing_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    let reply = event_of(event, len).map_or_else(
        || serde_json::json!({}),
        |parsed| pairing::pairing_reply(&parsed),
    );
    answer_with(&reply.to_string())
}

extern "C" fn pairing_closed(_instance: u64, _user_data: *mut c_void) {
    pairing::pairing_closed();
}

fn eye() -> &'static str {
    if lan::visible() {
        EYE
    } else {
        EYE_OFF
    }
}

pub(crate) fn paint_eye() {
    let host = host();
    if host.is_null() {
        return;
    }
    if let (Ok(id), Ok(drawn)) = (CString::new(VISIBLE_ID), CString::new(eye())) {
        unsafe { ((*host).set_header_icon)(id.as_ptr(), drawn.as_ptr()) };
    }
}

// The header fires a copy of its entry outside every borrow, so repainting from here is safe.
extern "C" fn on_visible_clicked(_user_data: *mut c_void, _parent_window: *mut c_void) {
    lan::set_visible(!lan::visible());
    net::lan(LanCmd::SetVisibility(lan::visibility()));
    paint_eye();
    notify::mark(notify::MULTICAST | notify::SETTINGS);
}

fn register_window(
    host: *const IcHost,
    id: &str,
    title: &str,
    (describe, on_event, closed): (IcViewDescribeFn, IcViewEventFn, IcViewClosedFn),
) -> c_int {
    let (Ok(id), Ok(title)) = (CString::new(id), CString::new(title)) else {
        return IC_ERR_INIT_FAILED;
    };
    let table = IcViewVTable {
        struct_size: std::mem::size_of::<IcViewVTable>() as u32,
        describe,
        on_event: Some(on_event),
        closed: Some(closed),
    };
    unsafe { ((*host).register_view)(id.as_ptr(), title.as_ptr(), &table, std::ptr::null_mut()) }
}

fn register_local_network(host: *const IcHost) -> c_int {
    let pair = register_window(
        host,
        pairing::PAIR_VIEW,
        "nodeinnet.pair_title",
        (pair_describe, pair_event, pair_closed),
    );
    if pair != IC_OK {
        return pair;
    }
    let pairing = register_window(
        host,
        pairing::PAIRING_VIEW,
        "nodeinnet.pairing_title",
        (pairing_describe, pairing_event, pairing_closed),
    );
    if pairing != IC_OK {
        return pairing;
    }
    let (Ok(button), Ok(svg), Ok(label), Ok(tooltip)) = (
        CString::new(VISIBLE_ID),
        CString::new(eye()),
        CString::new(""),
        CString::new(i18n::tr("nodeinnet.visible_tooltip")),
    ) else {
        return IC_ERR_INIT_FAILED;
    };
    unsafe {
        ((*host).add_header_button)(
            button.as_ptr(),
            svg.as_ptr(),
            label.as_ptr(),
            tooltip.as_ptr(),
            IC_SIDE_RIGHT,
            11,
            on_visible_clicked,
            std::ptr::null_mut(),
        )
    }
}

extern "C" fn on_clicked(_user_data: *mut c_void, _parent_window: *mut c_void) {
    let host = host();
    if host.is_null() {
        return;
    }
    let Ok(id) = CString::new(VIEW_ID) else {
        return;
    };
    unsafe {
        ((*host).open_view)(id.as_ptr(), std::ptr::null(), 0);
    }
}

#[cfg_attr(feature = "export-abi", no_mangle)]
pub extern "C" fn ic_plugin_init(host: *const IcHost, kind: *const c_char) -> c_int {
    match check_host(host, IC_ABI_VERSION, needs_platform_call()) {
        HostCheck::Ok => {}
        HostCheck::WrongMagic => return IC_ERR_HOST_UNKNOWN,
        HostCheck::TooOld { .. } | HostCheck::Truncated { .. } => return IC_ERR_HOST_TOO_OLD,
    }
    HOST.store(host as usize, Ordering::Relaxed);
    let kind = if kind.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(kind) }.to_string_lossy().to_string()
    };
    GTK.store(ic_plugin_api::draws_with_gtk(&kind), Ordering::Relaxed);

    let spoken = unsafe { ((*host).language)() };
    if !spoken.is_null() {
        i18n::speaks(&unsafe { CStr::from_ptr(spoken) }.to_string_lossy());
    }
    for (language, catalogue) in i18n::LOCALES {
        let Ok(tag) = CString::new(*language) else {
            continue;
        };
        unsafe {
            ((*host).register_locales)(tag.as_ptr(), catalogue.as_ptr(), catalogue.len() as u64)
        };
    }

    if let (Ok(owner), Ok(name)) = (CString::new(ID), CString::new(view::LOGIN_ICON)) {
        unsafe {
            ((*host).register_plugin_asset)(
                owner.as_ptr(),
                name.as_ptr(),
                LOGIN_PICTURE.as_ptr(),
                LOGIN_PICTURE.len() as u64,
            )
        };
    }

    let (Ok(view), Ok(button), Ok(svg), Ok(title), Ok(label)) = (
        CString::new(VIEW_ID),
        CString::new(HEADER_ID),
        // The stored session is read only below, so disconnected is true here.
        CString::new(DISCONNECTED),
        CString::new("Node In Net"),
        CString::new(""),
    ) else {
        return IC_ERR_INIT_FAILED;
    };

    let table = IcViewVTable {
        struct_size: std::mem::size_of::<IcViewVTable>() as u32,
        describe,
        on_event: Some(on_event),
        closed: Some(closed),
    };
    let registered = unsafe {
        ((*host).register_view)(view.as_ptr(), title.as_ptr(), &table, std::ptr::null_mut())
    };
    if registered != IC_OK {
        return registered;
    }

    let added = unsafe {
        ((*host).add_header_button)(
            button.as_ptr(),
            svg.as_ptr(),
            label.as_ptr(),
            title.as_ptr(),
            IC_SIDE_RIGHT,
            10,
            on_clicked,
            std::ptr::null_mut(),
        )
    };
    if added != IC_OK {
        return added;
    }

    if draws_with_gtk() {
        let registered = register_local_network(host);
        if registered != IC_OK {
            return registered;
        }
    }

    let Ok(kind_id) = CString::new(KIND_ID) else {
        return IC_ERR_INIT_FAILED;
    };
    // The host copies `connection`, but the filesystem table it points to must outlive the call.
    let filesystem: &'static IcFsVTable = Box::leak(Box::new(mount::filesystem()));
    let mut connection = mount::connection(filesystem);
    connection.describe = Some(kind_describe);
    connection.on_event = Some(kind_event);
    let registered = kind::document().to_string();
    let declared = unsafe {
        ((*host).register_connection_kind)(
            kind_id.as_ptr(),
            registered.as_ptr(),
            registered.len() as u64,
            &connection,
            std::ptr::null_mut(),
        )
    };
    if declared != IC_OK {
        return declared;
    }

    if draws_with_gtk() {
        let offered = unsafe {
            ((*host).register_drive_source)(
                kind_id.as_ptr(),
                c"".as_ptr(),
                drives::rows,
                std::ptr::null_mut(),
            )
        };
        if offered != IC_OK {
            return offered;
        }
    }

    let Ok(pinned_id) = CString::new(pinned::SOURCE_ID) else {
        return IC_ERR_INIT_FAILED;
    };
    let pinned = unsafe {
        ((*host).register_pinned_connections)(
            pinned_id.as_ptr(),
            pinned::rows,
            std::ptr::null_mut(),
        )
    };
    if pinned != IC_OK {
        return pinned;
    }

    net::unhalt();
    notify::start();
    if draws_with_gtk() {
        net::start(lan::launch());
        lan::multicast();
        if matches!(
            identity::key_state(),
            Some(KeyState::Locked | KeyState::Ephemeral)
        ) {
            identity::watch_keyring();
        }
    }

    let stored = stored_session();
    let resume = stored.has_account();
    replace(stored);
    if resume {
        off_the_loop(Ask::Check);
    }
    IC_OK
}

#[cfg_attr(feature = "export-abi", no_mangle)]
pub extern "C" fn ic_plugin_shutdown() {
    net::halt();
    identity::stop_watching();
    settle();
    net::stop(net::STOP_BUDGET);
    net::stop_control();
    lan::multicast();
    notify::stop();
    replace(Session::signed_out());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_says_what_it_is_before_it_is_initialised() {
        let name = unsafe { std::ffi::CStr::from_ptr(ic_plugin_name()) }
            .to_string_lossy()
            .to_string();
        assert_eq!(name, "Node In Net");
        assert!(!ic_plugin_about().is_null());
    }

    #[test]
    fn it_refuses_a_host_it_does_not_recognise() {
        assert_eq!(
            ic_plugin_init(std::ptr::null(), c"gtk".as_ptr()),
            IC_ERR_HOST_UNKNOWN
        );
    }
}
