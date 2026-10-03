//! `bind` and `sendto` stay allowed because WebRTC needs them; discovery crates and multicast do not.

use std::path::PathBuf;

const DISCOVERY: &[&str] = &[
    "libmdns",
    "simple-mdns",
    "astro-dnssd",
    "searchlight",
    "zeroconf",
    "libp2p-mdns",
    "libp2p",
];

/// `mdns-sd` is left out on purpose: the transport links it and LOCAL_DISCOVERY keeps it shut.
const NEIGHBOURLY: &[&str] = &[
    "libmdns",
    "simple-mdns",
    "astro-dnssd",
    "searchlight",
    "zeroconf",
    "libp2p",
    "igd",
    "igd-next",
];

fn libraries() -> Vec<PathBuf> {
    let name = if cfg!(target_os = "macos") {
        "libic_node_in_net_fs.dylib"
    } else {
        "libic_node_in_net_fs.so"
    };
    let mut found = vec![workspace_root().join("bin").join(name)];
    if let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    {
        found.push(data.join("ice-commander/plugins").join(name));
    }
    found.retain(|path| path.exists());
    found
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn locked_packages() -> Vec<String> {
    let lock = std::fs::read_to_string(workspace_root().join("Cargo.lock"))
        .expect("the workspace has a lock file");
    lock.lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(|name| name.to_string())
        .collect()
}

#[test]
fn nothing_that_browses_the_local_network_is_anywhere_in_the_lock_file() {
    let packages = locked_packages();
    assert!(
        packages.len() > 20,
        "the lock file was read, not merely opened: {} packages",
        packages.len()
    );
    let found: Vec<&String> = packages
        .iter()
        .filter(|held| DISCOVERY.contains(&held.as_str()))
        .collect();
    assert!(
        found.is_empty(),
        "these would let the plugin look for devices on the subnet: {found:?}"
    );
}

#[test]
fn the_plugins_own_source_sends_no_datagrams_and_joins_no_groups() {
    let mut offending = Vec::new();
    for crate_dir in ["node-in-net-fs", "nodeinnet-protocol"] {
        let src = workspace_root().join("src").join(crate_dir).join("src");
        walk(&src, &mut |path, body| {
            for (number, line) in body.lines().enumerate() {
                let bare = line.trim_start();
                if bare.starts_with("//") || bare.starts_with("///") {
                    continue;
                }
                for needle in [
                    "UdpSocket",
                    "SOCK_DGRAM",
                    "set_multicast",
                    "join_multicast",
                    "set_broadcast",
                    "224.0.0.251",
                    "ff02::",
                    "_tcp.local",
                ] {
                    if line.contains(needle) {
                        offending.push(format!("{}:{}: {needle}", path.display(), number + 1));
                    }
                }
            }
        });
    }
    assert!(offending.is_empty(), "{offending:?}");
}

fn walk(dir: &std::path::Path, visit: &mut dyn FnMut(&std::path::Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            if let Ok(body) = std::fs::read_to_string(&path) {
                visit(&path, &body);
            }
        }
    }
}

#[test]
fn local_discovery_is_started_off_and_cannot_be_switched_on() {
    let mut started = false;
    for file in ["node-in-net-fs/src/net.rs", "node-in-net-fs/src/lib.rs"] {
        let Ok(body) = std::fs::read_to_string(workspace_root().join("src").join(file)) else {
            continue;
        };
        for line in body.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("start_network_thread(") {
                started = true;
            }
            assert!(
                !line.contains("SetLocalDiscovery"),
                "{file}: nothing may ask the transport to start looking"
            );
        }
    }
    if !started {
        eprintln!("the transport is not started yet, so there is no flag to pass");
        return;
    }
    let body = std::fs::read_to_string(workspace_root().join("src/node-in-net-fs/src/net.rs"))
        .expect("net.rs is readable");
    assert!(
        body.contains("LOCAL_DISCOVERY: bool = false"),
        "the flag handed to the transport must be written down as false, here"
    );
}

/// Checks the linked library, not the lock file, which also lists dev-dependencies.
#[test]
fn nothing_beyond_the_transport_can_look_for_a_neighbour() {
    let libraries = libraries();
    assert!(!libraries.is_empty(), "run ./build.sh first");
    for library in &libraries {
        let body = std::fs::read(library).expect("the library is readable");
        let text = String::from_utf8_lossy(&body);
        for crate_name in NEIGHBOURLY {
            let marked = format!("/{crate_name}-");
            assert!(
                !text.contains(&marked),
                "{library:?} links {crate_name}, which can look for neighbours"
            );
        }
    }
}
