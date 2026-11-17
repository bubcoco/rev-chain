# RustEVM

A minimal Reth 2.11.0 Ethereum execution node configured for chain ID 235.
The Reth release is pinned to a single upstream revision; Rust 1.96.0 is
required by that release.

The genesis enables the requested Ethereum forks through Prague and sets a
30,000,000 gas limit. Merge activation is represented with terminal total
difficulty zero and `terminalTotalDifficultyPassed: true`, rather than the
Besu-specific `mergeNetSplitBlock` field.

The node uses Reth's local development engine. It schedules a block every
3 seconds while transactions are pending and an empty-block heartbeat every
45 seconds while idle. This is a local development producer, not QBFT and not
a production consensus implementation. It also does not register Besu custom
precompiles.

Run the node with:

```sh
cargo run --release -- node --chain ./config/genesis.json --dev --http
```

HTTP JSON-RPC is enabled on Reth's default local endpoint (`127.0.0.1:8545`).
