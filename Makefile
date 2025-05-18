.PHONY: deps run clean restart help \
build build-portal build-op-node build-op-geth \
logs op-node-logs op-geth-logs \
test-frag test-seal \
docs

# ──────────────────────────────────────────────
# Cross-platform networking shim
# ──────────────────────────────────────────────
OS := $(shell uname -s)

.PHONY: fix-compose
fix-compose:           ## normalise compose.yml for macOS & Linux
ifeq ($(OS),Darwin)    # Docker Desktop ignores network_mode:host  publish the port instead
	@sed -i.bak -e '/network_mode: *"host"/c\
\  ports:\n\    - "$${PORTAL_HOST_PORT:-8080}:8080"\n\  extra_hosts:\n\    - "host.docker.internal:host-gateway"' \
	      .local_main_node/compose.yml
endif

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

build: build-portal build-gateway build-op-node build-op-geth build-registry build-overseer## 🏗️ Build

build-no-gateway: build-portal build-op-node build-op-geth ## 🏗️ Build without gateway

build-portal: ## 🏗️ Build based portal
	docker build -t based_portal_local -f ./based/portal.Dockerfile --build-context reth=./reth ./based

build-registry: ## 🏗️ Build based registry
	docker build -t based_registry_local -f ./based/registry.Dockerfile --build-context reth=./reth ./based

build-gateway: ## 🏗️ Build based gateway
	docker build -t based_gateway_local -f ./based/gateway.Dockerfile --build-context reth=./reth ./based

build-key_to_address: ## 🏗️ Build based gateway from based directory
	docker build -t key_to_address -f ./based/key_to_address.Dockerfile --build-context reth=./reth ./based
build-overseer: ## 🏗️ Build based overseer
	docker build -t based_overseer_local -f ./based/overseer.Dockerfile --build-context reth=./reth ./based

build-follower-op-node: ## 🏗️ Build OP node from optimism directory
	cd ../based-optimism && \
	IMAGE_TAGS=develop \
	docker buildx bake \
	-f docker-bake.hcl \
	--set op-node.tags=based_op_node \
	--load \
	op-node

build-follower-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t based_op_geth ../based-op-geth

build-rabby-chrom: ## 🏗️ Build modified Rabby wallet for Google Chrome and Firefox
	cd rabby && \
		yarn && \
		yarn build:pro && \
		yarn build:pro:mv2


ifeq ($(filter start-gateway,$(MAKECMDGOALS)),start-gateway)
  ifeq ($(strip $(PORTAL)),)
    $(error PORTAL is undefined! \
           Please invoke like `make start-gateway \
           PORTAL=http://… GATEWAY_SEQUENCING_KEY=…`)
  endif
  ifeq ($(strip $(GATEWAY_SEQUENCING_KEY)),)
    $(error GATEWAY_SEQUENCING_KEY is undefined! \
           Please invoke like `make start-gateway \
           PORTAL=http://… GATEWAY_SEQUENCING_KEY=…`)
  endif
