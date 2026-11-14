use crate::account::{Phase, Session};
use crate::i18n::{tr, trf};
use crate::identity::KeyState;
use crate::shares;
use crate::sources::Online;
use client_core::LanDevice;
use ic_plugin_api::{IC_EVENT_ACTIVATE, IC_EVENT_OPENED};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};

pub const VIEW_ID: &str = "nodeinnet.account";
pub const KIND_ID: &str = "node-in-net";
pub const LOGIN_ICON: &str = "nodeinnet-login.svg";
const UNSHARE: &str = "unshare:";
const GRANT: &str = "grant:";
const FORGET_TRUSTED: &str = "forget_trusted:";
const FORGET_PAIRED: &str = "forget_paired:";
const UNBLOCK: &str = "unblock:";
const ALLOW_WAITING: &str = "allow_waiting:";
const DENY_WAITING: &str = "deny_waiting:";
const BROWSE: &str = "lan_browse";
const VISIBLE: &str = "lan_visible";
const TRUSTED_CONNECT: &str = "lan_trusted_connect";
const SHARE_TERMINAL: &str = "share_terminal";
const DEVICE_NAME: &str = "device_name";
const SAVE_NAME: &str = "save_device_name";
const RESUME: &str = "resume_pairing";
const NEW_KEY: &str = "new_device_key";

static SHOWING: AtomicBool = AtomicBool::new(false);
static NOTE_KEPT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    SignIn { login: String, password: String },
    Check,
    SignOut,
    AddShare { name: String, path: String },
    RemoveShare { name: String },
    SetDeviceName(String),
    Browse(bool),
    Visible(bool),
    TrustedConnect(bool),
    ShareTerminal(bool),
    Grant { id: String, terminal: bool },
    Forget(String),
    Unblock(String),
    AllowWaiting(String),
    DenyWaiting(String),
    ResumePairing,
    NewDeviceKey,
    SharesNoted,
}

impl Ask {
    pub fn needs_the_network(&self) -> bool {
        matches!(self, Ask::SignIn { .. } | Ask::Check | Ask::SignOut)
    }
}

pub fn showing() -> bool {
    SHOWING.load(Ordering::SeqCst)
}

pub fn closed() {
    SHOWING.store(false, Ordering::SeqCst);
    NOTE_KEPT.store(false, Ordering::SeqCst);
}

fn said(key: &str, en: &str) -> Value {
    json!({ "tr": key, "en": en })
}

fn from_data(key: &str) -> String {
    format!("{{data.{key}}}")
}

fn emitting(id: &str, label: Value, role: Option<&str>) -> Value {
    let mut button = json!({ "t": "button", "id": id, "label": label,
                             "intent": { "do": "emit", "node": id } });
    if let Some(role) = role {
        button["role"] = json!(role);
    }
    button
}

fn heading(text: Value) -> Value {
    json!({ "t": "text", "role": "heading", "wrap": true, "text": text })
}

fn dim(text: Value) -> Value {
    json!({ "t": "text", "role": "dim", "wrap": true, "text": text })
}

fn picture() -> Value {
    json!({ "t": "icon", "icon": format!("asset:{}/{LOGIN_ICON}", crate::ID), "height": 64 })
}

fn signed_out() -> Value {
    json!({
        "t": "column",
        "spacing": 12,
        "children": [
            { "t": "row", "spacing": 16, "children": [
                picture(),
                { "t": "column", "spacing": 4, "weight": 1, "children": [
                    { "t": "text", "role": "title1",
                      "text": { "tr": "nodeinnet.sign_in_title", "en": "Sign in to Node In Net" } },
                    { "t": "text", "role": "dim", "wrap": true,
                      "text": { "tr": "nodeinnet.sign_in_prompt",
                                "en": "Please sign in to connect to the P2P network." } }
                ]}
            ]},
            { "t": "input", "id": "login", "bind": "login",
              "placeholder": { "tr": "nodeinnet.login", "en": "Login" } },
            { "t": "input", "id": "password", "bind": "password", "variant": "masked_reveal",
              "placeholder": { "tr": "nodeinnet.password", "en": "Password" } },
            problem(),
            { "t": "text", "id": "working", "role": "dim",
              "text": { "tr": "nodeinnet.connecting", "en": "Connecting to NodeInNet network..." },
              "visible": { "truthy": "data.busy" } },
            { "t": "button", "id": "sign_in", "role": "primary",
              "label": { "tr": "nodeinnet.sign_in_button", "en": "Secure Login" },
              "sensitive": { "all": [
                  { "not": { "empty": "state.login" } },
                  { "not": { "truthy": "data.busy" } } ] },
              "intent": { "do": "emit", "node": "sign_in" } }
        ]
    })
}

