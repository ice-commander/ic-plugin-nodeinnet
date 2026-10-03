use serde_json::{json, Value};

pub fn chosen_as(peer_id: &str, resource_id: &str) -> String {
    format!("{peer_id}/{resource_id}")
}

pub fn split_chosen(chosen: &str) -> Option<(String, String)> {
    let (peer, resource) = chosen.split_once('/')?;
    (!peer.is_empty() && !resource.is_empty()).then(|| (peer.to_string(), resource.to_string()))
}

fn phrase(key: &str, english: &str) -> Value {
    json!({ "tr": key, "en": english })
}

pub fn offerable(sources: Vec<crate::sources::P2pSource>) -> Vec<crate::sources::P2pSource> {
    sources
        .into_iter()
        .filter(|source| !source.resource_id.is_empty())
        .collect()
}

pub fn signed_in() -> bool {
    crate::snapshot().is_signed_in()
}

pub fn offered_now() -> Vec<crate::sources::P2pSource> {
    let held = crate::snapshot();
    if !held.is_signed_in() {
        return Vec::new();
    }
    let seen = crate::net::snapshot();
    offerable(crate::sources::peers_online(&held.device_id, &seen.nodes))
}

fn offer_rows(shares: &[crate::sources::P2pSource]) -> Vec<Value> {
    shares
        .iter()
        .map(|share| {
            let id = format!("{ADD}{}", chosen_as(&share.peer_id, &share.resource_id));
            let also = if crate::p2p_rpc::peer_terminal(&share.peer_id).is_some() {
                phrase("nodeinnet.kind_row_with_shell", "files and a terminal")
            } else {
                phrase("nodeinnet.kind_row_files", "files")
            };
            json!({ "t": "row", "spacing": 8, "children": [
                { "t": "column", "weight": 1, "children": [
                    { "t": "text", "text": { "literal": share.name.clone() } },
                    { "t": "text", "role": "dim", "text": { "literal": share.subtitle.clone() } },
                    { "t": "text", "role": "dim", "text": also }
                ] },
                { "t": "button", "id": id.clone(),
                  "title": { "tr": "nodeinnet.kind_choose", "en": "Choose" },
                  "intent": { "do": "emit", "node": id } }
            ] })
        })
        .collect()
}

pub const ADD: &str = "add:";

pub fn offer_rows_for_test() -> Vec<Value> {
    offer_rows(&[crate::sources::P2pSource {
        peer_id: "peer".to_string(),
        resource_id: "share".to_string(),
        name: "Files".to_string(),
        subtitle: "Resource share".to_string(),
        key: "p2p://peer@share".to_string(),
        is_online: true,
        is_favorite: false,
    }])
}

pub fn standing(peer_id: &str, resource_id: &str) -> Value {
    if peer_id.is_empty() || resource_id.is_empty() {
        return json!({ "online": false, "shares": false, "terminal": false });
    }
    let seen = crate::net::snapshot();
    let found = seen.nodes.iter().find(|node| node.id == peer_id);
    let online = found.map(|node| node.is_online).unwrap_or(false);
    let shares = found
        .map(|node| node.resources.iter().any(|held| held.id == resource_id))
        .unwrap_or(false);
    json!({
        "online": online,
        "shares": shares,
        "terminal": crate::p2p_rpc::peer_terminal(peer_id).is_some(),
    })
}

/// Set on `opened`, because `describe` is not told which record the form is for.
pub fn standing_word(standing: &Value) -> &'static str {
    let flag = |key: &str| standing.get(key).and_then(Value::as_bool).unwrap_or(false);
    if !flag("online") {
        "offline"
    } else if !flag("shares") {
        "no_resource"
    } else if flag("terminal") {
        "ready_shell"
    } else {
        "ready"
    }
}