endif
BASED_GATEWAY_DATA_DIR?=.local_gateway_and_follower/data/gateway
BASED_OP_NODE_DATA_DIR?=.local_gateway_and_follower/data/node
BASED_OP_GETH_DATA_DIR?=.local_gateway_and_follower/data/geth
start-gateway: build-follower-op-node build-follower-op-geth build-gateway build-key_to_address build-overseer
	@if docker ps --format '{{.Names}}' | grep -wq based-op-gateway ; then \
		echo "❌  Gateway already running."; \
		exit 1; \
	fi
	@mkdir -p .local_gateway_and_follower/config

	@# generate jwt if missing
	@if [ ! -f .local_gateway_and_follower/config/jwt ]; then \
	  openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' > .local_gateway_and_follower/config/jwt; \
	fi

	@# generate .env and fetch JSON if missing
	@if [ ! -f .local_gateway_and_follower/.env ]; then \
	  cp follower_node/env_example .local_gateway_and_follower/.env; \
	  cp follower_node/compose.yml .local_gateway_and_follower/compose.yml; \
	  echo "Initializing gateway and follower op-node in ./.local_gateway_and_follower ..."; \
	  { \
	    echo "PORTAL=$(PORTAL)"; \
	    echo "GATEWAY_SEQUENCING_KEY=$(GATEWAY_SEQUENCING_KEY)"; \
	    echo "MAIN_OP_NODE_GOSSIP_STATIC=$$(curl -s -X POST -H 'Content-Type: application/json' \
	      --data '{"jsonrpc":"2.0","method":"portal_opNodeGossipStatic","params":[],"id":1}' \
	      $(PORTAL) | docker run --rm -i imega/jq -r '.result')"; \
	    echo "MAIN_OP_NODE_ENR=$$(curl -s -X POST -H 'Content-Type: application/json' \
	      --data '{"jsonrpc":"2.0","method":"portal_opNodeBootnodeEnr","params":[],"id":1}' \
	      $(PORTAL) | docker run --rm -i imega/jq -r '.result')"; \
	    echo "MAIN_OP_GETH_ENODE=$$(curl -s -X POST -H 'Content-Type: application/json' \
	      --data '{"jsonrpc":"2.0","method":"portal_opGethBootnodeEnode","params":[],"id":1}' \
	      $(PORTAL) | docker run --rm -i imega/jq -r '.result')"; \
	    echo "NETWORK_ID=$$(curl -s -X POST -H 'Content-Type: application/json' \
	      --data '{"jsonrpc":"2.0","method":"portal_l2ChainId","params":[],"id":1}' \
	      $(PORTAL) | docker run --rm -i imega/jq -r '.result')"; \
	  } >> .local_gateway_and_follower/.env; \
	  \
	  curl -s -X POST -H "Content-Type: application/json" \
	    --data '{"jsonrpc":"2.0","method":"portal_fileRollup","params":[],"id":1}' \
	    $(PORTAL) | docker run --rm -i imega/jq -r '.result' > .local_gateway_and_follower/config/rollup.json; \
	  curl -s -X POST -H "Content-Type: application/json" \
	    --data '{"jsonrpc":"2.0","method":"portal_fileGenesis","params":[],"id":1}' \
	    $(PORTAL) | docker run --rm -i imega/jq -r '.result' > .local_gateway_and_follower/config/genesis.json; \
	else \
	  NEW_GOSSIP=$$(curl -s -X POST -H 'Content-Type: application/json' \
	    --data '{"jsonrpc":"2.0","method":"portal_opNodeGossipStatic","params":[],"id":1}' \
	    $(PORTAL) | docker run --rm -i imega/jq -r '.result'); \
	  NEW_ENR=$$(curl -s -X POST -H 'Content-Type: application/json' \
	    --data '{"jsonrpc":"2.0","method":"portal_opNodeBootnodeEnr","params":[],"id":1}' \
	    $(PORTAL) | docker run --rm -i imega/jq -r '.result'); \
	  NEW_GETH=$$(curl -s -X POST -H 'Content-Type: application/json' \
	    --data '{"jsonrpc":"2.0","method":"portal_opGethBootnodeEnode","params":[],"id":1}' \
	    $(PORTAL) | docker run --rm -i imega/jq -r '.result'); \
	  sed -i -E \
	    -e "s#^MAIN_OP_NODE_GOSSIP_STATIC=.*#MAIN_OP_NODE_GOSSIP_STATIC=$${NEW_GOSSIP}#" \
	    -e "s#^MAIN_OP_NODE_ENR=.*#MAIN_OP_NODE_ENR=$${NEW_ENR}#" \
	    -e "s#^MAIN_OP_GETH_ENODE=.*#MAIN_OP_GETH_ENODE=$${NEW_GETH}#" \
	    .local_gateway_and_follower/.env; \
	fi
	@mkdir -p .local_gateway_and_follower/data
	@if [ "$(BASED_OP_GETH_DATA_DIR)" != ".local_gateway_and_follower/data/geth" ] && [ -d "$(BASED_OP_GETH_DATA_DIR)" ] && [ ! -d ".local_gateway_and_follower/data/geth" ]; then \
	    ln -s $(BASED_OP_GETH_DATA_DIR) .local_gateway_and_follower/data/geth; \
	else \
	    mkdir -p $(BASED_OP_GETH_DATA_DIR); \
	fi
	@if [ "$(BASED_OP_NODE_DATA_DIR)" != ".local_gateway_and_follower/data/node" ] && [ -d "$(BASED_OP_NODE_DATA_DIR)" ] && [ ! -d ".local_gateway_and_follower/data/node" ]; then \
	    ln -s $(BASED_OP_NODE_DATA_DIR) .local_gateway_and_follower/data/node; \
	else \
	    mkdir -p $(BASED_OP_NODE_DATA_DIR); \
	fi
	@if [ "$(BASED_GATEWAY_DATA_DIR)" != ".local_gateway_and_follower/data/gateway" ] && [ -d "$(BASED_GATEWAY_DATA_DIR)" ] && [ ! -d ".local_gateway_and_follower/data/gateway" ]; then \
	    ln -s $(BASED_GATEWAY_DATA_DIR) .local_gateway_and_follower/data/gateway; \
	else \
	    mkdir -p $(BASED_GATEWAY_DATA_DIR); \
	fi

	@wallet=$$(docker run -i key_to_address $(GATEWAY_SEQUENCING_KEY)); \
      echo "...Done"; \
      echo; \
      echo "Starting with the following generated .env:"; \
      cat .local_gateway_and_follower/.env; \
      echo; echo; \
      echo "Calling registerGateway method via JSON-RPC:"; \
      GATEWAY_URL=http://$$(curl -s ifconfig.me):$$(grep -m1 '^GATEWAY_PORT[[:space:]]*=' .local_gateway_and_follower/.env | cut -d= -f2); \
      GATEWAY_ADDRESS=$$wallet; \
      JWT=$$(cat .local_gateway_and_follower/config/jwt); \
      curl -X POST $$PORTAL \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\", \
             \"method\":\"registry_registerGateway\", \
             \"params\":[ \
               [\"$$GATEWAY_URL\", \"$$GATEWAY_ADDRESS\", \"$$JWT\"] \
             ], \
             \"id\":1}"; \
      echo; echo

	@cd .local_gateway_and_follower && docker compose up -d
	$(MAKE) start-overseer


