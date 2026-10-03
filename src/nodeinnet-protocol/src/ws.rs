use serde::{Deserialize, Serialize};

use crate::account::Device;
use crate::rtc::{InboundRtcSignal, RtcSignalEnvelope};
use crate::NodeInfo;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "cmd", content = "data")]
pub enum WsMessage {
    Ping,
    Pong {
        timestamp: u64,
    },

    #[serde(rename = "list_nodes")]
    ListNodes,
    #[serde(rename = "rtc_signal")]
    RtcSignal(RtcSignalEnvelope),
    #[serde(rename = "update_node_info")]
    UpdateNodeInfo(NodeInfo),

    #[serde(rename = "nodes_list")]
    NodesList {
        nodes: Vec<NodeInfo>,
    },
    #[serde(rename = "inbound_rtc_signal")]
    InboundRtcSignal(InboundRtcSignal),
    #[serde(rename = "device_updated")]
    DeviceUpdated {
        device_id: String,
    },
    #[serde(rename = "device_added")]
    DeviceAdded(Device),
    #[serde(rename = "device_deleted")]
    DeviceDeleted {
        device_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_websocket_command_keeps_the_name_the_server_dispatches_on() {
        let named = [
            (WsMessage::ListNodes, "list_nodes"),
            (WsMessage::NodesList { nodes: Vec::new() }, "nodes_list"),
            (
                WsMessage::DeviceUpdated {
                    device_id: "d1".to_string(),
                },
                "device_updated",
            ),
            (
                WsMessage::DeviceDeleted {
                    device_id: "d1".to_string(),
                },
                "device_deleted",
            ),
            (WsMessage::Ping, "Ping"),
        ];
        for (msg, name) in named {
            let json = serde_json::to_string(&msg).unwrap();
            assert!(
                json.contains(&format!(r#""cmd":"{name}""#)),
                "the reference client renames these the same way; \
                 {json} would not reach the server's handler"
            );
        }
    }

    #[test]
    fn a_nodes_list_round_trips_with_the_nodes_it_carries() {
        let msg = WsMessage::NodesList {
            nodes: vec![NodeInfo {
                id: "n1".to_string(),
                name: "Node".to_string(),
                os: "linux".to_string(),
                version: "1.0".to_string(),
                app_type: "desktop".to_string(),
                build_type: "release".to_string(),
                public_key: "key".to_string(),
                resources: Vec::new(),
                is_online: true,
                last_used: 0,
                is_temporary: false,
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: WsMessage = serde_json::from_str(&json).unwrap();
        match back {
            WsMessage::NodesList { nodes } => assert_eq!(nodes[0].id, "n1"),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
