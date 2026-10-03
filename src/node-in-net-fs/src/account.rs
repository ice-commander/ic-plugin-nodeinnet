use serde_json::Value;

pub const TOKEN_KEY: &str = "token";
pub const LOGIN_KEY: &str = "login";
pub const PREMIUM_KEY: &str = "premium";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub token: String,
    pub login: String,
    pub premium: bool,
    pub devices: Vec<Device>,
    // Never stored: every sign-in and token check hands both out afresh.
    pub access_token: String,
    pub ws_url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    SignedOut,
    Working(&'static str),
    SignedIn,
    /// The token is kept: an unreachable directory says nothing about whether it is good.
    Offline,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub phase: Phase,
    pub account: Account,
    pub device_id: String,
    pub shares: Vec<crate::shares::Share>,
    pub problem: String,
}

impl Session {
    pub fn signed_out() -> Self {
        Session::default()
    }

    pub fn resumed(token: &str, login: &str, premium: bool) -> Self {
        if token.is_empty() {
            return Session::signed_out();
        }
        Session {
            phase: Phase::SignedIn,
            account: Account {
                token: token.to_string(),
                login: login.to_string(),
                premium,
                devices: Vec::new(),
                access_token: String::new(),
                ws_url: String::new(),
            },
            shares: Vec::new(),
            device_id: String::new(),
            problem: String::new(),
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.phase == Phase::SignedIn
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.phase, Phase::Working(_))
    }

    pub fn has_account(&self) -> bool {
        !self.account.token.is_empty()
    }
}

pub fn header_label(session: &Session, peers: usize) -> String {
    match &session.phase {
        Phase::SignedOut => String::new(),
        Phase::Working(_) => "…".to_string(),
        Phase::Offline => session.account.login.trim().to_string(),
        Phase::SignedIn => {
            let who = session.account.login.trim();
            match (who.is_empty(), peers) {
                (true, 0) => String::new(),
                (true, many) => many.to_string(),
                (false, 0) => who.to_string(),
                (false, many) => format!("{who} · {many}"),
            }
        }
    }
}

pub fn is_connected(session: &Session) -> bool {
    matches!(session.phase, Phase::SignedIn)
}

