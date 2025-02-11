.PHONY: deps run clean restart help \
		build build-portal build-op-node build-op-geth \
		logs op-node-logs op-geth-logs \
		test-frag test-seal

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

build: build-portal build-op-node build-op-geth ## 🏗️ Build

build-portal:
	docker build -t based_portal_local --build-context reth=./reth ./based

build-op-node: ## 🏗️ Build OP node from optimism directory
	cd optimism && \
	IMAGE_TAGS=develop \
	PLATFORMS="linux/arm64" \
	docker buildx bake \
	-f docker-bake.hcl \
	--set op-node.tags=based_op_node \
	op-node

build-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t based_op_geth ./op-geth

run: ## 🚀 Run
	kurtosis run optimism-package --args-file config.yml --enclave based-op

run-follower: ## 🚀 Run a single follower node with RPC enabled.
	kurtosis run optimism-package --args-file config-geth-cluster.yml --enclave based-op

logs: ## 📜 Show logs
	kurtosis service logs -f based-op $(SERVICE)

dump:
	bash -c 'kurtosis files download based-op $$(kurtosis enclave inspect based-op | grep op-deployer-configs | awk "{print \$$1}") ./genesis'

gateway:
	RUST_LOG=debug cargo run --manifest-path ./based/Cargo.toml --profile=release-with-debug --bin bop-gateway -- \
	--db.datadir ./data \
	--rpc.fallback_url http://127.0.0.1:$(OP_EL_PORT) \
	--chain-spec ./genesis/genesis-2151908.json \
	--rpc.port 9997 \
	--gossip.root_peer_url http://127.0.0.1:$(BOP_NODE_PORT) \
	--test

based-portal-logs:
	$(MAKE) logs SERVICE=op-based-portal-1-op-kurtosis

op-node-logs:
	$(MAKE) logs SERVICE=op-cl-1-op-node-op-geth-op-kurtosis

op-geth-logs:
	$(MAKE) logs SERVICE=op-el-1-op-geth-op-node-op-kurtosis

clean: ## 🧹 Clean
	rm -rf ./genesis && kurtosis enclave rm  based-op --force

restart: clean dump run ## 🔄 Restart

# Testing

FOLLOWER_NODE_HOST?=http://localhost
BLOCK_NUMBER?=$(shell echo $$(( $$(cast block-number --rpc-url http://localhost:$(BOP_EL_PORT)) + 1 )))
DUMMY_RICH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DUMMY_TX=$(shell cast mktx --rpc-url  $(FOLLOWER_NODE_HOST):$(BOP_EL_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a | xxd -r -p | base64)

test-frag:
	curl --request POST   --url $(FOLLOWER_NODE_HOST):$(BOP_NODE_PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_newFrag", \
		"params": [ \
			{ \
				"signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
				"frag": { \
					"blockNumber": $(BLOCK_NUMBER), \
					"seq": $(SEQ), \
					"isLast": false, \
					"txs": ["$(DUMMY_TX)"], \
					"version": 0 \
				} \
			} \
		] \
	}'

test-seal:
	curl --request POST   --url $(FOLLOWER_NODE_HOST):$(BOP_NODE_PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_sealFrag", \
		"params": [ \
			{ \
				"signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
				"seal": { \
					"totalFrags": 2, \
					"blockNumber": $(BLOCK_NUMBER), \
					"gasUsed": 0, \
					"gasLimit": 0, \
					"parentHash": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"transactionsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"receiptsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"stateRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"blockHash": "0x1234567890123456789012345678901234567890123456789012345678901234" \
				} \
			} \
		] \
	}'

test-env:
	curl --request POST   --url $(FOLLOWER_NODE_HOST):$(BOP_NODE_PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_env", \
		"params": [ \
			{ \
				"signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
				"env": { \
					"totalFrags": 2, \
					"number": $(BLOCK_NUMBER), \
					"beneficiary": "0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a", \
					"timestamp": 2739281173, \
					"gasLimit": 0, \
					"baseFee": 0, \
					"difficulty": 0, \
					"prevrandao": "0x1234567890123456789012345678901234567890123456789012345678901234" \
				} \
			} \
		] \
	}'
