set dotenv-load

import './scripts/consts.just'
 
# Override where the local data for the main node will be stored
default_local_data := canonicalize(join(justfile_directory(), ".local"))
export LOCAL_DATA := canonicalize(env("LOCAL_DATA", default_local_data))
export DATA_FOLDER := env("MAIN_NODE_DATA_FOLDER", "local_main_node")

MAIN_NODE_DATA := join(LOCAL_DATA, DATA_FOLDER)
_main-node-data:
    echo {{MAIN_NODE_DATA}}

# Override these to change where geth and the main node store their data
export BASED_MAIN_NODE_DATA_DIR := join(MAIN_NODE_DATA, "data")
export BASED_OP_GETH_DATA_DIR := env("BASED_OP_GETH_DATA_DIR", join(BASED_MAIN_NODE_DATA_DIR, "geth"))
export BASED_OP_NODE_DATA_DIR := env("BASED_OP_NODE_DATA_DIR", join(BASED_MAIN_NODE_DATA_DIR, "node"))

# Override main node source dir
export MAIN_NODE_DIR := join(justfile_directory(), "..", "main_node")

# External environment configuration 
export PUBLIC_IP := env("PUBLIC_IP", shell("curl -s ifconfig.me"))

export OP_PROPOSER_KEY := env("OP_PROPOSER_KEY")
export OP_BATCHER_KEY := env("OP_BATCHER_KEY")
export OP_SEQUENCER_KEY := env("OP_SEQUENCER_KEY")

# Force config creation
export FORCE := env("FORCE", "")

# Invocation utils
self := "just -f " + justfile()

link-dir-if-exists-elsewhere := "just -f " + justfile_directory() + "/scripts/link.just link-dir-if-exists-elsewhere"
@_link-data dest=join(BASED_MAIN_NODE_DATA_DIR, "data"):
    {{link-dir-if-exists-elsewhere}} BASED_OP_GETH_DATA_DIR "{{dest}}/geth"
    {{link-dir-if-exists-elsewhere}} BASED_OP_NODE_DATA_DIR "{{dest}}/node"

_populate-chain-cfg $src $dest:
    #!/usr/bin/env bash
    set -euo pipefail

    function nc_copy() {
       # Copy without overriding destination
       if [ ! -f $dest/$1 ]; then
         cp $src/$1 $dest/$1
       fi
    }
    nc_copy rollup.json
    nc_copy state.json
    nc_copy genesis.json

_generate-dotenv $cfg=join(MAIN_NODE_DATA, "config") $dotenv=join(MAIN_NODE_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    MAIN_NODE_DATA=$(dirname $cfg)

    cp $MAIN_NODE_DIR/env_example $dotenv
    cp $MAIN_NODE_DIR/compose* $MAIN_NODE_DATA
    cp $MAIN_NODE_DIR/tx_receivers_example.json $cfg/tx_receivers.json

    # TODO: $(MAKE) fix-compose # Unable to find it

    game_factory=$(jq -r '.opChainDeployments[0].disputeGameFactoryProxyAddress' $cfg/state.json)
    l2_chain_id=$(jq -r '.l2_chain_id' $cfg/rollup.json)

    echo "DISPUTE_GAME_FACTORY_ADDRESS=$game_factory" >> $dotenv
    echo "NETWORK_ID=$l2_chain_id" >> $dotenv
    echo "L1_RPC_URL=$L1_RPC_URL" >> $dotenv
    echo "L1_BEACON_RPC_URL=$L1_BEACON_RPC_URL" >> $dotenv
    echo "OP_NODE_SEQUENCER_KEY=$OP_SEQUENCER_KEY" >> $dotenv
    echo "OP_NODE_GOSSIP_IP=$PUBLIC_IP" >> $dotenv
    echo "OP_BATCHER_PRIVATE_KEY=$OP_BATCHER_KEY" >> $dotenv
    echo "OP_PROPOSER_PRIVATE_KEY=$OP_PROPOSER_KEY" >> $dotenv

# Create the Main Node configuration
#
# Arguments:
# * `chain_cfg`: folder containing rollup.json, genesis.json and state.json
create-config chain_cfg $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail

    MAIN_NODE_DATA=$(dirname $cfg)
    echo "*** Setting up main node in $MAIN_NODE_DATA"

    # If FORCE is set, reset the config
    if [ -n "$FORCE" ]; then
      echo "FORCE set, removing existing config..."
      rm -rf $MAIN_NODE_DATA || true
    fi

    if [ -d $cfg ]; then
      echo "❌ Seems like the main node was already configured (see $cfg)."
      exit 1
    fi
    
    mkdir -p $cfg

    # Generate JWT if missing
    if [ ! -f $cfg/jwt ]; then
      openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/' > $cfg/jwt
    fi 
     
    # Copy chain config files
    {{self}} _populate-chain-cfg {{chain_cfg}} $cfg
     
    # Generate .env if missing
    dotenv=$MAIN_NODE_DATA/.env
    if [ ! -f $dotenv ]; then
      {{self}} _generate-dotenv
    fi

    # Initialize registry if missing
    registry=$cfg/registry.json
    if [ ! -f $registry ]; then
      echo "[]" > $registry
    fi 
    
    {{self}} _link-data "$MAIN_NODE_DATA/data"
    echo "*** Done initializing $MAIN_NODE_DATA" 

# Deploy a new chain and create the appropriate configuration
config-with-deploy $cfg=join(MAIN_NODE_DATA, "config"):
    just -f ./scripts/deploy.just deploy-chain $cfg

# Start the Main Node service
start $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail
    
    # Check Node running
    if [ docker ps --format '{{{{.Names}}' | grep -wq op-node  ]; then
        echo "❌ Main node already running."
        exit 1
    fi 

    if [ ! -d $cfg ]; then
      echo "❌ $cfg does not exist. run `create-config` to configure"
      exit 1
    fi
    
    MAIN_NODE_DATA=$(dirname $cfg)
      
    docker compose -f $MAIN_NODE_DATA/compose.yml up -d
    docker compose -f {{justfile_directory()}}/../monitoring/compose.yml up -d

stop $cfg=join(MAIN_NODE_DATA, "config"):
    #!/usr/bin/env bash
    set -euo pipefail

    MAIN_NODE_DATA=$(dirname $cfg)
    
    docker compose -f $MAIN_NODE_DATA/compose.yml down
    docker compose -f {{justfile_directory()}}/../monitoring/compose.yml down