fn signed_in() -> Value {
    json!({
        "t": "column",
        "spacing": 12,
        "children": [
            { "t": "row", "spacing": 16, "children": [
                picture(),
                { "t": "column", "spacing": 4, "weight": 1, "children": [
                    { "t": "text", "id": "standing", "role": "dim", "text": "{data.standing}" },
                    { "t": "row", "spacing": 12, "children": [
                        { "t": "text", "id": "who", "role": "title2", "text": "{data.login}" },
                        { "t": "text", "id": "premium", "role": "heading",
                          "text": { "tr": "nodeinnet.premium", "en": "Premium" },
                          "visible": { "truthy": "data.premium" } }
                    ]}
                ]},
                { "t": "button", "id": "sign_out", "role": "destructive",
                  "label": { "tr": "nodeinnet.log_out", "en": "Log Out" },
                  "sensitive": { "not": { "truthy": "data.busy" } },
                  "intent": { "do": "emit", "node": "sign_out" } }
            ]},
            { "t": "separator" },
            { "t": "text", "id": "status", "role": "heading", "wrap": true,
              "text": "{data.status}",
              "visible": { "not": { "empty": "data.status" } } },
            { "t": "text", "id": "status_help", "role": "dim", "wrap": true,
              "text": "{data.status_help}",
              "visible": { "not": { "empty": "data.status_help" } } },
            { "t": "table", "id": "devices", "rows_key": "devices",
              "visible": { "not": { "empty": "data.devices" } },
              "columns": [ { "key": "name", "width": 200 }, { "key": "shares" } ] },
            { "t": "button", "id": "reconnect",
              "label": { "tr": "nodeinnet.connect_again", "en": "Connect again" },
              "visible": { "truthy": "data.offline" },
              "sensitive": { "not": { "truthy": "data.busy" } },
              "intent": { "do": "emit", "node": "reconnect" } },
            problem()
        ]
    })
}

fn shares_noted() -> bool {
    !crate::read_setting(shares::NOTED_KEY).is_empty()
}

fn shared_folders(session: &Session) -> Value {
    let mut children = vec![json!({ "t": "text", "role": "heading",
        "text": { "tr": "nodeinnet.shares", "en": "Shared Folders (P2P)" } })];
    let noting = !shares_noted() || NOTE_KEPT.load(Ordering::SeqCst);
    if noting && !session.shares.is_empty() {
        children.push(json!({ "t": "text", "id": "shares_noted", "wrap": true,
            "text": said("nodeinnet.shares_noted",
                         "These folders are now shared with your devices") }));
    }
    if session.shares.is_empty() {
        children.push(json!({ "t": "text", "id": "no_shares", "role": "dim",
            "text": { "tr": "nodeinnet.no_folders_shared", "en": "No folders shared" } }));
    }
    for share in &session.shares {
        let id = format!("{UNSHARE}{}", share.name);
        children.push(json!({ "t": "row", "spacing": 8, "children": [
            { "t": "column", "weight": 1, "children": [
                { "t": "text", "text": share.name },
                { "t": "text", "role": "dim", "text": share.path }
            ]},
            { "t": "button", "id": id, "role": "flat",
              "label": { "tr": "nodeinnet.stop_sharing", "en": "Stop sharing folder" },
              "intent": { "do": "emit", "node": id } }
        ]}));
    }
    children.push(json!({ "t": "row", "spacing": 8, "children": [
        { "t": "input", "id": "share_path", "bind": "share_path", "weight": 1,
          "variant": "path",
          "picker": { "mode": "folder",
                      "title": { "tr": "nodeinnet.select_folder_to_share",
                                 "en": "Select Folder to Share" } },
          "placeholder": { "tr": "nodeinnet.select_folder_to_share",
                           "en": "Select Folder to Share" } },
        { "t": "input", "id": "share_name", "bind": "share_name", "width": 160,
          "placeholder": { "tr": "nodeinnet.resource_name_placeholder",
                           "en": "e.g. MySharedFiles" } },
        { "t": "button", "id": "add_share", "role": "primary",
          "label": { "tr": "nodeinnet.share", "en": "Share" },
          "sensitive": { "not": { "empty": "state.share_path" } },
          "intent": { "do": "emit", "node": "add_share" } }
    ]}));
    json!({ "t": "column", "id": "shares", "spacing": 8, "children": children })
}

