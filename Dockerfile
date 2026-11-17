FROM rust:1.96-bookworm AS builder

RUN apt-get update \
    && apt-get install --no-install-recommends -y libclang-dev \
    && rm -rf /var/lib/apt/lists/*

ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY config ./config
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates libgcc-s1 libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /build/target/release/rust-evm /usr/local/bin/rust-evm
COPY config ./config

VOLUME ["/data"]
EXPOSE 8545 8551
ENTRYPOINT ["rust-evm", "node", "--chain", "/app/config/genesis.json", "--datadir", "/data", "--dev", "--http", "--http.addr", "0.0.0.0", "--http.api", "eth,net,web3,txpool"]
