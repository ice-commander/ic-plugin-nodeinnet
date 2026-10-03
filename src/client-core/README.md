# client-core

The client side of the [node.in.net](https://node.in.net) stack, vendored from
[node-in-net/p2p-common](https://github.com/node-in-net/p2p-common).

Connects to peers and drives the session: WebRTC transport, the signalling-server
WebSocket, peer orchestration and reconnection, the chunked framing that carries
BSON messages over a DataChannel, remote-desktop media decoding, and account
authentication over HTTP.

Headless and UI-free.

## License

[Apache License 2.0](../../LICENSE-APACHE) or [MIT](../../LICENSE-MIT), at your option.
