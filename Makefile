.PHONY: clean help \
build build-portal build-op-node build-op-geth \
logs op-node-logs op-geth-logs \
test-frag test-seal \
docs 

# ──────────────────────────────────────────────
# Cross-platform networking shim
# ──────────────────────────────────────────────
OS := $(shell uname -s)

.DEFAULT_GOAL := help

# Variables
IMAGE_KEY_TO_ADDRESS:=ghcr.io/gattaca-com/based-op/key-to-address:latest
## This image is totally vanilla, but automatically sets isthmus at genesis when using v3.0.0 contracts
IMAGE_OP_DEPLOYER:=ghcr.io/gattaca-com/based-optimism/based-op-deployer:latest


START_GATEWAY_COMPOSE_FILES := -f .local_gateway_and_follower/compose.yml
START_MAIN_NODE_COMPOSE_FILES := -f .local_main_node/compose.yml
START_MONITORING_COMPOSE_FILES := -f monitoring/compose.yml

# Overridable Variables
L1_CHAIN_ID?=11155111
L2_CHAIN_ID?=$(shell \
    RAW=$$(od -An -N2 -tu2 /dev/urandom | tr -d ' '); \
    echo $$((RAW % 50000 + 1)); \
)
L2_CHAIN_ID_HEX:=$(shell printf "0x%064x" $(L2_CHAIN_ID))
PORTAL?=http://0.0.0.0:8080
TX_PROXY?=http://0.0.0.0:8090
L1_RPC_URL?=http://34.194.193.217:8545
L1_BEACON_RPC_URL?=http://34.194.193.217:5052
PUBLIC_IP?=$(shell curl ifconfig.me)
# if GATEWAY_SEQUENCING_KEY is set, use that one, otherwise key_to_address will generate a new one
GATEWAY_SEQUENCING_KEY ?= $(shell                                    \
  [ -f .local_gateway_and_follower/.env ] &&                        \
  grep -m1 '^GATEWAY_SEQUENCING_KEY=' .local_gateway_and_follower/.env \
    | cut -d= -f2                                                   \
)
_GATEWAY_KEY_AND_WALLET:=$(shell docker run --rm -i $(IMAGE_KEY_TO_ADDRESS) $(GATEWAY_SEQUENCING_KEY))
GATEWAY_SEQUENCING_KEY:=$(word 1,$(_GATEWAY_KEY_AND_WALLET))
GATEWAY_SEQUENCING_ADDRESS:=$(word 2,$(_GATEWAY_KEY_AND_WALLET))

BASED_GATEWAY_DATA_DIR?=.local_gateway_and_follower/data/gateway
BASED_OP_NODE_DATA_DIR?=.local_gateway_and_follower/data/node
BASED_OP_GETH_DATA_DIR?=.local_gateway_and_follower/data/geth

DEPLOYER_CACHE_DIR:=/tmp/op-deployer-cache

# Some servers default to executing shell scripts below with /bin/sh, we set bash to make sure our bash syntax works
SHELL := /bin/bash

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

docs: ## 📚 Build local docs
	cd docs && \
	npm i && \
	npm run build && \
	npm run start

build: build-portal build-gateway build-based-op-node build-based-op-geth build-registry build-txproxy ## 🏗️ Build

build-portal: ## 🏗️ Build based portal
	docker build -t local_based_portal -f ./based/portal.Dockerfile --build-context reth=./reth ./based

build-registry: ## 🏗️ Build based registry
	docker build -t local_based_registry -f ./based/registry.Dockerfile --build-context reth=./reth ./based

build-gateway: ## 🏗️ Build based gateway
	docker build -t local_based_gateway -f ./based/gateway.Dockerfile --build-context reth=./reth ./based

build-txproxy: ## 🏗️ Build based txproxy
	docker build -t local_based_txproxy -f ./based/txproxy.Dockerfile --build-context reth=./reth ./based --load

build-metrics-exporter: ## 🏗️ Build metrics exporter
	docker build -t local_based_metrics_exporter -f ./based/metrics-exporter.Dockerfile --build-context reth=./reth ./based --load

build-based-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t local_based_op_geth ../based-op-geth

build-based-op-node: ## 🏗️ Build OP geth from op-eth directory
	cd ../based-optimism && \
    IMAGE_TAGS=develop \
    docker buildx bake \
    -f docker-bake.hcl \
    --set op-node.tags=local_based_op_node \
    --load \
    op-node

build-based-op-deployer: ## 🏗️ Build OP deployer from op-eth directory
	cd ../based-optimism && \
    IMAGE_TAGS=develop \
    docker buildx bake \
    -f docker-bake.hcl \
    --set op-deployer.tags=local_based_op_deployer \
    --load \
    op-deployer

build-rabby-chrom: ## 🏗️ Build modified Rabby wallet for Google Chrome and Firefox
	cd rabby && \
		yarn && \
		yarn build:pro && \
		yarn build:pro:mv2

create-network:
	docker network inspect based_op_net >/dev/null 2>&1 || docker network create based_op_net

