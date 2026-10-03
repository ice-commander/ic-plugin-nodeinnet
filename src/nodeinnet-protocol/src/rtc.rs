use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RtcSignal {
    Offer {
        sdp: String,
        #[serde(default)]
        ice_restart: bool,
    },

    Answer {
        sdp: String,
    },

    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtcSignalEnvelope {
    pub to_node_id: String,
    pub signal: RtcSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundRtcSignal {
    pub from_node_id: String,
    pub signal: RtcSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCredentials {
    pub username: String,
    pub credential: String,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnRegion {
    #[default]
    Auto,
    Eu,
    Us,
    Main,
    Custom,
}

impl TurnRegion {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnRegion::Auto => "auto",
            TurnRegion::Eu => "eu",
            TurnRegion::Us => "us",
            TurnRegion::Main => "main",
            TurnRegion::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<TurnRegion> {
        match s {
            "auto" => Some(TurnRegion::Auto),
            "eu" => Some(TurnRegion::Eu),
            "us" => Some(TurnRegion::Us),
            "main" => Some(TurnRegion::Main),
            "custom" => Some(TurnRegion::Custom),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_round_trips_through_its_string_form() {
        for r in [
            TurnRegion::Auto,
            TurnRegion::Eu,
            TurnRegion::Us,
            TurnRegion::Main,
            TurnRegion::Custom,
        ] {
            assert_eq!(TurnRegion::parse(r.as_str()), Some(r));
        }
        assert_eq!(TurnRegion::parse("moon"), None);
    }

    #[test]
    fn region_serializes_lowercase_and_defaults_to_auto() {
        assert_eq!(serde_json::to_string(&TurnRegion::Eu).unwrap(), r#""eu""#);
        assert_eq!(
            serde_json::from_str::<TurnRegion>(r#""us""#).unwrap(),
            TurnRegion::Us
        );
        assert_eq!(TurnRegion::default(), TurnRegion::Auto);
    }

    #[test]
    fn rtc_signal_offer_roundtrips_json() {
        let signal = RtcSignal::Offer {
            sdp: "v=0\r\n".to_string(),
            ice_restart: false,
        };
        let json = serde_json::to_string(&signal).unwrap();
        let parsed: RtcSignal = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, RtcSignal::Offer { sdp, .. } if sdp == "v=0\r\n"));
    }

    #[test]
    fn rtc_signal_offer_without_ice_restart_field_defaults_false() {
        let json = r#"{"type":"Offer","payload":{"sdp":"v=0\r\n"}}"#;
        let parsed: RtcSignal = serde_json::from_str(json).unwrap();
        assert!(matches!(
            parsed,
            RtcSignal::Offer {
                ice_restart: false,
                ..
            }
        ));
    }

    #[test]
    fn turn_credentials_round_trip_as_the_website_sends_them() {
        let creds = TurnCredentials {
            username: "1700000000:alice".to_string(),
            credential: "c2VjcmV0".to_string(),
            uris: vec!["turn:node.in.net:3478".to_string()],
        };
        let json = serde_json::to_string(&creds).unwrap();
        let back: TurnCredentials = serde_json::from_str(&json).unwrap();
        assert_eq!(back.username, creds.username);
        assert_eq!(back.uris, creds.uris);
    }
}
