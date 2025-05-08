.PHONY: deps run clean restart help \
		build build-portal build-op-node build-op-geth \
		logs op-node-logs op-geth-logs \
		test-frag test-seal \
		docs

.DEFAULT_GOAL := help

# Variables

# The following port variables are:
#
# - OP_EL_PORT: This is the port of the Sequencer's OP-Node.
# - BOP_NODE_PORT: This is the port of the Follower's BOP-Node.
# - BOP_EL_PORT: This is the port of the Follower's BOP-Node.
#
# Note: The Kurtosis enclave must be running for these to work.
#OP_EL_PORT=$(shell kurtosis service inspect based-op op-el-1-op-reth-op-node-op-kurtosis | grep 'rpc: 8545/tcp -> http://127.0.0.1:' | cut -d : -f 4)
#BOP_NODE_PORT=$(shell kurtosis service inspect based-op op-cl-2-op-node-op-geth-op-kurtosis | grep ' http: 8547/tcp -> http://127.0.0.1:' | cut -d : -f 4)
#BOP_EL_PORT=$(shell kurtosis service inspect based-op op-el-2-op-geth-op-node-op-kurtosis | grep 'rpc: 8545/tcp -> http://127.0.0.1:' | cut -d : -f 4)
#PORTAL_PORT=$(shell kurtosis service inspect based-op op-based-portal-1-op-kurtosis | grep 'rpc: 8541/tcp -> http://127.0.0.1:' | cut -d : -f 4)

# Some servers default to executing shell scripts below with /bin/sh, we set bash to make sure our bash syntax works
SHELL := /bin/bash

# Recipes

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

docs: ## 📚 Build local docs
	cd docs && \
	npm i && \
	npm run build && \
	npm run start

build: build-portal build-gateway build-op-node build-op-geth build-registry ## 🏗️ Build

build-no-gateway: build-portal build-op-node build-op-geth ## 🏗️ Build without gateway

build-portal: ## 🏗️ Build based portal from based directory
	docker build -t based_portal_local -f ./based/portal.Dockerfile --build-context reth=./reth ./based

build-registry: ## 🏗️ Build based registry from based directory
	docker build -t based_registry_local -f ./based/registry.Dockerfile --build-context reth=./reth ./based

build-gateway: ## 🏗️ Build based gateway from based directory
	docker build -t based_gateway_local -f ./based/gateway.Dockerfile --build-context reth=./reth ./based

run-main:
	@if [ -f main_node/compose.yml ] && [ -f main_node/config/rollup.json ] && [ -f main_node/config/jwt ] && [ -f main_node/config/genesis.json ] && [ -f main_node/config/registry.json ]; then \
		cd main_node && docker compose up -d; \
	else \
		echo "Missing required files! Look in the main_node subdir. We need: main_node/compose.yml, main_node/config/jwt, main_node/config/rollup.json, main_node/config/genesis.json, main_node/config/registry.json"; \
		exit 1; \
	fi

run-registry:
	@if [ -f main_node/.env ] && [ -f main_node/config/registry.json ]; then \
		cd main_node && docker compose up -d registry; \
	else \
		echo "Missing required files! Look in the main_node subdir. We need: main_node/.env, main_node/config/registry.json"; \
		exit 1; \
	fi

run-portal:
	@if [ -f main_node/.env ] && [ -f main_node/config/jwt ]; then \
		cd main_node && docker compose up -d portal; \
	else \
		echo "Missing required files! Look in the main_node subdir. We need: main_node/.env, main_node/config/jwt"; \
		exit 1; \
	fi

build-follower-op-node: ## 🏗️ Build OP node from optimism directory
	cd optimism && \
	IMAGE_TAGS=develop \
	docker buildx bake \
	-f docker-bake.hcl \
	--set op-node.tags=based_op_node \
	--no-cache \
	--load \
	op-node

build-follower-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t based_op_geth ./op-geth

build-rabby-chrom: ## 🏗️ Build modified Rabby wallet for Google Chrome and Firefox
	cd rabby && \
		yarn && \
		yarn build:pro && \
		yarn build:pro:mv2