ifeq ($(filter start-overseer,$(MAKECMDGOALS)),start-gateway)
  ifeq ($(strip $(PORTAL)),)
    $(error PORTAL is undefined! \
           Please invoke like `make start-gateway \
           PORTAL=http://… GATEWAY_SEQUENCING_KEY=…`)
  endif
endif
start-overseer: build-overseer
	docker exec -it based-op-gateway overseer --portal-url $(PORTAL)

L1_CHAIN_ID?=11155111
L2_CHAIN_ID?=$(shell \
    RAW=$$(od -An -N2 -tu2 /dev/urandom | tr -d ' '); \
    echo $$((RAW % 50000 + 1)); \
)
L2_CHAIN_ID_HEX := $(shell printf "0x%064x" $(L2_CHAIN_ID))
L1_RPC_URL?=https://ethereum-sepolia-rpc.publicnode.com
L1_BEACON_RPC_URL?=https://ethereum-sepolia-beacon-api.publicnode.com


# ────────────────────────────────────────────────────────────────────────────────
# Only perform these parse-time checks if the user asked for deploy-chain
# or start-main-node on the command line.
# ────────────────────────────────────────────────────────────────────────────────
ifneq ($(filter deploy-chain start-main-node,$(MAKECMDGOALS)),)

ifndef OP_PROPOSER_KEY
$(error OP_PROPOSER_KEY is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) OP_BATCHER_KEY=… OP_PROPOSER_KEY=… MAIN_KEY=…`)
endif

ifndef MAIN_KEY
$(error MAIN_KEY is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) OP_BATCHER_KEY=… OP_PROPOSER_KEY=… MAIN_KEY=…`)
endif

