.PHONY: deps run clean restart help \
		build build-portal build-op-node build-op-geth \
		logs op-node-logs op-geth-logs \
		test-frag test-seal

.DEFAULT_GOAL := help

# Variables

# The following port variables are:
#
# - OP_EL_PORT: This is the port of the Sequencer's OP-Node.
# - BOP_NODE_PORT: This is the port of the Follower's BOP-Node.
# - BOP_EL_PORT: This is the port of the Follower's BOP-Node.
#
# Note: The Kurtosis enclave must be running for these to work.
OP_EL_PORT=$(shell kurtosis service inspect based-op op-el-1-op-reth-op-node-op-kurtosis | grep 'rpc: 8545/tcp -> http://127.0.0.1:' | cut -d : -f 4)
BOP_NODE_PORT=$(shell kurtosis service inspect based-op op-cl-2-op-node-op-geth-op-kurtosis | grep ' http: 8547/tcp -> http://127.0.0.1:' | cut -d : -f 4)
BOP_EL_PORT=$(shell kurtosis service inspect based-op op-el-2-op-geth-op-node-op-kurtosis | grep 'rpc: 8545/tcp -> http://127.0.0.1:' | cut -d : -f 4)
PORTAL_PORT=$(shell kurtosis service inspect based-op op-based-portal-1-op-kurtosis | grep 'rpc: 8541/tcp -> http://127.0.0.1:' | cut -d : -f 4)

# Recipes

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
	if [[ "$$(uname -m)" == "arm"* ]]; then \
		docker pull --platform=linux/amd64 ghcr.io/blockscout/smart-contract-verifier:v1.9.0; \
	fi

build: build-portal build-gateway build-op-node build-op-geth ## 🏗️ Build

build-no-gateway: build-portal build-op-node build-op-geth ## 🏗️ Build without gateway
build-portal: ## 🏗️ Build based portal from based directory
	docker build -t based_portal_local -f ./based/portal.Dockerfile --build-context reth=./reth ./based

build-gateway: ## 🏗️ Build based gateway from based directory
	docker build -t based_gateway_local -f ./based/gateway.Dockerfile --build-context reth=./reth ./based

build-op-node: ## 🏗️ Build OP node from optimism directory
	cd optimism && \
	IMAGE_TAGS=develop \
	docker buildx bake \
	-f docker-bake.hcl \
	--set op-node.tags=based_op_node \
	op-node

build-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t based_op_geth ./op-geth

build-rabby-chrom: ## 🏗️ Build modified Rabby wallet for Google Chrome and Firefox
	cd rabby && \
		yarn && \
		yarn build:pro && \
		yarn build:pro:mv2

run: ## 🚀 Run
	kurtosis run optimism-package --args-file config.yml --enclave based-op && $(MAKE) dump

run-maxgas: ## 🚀 Run
	kurtosis run optimism-package --args-file config_maxgas.yml --enclave based-op && $(MAKE) dump

run-multiple: ## 🚀 Run
	kurtosis run optimism-package --args-file config_multiple_gateways.yml --enclave based-op && $(MAKE) dump

restart-no-gateway: clean build-no-gateway run ## rip rebuild run

run-follower: build-op-node build-op-geth ## 🚀 Run a single follower node with RPC enabled.
	cd follower-node && \
		docker compose -f compose.yml up -d

logs: ## 📜 Show logs
	kurtosis service logs -f based-op $(SERVICE)

dump:
	bash -c 'kurtosis files download based-op $$(kurtosis enclave inspect based-op | grep op-deployer-configs | awk "{print \$$1}") ./genesis'

gateway: ## 🚀 Run the gateway
	cargo run --manifest-path ./based/Cargo.toml --profile=release-with-debug --bin bop-gateway --features shmem -- \
	--db.datadir $(datadir) \
	--rpc.fallback_url http://127.0.0.1:$(OP_EL_PORT) \
	--chain ./genesis/genesis-2151908.json \
	--rpc.port $(port) \
	--gossip.root_peer_url http://127.0.0.1:$(BOP_NODE_PORT)


batcher-logs:
	$(MAKE) logs SERVICE=op-batcher-op-kurtosis