fn problem() -> Value {
    json!({
        "t": "text", "id": "problem", "role": "error", "wrap": true,
        "text": "{data.problem}",
        "visible": { "not": { "empty": "data.problem" } }
    })
}

fn device_rows(devices: &[Online]) -> Value {
    Value::Array(
        devices
            .iter()
            .map(|device| json!({ "name": device.name, "shares": device.shares.join(", ") }))
            .collect(),
    )
}

fn standing(session: &Session) -> String {
    match session.phase {
        Phase::SignedIn => tr("nodeinnet.authorized"),
        Phase::Offline => tr("nodeinnet.disconnected"),
        _ => tr("nodeinnet.connecting"),
    }
}

fn link_status(session: &Session, seen: &crate::net::Seen, devices: usize) -> (String, String) {
    if !session.is_signed_in() {
        return (String::new(), String::new());
    }
    if !seen.problem.is_empty() {
        let said = trf("nodeinnet.connection_error", &[("error", &seen.problem)]);
        return (said, String::new());
    }
    if !seen.connected {
        return (tr("nodeinnet.connecting"), String::new());
    }
    if devices == 0 {
        return (tr("nodeinnet.no_devices"), tr("nodeinnet.no_devices_help"));
    }
    let count = devices.to_string();
    (
        trf("nodeinnet.connected_devices", &[("count", &count)]),
        String::new(),
    )
}

fn body(session: &Session) -> Value {
    if session.has_account() {
        signed_in()
    } else {
        signed_out()
    }
}

fn fields(session: &Session) -> Value {
    let mut fields = vec![
        json!({ "bind": "share_path", "type": "text" }),
        json!({ "bind": "share_name", "type": "text" }),
    ];
    if !session.has_account() {
        fields.push(json!({ "bind": "login", "type": "text", "required": true }));
        fields.push(json!({ "bind": "password", "type": "text" }));
    }
    if crate::draws_with_gtk() {
        fields.push(json!({ "bind": DEVICE_NAME, "type": "text",
                            "default": crate::identity::device_name() }));
    }
    Value::Array(fields)
}

