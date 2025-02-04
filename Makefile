.PHONY: deps \
		install-kurtosis \
		build-mux build-gateway build-op-node build-op-geth \
		secrets \
		gateway mux \
		op-geth op-node

.DEFAULT_GOAL := help

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

# Install

deps: ## 🚀 Install all dependencies
	# Kurtosis
	if [[ "$$(uname -s)" == "Darwin" ]]; then \
		xcode-select --install; \
		brew install kurtosis-tech/tap/kurtosis-cli; \
	elif [[ "$$(uname -s)" == "Linux" ]]; then \
		echo "deb [trusted=yes] https://apt.fury.io/kurtosis-tech/ /" | sudo tee /etc/apt/sources.list.d/kurtosis.list; \
		sudo apt update; \
		sudo apt install -y kurtosis-cli; \
	fi
	# Rust
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Build

build: build-mux build-op-node build-op-geth ## 🏗️ Build all binaries

build-mux: ## Locally build the BOP mux binary
	cargo build --release --manifest-path based/Cargo.toml --bin bop-mux

build-gateway: ## Locally build the BOP gateway binary
	cd based/bin/gateway && cargo build --release

build-op-node: ## Locally build OP Node
	make -C optimism/op-node op-node

build-op-geth: ## Locally build OP Geth
	make -C op-geth

# Secrets

SECRETS_DIR=$(HOME)/secrets
SECRETS_PATH=$(SECRETS_DIR)/jwt.hex

secrets: ## 🔐 Generate a new JWT secret
	mkdir -p $(SECRETS_DIR)
	openssl rand -hex 32 > $(SECRETS_PATH)

# Runners

gateway:
	cargo run --release --manifest-path based/bin/gateway/Cargo.toml

mux:
	./based/target/release/bop-mux \
	--mux.port=8541 \
	--fallback.url=http://localhost:9551 \
	--fallback.jwt_path=$(SECRETS_PATH) \
	--gateway.url=http://localhost:8551 \
	--gateway.jwt_path=$(SECRETS_PATH)


op-geth:
	./op-geth/build/bin/geth \
	--http \
	--http.port=8545 \
	--http.addr=localhost \
	--authrpc.addr=localhost \
	--authrpc.jwtsecret=./jwt.txt \
	--verbosity=3 \

op-node:
	./optimism/op-node/bin/op-node \
	--l1 http://localhost:8545 \
	--l1.beacon http://localhost:4000 \
	--l2 http://localhost:9001 \
	--l2.enginekind geth \
	--l2.jwt-secret $(SECRETS_PATH) \
	--p2p.listen.tcp=9222
	--p2p.listen.udp=9222
	--rpc.port=7000 \
	--syncmode=execution-layer
