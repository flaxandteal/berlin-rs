FROM rust:1.68 as chef
WORKDIR /app
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

# Build dependencies - this is the caching Docker layer
FROM chef AS deps-builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Actually build with our source code (not only deps)
FROM deps-builder as builder
COPY . .
RUN cargo build --release
