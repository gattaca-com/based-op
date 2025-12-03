set dotenv-load

import './consts.just'

export DATA_FOLDER := env("MAIN_NODE_DATA_FOLDER", "local_main_node")

MAIN_NODE_DATA := join(LOCAL_DATA, DATA_FOLDER)

# Override main node source dir
export MAIN_NODE_DIR := join(justfile_directory(), "..", "..", "main_node")

# External environment configuration 
export L1_CHAIN_ID := env("L1_CHAIN_ID", "11155111") # Default to Sepolia
export L2_CHAIN_ID := env("L2_CHAIN_ID", shell(gen-l2-chain-id))

export OP_PROPOSER_KEY := env("OP_PROPOSER_KEY")
export OP_BATCHER_KEY := env("OP_BATCHER_KEY")
export OP_SEQUENCER_KEY := env("OP_SEQUENCER_KEY")

# Force config creation
export FORCE := env("FORCE", "")

BATCHER_WALLET_NAME := env("BATCHER_WALLET", "batcher")
PROPOSER_WALLET_NAME := env("PROPOSER_WALLET", "proposer")
SEQUENCER_WALLET_NAME := env("SEQUENCER_WALLET", "sequencer")

# Override OP deployer image and cache dir
op_deployer_image := env("IMAGE_OP_DEPLOYER", "us-docker.pkg.dev/oplabs-tools-artifacts/images/op-deployer:v0.4.5")
export DEPLOYER_CACHE_DIR:= env("DEPLOYER_CACHE_DIR", "/tmp/op-deployer-cache")

export L2_CHAIN_ID_HEX := shell('printf "0x%064x" ' + L2_CHAIN_ID)
# NOTE: Having this as s Just recipe would lead to loops
gen-l2-chain-id := "
RAW=$(od -An -N2 -tu2 /dev/urandom | tr -d ' ')
echo $(($RAW % 50000 + 1))
"

# Invocation utils
self := "just -f " + justfile()
wallet := "just -f " + justfile_directory() + "/wallet.just"

_op-deployer $cfg *runtime-args:
    docker run -v $cfg:/config -e DEPLOYER_CACHE_DIR=$DEPLOYER_CACHE_DIR \
      {{op_deployer_image}} {{runtime-args}}

_fix-permissions $cfg:
    docker run -v $cfg:/config --entrypoint sh {{op_deployer_image}} -c \
      "find /config -mindepth 1 -exec chmod 666 {} \; && chmod 755 /config"

_populate-intent $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail

    sequencer=$({{wallet}} ensure {{SEQUENCER_WALLET_NAME}} $OP_SEQUENCER_KEY)
    proposer=$({{wallet}} ensure {{PROPOSER_WALLET_NAME}} $OP_PROPOSER_KEY)
    batcher=$({{wallet}} ensure {{BATCHER_WALLET_NAME}} $OP_SEQUENCER_KEY)

    sed -E \
          -e "s@L1_CHAIN_ID@$L1_CHAIN_ID@g" \
          -e "s@L2_CHAIN_ID@$L2_CHAIN_ID_HEX@g" \
          -e "s@VAULT_WALLET@$sequencer@g" \
          -e "s@OP_BATCHER_WALLET@$batcher@g" \
          -e "s@OP_PROPOSER_WALLET@$proposer@g" \
          {{MAIN_NODE_DIR}}/intent.template.toml \
          > $cfg/intent.toml

# Create a new chain configuration
create-config $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail

    MAIN_NODE_DATA=$(dirname $cfg)

    if [ -n "$FORCE" ]; then
      echo "FORCE set, removing existing config at $MAIN_NODE_DATA..."
      rm -rf $MAIN_NODE_DATA || true
    fi

    if [ -d $cfg ]; then
        echo "❌ Seems like information of a previous chain is already present. Please remove $cfg to deploy a new one."
        exit 1
    fi 

    mkdir -p $cfg

    {{self}} _op-deployer $cfg op-deployer init --l1-chain-id $L1_CHAIN_ID --l2-chain-ids $L2_CHAIN_ID --workdir /config
    {{self}} _populate-intent
    {{self}} _fix-permissions $cfg

    {{self}} _populate-intent

    exit 0

@_dump-chain-config $dest=join(MAIN_NODE_DATA, "config"):
    mkdir -p {{dest}}

    {{self}} _op-deployer $cfg op-deployer inspect genesis --workdir /config $L2_CHAIN_ID_HEX > $dest/genesis.json
    {{self}} _op-deployer $cfg op-deployer inspect rollup --workdir /config $L2_CHAIN_ID_HEX > $dest/rollup.json

# Setup and deploy a new chain on the configured L1
deploy-chain $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail

    echo "*** Setting up new Chain with id: $L2_CHAIN_ID"
    {{self}} create-config $cfg

    echo "*** Deploying new chain..."
    sequencer=$({{wallet}} key {{SEQUENCER_WALLET_NAME}})

    {{self}} _op-deployer $cfg op-deployer apply --workdir /config --l1-rpc-url $L1_RPC_URL --private-key $sequencer

    echo "*** Retrieving newly created chain data..."
    {{self}} _dump-chain-config $cfg
     
    # World accessible permissions
    {{self}} _fix-permissions $cfg

    echo "*** Deployment complete! See chain config in $cfg"
    
    exit 0