ifndef OP_BATCHER_KEY
$(error OP_BATCHER_KEY is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) OP_BATCHER_KEY=… OP_PROPOSER_KEY=… MAIN_KEY=…`)
endif

endif
# ────────────────────────────────────────────────────────────────────────────────

deploy-chain: build-key_to_address
	@echo "Deploying new Chain with id: $(L2_CHAIN_ID)"
	@if [ -d .local_main_node/config ]; then \
		echo "❌  Seems like information of a previous chain is already present. Please remove .local_main_node to deploy a new one."; \
		exit 1; \
	fi
	@mkdir -p .local_main_node/config
	@docker run -v $$(pwd)/.local_main_node/config:/config --entrypoint sh  --rm us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.0.11 -c "/op-deployer init --l1-chain-id $(L1_CHAIN_ID) --l2-chain-ids $(L2_CHAIN_ID) --workdir /config && chmod 666 /config/*"
	@wallet_batcher=$$(docker run -i key_to_address $(OP_BATCHER_KEY)); \
	wallet_proposer=$$(docker run -i key_to_address $(OP_PROPOSER_KEY)); \
	wallet_main=$$(docker run -i key_to_address $(MAIN_KEY)); \
	sed -E \
		  -e "s@L1_CHAIN_ID@$(L1_CHAIN_ID)@g" \
		  -e "s@L2_CHAIN_ID@$(L2_CHAIN_ID_HEX)@g" \
		  -e "s@VAULT_WALLET@$${wallet_main}@g" \
		  -e "s@OP_BATCHER_WALLET@$${wallet_batcher}@g" \
		  -e "s@OP_PROPOSER_WALLET@$${wallet_proposer}@g" \
		  main_node/intent.template.toml \
		  > .local_main_node/config/intent.toml

	@docker run -v $$(pwd)/.local_main_node/config:/config --rm us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.0.11 apply --workdir /config --l1-rpc-url $(L1_RPC_URL) --private-key $(MAIN_KEY) 
	@docker run -v $$(pwd)/.local_main_node/config:/config --rm us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.2.0 inspect genesis --workdir /config $(L2_CHAIN_ID_HEX) > $$(pwd)/.local_main_node/config/genesis.json
	@docker run -v $$(pwd)/.local_main_node/config:/config --rm us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.2.0 inspect rollup --workdir /config $(L2_CHAIN_ID_HEX) > $$(pwd)/.local_main_node/config/rollup.json
	@docker run -v $$(pwd)/.local_main_node/config:/config --entrypoint sh --rm us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.2.0 -c "chmod 666 /config/*"
	@docker run -v $$(pwd)/.local_main_node/config:/config --rm -i imega/jq '.chain_op_config = {"eip1559Elasticity":6, "eip1559Denominator":50, "eip1559DenominatorCanyon":250}' /config/rollup.json \
    > $$(pwd)/.local_main_node/config/rollup.json.tmp && mv $$(pwd)/.local_main_node/config/rollup.json.tmp $$(pwd)/.local_main_node/config/config.json
	@blockNumber=$$(docker run -v $$(pwd)/.local_main_node/config:/config -i imega/jq -r '.genesis.l1.number' /config/rollup.json); \
	 hex=$$(printf "0x%x" $$blockNumber); \
	 hash=$$(curl -s -X POST -H 'Content-Type: application/json' \
	   --data '{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["'"$$hex"'",false]}' \
	   $(L1_RPC_URL) | docker run -i imega/jq -r '.result.hash'); \
	 docker run -v $$(pwd)/.local_main_node/config:/config --rm -i imega/jq --arg h "$$hash" '.genesis.l1.hash = $$h' /config/rollup.json > $$(pwd)/.local_main_node/config/rollup.json.tmp && mv $$(pwd)/.local_main_node/config/rollup.json.tmp $$(pwd)/.local_main_node/config/rollup.json

	@openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' > .local_main_node/config/jwt
	@echo "...Done deploying. See chain config in"
	@echo ".local_main_node/config" 
	@echo
	@echo "start sequencing the chain using"
	@echo "make start-main-node OP_BATCHER_KEY=<private-key matching batcher address> OP_PROPOSER_KEY=<private-key matching proposer address> MAIN_KEY=<op-node sequencer private key>"
	@echo
	@echo

# ────────────────────────────────────────────────────────────────────────────────
# Only perform these parse-time checks if the user asked for config-main-node
# ────────────────────────────────────────────────────────────────────────────────
ifneq ($(filter config-main-node,$(MAKECMDGOALS)),)

ifndef ROLLUP_JSON
$(error ROLLUP_JSON is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) ROLLUP_JSON=… GENESIS_JSON=… STATE_JSON=… OP_GETH_DATA_DIR=… OP_NODE_DATA_DIR=…`)
endif

