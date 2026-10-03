use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub resources: Vec<u8>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub email: String,
    pub login: String,
    pub node_name: String,
    #[serde(default)]
    pub premium: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub login: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
    #[serde(default)]
    pub region: crate::rtc::TurnRegion,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub devices: Vec<Device>,
    pub session_id: String,
    pub ws_url: String,
    #[serde(default)]
    pub turn: Option<crate::rtc::TurnCredentials>,
    #[serde(default)]
    pub premium: i32,
    #[serde(default)]
    pub turn_region: crate::rtc::TurnRegion,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshRequest {
    pub refresh_token: String,
    #[serde(default)]
    pub region: crate::rtc::TurnRegion,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub devices: Vec<Device>,
    pub ws_url: String,
    #[serde(default)]
    pub turn: Option<crate::rtc::TurnCredentials>,
    #[serde(default)]
    pub premium: i32,
    #[serde(default)]
    pub turn_region: crate::rtc::TurnRegion,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ErrorResponse {
    pub msg: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EditRequest {
    pub username: String,
    pub email: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refresh_response_without_turn_or_premium_still_parses() {
        let json_text = r#"{"access_token":"eyJ","refresh_token":"64c","ws_url":"wss://test","devices":[{"id":"59f","name":"VM","resources":[123,34,102],"created_at":"2026-04-06T22:58:26.064451950Z"}]}"#;
        let res = serde_json::from_str::<RefreshResponse>(json_text);
        assert!(res.is_ok(), "Parse failed: {:?}", res.err());
        let res = res.unwrap();
        assert!(res.turn.is_none());
        assert_eq!(res.premium, 0);
        assert_eq!(res.turn_region, crate::rtc::TurnRegion::Auto);
    }

    #[test]
    fn a_login_response_carrying_fields_we_do_not_know_still_signs_us_in() {
        let json_text = r#"{
            "access_token":"eyJ",
            "refresh_token":"64c",
            "devices":[],
            "session_id":"s1",
            "ws_url":"wss://test",
            "premium":1,
            "turn_region":"eu",
            "quota_bytes":42,
            "server_flags":{"beta":true},
            "relays":["turn:new.example:3478"]
        }"#;
        let res = serde_json::from_str::<LoginResponse>(json_text)
            .expect("a newer website must not lock this client out");
        assert_eq!(res.access_token, "eyJ");
        assert_eq!(res.premium, 1);
        assert_eq!(res.turn_region, crate::rtc::TurnRegion::Eu);
    }

    #[test]
    fn a_device_round_trips_with_the_instant_it_was_created() {
        let device = Device {
            id: "59f".to_string(),
            name: "VM".to_string(),
            resources: vec![123, 34, 102],
            created_at: "2026-04-06T22:58:26.064451950Z"
                .parse::<DateTime<Utc>>()
                .unwrap(),
            last_used: 17,
        };
        let json = serde_json::to_string(&device).unwrap();
        let back: Device = serde_json::from_str(&json).unwrap();
        assert_eq!(back.created_at, device.created_at);
        assert_eq!(back.resources, device.resources);
        assert_eq!(back.last_used, 17);
    }

    #[test]
    fn a_login_request_defaults_its_region_to_auto() {
        let parsed: LoginRequest =
            serde_json::from_str(r#"{"login":"alice","password":"pw"}"#).unwrap();
        assert_eq!(parsed.region, crate::rtc::TurnRegion::Auto);
    }
}