pub fn document() -> Value {
    let shares = offered_now();
    let line = |word: &str, key: &str, english: &str| json!({ "when": { "eq": ["view.standing", word] }, "then": phrase(key, english) });

    json!({
        "schema": 1,
        "kind": crate::view::KIND_ID,
        "label": { "literal": "P2P" },
        "identity": "name",
        "opens_at": "remote_path",
        "immutable_after_create": ["peer_id", "resource_id"],
        "fields": [
            { "bind": "name", "type": "text", "scope": "record", "required": true },
            { "bind": "peer_id", "type": "text", "required": true },
            { "bind": "resource_id", "type": "text", "required": true },
            { "bind": "remote_path", "type": "path", "empty_as_absent": true }
        ],
        "summary": { "fmt": "{name}", "fallback": { "fmt": "{resource_id}" } },
        "form": { "t": "view", "surface": "embedded", "scroll": "vertical", "spacing": 8,
          "sensitive": { "ne": ["view.mode", "view"] },
          "children": [
            { "t": "column", "id": "offer", "spacing": 8,
              "visible": { "not": { "truthy": "state.resource_id" } },
              "children": [
                { "t": "text", "id": "offer_title",
                  "text": phrase(
                      "nodeinnet.kind_pick",
                      "Shared folders on your devices — choose one, then Add Connection"
                  ),
                  "visible": { "eq": [shares.is_empty(), false] } },
                { "t": "column", "id": "offered", "spacing": 6, "children": offer_rows(&shares) },
                { "t": "text", "id": "not_signed_in", "role": "dim", "wrap": true,
                  "text": phrase(
                      "nodeinnet.kind_not_signed_in",
                      "You are not signed in, so there is nothing to choose. Sign in from the account window in the header."
                  ),
                  "visible": { "eq": [signed_in(), false] } },
                { "t": "text", "id": "nothing_offered", "role": "dim", "wrap": true,
                  "text": phrase(
                      "nodeinnet.kind_nothing_offered",
                      "No device is sharing a folder right now"
                  ),
                  "visible": { "all": [
                      { "eq": [signed_in(), true] },
                      { "eq": [shares.is_empty(), true] }
                  ] } }
              ] },
            { "t": "column", "id": "chosen", "spacing": 8,
              "visible": { "truthy": "state.resource_id" },
              "children": [
                { "t": "text", "id": "standing", "role": "dim", "wrap": true,
                  "text": { "cases": [
                      line("offline", "nodeinnet.kind_offline", "The device is not reachable now"),
                      line("no_resource", "nodeinnet.kind_no_resource",
                           "The device is online, but no longer shares this folder"),
                      line("ready_shell", "nodeinnet.kind_ready_with_shell",
                           "Online, the folder is shared, and a terminal is available"),
                      line("ready", "nodeinnet.kind_ready", "Online, and the folder is shared")
                  ], "else": phrase("nodeinnet.kind_asking", "Asking the network…") } },
                { "t": "text", "id": "which", "role": "dim", "text": "{view.chosen_name}" },
                { "t": "text", "id": "then_save", "role": "dim", "wrap": true,
                  "text": phrase(
                      "nodeinnet.kind_then_save",
                      "Press Add Connection to keep it in the list, like any other connection"
                  ) }
              ] },
            { "t": "input", "id": "name", "bind": "name", "chrome": "bare",
              "placeholder": phrase("conn_manager.conn_name_placeholder", "Connection Name") },
            { "t": "input", "id": "remote_path", "bind": "remote_path", "chrome": "bare",
              "placeholder": phrase("nodeinnet.kind_start_folder_hint", "Start folder inside the share (optional)") }
          ] },
        "actions": []
    })
}

pub fn reply_for(event: &Value) -> Value {
    let values = event.get("values").cloned().unwrap_or_else(|| json!({}));
    let text = |key: &str| {
        values
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "opened" => json!({
            "set": { "view.standing": standing_word(&standing(&text("peer_id"), &text("resource_id"))) }
        }),
        "activate" => {
            let node = event
                .get("node")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(share) = node.strip_prefix(ADD) else {
                return json!({});
            };
            picked(share, &text("name"))
        }
        _ => json!({}),
    }
}

