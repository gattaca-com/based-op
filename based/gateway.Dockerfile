FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

RUN apt-get update && apt-get install -y clang

FROM chef AS planner
COPY . .
RUN --mount=from=reth,target=/reth cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=from=reth,target=/reth cargo chef cook --recipe-path recipe.json

COPY . .
RUN --mount=from=reth,target=/reth cargo build --bin bop-gateway


FROM debian:stable-slim AS runtime
WORKDIR /app

RUN apt-get update
RUN apt-get install -y openssl ca-certificates libssl3 libssl-dev

COPY --from=builder /app/target/debug/bop-gateway /usr/local/bin
ENTRYPOINT ["/usr/local/bin/bop-gateway"]
