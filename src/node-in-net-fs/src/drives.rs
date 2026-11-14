use crate::i18n::{tr, trf};
use client_core::LanDevice;
use ic_plugin_api::{IcDrive, IcDrives};
use p2p_node::lan::id::{fingerprint, id8, id_of};
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::CString;
use std::os::raw::c_void;
use std::sync::{Mutex, MutexGuard, OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub name: String,
    pub subtitle: String,
    pub settings: String,
    pub online: bool,
}

thread_local! {
    static HELD: RefCell<(Vec<CString>, Vec<IcDrive>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

fn account_twins() -> HashSet<String> {
    let seen = crate::net::snapshot();
    if !seen.connected {
        return HashSet::new();
    }
    seen.nodes
        .iter()
        .filter(|node| node.is_online)
        .filter_map(|node| id_of(&node.public_key))
        .collect()
}

fn row_of(device: &LanDevice) -> Option<Row> {
    let short = fingerprint(id8(&device.id));
    let shown = [("id", short.as_str())];
    let (subtitle, settings, online) = if device.opened_from_here {
        (
            trf("nodeinnet.drive_paired", &shown),
            json!({
                "peer_id": device.id,
                "resource_id": format!("fs-{}", device.id),
                "link": "lan",
            }),
            device.online,
        )
    } else if device.online {
        (
            trf("nodeinnet.drive_unpaired", &shown),
            json!({ "pair": device.id }),
            true,
        )
    } else {
        return None;
    };
    let subtitle = if device.name_clash {
        format!("{subtitle} · {}", tr("nodeinnet.drive_same_name"))
    } else {
        subtitle
    };
    let name = if device.name.is_empty() {
        short
    } else {
        device.name.clone()
    };
    Some(Row {
        key: format!("nin:{}", device.id),
        name,
        subtitle,
        settings: settings.to_string(),
        online,
    })
}

pub fn listed() -> Vec<Row> {
    let Some(state) = crate::lan::snapshot().state else {
        return Vec::new();
    };
    let twins = account_twins();
    state
        .devices
        .iter()
        .filter(|device| !twins.contains(&device.id))
        .filter_map(row_of)
        .collect()
}

fn announced() -> MutexGuard<'static, Vec<Row>> {
    static HELD: OnceLock<Mutex<Vec<Row>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn changed() -> bool {
    let now = listed();
    let mut held = announced();
    if *held == now {
        return false;
    }
    *held = now;
    true
}

pub extern "C" fn rows(_user_data: *mut c_void) -> IcDrives {
    let listed = listed();
    let mut text = Vec::with_capacity(listed.len() * 4);
    for row in &listed {
        for line in [&row.key, &row.name, &row.subtitle, &row.settings] {
            text.push(CString::new(line.as_str()).unwrap_or_default());
        }
    }
    let drives: Vec<IcDrive> = listed
        .iter()
        .enumerate()
        .map(|(at, row)| IcDrive {
            key: text[at * 4].as_ptr(),
            name: text[at * 4 + 1].as_ptr(),
            subtitle: text[at * 4 + 2].as_ptr(),
            settings: text[at * 4 + 3].as_ptr(),
            svg: std::ptr::null(),
            online: i32::from(row.online),
        })
        .collect();
    HELD.with(|held| {
        *held.borrow_mut() = (text, drives);
        let held = held.borrow();
        IcDrives {
            count: held.1.len() as u32,
            rows: held.1.as_ptr(),
            ..IcDrives::EMPTY
        }
    })
}
