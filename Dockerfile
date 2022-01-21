FROM rust:1.58 as chef
RUN cargo install cargo-chef
WORKDIR app

FROM chef AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer
RUN cargo chef cook --release --recipe-path recipe.json

# Actually build with our source code (not only deps)
COPY . .
RUN cargo build --release

# We do not need the Rust toolchain to run the binary
FROM debian:bullseye-slim AS nft
WORKDIR app
COPY --from=builder /app/target/release/berlin-web .
COPY --from=builder /app/data/ ./data/
ENTRYPOINT ["./berlin-web"]
