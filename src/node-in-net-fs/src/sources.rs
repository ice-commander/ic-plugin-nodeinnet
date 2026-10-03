use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(serde::Deserialize, Clone, Debug)]
pub struct SavedPeerConnection {
    pub name: String,
    pub peer_id: String,
}

#[derive(Clone)]
pub struct P2pContext {
    pub saved: Vec<SavedPeerConnection>,
    pub my_id: String,
    pub online_nodes: Rc<RefCell<Vec<nodeinnet_protocol::NodeInfo>>>,
}

impl P2pContext {
    pub fn new(
        saved: Vec<SavedPeerConnection>,
        my_id: String,
        online_nodes: Rc<RefCell<Vec<nodeinnet_protocol::NodeInfo>>>,
    ) -> Self {
        Self {
            saved,
            my_id,
            online_nodes,
        }
    }

    pub fn is_self(&self, peer_id: &str) -> bool {
        !self.my_id.is_empty() && peer_id == self.my_id
    }

    pub fn saved_peers(&self) -> Vec<SavedPeerConnection> {
        self.saved.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct P2pSource {
    pub peer_id: String,
    pub resource_id: String,
    pub name: String,
    pub subtitle: String,
    pub key: String,
    pub is_online: bool,
    pub is_favorite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Online {
    pub id: String,
    pub name: String,
    pub shares: Vec<String>,
}

pub fn devices_online(my_id: &str, nodes: &[nodeinnet_protocol::NodeInfo]) -> Vec<Online> {
    let mut found: Vec<Online> = Vec::new();
    for node in nodes {
        let is_self = !my_id.is_empty() && node.id == my_id;
        if !node.is_online || is_self || found.iter().any(|held| held.id == node.id) {
            continue;
        }
        found.push(Online {
            id: node.id.clone(),
            name: if node.name.is_empty() {
                node.id.clone()
            } else {
                node.name.clone()
            },
            shares: filesystem_shares(node)
                .into_iter()
                .map(|share| share.name.clone())
                .collect(),
        });
    }
    found
}

pub fn peers_online(my_id: &str, nodes: &[nodeinnet_protocol::NodeInfo]) -> Vec<P2pSource> {
    let context = P2pContext::new(
        Vec::new(),
        my_id.to_string(),
        Rc::new(RefCell::new(nodes.to_vec())),
    );
    peer_sources(&context, None, &[])
}

pub fn peer_sources(
    ctx: &P2pContext,
    active: Option<(String, String)>,
    favorites: &[String],
) -> Vec<P2pSource> {
    let peers = ctx.saved_peers();
    let online_nodes = &*ctx.online_nodes.borrow();
    let mut sources: Vec<P2pSource> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let active_key = active
        .as_ref()
        .map(|(peer_id, resource_id)| format!("p2p://{peer_id}@{resource_id}"));

    let push = |sources: &mut Vec<P2pSource>,
                seen: &mut HashSet<String>,
                peer_id: &str,
                resource_id: &str,
                name: String,
                subtitle: String,
                key: String,
                is_online: bool| {
        if seen.insert(key.clone()) {
            sources.push(P2pSource {
                peer_id: peer_id.to_string(),
                resource_id: resource_id.to_string(),
                name,
                subtitle,
                is_favorite: favorites.contains(&key),
                key,
                is_online,
            });
        }
    };

    if let Some((peer_id, resource_id)) = active.filter(|(id, _)| !ctx.is_self(id)) {
        let key = format!("p2p://{peer_id}@{resource_id}");
        let name = drive_name(&peer_id, &resource_id, &ctx.saved);
        let subtitle = format!("Resource ID: {resource_id} | Peer ID: {peer_id}");
        push(
            &mut sources,
            &mut seen,
            &peer_id,
            &resource_id,
            name,
            subtitle,
            key,
            true,
        );
    }

    for peer in &peers {
        if ctx.is_self(&peer.peer_id) {
            continue;
        }
        let online = online_nodes
            .iter()
            .find(|n| n.id == peer.peer_id && n.is_online);
        let Some(node) = online else {
            let key = format!("p2p://{}", peer.peer_id);
            let name = format!("{} (offline)", peer.name);
            push(
                &mut sources,
                &mut seen,
                &peer.peer_id,
                "",
                name,
                peer.peer_id.clone(),
                key,
                false,
            );
            continue;
        };

        let shares = filesystem_shares(node);
        if shares.is_empty() {
            let key = format!("p2p://{}", peer.peer_id);
            push(
                &mut sources,
                &mut seen,
                &peer.peer_id,
                "",
                peer.name.clone(),
                peer.peer_id.clone(),
                key,
                true,
            );
            continue;
        }
        for share in shares {
            let key = format!("p2p://{}@{}", peer.peer_id, share.id);
            let name = format!("{} ({})", share.name, peer.name);
            let subtitle = format!("Resource ID: {} | Peer ID: {}", share.id, peer.peer_id);
            push(
                &mut sources,
                &mut seen,
                &peer.peer_id,
                &share.id,
                name,
                subtitle,
                key,
                true,
            );
        }
    }

    for node in online_nodes {
        if !node.is_online || ctx.is_self(&node.id) {
            continue;
        }
        if peers.iter().any(|p| p.peer_id == node.id) {
            continue;
        }
        for share in filesystem_shares(node) {
            let key = format!("p2p://{}@{}", node.id, share.id);
            if Some(&key) == active_key.as_ref() {
                continue;
            }
            let device = if node.name.is_empty() {
                "Unnamed device".to_string()
            } else {
                node.name.clone()
            };
            let name = format!("{} ({})", share.name, device);
            let subtitle = format!("Resource ID: {} | Peer ID: {}", share.id, node.id);
            push(
                &mut sources,
                &mut seen,
                &node.id,
                &share.id,
                name,
                subtitle,
                key,
                true,
            );
        }
    }

    sources
}

fn filesystem_shares(
    node: &nodeinnet_protocol::NodeInfo,
) -> Vec<&nodeinnet_protocol::SharedResource> {
    node.resources
        .iter()
        .filter(|r| r.resource_type == nodeinnet_protocol::ResourceType::Filesystem)
        .collect()
}

pub fn drive_name(peer_id: &str, resource_id: &str, saved: &[SavedPeerConnection]) -> String {
    let peer_name = saved
        .iter()
        .find(|p| p.peer_id == peer_id)
        .map(|p| p.name.clone());

    let resource_name = resource_id.to_string();

    match peer_name {
        Some(peer) => format!("{resource_name} ({peer})"),
        None if peer_id.len() > 12 => format!(
            "{resource_name} ({}...{})",
            &peer_id[0..6],
            &peer_id[peer_id.len() - 6..]
        ),
        None => format!("{resource_name} ({peer_id})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(id: &str) -> nodeinnet_protocol::SharedResource {
        nodeinnet_protocol::SharedResource {
            id: id.to_string(),
            name: "Files".to_string(),
            resource_type: nodeinnet_protocol::ResourceType::Filesystem,
            config: None,
            is_active: true,
            session_token: None,
        }
    }

    fn node(id: &str, resource: &str) -> nodeinnet_protocol::NodeInfo {
        nodeinnet_protocol::NodeInfo {
            id: id.to_string(),
            name: format!("device-{id}"),
            os: "linux".to_string(),
            version: "0.0.0".to_string(),
            app_type: "gui".to_string(),
            build_type: "deb".to_string(),
            public_key: String::new(),
            resources: vec![share(resource)],
            is_online: true,
            last_used: 0,
            is_temporary: false,
        }
    }

    fn saved(entries: &[(&str, &str)]) -> Vec<SavedPeerConnection> {
        entries
            .iter()
            .map(|(name, id)| SavedPeerConnection {
                name: name.to_string(),
                peer_id: id.to_string(),
            })
            .collect()
    }

    fn ctx(my_id: &str, nodes: Vec<nodeinnet_protocol::NodeInfo>) -> P2pContext {
        P2pContext::new(Vec::new(), my_id.to_string(), Rc::new(RefCell::new(nodes)))
    }

    fn peer_ids(sources: &[P2pSource]) -> Vec<String> {
        sources.iter().map(|s| s.peer_id.clone()).collect()
    }

    #[test]
    fn a_device_is_listed_once_whatever_it_shares_and_we_are_not_among_them() {
        let mut quiet = node("quiet", "fs-q");
        quiet.resources.clear();
        let mut offline = node("gone", "fs-g");
        offline.is_online = false;
        let mut twice = node("peer", "fs-a");
        twice.resources.push(share("fs-b"));
        let nodes = vec![node("me", "fs-mine"), twice.clone(), twice, quiet, offline];

        let found = devices_online("me", &nodes);
        let ids: Vec<&str> = found.iter().map(|held| held.id.as_str()).collect();
        assert_eq!(ids, vec!["peer", "quiet"]);
        assert_eq!(found[0].name, "device-peer");
        assert_eq!(found[0].shares, vec!["Files", "Files"]);
        assert!(found[1].shares.is_empty(), "sharing nothing, still there");
        assert_eq!(
            peers_online("me", &nodes).len(),
            2,
            "where the drives count folders, not devices"
        );
    }

    #[test]
    fn our_own_share_is_never_offered_as_a_source() {
        let c = ctx("me", vec![node("me", "fs-mine"), node("peer", "fs-theirs")]);
        assert_eq!(peer_ids(&peer_sources(&c, None, &[])), vec!["peer"]);
    }

    #[test]
    fn a_peer_saved_under_our_own_id_is_still_refused() {
        let mut c = ctx("me2", vec![node("me2", "fs-mine")]);
        c.saved = saved(&[("myself", "me2")]);
        assert!(peer_sources(&c, None, &[]).is_empty());
    }

    #[test]
    fn an_active_connection_to_ourselves_is_dropped() {
        let c = ctx("me3", vec![node("me3", "fs-mine")]);
        let active = Some(("me3".to_string(), "fs-mine".to_string()));
        assert!(peer_sources(&c, active, &[]).is_empty());
    }

    #[test]
    fn a_real_peer_still_reaches_the_list() {
        let c = ctx("me4", vec![node("other", "fs-theirs")]);
        assert_eq!(peer_ids(&peer_sources(&c, None, &[])), vec!["other"]);
    }

    #[test]
    fn every_share_of_a_saved_peer_becomes_its_own_source() {
        let mut peer = node("friend", "fs-docs");
        peer.resources.push(share("fs-media"));
        let mut c = ctx("me5", vec![peer]);
        c.saved = saved(&[("Friend", "friend")]);

        let sources = peer_sources(&c, None, &[]);
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().all(|s| s.is_online));
    }

    #[test]
    fn a_saved_peer_that_is_not_online_is_still_listed_as_offline() {
        let mut c = ctx("me6", vec![]);
        c.saved = saved(&[("Friend", "friend")]);

        let sources = peer_sources(&c, None, &[]);
        assert_eq!(sources.len(), 1);
        assert!(!sources[0].is_online);
        assert_eq!(sources[0].key, "p2p://friend");
    }

    #[test]
    fn a_peer_sharing_nothing_is_offered_without_a_resource() {
        let mut peer = node("bare", "unused");
        peer.resources.clear();
        let mut c = ctx("me7", vec![peer]);
        c.saved = saved(&[("Bare", "bare")]);

        let sources = peer_sources(&c, None, &[]);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].resource_id.is_empty());
        assert!(sources[0].is_online);
    }

    #[test]
    fn a_favourite_peer_share_is_flagged() {
        let c = ctx("me8", vec![node("friend", "fs-docs")]);
        let key = "p2p://friend@fs-docs".to_string();
        assert!(peer_sources(&c, None, std::slice::from_ref(&key))[0].is_favorite);
    }

    #[test]
    fn the_same_share_is_never_listed_twice() {
        let mut c = ctx("me9", vec![node("friend", "fs-docs")]);
        c.saved = saved(&[("Friend", "friend")]);
        let active = Some(("friend".to_string(), "fs-docs".to_string()));
        assert_eq!(peer_sources(&c, active, &[]).len(), 1);
    }

    #[test]
    fn a_saved_peer_lends_its_name_to_the_drive() {
        let held = saved(&[("Alice", "alice-id")]);
        assert_eq!(drive_name("alice-id", "fs-1", &held), "fs-1 (Alice)");
    }

    #[test]
    fn an_unknown_peer_is_named_by_a_shortened_id() {
        assert_eq!(drive_name("short", "fs-1", &[]), "fs-1 (short)");
        assert_eq!(
            drive_name("abcdef0123456789", "fs-1", &[]),
            "fs-1 (abcdef...456789)"
        );
    }
}