pub fn picked(chosen: &str, named: &str) -> Value {
    let Some((peer_id, resource_id)) = split_chosen(chosen) else {
        return json!({});
    };
    let name = offerable(offered_now())
        .into_iter()
        .find(|share| share.peer_id == peer_id && share.resource_id == resource_id)
        .map(|share| share.name)
        .unwrap_or_else(|| resource_id.clone());
    let mut set = json!({
        "state.peer_id": peer_id.clone(),
        "state.resource_id": resource_id.clone(),
        "view.chosen_name": name.clone(),
        "view.standing": standing_word(&standing(&peer_id, &resource_id)),
    });
    if named.trim().is_empty() {
        set["state.name"] = json!(name);
    }
    json!({ "set": set })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(peer: &str, resource: &str, online: bool) -> crate::sources::P2pSource {
        crate::sources::P2pSource {
            peer_id: peer.to_string(),
            resource_id: resource.to_string(),
            name: format!("Files ({peer})"),
            subtitle: format!("Resource ID: {resource} | Peer ID: {peer}"),
            key: format!("p2p://{peer}@{resource}"),
            is_online: online,
            is_favorite: false,
        }
    }

    #[test]
    fn a_peer_with_nothing_shared_is_not_offered_as_a_drive() {
        let listed = offerable(vec![
            source("peer-1", "fs-1", true),
            source("bare", "", true),
        ]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].resource_id, "fs-1");
    }

    #[test]
    fn an_offline_share_is_still_offered_so_it_keeps_its_place_in_the_list() {
        let listed = offerable(vec![source("peer-1", "fs-1", false)]);
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].is_online);
    }

    #[test]
    fn a_share_is_spelled_as_both_halves_and_reads_back() {
        let spelled = chosen_as("peer-1", "fs-7");
        assert_eq!(spelled, "peer-1/fs-7");
        assert_eq!(
            split_chosen(&spelled),
            Some(("peer-1".to_string(), "fs-7".to_string()))
        );
        assert_eq!(split_chosen("peer-only"), None);
        assert_eq!(split_chosen("/fs-7"), None);
        assert_eq!(split_chosen("peer-1/"), None);
    }

    #[test]
    fn pressing_the_button_beside_a_share_chooses_it() {
        let answered = reply_for(&json!({
            "type": "activate",
            "node": "add:peer-1/fs-7",
            "values": { "name": "" }
        }));
        assert_eq!(answered["set"]["state.peer_id"], json!("peer-1"));
        assert_eq!(answered["set"]["state.resource_id"], json!("fs-7"));
        assert_eq!(
            reply_for(&json!({ "type": "activate", "node": "something_else" })),
            json!({}),
            "a button that is not one of ours chooses nothing"
        );
    }

    #[test]
    fn opening_the_form_says_how_the_record_is_standing() {
        let answered = reply_for(&json!({
            "type": "opened",
            "values": { "peer_id": "nobody", "resource_id": "fs-7" }
        }));
        assert_eq!(
            answered["set"]["view.standing"],
            json!("offline"),
            "a peer the listing does not hold is not reachable"
        );

        let fresh = reply_for(&json!({ "type": "opened", "values": {} }));
        assert_eq!(
            fresh["set"]["view.standing"],
            json!("offline"),
            "and a connection being added stands for nothing yet"
        );
    }

    #[test]
    fn the_word_the_form_matches_on_follows_what_the_network_says() {
        let word = |online, shares, terminal| {
            standing_word(&json!({ "online": online, "shares": shares, "terminal": terminal }))
        };
        assert_eq!(word(false, true, true), "offline");
        assert_eq!(word(true, false, true), "no_resource");
        assert_eq!(word(true, true, true), "ready_shell");
        assert_eq!(word(true, true, false), "ready");
    }

    #[test]
    fn picking_a_share_fills_in_what_is_stored_and_leaves_a_typed_name_alone() {
        let answered = picked("peer-1/fs-7", "");
        assert_eq!(answered["set"]["state.peer_id"], json!("peer-1"));
        assert_eq!(answered["set"]["state.resource_id"], json!("fs-7"));
        assert_eq!(
            answered["set"]["state.name"],
            json!("fs-7"),
            "with nothing on offer the resource is the only name there is"
        );

        let typed = picked("peer-1/fs-7", "my laptop");
        assert!(
            typed["set"].get("state.name").is_none(),
            "a name the user typed is not overwritten"
        );

        assert_eq!(picked("rubbish", ""), json!({}));
    }

    #[test]
    fn a_record_whose_peer_is_nowhere_reads_as_offline() {
        let standing = standing("nobody", "fs-7");
        assert_eq!(standing["online"], json!(false));
        assert_eq!(standing["shares"], json!(false));
        let empty = standing_of_nothing();
        assert_eq!(empty["online"], json!(false));
    }

    fn standing_of_nothing() -> Value {
        standing("", "")
    }
}