fn data(session: &Session) -> Map<String, Value> {
    let seen = crate::net::snapshot();
    let devices = crate::sources::devices_online(&session.device_id, &seen.nodes);
    let (status, status_help) = link_status(session, &seen, devices.len());
    let held = json!({
        "login": session.account.login,
        "premium": session.account.premium,
        "problem": session.problem,
        "busy": session.is_busy(),
        "offline": session.phase == Phase::Offline,
        "standing": standing(session),
        "status": status,
        "status_help": status_help,
        "devices": if session.is_signed_in() { device_rows(&devices) } else { json!([]) }
    });
    match held {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

fn switched(data: &mut Map<String, Value>, id: &str, on: bool) {
    let (state, action) = if on {
        ("nodeinnet.on", "nodeinnet.turn_off")
    } else {
        ("nodeinnet.off", "nodeinnet.turn_on")
    };
    data.insert(format!("{id}_state"), json!(tr(state)));
    data.insert(format!("{id}_action"), json!(tr(action)));
}

fn toggle(id: &str, title: Value) -> Value {
    json!({ "t": "row", "spacing": 8, "children": [
        { "t": "column", "weight": 1, "spacing": 2, "children": [
            { "t": "text", "wrap": true, "text": title },
            { "t": "text", "id": format!("{id}_state"), "role": "dim",
              "text": from_data(&format!("{id}_state")) }
        ]},
        emitting(id, json!(from_data(&format!("{id}_action"))), None)
    ]})
}

fn device_row(
    list: &str,
    device: &LanDevice,
    data: &mut Map<String, Value>,
    extra: Vec<Value>,
    buttons: Vec<Value>,
) -> Value {
    let name = format!("{list}_name:{}", device.id);
    let fingerprint = format!("{list}_id:{}", device.id);
    data.insert(name.clone(), json!(device.name));
    data.insert(fingerprint.clone(), json!(device.fingerprint));
    let mut about = vec![
        json!({ "t": "text", "wrap": true, "text": from_data(&name) }),
        json!({ "t": "text", "role": "dim", "text": from_data(&fingerprint) }),
    ];
    about.extend(extra);
    let mut children = vec![json!({ "t": "column", "weight": 1, "spacing": 2, "children": about })];
    children.extend(buttons);
    json!({ "t": "row", "spacing": 8, "children": children })
}

fn trusted_row(device: &LanDevice, terminal: bool, data: &mut Map<String, Value>) -> Value {
    let forget = format!("{FORGET_TRUSTED}{}", device.id);
    let mut buttons = Vec::new();
    let mut extra = Vec::new();
    if terminal {
        let grant = format!("{GRANT}{}", device.id);
        let granted = device.may_open_me.is_some_and(|grants| grants.terminal);
        let (state, action) = if granted {
            ("nodeinnet.terminal_allowed", "nodeinnet.revoke_terminal")
        } else {
            ("nodeinnet.terminal_not_allowed", "nodeinnet.allow_terminal")
        };
        let state_key = format!("{grant}_state");
        let action_key = format!("{grant}_action");
        data.insert(state_key.clone(), json!(tr(state)));
        data.insert(action_key.clone(), json!(tr(action)));
        extra.push(json!({ "t": "text", "role": "dim", "text": from_data(&state_key) }));
        buttons.push(emitting(&grant, json!(from_data(&action_key)), None));
    }
    buttons.push(emitting(
        &forget,
        said("nodeinnet.forget", "Forget"),
        Some("flat"),
    ));
    device_row("trusted", device, data, extra, buttons)
}

fn waiting_row(row: &crate::pairing::WaitingRow, data: &mut Map<String, Value>) -> Value {
    let who = format!("waiting_name:{}", row.request);
    let code = format!("waiting_code:{}", row.request);
    let ready = format!("allow_ready:{}", row.request);
    data.insert(
        who.clone(),
        json!(trf(
            "nodeinnet.waiting_who",
            &[("name", &row.name), ("os", &row.os)]
        )),
    );
    data.insert(
        code.clone(),
        json!(trf(
            "nodeinnet.waiting_code",
            &[("code", &row.code), ("fingerprint", &row.fingerprint)]
        )),
    );
    data.insert(ready.clone(), json!(row.allow_ready));
    let mut about = vec![
        json!({ "t": "text", "wrap": true, "text": from_data(&who) }),
        json!({ "t": "text", "role": "dim", "wrap": true, "text": from_data(&code) }),
    ];
    if row.known {
        about.push(json!({ "t": "text", "wrap": true,
            "text": said("nodeinnet.pair_known", "This device is already in your list") }));
    }
    if row.name_clash {
        about.push(json!({ "t": "text", "role": "error", "wrap": true,
            "text": said("nodeinnet.pair_name_clash",
                         "Another device on this network has the same name") }));
    }
    if row.quiet {
        about.push(dim(said(
            "nodeinnet.waiting_quiet",
            "Shown only here: many requests came at once",
        )));
    }
    let deny = format!("{DENY_WAITING}{}", row.request);
    let allow = format!("{ALLOW_WAITING}{}", row.request);
    let mut allowing = emitting(&allow, said("nodeinnet.allow", "Allow"), Some("primary"));
    allowing["sensitive"] = json!({ "truthy": format!("data.{ready}") });
    json!({ "t": "row", "spacing": 8, "children": [
        { "t": "column", "weight": 1, "spacing": 2, "children": about },
        emitting(&deny, said("nodeinnet.pair_deny", "Deny"), None),
        allowing
    ]})
}

fn key_state_part(key: Option<KeyState>) -> Vec<Value> {
    match key {
        Some(KeyState::Ephemeral) => vec![json!({ "t": "text", "id": "key_ephemeral",
            "role": "error", "wrap": true,
            "text": said("nodeinnet.key_ephemeral",
                         "There is no keyring to keep this device's key, so pairings last only until Ice Commander closes") })],
        Some(KeyState::Locked) => vec![
            json!({ "t": "text", "id": "key_locked", "role": "error", "wrap": true,
                    "text": said("nodeinnet.key_locked",
                                 "This device's key is in a keyring that is locked or not available. Unlock it to use the local network, or create a new device key.") }),
            json!({ "t": "row", "spacing": 8, "children": [
                { "t": "text", "role": "dim", "wrap": true, "weight": 1,
                  "text": said("nodeinnet.new_device_key_note",
                               "Devices paired with this one will have to pair again") },
                emitting(NEW_KEY, said("nodeinnet.new_device_key", "Create a new device key"),
                         Some("destructive"))
            ]}),
        ],
        _ => Vec::new(),
    }
}

fn local_network(data: &mut Map<String, Value>) -> Value {
    if !crate::draws_with_gtk() {
        return json!({ "t": "column", "id": "local_network", "children": [
            json!({ "t": "text", "id": "lan_elsewhere", "role": "dim", "wrap": true,
                    "text": said("nodeinnet.lan_elsewhere",
                                 "Devices on your local network are paired in the desktop and Android apps") })
        ]});
    }
    let heard = crate::lan::snapshot();
    let state = heard.state;
    let terminal = crate::lan::shares_terminal();
    switched(data, BROWSE, crate::lan::browsing());
    switched(data, VISIBLE, crate::lan::visible());
    switched(data, TRUSTED_CONNECT, crate::lan::trusted_connect());
    switched(data, SHARE_TERMINAL, terminal);
    let problem = if heard.problem.is_empty() {
        String::new()
    } else {
        trf("nodeinnet.lan_problem", &[("problem", &heard.problem)])
    };
    data.insert("lan_problem".to_string(), json!(problem));
    data.insert(
        "default_device_name".to_string(),
        json!(crate::identity::default_device_name()),
    );
    data.insert(
        "lan_restarting".to_string(),
        json!(crate::net::restarting() || crate::renewing()),
    );
    data.insert(
        "lan_paused".to_string(),
        json!(state.as_ref().is_some_and(|state| state.paused)),
    );

    let mut children = vec![
        heading(said("nodeinnet.lan_heading", "Local network")),
        dim(said(
            "nodeinnet.lan_note",
            "Without an account you find devices only on your local network",
        )),
    ];
    children.extend(key_state_part(crate::identity::key_state()));
    children.push(json!({ "t": "text", "id": "lan_restarting", "role": "dim", "wrap": true,
        "text": said("nodeinnet.lan_restarting", "Starting the local network again…"),
        "visible": { "truthy": "data.lan_restarting" } }));
    children.push(json!({ "t": "text", "id": "lan_problem", "role": "error", "wrap": true,
        "text": "{data.lan_problem}",
        "visible": { "not": { "empty": "data.lan_problem" } } }));
    children.push(json!({ "t": "text", "wrap": true,
        "text": said("nodeinnet.device_name", "Device name") }));
    children.push(json!({ "t": "row", "spacing": 8, "children": [
        { "t": "input", "id": DEVICE_NAME, "bind": DEVICE_NAME, "weight": 1,
          "placeholder": "{data.default_device_name}" },
        emitting(SAVE_NAME, said("nodeinnet.save", "Save"), None)
    ]}));
    children.push(toggle(
        BROWSE,
        said("nodeinnet.lan_browse", "Find devices on this network"),
    ));
    children.push(toggle(
        VISIBLE,
        said("nodeinnet.lan_visible", "Let devices find and connect to me"),
    ));
    children.push(toggle(
        TRUSTED_CONNECT,
        said("nodeinnet.lan_trusted_connect", "Let my trusted devices connect"),
    ));

    let waiting = crate::pairing::waiting_rows();
    if !waiting.is_empty() {
        children.push(heading(said("nodeinnet.waiting_heading", "Waiting to connect")));
        for row in &waiting {
            children.push(waiting_row(row, data));
        }
    }
    children.push(json!({ "t": "row", "spacing": 8,
        "visible": { "truthy": "data.lan_paused" },
        "children": [
            { "t": "text", "role": "dim", "wrap": true, "weight": 1,
              "text": said("nodeinnet.lan_paused",
                           "New devices are refused for a while after several declined requests") },
            emitting(RESUME, said("nodeinnet.resume_pairing", "Allow new devices now"), None)
        ]}));

    if !cfg!(target_os = "android") {
        children.push(toggle(
            SHARE_TERMINAL,
            said("nodeinnet.share_terminal", "Share terminal"),
        ));
        children.push(dim(said(
            "nodeinnet.share_terminal_note",
            "Gives a full shell to the devices you allow below; never to account devices",
        )));
    }

    let devices = state.as_ref().map_or(&[][..], |state| &state.devices[..]);
    let blocked = state.as_ref().map_or(&[][..], |state| &state.blocked[..]);
    let trusted: Vec<&LanDevice> = devices
        .iter()
        .filter(|device| device.may_open_me.is_some())
        .collect();
    let paired: Vec<&LanDevice> = devices
        .iter()
        .filter(|device| device.opened_from_here)
        .collect();
    if !trusted.is_empty() {
        children.push(heading(said("nodeinnet.trusted_heading", "Can open this device")));
        for device in trusted.iter().copied() {
            children.push(trusted_row(device, terminal, data));
        }
    }
    if !paired.is_empty() {
        children.push(heading(said("nodeinnet.paired_heading", "Opened from here")));
        for device in paired.iter().copied() {
            let forget = format!("{FORGET_PAIRED}{}", device.id);
            let button = emitting(&forget, said("nodeinnet.forget", "Forget"), Some("flat"));
            children.push(device_row("paired", device, data, Vec::new(), vec![button]));
        }
    }
    if !blocked.is_empty() {
        children.push(heading(said("nodeinnet.blocked_heading", "Blocked")));
        for device in blocked {
            let unblock = format!("{UNBLOCK}{}", device.id);
            let button = emitting(&unblock, said("nodeinnet.unblock", "Unblock"), Some("flat"));
            children.push(device_row("blocked", device, data, Vec::new(), vec![button]));
        }
    }
    if trusted.is_empty() && paired.is_empty() && blocked.is_empty() {
        children.push(json!({ "t": "text", "id": "no_lan_devices", "role": "dim", "wrap": true,
            "text": said("nodeinnet.no_lan_devices", "No devices paired yet") }));
    }
    json!({ "t": "column", "id": "local_network", "spacing": 8, "children": children })
}

pub fn document(session: &Session) -> Value {
    let mut data = data(session);
    let local = local_network(&mut data);
    json!({
        "schema": 1,
        "kind": VIEW_ID,
        "data": data,
        "fields": fields(session),
        "form": {
            "t": "view",
            "surface": "dialog",
            "width": 560,
            "height": 640,
            "padding": 18,
            "spacing": 8,
            "scroll": "vertical",
            "children": [
                local,
                { "t": "separator" },
                shared_folders(session),
                { "t": "separator" },
                heading(said("nodeinnet.account_heading", "Account (optional)")),
                body(session)
            ]
        }
    })
}

fn known(id: &str, wanted: impl Fn(&LanDevice) -> bool, blocked: bool) -> bool {
    crate::lan::snapshot().state.is_some_and(|state| {
        let list = if blocked {
            &state.blocked
        } else {
            &state.devices
        };
        list.iter().any(|device| device.id == id && wanted(device))
    })
}

fn local_reply(node: &str, event: &Value) -> Option<(Value, Option<Ask>)> {
    if !crate::draws_with_gtk() {
        return None;
    }
    let asked = match node {
        SAVE_NAME => {
            let typed = event["values"][DEVICE_NAME].as_str()?.trim().to_string();
            let shown = if typed.is_empty() {
                crate::identity::default_device_name()
            } else {
                typed.clone()
            };
            return Some((
                json!({ "put": { DEVICE_NAME: shown } }),
                Some(Ask::SetDeviceName(typed)),
            ));
        }
        BROWSE => Ask::Browse(!crate::lan::browsing()),
        VISIBLE => Ask::Visible(!crate::lan::visible()),
        TRUSTED_CONNECT => Ask::TrustedConnect(!crate::lan::trusted_connect()),
        SHARE_TERMINAL if !cfg!(target_os = "android") => {
            Ask::ShareTerminal(!crate::lan::shares_terminal())
        }
        RESUME => Ask::ResumePairing,
        NEW_KEY if crate::identity::key_state() == Some(KeyState::Locked) => Ask::NewDeviceKey,
        _ => {
            if let Some(id) = node.strip_prefix(GRANT) {
                if !crate::lan::shares_terminal() {
                    return None;
                }
                let granted = crate::lan::snapshot().state.and_then(|state| {
                    state
                        .devices
                        .into_iter()
                        .find(|device| device.id == id)
                        .and_then(|device| device.may_open_me)
                })?;
                Ask::Grant {
                    id: id.to_string(),
                    terminal: !granted.terminal,
                }
            } else if let Some(id) = node
                .strip_prefix(FORGET_TRUSTED)
                .or_else(|| node.strip_prefix(FORGET_PAIRED))
            {
                let listed = |device: &LanDevice| {
                    device.may_open_me.is_some() || device.opened_from_here
                };
                if !known(id, listed, false) {
                    return None;
                }
                Ask::Forget(id.to_string())
            } else if let Some(id) = node.strip_prefix(UNBLOCK) {
                if !known(id, |_| true, true) {
                    return None;
                }
                Ask::Unblock(id.to_string())
            } else if let Some(request) = node.strip_prefix(ALLOW_WAITING) {
                Ask::AllowWaiting(request.to_string())
            } else {
                Ask::DenyWaiting(node.strip_prefix(DENY_WAITING)?.to_string())
            }
        }
    };
    Some((json!({ "redescribe": true }), Some(asked)))
}

pub fn reply_for(event: &Value, session: &Session) -> (Value, Option<Ask>) {
    let kind = event["type"].as_str().unwrap_or_default();
    let node = event["node"].as_str().unwrap_or_default();
    if kind == IC_EVENT_OPENED {
        SHOWING.store(true, Ordering::SeqCst);
        if shares_noted() {
            return (json!({}), None);
        }
        NOTE_KEPT.store(true, Ordering::SeqCst);
        return (json!({}), Some(Ask::SharesNoted));
    }
    if kind != IC_EVENT_ACTIVATE {
        return (json!({}), None);
    }
    let working = json!({ "set": { "data.problem": "", "data.busy": true } });
    match node {
        "sign_in" if !session.has_account() && !session.is_busy() => {
            let login = event["values"]["login"].as_str().unwrap_or_default();
            let password = event["values"]["password"].as_str().unwrap_or_default();
            if login.trim().is_empty() {
                return (json!({}), None);
            }
            (
                working,
                Some(Ask::SignIn {
                    login: login.trim().to_string(),
                    password: password.to_string(),
                }),
            )
        }
        "reconnect" if session.phase == Phase::Offline => (working, Some(Ask::Check)),
        "sign_out" if session.has_account() && !session.is_busy() => (working, Some(Ask::SignOut)),
        "add_share" => {
            let path = event["values"]["share_path"].as_str().unwrap_or_default();
            let name = event["values"]["share_name"].as_str().unwrap_or_default();
            match shares::add(&session.shares, name, path) {
                Ok(_) => (
                    json!({ "put": { "share_path": "", "share_name": "" }, "redescribe": true }),
                    Some(Ask::AddShare {
                        name: name.to_string(),
                        path: path.to_string(),
                    }),
                ),
                Err(why) => (json!({ "set": { "data.problem": why } }), None),
            }
        }
        _ if node.starts_with(UNSHARE) => {
            let name = &node[UNSHARE.len()..];
            if !session.shares.iter().any(|share| share.name == name) {
                return (json!({}), None);
            }
            (
                json!({ "redescribe": true }),
                Some(Ask::RemoveShare {
                    name: name.to_string(),
                }),
            )
        }
        _ => local_reply(node, event).unwrap_or_else(|| (json!({}), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(node: &str, values: Value) -> Value {
        json!({ "type": "activate", "node": node, "values": values })
    }

    fn signed_in_session() -> Session {
        Session::resumed("r", "alice", true)
    }

    fn in_phase(phase: Phase) -> Session {
        Session {
            phase,
            ..signed_in_session()
        }
    }

    fn sharing(paths: &[&str]) -> Session {
        let mut session = signed_in_session();
        for path in paths {
            session.shares = shares::add(&session.shares, "", path).expect("added");
        }
        session
    }

    fn node_ids(document: &Value) -> Vec<String> {
        fn walk(value: &Value, found: &mut Vec<String>) {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                found.push(id.to_string());
            }
            match value {
                Value::Object(fields) => fields.values().for_each(|held| walk(held, found)),
                Value::Array(items) => items.iter().for_each(|held| walk(held, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        walk(document.get("form").unwrap_or(document), &mut found);
        found
    }

    #[test]
    fn an_offline_account_offers_to_connect_again_and_nothing_else_does() {
        let offline = document(&in_phase(Phase::Offline));
        assert_eq!(offline["data"]["offline"], json!(true));
        assert_eq!(
            offline["data"]["standing"],
            json!(tr("nodeinnet.disconnected"))
        );
        assert_eq!(
            reply_for(&pressed("reconnect", json!({})), &in_phase(Phase::Offline)).1,
            Some(Ask::Check)
        );
        for session in [signed_in_session(), Session::signed_out()] {
            assert_eq!(document(&session)["data"]["offline"], json!(false));
            assert_eq!(
                reply_for(&pressed("reconnect", json!({})), &session).1,
                None
            );
        }
    }

    #[test]
    fn the_status_line_speaks_about_the_other_devices() {
        let signed = signed_in_session();
        let seen = |connected: bool, problem: &str| crate::net::Seen {
            connected,
            problem: problem.to_string(),
            nodes: Vec::new(),
        };
        assert_eq!(
            link_status(&signed, &seen(false, ""), 0),
            (tr("nodeinnet.connecting"), String::new())
        );
        assert_eq!(
            link_status(&signed, &seen(true, ""), 0),
            (tr("nodeinnet.no_devices"), tr("nodeinnet.no_devices_help"))
        );
        assert_eq!(
            link_status(&signed, &seen(true, ""), 3).0,
            trf("nodeinnet.connected_devices", &[("count", "3")])
        );
        assert_eq!(
            link_status(&signed, &seen(false, "socket closed"), 0).0,
            trf("nodeinnet.connection_error", &[("error", "socket closed")])
        );
        assert_eq!(
            link_status(&in_phase(Phase::Offline), &seen(true, ""), 3),
            (String::new(), String::new()),
            "no link while offline, whatever was last seen"
        );
    }

    #[test]
    fn with_nothing_shared_the_page_says_so() {
        let ids = node_ids(&document(&signed_in_session()));
        assert!(ids.contains(&"no_shares".to_string()));
        assert!(!ids.iter().any(|id| id.starts_with(UNSHARE)));
        assert!(ids.contains(&"add_share".to_string()));
    }

    #[test]
    fn a_folder_can_be_offered() {
        let (answer, ask) = reply_for(
            &pressed(
                "add_share",
                json!({ "share_path": "/home/user/docs", "share_name": "" }),
            ),
            &signed_in_session(),
        );
        assert_eq!(
            ask,
            Some(Ask::AddShare {
                name: String::new(),
                path: "/home/user/docs".to_string()
            })
        );
        assert_eq!(answer["redescribe"], json!(true));
        assert_eq!(
            answer["put"]["share_path"],
            json!(""),
            "the field is emptied"
        );
        assert!(!ask.as_ref().unwrap().needs_the_network());
    }

    #[test]
    fn a_folder_already_shared_is_refused_in_the_dialog_rather_than_asked_for() {
        let session = sharing(&["/home/user/docs"]);
        let (answer, ask) = reply_for(
            &pressed(
                "add_share",
                json!({ "share_path": "/home/user/docs", "share_name": "" }),
            ),
            &session,
        );
        assert_eq!(ask, None);
        assert_eq!(
            answer["set"]["data.problem"],
            json!(trf("nodeinnet.name_taken", &[("name", "docs")]))
        );
    }

    #[test]
    fn the_devices_listed_are_the_ones_online_one_row_each() {
        let rows = device_rows(&[
            Online {
                id: "a".to_string(),
                name: "laptop".to_string(),
                shares: vec!["docs".to_string(), "music".to_string()],
            },
            Online {
                id: "b".to_string(),
                name: "phone".to_string(),
                shares: Vec::new(),
            },
        ]);
        assert_eq!(rows[0]["name"], json!("laptop"));
        assert_eq!(rows[0]["shares"], json!("docs, music"));
        assert_eq!(rows[1]["shares"], json!(""));
        assert_eq!(
            document(&in_phase(Phase::Offline))["data"]["devices"],
            json!([]),
            "nobody is listed without a link"
        );
    }

    #[test]
    fn signing_in_asks_for_the_network_and_says_it_is_working() {
        let (answer, ask) = reply_for(
            &pressed(
                "sign_in",
                json!({ "login": " alice ", "password": "hunter2" }),
            ),
            &Session::signed_out(),
        );
        assert_eq!(answer["set"]["data.busy"], json!(true));
        assert_eq!(
            ask,
            Some(Ask::SignIn {
                login: "alice".to_string(),
                password: "hunter2".to_string()
            })
        );
    }

    #[test]
    fn signing_in_without_a_login_asks_for_nothing() {
        let (_, ask) = reply_for(
            &pressed("sign_in", json!({ "login": "  ", "password": "hunter2" })),
            &Session::signed_out(),
        );
        assert_eq!(ask, None);
    }

    #[test]
    fn a_second_sign_in_while_one_is_in_flight_is_ignored() {
        let busy = Session {
            phase: Phase::Working("signing in"),
            ..Session::default()
        };
        let (_, ask) = reply_for(
            &pressed("sign_in", json!({ "login": "alice", "password": "x" })),
            &busy,
        );
        assert_eq!(ask, None);
    }

    #[test]
    fn signing_out_is_only_offered_to_an_account_and_not_twice() {
        assert_eq!(
            reply_for(&pressed("sign_out", json!({})), &Session::signed_out()).1,
            None
        );
        for phase in [Phase::SignedIn, Phase::Offline] {
            assert_eq!(
                reply_for(&pressed("sign_out", json!({})), &in_phase(phase)).1,
                Some(Ask::SignOut)
            );
        }
        assert_eq!(
            reply_for(
                &pressed("sign_out", json!({})),
                &in_phase(Phase::Working("signing out"))
            )
            .1,
            None
        );
    }

    #[test]
    fn an_event_the_dialog_does_not_know_changes_nothing() {
        assert_eq!(
            reply_for(&pressed("nosuchbutton", json!({})), &signed_in_session()),
            (json!({}), None)
        );
        assert_eq!(
            reply_for(
                &json!({ "type": "change", "node": "login" }),
                &Session::signed_out()
            ),
            (json!({}), None)
        );
    }
}