run-follower: build-follower-op-node build-follower-op-geth build-gateway## 🚀 Run a single follower node with RPC enabled.
	ifndef PORTAL
		$(error PORTAL is undefined! Please invoke like `make target PORTAL=http://ip_to_portal:8080 GATEWAY_SEQUENCING_KEY=0xyour_private_key_here`)
	endif
	ifndef GATEWAY_SEQUENCING_KEY
		$(error GATEWAY_SEQUENCING_KEY is undefined! Please invoke like `make target PORTAL=http://ip_to_portal:8080 GATEWAY_SEQUENCING_KEY=0xyour_private_key_here`)
	endif
	@if [ ! -d follower_node/config ]; then \
		mkdir follower_node/config; \
	fi \
	@if [ ! -f follower_node/config/jwt ]; then \
	    echo "Generating a new jwt file, please communicate this with the registry operator"; \
		openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' | tee follower_node/config/jwt; \
	fi \
	@if [ ! -f follower_node/.env ]; then \
	    echo "Generating a .env from PORTAL"; \
	    echo "PORTAL=$(PORTAL)" >> follower_node/.env; \
	    echo "GATEWAY_SEQUENCING_KEY=$(GATEWAY_SEQUENCING_KEY)" >> follower_node/.env; \
	    echo "MAIN_OP_NODE_GOSSIP_STATIC=$(curl -s -X POST -H \"Content-Type: application/json\" --data \'{\"jsonrpc\":\"2.0\",\"method\":\"portal_opNodeGossipStatic\",\"params\":[],\"id\":1}\' $PORTAL | jq -r \'.result\')" >> follower_node/.env; \
	    echo "MAIN_OP_NODE_ENR=$(curl -s -X POST -H \"Content-Type: application/json\" --data \'{\"jsonrpc\":\"2.0\",\"method\":\"portal_opNodeBootnodeEnr\",\"params\":[],\"id\":1}\' $PORTAL | jq -r \'.result\')" >> follower_node/.env; \
	    echo "MAIN_OP_GETH_ENODE=$(curl -s -X POST -H \"Content-Type: application/json\" --data \'{\"jsonrpc\":\"2.0\",\"method\":\"portal_opGethBootnodeEnode\",\"params\":[],\"id\":1}\' $PORTAL | jq -r \'.result\')" >> follower_node/.env; \
	    echo "NETWORK_ID=$(curl -s -X POST -H \"Content-Type: application/json\" --data \'{\"jsonrpc\":\"2.0\",\"method\":\"portal_l2ChainId\",\"params\":[],\"id\":1}\' $PORTAL | jq -r \'.result\')" >> follower_node/.env; \
	    echo "MAIN_OP_NODE_ENR=$(curl -s -X POST -H \"Content-Type: application/json\" --data '{\"jsonrpc\":\"2.0\",\"method\":\"portal_opNodeBootnodeEnr\",\"params\":[],\"id\":1}' $PORTAL | jq -r '.result')" >> follower_node/.env; \
	    curl -s -X POST -H \"Content-Type: application/json\" --data '{\"jsonrpc\":\"2.0\",\"method\":\"portal_fileRollup\",\"params\":[],\"id\":1}' $PORTAL | jq -r '.result' >> follower_node/config/rollup.json; \
	    curl -s -X POST -H \"Content-Type: application/json\" --data '{\"jsonrpc\":\"2.0\",\"method\":\"portal_fileGenesis\",\"params\":[],\"id\":1}' $PORTAL | jq -r '.result' >> follower_node/config/genesis.json; \
	fi \
	 
	cd follower_node && \
		docker compose up -d

logs-portal: ## 📜 Show portal logs
	docker logs main_node-portal-1 --tail 100 -f

logs-gateway: ## 📜 Show gateway logs
	docker logs follower_node-gateway-1 --tail 100 -f
	
logs-main-op-node: ## 📜 Show main op-node logs (when on main box)
	docker logs main_node-op-node-1 --tail 100 -f
	
logs-main-op-geth: ## 📜 Show main op-geth logs (when on main box)
	docker logs main_node-op-geth-1 --tail 100 -f
	
logs-follower-op-geth: ## 📜 Show follower op-geth logs (when on running a follower node)
	docker logs follower_node-op-geth-1 --tail 100 -f
	
logs-follower-op-node: ## 📜 Show follower op-node logs (when on running a follower node)
	docker logs follower_node-op-node-1 --tail 100 -f 

logs-batcher:
	docker logs main_node-op-batcher-1 --tail 100 -f
	
logs-proposer:
	docker logs main_node-op-proposer-1 --tail 100 -f 


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

gateway-spam:
	cargo run --manifest-path ./based/Cargo.toml --profile=release --bin bop-gateway --features shmem -- \
	--db.datadir $(datadir) \
	--rpc.fallback_url http://127.0.0.1:$(OP_EL_PORT) \
	--chain ./genesis/genesis-2151908.json \
	--rpc.port $(port) \
	--gossip.root_peer_url http://127.0.0.1:$(BOP_NODE_PORT) \
	--mock Spammer \
	--sequencer.commit_sealed_frags_to_db

gateway-bench:
	cargo run --manifest-path ./based/Cargo.toml --profile=release-with-debug --bin bop-gateway --features shmem -- \
	--db.datadir $(datadir) \
	--rpc.fallback_url http://127.0.0.1:$(OP_EL_PORT) \
	--chain ./genesis/genesis-2151908.json \
	--rpc.port $(port) \
	--gossip.root_peer_url http://127.0.0.1:$(BOP_NODE_PORT) \
	--mock Benchmark 