fn devices_from(answer: &Value) -> Vec<Device> {
    answer["devices"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|device| Device {
                    id: device["id"].as_str().unwrap_or_default().to_string(),
                    name: device["name"].as_str().unwrap_or_default().to_string(),
                })
                .filter(|device| !device.id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The login is the caller's: the reply never says who asked.
pub fn account_from(answer: &Value, login: &str) -> Result<Account, String> {
    let token = answer["refresh_token"].as_str().unwrap_or_default();
    if token.is_empty() {
        return Err("the answer carried no token".to_string());
    }
    Ok(Account {
        token: token.to_string(),
        login: login.to_string(),
        premium: answer["premium"].as_i64().unwrap_or(0) != 0,
        devices: devices_from(answer),
        access_token: answer["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        ws_url: answer["ws_url"].as_str().unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer() -> Value {
        json!({
            "access_token": "a",
            "refresh_token": "r",
            "ws_url": "wss://node.in.net/ws",
            "premium": 1,
            "devices": [
                { "id": "dev-1", "name": "laptop" },
                { "id": "dev-2", "name": "desktop" }
            ]
        })
    }

    #[test]
    fn a_reply_becomes_an_account_with_its_devices() {
        let account = account_from(&answer(), "alice").expect("it parses");
        assert_eq!(account.token, "r");
        assert_eq!(account.login, "alice");
        assert!(account.premium);
        assert_eq!(account.devices.len(), 2);
        assert_eq!(account.devices[0].name, "laptop");
    }

    #[test]
    fn the_reply_also_carries_what_the_signalling_connection_needs() {
        let account = account_from(&answer(), "alice").expect("it parses");
        assert_eq!(account.access_token, "a");
        assert_eq!(account.ws_url, "wss://node.in.net/ws");
    }

    #[test]
    fn a_reply_without_a_token_is_refused_rather_than_stored_empty() {
        let mut bad = answer();
        bad["refresh_token"] = json!("");
        assert!(account_from(&bad, "alice").is_err());
        assert!(account_from(&json!({}), "alice").is_err());
    }

    #[test]
    fn a_device_without_an_id_is_dropped() {
        let mut odd = answer();
        odd["devices"] = json!([{ "name": "ghost" }, { "id": "dev-3", "name": "real" }]);
        let account = account_from(&odd, "alice").expect("it parses");
        assert_eq!(account.devices.len(), 1);
        assert_eq!(account.devices[0].id, "dev-3");
    }

    #[test]
    fn a_missing_premium_flag_is_read_as_no() {
        let mut plain = answer();
        plain["premium"] = json!(0);
        assert!(!account_from(&plain, "alice").unwrap().premium);
        let bare = json!({ "refresh_token": "r" });
        assert!(!account_from(&bare, "alice").unwrap().premium);
    }

    #[test]
    fn a_stored_token_signs_us_in_before_the_network_is_asked() {
        let resumed = Session::resumed("r", "alice", true);
        assert!(resumed.is_signed_in());
        assert_eq!(resumed.account.login, "alice");
        assert!(
            resumed.account.devices.is_empty(),
            "those come from the directory"
        );
    }

    #[test]
    fn no_stored_token_means_signed_out_whatever_else_was_kept() {
        let resumed = Session::resumed("", "alice", true);
        assert!(!resumed.is_signed_in());
        assert_eq!(resumed, Session::signed_out());
    }

    #[test]
    fn the_header_says_who_is_signed_in_and_how_many_others_are_there() {
        assert_eq!(header_label(&Session::signed_out(), 3), "");

        let signed_in = Session::resumed("r", "alice", false);
        assert_eq!(header_label(&signed_in, 0), "alice");
        assert_eq!(header_label(&signed_in, 2), "alice · 2");

        let busy = Session {
            phase: Phase::Working("signing in"),
            ..Session::default()
        };
        assert_eq!(header_label(&busy, 5), "…");

        let offline = Session {
            phase: Phase::Offline,
            ..Session::resumed("r", "alice", false)
        };
        assert_eq!(
            header_label(&offline, 2),
            "alice",
            "whoever it is, nobody is reachable"
        );
    }

    #[test]
    fn an_account_with_no_name_falls_back_to_the_count() {
        let nameless = Session::resumed("r", "", false);
        let nameless = Session {
            phase: Phase::SignedIn,
            ..nameless
        };
        assert_eq!(header_label(&nameless, 0), "");
        assert_eq!(header_label(&nameless, 4), "4");
    }

    #[test]
    fn the_button_is_drawn_as_connected_only_once_signing_in_is_done() {
        assert!(!is_connected(&Session::signed_out()));
        assert!(is_connected(&Session::resumed("r", "alice", false)));

        let busy = Session {
            phase: Phase::Working("signing in"),
            ..Session::default()
        };
        assert!(!is_connected(&busy));

        let offline = Session {
            phase: Phase::Offline,
            ..Session::resumed("r", "alice", false)
        };
        assert!(!is_connected(&offline));
    }

    #[test]
    fn an_account_is_there_to_show_whenever_a_token_is_held() {
        assert!(!Session::signed_out().has_account());
        let held = Session::resumed("r", "alice", false);
        for phase in [Phase::SignedIn, Phase::Offline, Phase::Working("checking")] {
            let session = Session {
                phase: phase.clone(),
                ..held.clone()
            };
            assert!(session.has_account(), "{phase:?}");
        }
        let signing_in = Session {
            phase: Phase::Working("signing in"),
            ..Session::default()
        };
        assert!(!signing_in.has_account());
    }
}
