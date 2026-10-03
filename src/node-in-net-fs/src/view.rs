use crate::account::{Phase, Session};
use crate::i18n::{tr, trf};
use crate::shares;
use crate::sources::Online;
use serde_json::{json, Value};

pub const VIEW_ID: &str = "nodeinnet.account";
pub const KIND_ID: &str = "node-in-net";
pub const LOGIN_ICON: &str = "nodeinnet-login.svg";
const UNSHARE: &str = "unshare:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    SignIn { login: String, password: String },
    Check,
    SignOut,
    AddShare { name: String, path: String },
    RemoveShare { name: String },
}

impl Ask {
    pub fn needs_the_network(&self) -> bool {
        matches!(self, Ask::SignIn { .. } | Ask::Check | Ask::SignOut)
    }
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

fn signed_in(session: &Session) -> Value {
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
            problem(),
            { "t": "separator" },
            shared_folders(session)
        ]
    })
}

fn shared_folders(session: &Session) -> Value {
    let mut children = vec![json!({ "t": "text", "role": "heading",
        "text": { "tr": "nodeinnet.shares", "en": "Shared Folders (P2P)" } })];
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
        signed_in(session)
    } else {
        signed_out()
    }
}

fn fields(session: &Session) -> Value {
    if session.has_account() {
        json!([
            { "bind": "share_path", "type": "text" },
            { "bind": "share_name", "type": "text" }
        ])
    } else {
        json!([
            { "bind": "login", "type": "text", "required": true },
            { "bind": "password", "type": "text" }
        ])
    }
}

fn data(session: &Session) -> Value {
    let seen = crate::net::snapshot();
    let devices = crate::sources::devices_online(&session.device_id, &seen.nodes);
    let (status, status_help) = link_status(session, &seen, devices.len());
    json!({
        "login": session.account.login,
        "premium": session.account.premium,
        "problem": session.problem,
        "busy": session.is_busy(),
        "offline": session.phase == Phase::Offline,
        "standing": standing(session),
        "status": status,
        "status_help": status_help,
        "devices": if session.is_signed_in() { device_rows(&devices) } else { json!([]) }
    })
}

pub fn document(session: &Session) -> Value {
    json!({
        "schema": 1,
        "kind": VIEW_ID,
        "data": data(session),
        "fields": fields(session),
        "form": {
            "t": "view",
            "surface": "dialog",
            "width": 560,
            "height": 520,
            "padding": 18,
            "spacing": 8,
            "scroll": "vertical",
            "children": [body(session)]
        }
    })
}

pub fn reply_for(event: &Value, session: &Session) -> (Value, Option<Ask>) {
    let kind = event["type"].as_str().unwrap_or_default();
    let node = event["node"].as_str().unwrap_or_default();
    if kind != "activate" {
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
        "add_share" if session.has_account() => {
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
        _ if session.has_account() && node.starts_with(UNSHARE) => {
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
        _ => (json!({}), None),
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