start-based-gateway: create-network
	@if docker ps --format '{{.Names}}' | grep -wq based-gateway ; then \
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
	  cp follower_node/compose* .local_gateway_and_follower; \
	  echo "Initializing gateway and follower op-node in ./.local_gateway_and_follower ..."; \
	  echo "Gateway Sequencing Private Key: $(GATEWAY_SEQUENCING_KEY)"; \
	  echo "Gateway Sequencing Wallet:      $(GATEWAY_SEQUENCING_ADDRESS)"; \
	  { \
	    echo "PORTAL=$(PORTAL)"; \
	    echo "OP_NODE_GOSSIP_IP=$(PUBLIC_IP)"; \
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

	@wallet=$$(docker run --rm -i $(IMAGE_KEY_TO_ADDRESS) $(GATEWAY_SEQUENCING_KEY)); \
      echo "...Done"; \
      echo; \
      echo "Starting with the following generated .env:"; \
      cat .local_gateway_and_follower/.env; \
      echo; echo; \
      echo "Calling registerGateway method via JSON-RPC:"; \
      GATEWAY_URL=http://$(PUBLIC_IP):$$(grep -m1 '^GATEWAY_PORT[[:space:]]*=' .local_gateway_and_follower/.env | cut -d= -f2); \
      GATEWAY_ADDRESS=$$wallet; \
      JWT=$$(cat .local_gateway_and_follower/config/jwt); \
      curl -X POST "$(PORTAL)" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\", \
             \"method\":\"registry_registerGateway\", \
             \"params\":[ \
               [\"$$GATEWAY_URL\", \"$(GATEWAY_SEQUENCING_ADDRESS)\", \"$$JWT\"] \
             ], \
             \"id\":1}"; \
      echo; echo

	@docker compose $(START_GATEWAY_COMPOSE_FILES) up -d
	@docker compose $(START_MONITORING_COMPOSE_FILES) up -d
	$(MAKE) start-overseer

start-overseer: 
	docker exec -it based-gateway overseer --portal-url $(PORTAL) --rich-wallet-key $(DUMMY_RICH_WALLET_PRIVATE_KEY)

start-spamoor:
	docker run \
		--pull always \
		--network host \
		--volume ./spamoor-config.yml:/etc/spamoor-config.yml \
	ghcr.io/chainbound/spamoor-op-geth run \
		/etc/spamoor-config.yml \
		--privkey $(DUMMY_RICH_WALLET_PRIVATE_KEY) \
		--rpchost http://0.0.0.0:$(BASED_OP_GETH_PORT)

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

deploy-chain: 
	@echo "Deploying new Chain with id: $(L2_CHAIN_ID)"
	@if [ -d .local_main_node/config ]; then \
		echo "❌  Seems like information of a previous chain is already present. Please remove .local_main_node to deploy a new one."; \
		exit 1; \
	fi
	@mkdir -p .local_main_node/config
	@docker run -v $$(pwd)/.local_main_node/config:/config --entrypoint sh -e DEPLOYER_CACHE_DIR=$(DEPLOYER_CACHE_DIR) $(IMAGE_OP_DEPLOYER) -c "/usr/local/bin/op-deployer init --l1-chain-id $(L1_CHAIN_ID) --l2-chain-ids $(L2_CHAIN_ID)  --workdir /config && chmod 666 /config/*"
	@wallet_batcher=$$(docker run --rm -i $(IMAGE_KEY_TO_ADDRESS) $(OP_BATCHER_KEY) | tail -n1); \
	wallet_proposer=$$(docker run --rm -i $(IMAGE_KEY_TO_ADDRESS) $(OP_PROPOSER_KEY) | tail -n1); \
	wallet_main=$$(docker run --rm -i $(IMAGE_KEY_TO_ADDRESS) $(MAIN_KEY) | tail -n1); \
	sed -E \
		  -e "s@L1_CHAIN_ID@$(L1_CHAIN_ID)@g" \
		  -e "s@L2_CHAIN_ID@$(L2_CHAIN_ID_HEX)@g" \
		  -e "s@VAULT_WALLET@$${wallet_main}@g" \
		  -e "s@OP_BATCHER_WALLET@$${wallet_batcher}@g" \
		  -e "s@OP_PROPOSER_WALLET@$${wallet_proposer}@g" \
		  main_node/intent.template.toml \
		  > .local_main_node/config/intent.toml

	@docker run -v $$(pwd)/.local_main_node/config:/config -e DEPLOYER_CACHE_DIR=$(DEPLOYER_CACHE_DIR) $(IMAGE_OP_DEPLOYER) op-deployer apply --workdir /config --l1-rpc-url $(L1_RPC_URL) --private-key $(MAIN_KEY)
	@docker run -v $$(pwd)/.local_main_node/config:/config -e DEPLOYER_CACHE_DIR=$(DEPLOYER_CACHE_DIR) $(IMAGE_OP_DEPLOYER) op-deployer inspect genesis --workdir /config $(L2_CHAIN_ID_HEX) > $$(pwd)/.local_main_node/config/genesis.json
	@docker run -v $$(pwd)/.local_main_node/config:/config -e DEPLOYER_CACHE_DIR=$(DEPLOYER_CACHE_DIR) $(IMAGE_OP_DEPLOYER) op-deployer inspect rollup --workdir /config $(L2_CHAIN_ID_HEX) > $$(pwd)/.local_main_node/config/rollup.json
	@docker run -v $$(pwd)/.local_main_node/config:/config -e DEPLOYER_CACHE_DIR=$(DEPLOYER_CACHE_DIR) --entrypoint sh  $(IMAGE_OP_DEPLOYER) -c "chmod 666 /config/*"

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
endif

OP_GETH_DATA_DIR?=.local_main_node/data/geth
OP_NODE_DATA_DIR?=.local_main_node/data/node
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
	@if [ "$(OP_GETH_DATA_DIR)" != ".local_main_node/data/geth" ] && [ ! -d ".local_main_node/data/geth" ] && [ -d "$(OP_GETH_DATA_DIR)"]; then \
	    ln -s $(OP_GETH_DATA_DIR) .local_main_node/data/geth; \
	else \
	    mkdir -p $(BASED_OP_GETH_DATA_DIR); \
	fi
	@if [ "$(OP_NODE_DATA_DIR)" != ".local_main_node/data/node" ] && [ ! -d ".local_main_node/data/node" ] && [ -d "$(OP_NODE_DATA_DIR)"]; then \
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
start-main-node: create-network
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
	  cp main_node/compose* .local_main_node; \
	  cp main_node/tx_receivers_example.json .local_main_node/config/tx_receivers.json; \
	  $(MAKE) fix-compose; \
	  echo "Initializing all components of a main sequencing node in ./.local_main_node ..."; \
	  { \
	    echo "DISPUTE_GAME_FACTORY_ADDRESS=$$(docker run -v $$(pwd)/.local_main_node/config:/config -i --rm imega/jq -r '.implementationsDeployment.disputeGameFactoryImplAddress' /config/state.json)"; \
	    echo "NETWORK_ID=$$(docker run -v $$(pwd)/.local_main_node/config:/config -i --rm imega/jq -r '.l2_chain_id' /config/rollup.json)"; \
	    echo "L1_RPC_URL=$(L1_RPC_URL)"; \
	    echo "L1_BEACON_RPC_URL=$(L1_BEACON_RPC_URL)"; \
	    echo "OP_NODE_SEQUENCER_KEY=$(MAIN_KEY)"; \
	    echo "OP_NODE_GOSSIP_IP=$(PUBLIC_IP)"; \
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

	@docker compose $(START_MAIN_NODE_COMPOSE_FILES) up -d
	@docker compose $(START_MONITORING_COMPOSE_FILES) up -d
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

stop-based-gateway:
	cd .local_gateway_and_follower && docker compose down

stop-main-node:
	@if [ ! -d .local_main_node/config ]; then \
		echo ".local_main_node/config does not seem to have been started. run make start-main-node."; \
		exit 1; \
	fi
	docker compose -f .local_main_node/compose.yml down
	# also stop monitoring services, if they are running
	$(MAKE) stop-monitoring

stop-monitoring:
	docker compose $(START_MONITORING_COMPOSE_FILES) down

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

logs-portal: ## 📜 Show portal logs (only for main sequencing node)
	docker logs based-portal --tail 100 -f

logs-registry: ## 📜 Show registry logs (only for main sequencing node)
	docker logs based-registry --tail 100 -f

logs-gateway: ## 📜 Show gateway logs
	docker logs based-gateway --tail 100 -f

logs-based-op-node: ## 📜 Show based op-node logs
	docker logs based-op-node --tail 100 -f

logs-based-op-geth: ## 📜 Show based op-geth logs
	docker logs based-op-geth --tail 100 -f
	
logs-op-node: ## 📜 Show main op-node logs (only for main sequencing node)
	docker logs op-node --tail 100 -f
	
logs-op-geth: ## 📜 Show main op-geth logs (only for main sequencing node)
	docker logs op-geth --tail 100 -f
	
logs-batcher:
	docker logs op-batcher --tail 100 -f
	
logs-proposer:
	docker logs op-proposer --tail 100 -f 


FOLLOWER_NODE_HOST?=http://localhost
BLOCK_NUMBER?=$(shell echo $$(( $$(cast block-number --rpc-url http://localhost:$(BOP_EL_PORT)) + 1 )))
DUMMY_RICH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DUMMY_TX=$(shell cast mktx --rpc-url  $(FOLLOWER_NODE_HOST):$(BOP_EL_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a | xxd -r -p | base64)
BASED_OP_GETH_PORT?=8645

test-tx:
	cast send --rpc-url  http://0.0.0.0:$(BASED_OP_GETH_PORT) --private-key $(DUMMY_RICH_WALLET_PRIVATE_KEY) --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a

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
