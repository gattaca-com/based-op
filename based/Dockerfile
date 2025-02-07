FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

RUN apt-get update && apt-get install -y clang

FROM chef AS planner
COPY . .
RUN --mount=from=reth,target=/reth cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=from=reth,target=/reth cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=from=reth,target=/reth cargo build --release --bin based-portal


FROM debian:stable-slim AS runtime
WORKDIR /app

RUN apt-get update
RUN apt-get install -y openssl ca-certificates libssl3 libssl-dev

COPY --from=builder /app/target/release/based-portal /usr/local/bin
ENTRYPOINT ["/usr/local/bin/based-portal"]
