# p2p-node

The peer node of the [node.in.net](https://node.in.net) stack, vendored from
[node-in-net/p2p-common](https://github.com/node-in-net/p2p-common).

Holds the node's context and guards what peers may reach: every incoming
`P2pMessage` must target a resource this node actually shares, or it is dropped.
It also carries the local mesh (mDNS discovery and Noise-encrypted direct
sessions for peers on the same network), which this plugin keeps switched off.

Serving a capability is somebody else's job. An application installs a
`MessageHandler` (here, the one from `p2p-handlers`) and this crate delegates to
it after the access check. With none installed the node serves nothing, and
still consumes whatever its peers share.

## License

[Apache License 2.0](../../LICENSE-APACHE) or [MIT](../../LICENSE-MIT), at your option.
