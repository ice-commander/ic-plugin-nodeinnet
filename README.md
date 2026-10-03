# Node In Net

An [Ice Commander](https://github.com/ice-commander) plugin that opens folders shared by your
other devices over the [Node In Net](https://node.in.net) peer-to-peer network, and a terminal
on those devices when they offer one.

## What it does

**Account.** A header button opens the account page. Signing in goes to `https://node.in.net`
(override with the `NODEINNET_API` environment variable). The refresh token is stored through
the host as a secret; the login and the premium flag are stored as plain settings. On the next
start the plugin checks the stored token on its own thread. If the directory refuses it (4xx),
the plugin signs out. If the directory is down or unreachable, the account is kept, marked
offline, and the page shows a **Connect again** button. The header label shows the login and
how many of your other devices are online.

**Signalling and transport.** After sign-in the plugin connects to the signalling WebSocket the
directory returned and reaches peers over WebRTC data channels. It uses the TURN credentials the
server hands out, or falls back to STUN only. Peers are reached only through the signalling
server: local-network discovery is switched off (`LOCAL_DISCOVERY` in
`src/node-in-net-fs/src/net.rs`), and `tests/no_lan_discovery.rs` enforces this.

**Device identity.** The device id and an ed25519 key pair are created on first use and kept in
the plugin's settings, with the private key stored as a secret.

**Peer folders.** The plugin registers the connection kind `node-in-net`. Its form lists the
folders your online devices share, and the one you choose is saved like any other connection.
Opening the connection mounts that folder. You can list, read, write, create folders, delete and
rename. When the peer offers a terminal resource, the filesystem's `shell_*` entries open a
terminal on the peer. This side is a client only: it never serves its own shell.

**Pinned entry.** While an account is held, it appears in the connections list and opens the
account page.

**Shared folders.** The account page keeps a list of folders this device offers, each with a
unique name and path. The list belongs to the device and survives sign-out.

The UI is translated into 15 languages (`src/node-in-net-fs/locales`).

## Layout

| crate | role |
|---|---|
| `src/node-in-net-fs` | the plugin (`cdylib`): account, views, connection kind, peer filesystem and terminal |
| `src/nodeinnet-protocol` | wire types: account payloads, node and resource model, peer messages, crypto helpers |
| `src/client-core` | signalling WebSocket, WebRTC transport, peer orchestration |
| `src/p2p-node` | peer node context and access control |
| `src/p2p-handlers` | what this device serves to peers (files only in this build) |
| `src/node-functions` | the local filesystem that `p2p-handlers` serves |

The plugin interface is [ice-commander/plugin-api](https://github.com/ice-commander/plugin-api).
The host must support pinned connections.

## Build, test, deploy

```sh
./build.sh          # release build; libraries are collected into bin/
./test.sh           # cargo test --workspace
./deploy-local.sh   # copies bin/ libraries into the host's plugin folder
```

`tests/no_lan_discovery.rs` inspects the built library, so run `./build.sh` before the tests.
`deploy-local.sh` writes to the platform's Ice Commander plugin folder unless `IC_PLUGIN_DIR` is
set. After deploying, enable the plugin in **Settings → Plugins** and restart.

## Known limitations

- Shared folders are saved but not announced: `identity::announcing` sends an empty resource
  list, so peers cannot see or open them.
- The device table shows the devices the signalling server reports online. The account's own
  device list from the directory (`Account::devices`) is parsed but never shown, and there is no
  graph view.
- The sign-in page has no guest mode and no device-name field. The device name comes from the
  `device_name` setting, which nothing writes, then `HOSTNAME`, then "Ice Commander".
  `auth::register_device` is never called.
- A request to a peer has no timeout (`error::net_content_wait`).
- Files are read and written whole, with no progress reporting. The download, upload, progress,
  permission and mount-bridge helpers in `p2p_rpc.rs` are not reachable from the host.
- This device serves files only. The terminal, desktop, registry, proxy, sync and system-info
  handlers in `src/p2p-handlers` do not build here, because `node-functions` carries only
  `fs_local`.

## License

MIT or Apache-2.0, at your option. The icons are an exception; see
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
