.PHONY: deps run clean restart help \
		build build-mux build-op-node build-reth \
		logs op-node-logs op-reth-logs \

.DEFAULT_GOAL := help

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

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

build: build-mux build-op-node build-reth ## 🏗️ Build

build-mux:
	docker build  -t bop-mux --build-context reth=./reth ./based

build-op-node: ## 🏗️ Build OP node from optimistic directory
	cd optimism && \
	IMAGE_TAGS=develop \
	PLATFORMS="linux/arm64" \
	docker buildx bake \
	-f docker-bake.hcl \
	op-node

build-reth: ## 🏗️ Build Reth from reth directory
	docker build -t ghcr.io/paradigmxyz/op-reth ./reth

run: ## 🚀 Run
	kurtosis run optimism-package --args-file config.yml --enclave based-op

logs: ## 📜 Show logs
	kurtosis service logs -f based-op $(SERVICE)

rollup-boost-logs:
	$(MAKE) logs SERVICE=op-rollup-boost-1-op-kurtosis

op-node-logs:
	$(MAKE) logs SERVICE=op-cl-1-op-node-op-reth-op-kurtosis

op-reth-logs:
	$(MAKE) logs SERVICE=op-el-1-op-reth-op-node-op-kurtosis

clean: ## 🧹 Clean
	kurtosis enclave rm  based-op --force

restart: clean run ## 🔄 Restart
