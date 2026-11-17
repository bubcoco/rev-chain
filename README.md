# RustEVM

A minimal Reth 2.11.0 Ethereum execution node configured for chain ID 235.
The Reth release is pinned to a single upstream revision; Rust 1.96.0 is
required by that release.

The genesis enables the requested Ethereum forks through Prague and sets a
30,000,000 gas limit. Merge activation is represented with terminal total
difficulty zero and `terminalTotalDifficultyPassed: true`, rather than the
Besu-specific `mergeNetSplitBlock` field.

The node registers Rust implementations of Besu's six custom precompiles at
`0x1001`–`0x1006`: NativeMinter, AddressRegistry, GasPrice, RevenueRatio,
TreasuryRegistry, and GasFeeGrant. GasFeeGrant execution includes custom
transaction hooks for sponsorship settlement and revenue distribution, with
pool admission checks for grant candidates and the GasPrice floor. These
consensus-sensitive paths compile and have unit coverage for selected helpers,
but have not yet been validated against Besu integration tests or a production
network.

The node uses Reth's local development engine. It schedules a block every
3 seconds while transactions are pending and an empty-block heartbeat every
45 seconds while idle. This is a local development producer, not QBFT and not
a production consensus implementation.

Run the node with:

```sh
cargo run --release -- node --chain ./config/genesis.json --dev --http
```

HTTP JSON-RPC is enabled on Reth's default local endpoint (`127.0.0.1:8545`).

Build and run the container with:

```sh
docker build -t rust-evm:local .
docker run --rm -p 8545:8545 -v rust-evm-data:/data rust-evm:local
```

The container uses the same genesis and local development engine, exposes
HTTP JSON-RPC on `0.0.0.0:8545`, and stores node data in `/data`.

For Compose-managed startup and persistent chain data:

```sh
./start-node.sh
```

By default, JSON-RPC is published only on `127.0.0.1:8545`. Set
`RUSTEVM_BIND_ADDRESS=0.0.0.0` to expose it on all host interfaces, or set
`RUSTEVM_HTTP_PORT` to change the host port.

To stop the node and permanently clear its chain data:

```sh
./reset.sh
```

The reset script requires typing `reset` before it removes the Compose volume.
For a normal stop that preserves chain data, run
`docker compose -f docker-compose.yml down`.
