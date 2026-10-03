use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

pub mod account;
pub mod crypto;
pub mod p2p;
pub mod rtc;
pub mod ws;

pub use account::*;
pub use p2p::*;
pub use rtc::*;
pub use ws::*;

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedResource {
    pub id: String,
    pub name: String,
    pub resource_type: ResourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
}

impl SharedResource {
    pub fn without_config(&self) -> SharedResource {
        SharedResource {
            config: None,
            ..self.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Filesystem,
    SystemInfo,
    Terminal,
    Registry,
    SharedNetwork,

    SyncFolder,
    RemoteDesktop,
}

impl ResourceType {
    pub fn is_only_local(&self) -> bool {
        matches!(self, ResourceType::SyncFolder)
    }

    pub fn is_only_remote(&self) -> bool {
        matches!(
            self,
            ResourceType::Terminal | ResourceType::SharedNetwork | ResourceType::RemoteDesktop
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub os: String,
    pub version: String,
    pub app_type: String,
    pub build_type: String,
    pub public_key: String,
    pub resources: Vec<SharedResource>,
    #[serde(default = "default_is_online")]
    pub is_online: bool,
    #[serde(default)]
    pub last_used: i64,
    #[serde(default)]
    pub is_temporary: bool,
}

impl NodeInfo {
    pub fn announced(&self) -> NodeInfo {
        NodeInfo {
            resources: self.resources.iter().map(|r| r.without_config()).collect(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceWrapper {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub app_type: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub resources: Vec<SharedResource>,
}

fn default_is_online() -> bool {
    true
}

pub fn resource_id_base(rt: &ResourceType) -> Option<&'static str> {
    Some(match rt {
        ResourceType::Filesystem => "fs",
        ResourceType::Terminal => "terminal",
        ResourceType::RemoteDesktop => "desktop",
        ResourceType::SharedNetwork => "network",
        ResourceType::Registry => "registry",
        ResourceType::SystemInfo => "sysinfo",
        ResourceType::SyncFolder => return None,
    })
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PeerConfig {
    pub name: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub last_known_addresses: Vec<String>,
}

pub trait PeerStore: Send + Sync {
    fn load(&self) -> std::collections::HashMap<String, PeerConfig>;
    fn update(&self, f: &mut dyn FnMut(&mut std::collections::HashMap<String, PeerConfig>));
}

#[derive(Default)]
pub struct MemoryPeerStore {
    peers: std::sync::Mutex<std::collections::HashMap<String, PeerConfig>>,
}

impl PeerStore for MemoryPeerStore {
    fn load(&self) -> std::collections::HashMap<String, PeerConfig> {
        self.peers.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn update(&self, f: &mut dyn FnMut(&mut std::collections::HashMap<String, PeerConfig>)) {
        if let Ok(mut peers) = self.peers.lock() {
            f(&mut peers);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIRE_SPELLING: [(ResourceType, &str); 7] = [
        (ResourceType::Filesystem, "Filesystem"),
        (ResourceType::SystemInfo, "SystemInfo"),
        (ResourceType::Terminal, "Terminal"),
        (ResourceType::Registry, "Registry"),
        (ResourceType::SharedNetwork, "SharedNetwork"),
        (ResourceType::SyncFolder, "SyncFolder"),
        (ResourceType::RemoteDesktop, "RemoteDesktop"),
    ];

    #[test]
    fn resource_type_serialises_with_the_wire_spelling() {
        for (variant, spelling) in WIRE_SPELLING {
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{spelling}\""),
                "the reference client declares this enum undecorated, so serde emits the \
                 variant name verbatim; renaming it here would make its peers reject our \
                 resources"
            );
        }
    }

    #[test]
    fn resource_type_reads_back_every_wire_spelling() {
        for (variant, spelling) in WIRE_SPELLING {
            let parsed: ResourceType =
                serde_json::from_str(&format!("\"{spelling}\"")).expect(spelling);
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn resource_type_keeps_its_spelling_through_bson() {
        for (variant, spelling) in WIRE_SPELLING {
            let resource = SharedResource {
                id: "r1".to_string(),
                name: "Root".to_string(),
                resource_type: variant.clone(),
                config: None,
                is_active: true,
                session_token: None,
            };
            let bytes = to_bson_vec(&resource).expect("serialises");
            let back: SharedResource = from_bson_slice(&bytes).expect("deserialises");
            assert_eq!(back.resource_type, variant);
            assert!(
                bytes
                    .windows(spelling.len())
                    .any(|w| w == spelling.as_bytes()),
                "{spelling} should travel as its own name inside the bson document"
            );
        }
    }

    #[test]
    fn a_resource_without_a_config_announces_the_same_shape() {
        let resource = SharedResource {
            id: "r1".to_string(),
            name: "Root".to_string(),
            resource_type: ResourceType::Filesystem,
            config: Some("/srv/share".to_string()),
            is_active: true,
            session_token: None,
        };
        assert!(resource.without_config().config.is_none());
    }

    #[test]
    fn an_announced_node_strips_the_config_of_every_resource() {
        let node = NodeInfo {
            id: "n1".to_string(),
            name: "Node".to_string(),
            os: "linux".to_string(),
            version: "1.0".to_string(),
            app_type: "desktop".to_string(),
            build_type: "release".to_string(),
            public_key: "key".to_string(),
            resources: vec![SharedResource {
                id: "r1".to_string(),
                name: "Root".to_string(),
                resource_type: ResourceType::Filesystem,
                config: Some("/srv/share".to_string()),
                is_active: true,
                session_token: None,
            }],
            is_online: true,
            last_used: 0,
            is_temporary: false,
        };
        assert!(node
            .announced()
            .resources
            .iter()
            .all(|r| r.config.is_none()));
    }

    #[test]
    fn a_node_without_the_optional_fields_is_read_as_online() {
        let json = r#"{"id":"n1","name":"Node","os":"linux","version":"1.0","app_type":"desktop","build_type":"release","public_key":"key","resources":[]}"#;
        let node: NodeInfo = serde_json::from_str(json).unwrap();
        assert!(node.is_online);
        assert_eq!(node.last_used, 0);
        assert!(!node.is_temporary);
    }

    #[test]
    fn every_shareable_resource_type_has_an_id_base_except_the_local_one() {
        assert_eq!(resource_id_base(&ResourceType::Filesystem), Some("fs"));
        assert_eq!(resource_id_base(&ResourceType::Terminal), Some("terminal"));
        assert_eq!(resource_id_base(&ResourceType::SyncFolder), None);
    }

    #[test]
    fn a_memory_peer_store_returns_what_was_written_into_it() {
        let store = MemoryPeerStore::default();
        store.update(&mut |peers| {
            peers.insert(
                "n1".to_string(),
                PeerConfig {
                    name: "Node".to_string(),
                    ..PeerConfig::default()
                },
            );
        });
        assert_eq!(
            store.load().get("n1").map(|p| p.name.as_str()),
            Some("Node")
        );
    }
}

pub static KNOWN_PUBLIC_KEYS: std::sync::OnceLock<RwLock<HashMap<String, String>>> =
    std::sync::OnceLock::new();

pub static KNOWN_PEER_NAMES: std::sync::OnceLock<RwLock<HashMap<String, String>>> =
    std::sync::OnceLock::new();

pub static KNOWN_RESOURCE_NAMES: std::sync::OnceLock<RwLock<HashMap<(String, String), String>>> =
    std::sync::OnceLock::new();

pub fn get_known_public_key(node_id: &str) -> Option<String> {
    KNOWN_PUBLIC_KEYS.get()?.read().ok()?.get(node_id).cloned()
}

pub fn get_known_peer_name(node_id: &str) -> Option<String> {
    KNOWN_PEER_NAMES.get()?.read().ok()?.get(node_id).cloned()
}

pub fn get_known_resource_name(peer_id: &str, resource_id: &str) -> Option<String> {
    KNOWN_RESOURCE_NAMES
        .get()?
        .read()
        .ok()?
        .get(&(peer_id.to_string(), resource_id.to_string()))
        .cloned()
}

pub fn update_known_public_keys(nodes: &[NodeInfo]) {
    let map = KNOWN_PUBLIC_KEYS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut write) = map.write() {
        for n in nodes {
            write.insert(n.id.clone(), n.public_key.clone());
        }
    }
    let names_map = KNOWN_PEER_NAMES.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut write) = names_map.write() {
        for n in nodes {
            write.insert(n.id.clone(), n.name.clone());
        }
    }
    let res_map = KNOWN_RESOURCE_NAMES.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut write) = res_map.write() {
        for n in nodes {
            for r in &n.resources {
                write.insert((n.id.clone(), r.id.clone()), r.name.clone());
            }
        }
    }
}
