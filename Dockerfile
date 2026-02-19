FROM rust:1.91.1-slim AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY .cargo ./.cargo

RUN rustup target add x86_64-unknown-linux-musl \
 && apt-get update && apt-get install -y --no-install-recommends musl-tools \
 && cargo build --release --target x86_64-unknown-linux-musl \
 && strip target/x86_64-unknown-linux-musl/release/confctl

FROM scratch

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/confctl /confctl

ENTRYPOINT ["/confctl"]
