FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
ARG TARGETPLATFORM
ARG BUILDPLATFORM
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


FROM alpine:latest AS runtime
WORKDIR /app

RUN apk update && apk add openssl ca-certificates

COPY --from=builder /app/target/release/based-portal /usr/local/bin
ENTRYPOINT ["/usr/local/bin/based-portal"]