ifndef GENESIS_JSON
$(error ROLLUP_JSON is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) ROLLUP_JSON=… GENESIS_JSON=… STATE_JSON=… OP_GETH_DATA_DIR=… OP_NODE_DATA_DIR=…`)
endif

ifndef STATE_JSON
$(error STATE_JSON is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) ROLLUP_JSON=… GENESIS_JSON=… STATE_JSON=… OP_GETH_DATA_DIR=… OP_NODE_DATA_DIR=…`)
endif

ifndef OP_GETH_DATA_DIR
$(error  OP_GETH_DATA_DIR is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) ROLLUP_JSON=… GENESIS_JSON=… STATE_JSON=… OP_GETH_DATA_DIR=… OP_NODE_DATA_DIR=…`)
endif

ifndef OP_NODE_DATA_DIR
$(error  OP_NODE_DATA_DIR is undefined!  Please invoke like \
    `make $(MAKECMDGOALS) ROLLUP_JSON=… GENESIS_JSON=… STATE_JSON=… OP_GETH_DATA_DIR=… OP_NODE_DATA_DIR=…`)
endif

endif
# ────────────────────────────────────────────────────────────────────────────────
config-main-node:
	@if [ -d .local_main_node/config ]; then \
		echo "❌  Seems like the main node was already configured (see .local_main_node/config)."; \
		exit 1; \
	fi
	@mkdir -p .local_main_node/config
	@mkdir -p .local_main_node/data
	@openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' > .local_main_node/config/jwt
	@cp $(ROLLUP_JSON) .local_main_node/config
	@cp $(GENESIS_JSON) .local_main_node/config
	@cp $(STATE_JSON) .local_main_node/config
	@if [ "$(OP_GETH_DATA_DIR)" != ".local_main_node/data/geth" ] && [ ! -d ".local_main_node/data/geth" ]; then \
	    ln -s $(OP_GETH_DATA_DIR) .local_main_node/data/geth; \
	else \
	    mkdir -p $(BASED_OP_GETH_DATA_DIR); \
	fi
	@if [ "$(OP_NODE_DATA_DIR)" != ".local_main_node/data/node" ] && [ ! -d ".local_main_node/data/node" ]; then \
	    ln -s $(OP_NODE_DATA_DIR) .local_main_node/data/node; \
	else \
	    mkdir -p $(OP_NODE_DATA_DIR); \
	fi
	@echo "...Done initializing .local_main_node" 
	@echo "dir structure is:"
	@ls -la .local_main_node
	@echo
	@echo "start sequencing the chain using"
	@echo "make start-main-node OP_BATCHER_KEY=<private-key matching batcher address> OP_PROPOSER_KEY=<private-key matching proposer address> MAIN_KEY=<op-node sequencer private key>"
	@echo
	@echo

# By default these will be pointing to directories under .local_<xyz>
start-main-node: build-portal build-registry
	@if docker ps --format '{{.Names}}' | grep -wq op-node ; then \
		echo "❌  Main node already running."; \
		exit 1; \
	fi
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not exist. run make config-main-node to configure, or first run make deploy-chain to deploy a new sepolia chain."; \
		exit 1; \
	fi
	@# generate jwt if missing
	@if [ ! -f .local_main_node/config/jwt ]; then \
	  openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' > .local_main_node/config/jwt; \
	fi
	@# generate .env and fetch JSON if missing
	@if [ ! -f .local_main_node/.env ]; then \
	  cp main_node/env_example .local_main_node/.env; \
	  cp main_node/compose.yml .local_main_node/compose.yml; \
	  $(MAKE) fix-compose; \
	  echo "Initializing all components of a main sequencing node in ./.local_main_node ..."; \
	  { \
	    echo "DISPUTE_GAME_FACTORY_ADDRESS=$$(docker run -v $$(pwd)/.local_main_node/config:/config -i --rm imega/jq -r '.implementationsDeployment.disputeGameFactoryImplAddress' /config/state.json)"; \
	    echo "NETWORK_ID=$$(docker run -v $$(pwd)/.local_main_node/config:/config -i --rm imega/jq -r '.l2_chain_id' /config/rollup.json)"; \
	    echo "L1_RPC_URL=$(L1_RPC_URL)"; \
	    echo "L1_BEACON_RPC_URL=$(L1_BEACON_RPC_URL)"; \
	    echo "OP_NODE_SEQUENCER_KEY=$(MAIN_KEY)"; \
	    echo "OP_NODE_GOSSIP_IP=$$(curl ifconfig.me)"; \
	    echo "OP_BATCHER_PRIVATE_KEY=$(OP_BATCHER_KEY)"; \
	    echo "OP_PROPOSER_PRIVATE_KEY=$(OP_PROPOSER_KEY)"; \
	  } >> .local_main_node/.env; \
	fi
	@if [ ! -f .local_main_node/config/registry.json ]; then \
		echo "[]" > .local_main_node/config/registry.json; \
	fi
	@echo "...Done"
	@echo
	@echo "Starting with the following generated .env:"
	@cat .local_main_node/.env
	@echo
	@echo

	@cd .local_main_node && docker compose up -d
	$(MAKE) logs-main-node

start-portal: build-portal
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	$(MAKE) fix-compose
	docker compose -f .local_main_node/compose.yml up -d based-portal 
	$(MAKE) logs-portal
	
start-registry: build-registry
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	$(MAKE) fix-compose
	docker compose -f .local_main_node/compose.yml up -d based-registry 
	$(MAKE) logs-registry
	
stop-gateway:
	cd .local_gateway_and_follower && docker compose down

stop-main-node:
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_main_node/compose.yml down

stop-portal:
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_main_node/compose.yml down based-portal

stop-registry:
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_main_node/compose.yml down based-registry

logs-main-node:
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_main_node/compose.yml logs --tail 100 -f

logs-follower-node:
	@if [ ! -d .local_gateway_and_follower/config ]; then \
		echo ".local_gateway_and_follower/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_gateway_and_follower/compose.yml logs --tail 100 -f

logs-portal: ## 📜 Show portal logs
	docker logs based-portal --tail 100 -f

logs-registry: ## 📜 Show registry logs
	docker logs based-registry --tail 100 -f

logs-gateway: ## 📜 Show gateway logs
	docker logs based-op-gateway --tail 100 -f
	
logs-op-node: ## 📜 Show main op-node logs (when on main box)
	docker logs op-node --tail 100 -f
	
logs-op-geth: ## 📜 Show main op-geth logs (when on main box)
	docker logs op-geth --tail 100 -f
	
logs-batcher:
	docker logs op-batcher --tail 100 -f
	
logs-proposer:
	docker logs op-proposer --tail 100 -f 


FOLLOWER_NODE_HOST?=http://localhost
BLOCK_NUMBER?=$(shell echo $$(( $$(cast block-number --rpc-url http://localhost:$(BOP_EL_PORT)) + 1 )))
DUMMY_RICH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DUMMY_TX=$(shell cast mktx --rpc-url  $(FOLLOWER_NODE_HOST):$(BOP_EL_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a | xxd -r -p | base64)
PORTAL_PORT?=8080

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
	docker run --rm --network kt-based-op -p 8545:8545 cars10/simprox simprox --skip-ssl-verify=true -l 127.0.0.1:8545 -t op-el-2-op-geth-op-node-op-kurtosis:8545

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