portal-logs:
	$(MAKE) logs SERVICE=op-based-portal-1-op-kurtosis

gateway-logs:
	$(MAKE) logs SERVICE=gateway-1-gateway-op-kurtosis

op-node-logs:
	$(MAKE) logs SERVICE=op-cl-1-op-node-op-reth-op-kurtosis

op-geth-logs:
	$(MAKE) logs SERVICE=op-el-2-op-geth-op-node-op-kurtosis

clean: ## 🧹 Clean
	rm -rf ./genesis && kurtosis enclave rm  based-op --force && rm -rf ./data

restart: clean run ## 🔄 Restart

# Testing

FOLLOWER_NODE_HOST?=http://localhost
BLOCK_NUMBER?=$(shell echo $$(( $$(cast block-number --rpc-url http://localhost:$(BOP_EL_PORT)) + 1 )))
DUMMY_RICH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DUMMY_TX=$(shell cast mktx --rpc-url  $(FOLLOWER_NODE_HOST):$(BOP_EL_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a | xxd -r -p | base64)

test-tx:
	cast send --rpc-url  http://127.0.0.1:$(PORTAL_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a

test-frag:
	curl --request POST   --url $(FOLLOWER_NODE_HOST):$(BOP_NODE_PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_newFrag", \
		"params": [ \
			{ \
				"signature": "0xa47da12abd5563f45332e637d1de946c3576902a245511d86826743c8af1f1e2093d4f5efd5b9630c0acc5f2bb23f236b4f7bdbe0d21d281b2bd2ff60c6cf1861b",  \
				"message": { \
					"blockNumber": $(BLOCK_NUMBER), \
					"seq": $(SEQ), \
					"isLast": true, \
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
				"signature": "0x090f69ccf02e0f468cac96f71bbf4b7732c63f3d50a4881f8665c1718570928e4497706eac2fe7da8b47ce355482ada8763614a3575a1af066ad06320b707c531b",  \
				"message": { \
					"totalFrags": 8, \
					"blockNumber": $(BLOCK_NUMBER), \
					"gasUsed": 43806, \
					"gasLimit": 60000000, \
					"parentHash": "0x3d0f61f441af7d1640cb15cd7250bae72d8b334e27245ea44b536407892ec57c", \
					"transactionsRoot": "0x783425e75723ac77ea7f0f47fb4a7858f63deceb80137a0e53fa09703f477cc0", \
					"receiptsRoot": "0x6ff8f783179faedd1aef7e55889a1017ec700504ba6bedffd826a28a47b1a5a2", \
					"stateRoot": "0xc6a987cccdd0665f4d38c730dc05fb8b69497d45094b2b3615954686ff765f87", \
					"blockHash": "0xf3b170b6aee95faa665f77ad1ed0efe7bd29553aa2402e35de7ba3ce55d6974f" \
				} \
			} \
		] \
	}'

test-env:
	curl --request POST --url $(FOLLOWER_NODE_HOST):$(BOP_NODE_PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_env", \
		"params": [ \
			{ \
				"signature": "0x4fc733cc2f0b680e15452db40b9453412ccb25507582b192c1ea4fc4deaf709845002ab44af42327ed4b8b12943412810a8d9984ea1609dfc6f77338f8c395b41c",  \
				"message": { \
					"totalFrags": 2, \
					"number": $(BLOCK_NUMBER), \
					"beneficiary": "0x1234567890123456789012345678901234567890", \
					"timestamp": 2739281173, \
					"gasLimit": 3, \
					"baseFee": 4, \
					"difficulty": "0x5", \
					"prevrandao": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
					"parentHash": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
					"parentBeaconRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
					"extraData": "0x010203" \
				} \
			} \
		] \
	}'

follower-node-proxy:
	docker run --rm --network kt-based-op -p 8545:8545 cars10/simprox simprox --skip-ssl-verify=true -l 0.0.0.0:8545 -t op-el-2-op-geth-op-node-op-kurtosis:8545

spam: ## 🚀 Run the gateway
	PORTAL_PORT=$(PORTAL_PORT) BOP_EL_PORT=$(BOP_EL_PORT) cargo test --manifest-path ./based/Cargo.toml --release -- tx_spammer --ignored --nocapture
