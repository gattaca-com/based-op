set dotenv-load
mod docker

# Override where the local data for the gateway will be stored
default_local_data := canonicalize(join(justfile_directory(), ".local"))
export LOCAL_DATA := canonicalize(env("LOCAL_DATA", default_local_data))
export DATA_FOLDER := env("GATEWAY_DATA_FOLDER", "gateway_and_follower")

GATEWAY_DATA := join(LOCAL_DATA, DATA_FOLDER)

# Override these to change where geth, the main node and the gateway store their data
export BASED_OP_DATA_DIR := join(GATEWAY_DATA, "data")
export BASED_OP_GETH_DATA_DIR := env("BASED_OP_GETH_DATA_DIR", join(BASED_OP_DATA_DIR, "geth"))
export BASED_OP_NODE_DATA_DIR := env("BASED_OP_NODE_DATA_DIR", join(BASED_OP_DATA_DIR, "node"))
export BASED_GATEWAY_DATA_DIR := env("BASED_GATEWAY_DATA_DIR", join(BASED_OP_DATA_DIR, "gateway"))

# Override follower node source dir
export FOLLOWER_NODE_DIR := join(justfile_directory(), "..", "follower_node")

# External environment configuration
export PORTAL := env("PORTAL", shell(portal + " address"))
export PUBLIC_IP := env("PUBLIC_IP", shell("curl -s ifconfig.me"))

# Force config creation
export FORCE := env("FORCE", "")

WALLET_NAME := env("GATEWAY_WALLET", "gateway")

# Invocation utils
self := "just -f " + justfile()
portal := "just -f " + justfile_directory() + "/portal.just"
wallet := "just -f " + justfile_directory() + "/scripts/wallet.just"
link-dir-if-exists-elsewhere := "just -f " + justfile_directory() + "/scripts/link.just link-dir-if-exists-elsewhere"

_generate-dotenv gossip enr enode $dotenv=join(GATEWAY_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail
    
    cp $FOLLOWER_NODE_DIR/env_example $dotenv
    cp $FOLLOWER_NODE_DIR/compose* $(dirname $dotenv) 

    l2_chain_id=$({{portal}} l2_chain_id)

    echo "PORTAL=$PORTAL" >> $dotenv
    echo "OP_NODE_GOSSIP_IP={{public_ip}}" >> $dotenv
    echo "GATEWAY_SEQUENCING_KEY=$GATEWAY_SEQUENCING_KEY" >> $dotenv
    echo "MAIN_OP_NODE_GOSSIP_STATIC={{gossip}}" >> $dotenv
    echo "MAIN_OP_NODE_ENR={{enr}}" >> $dotenv
    echo "MAIN_OP_GETH_ENODE={{enode}}" >> $dotenv
    echo "NETWORK_ID=$l2_chain_id" >> $dotenv

_update-dotenv gossip enr enode $dotenv=join(GATEWAY_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    sed -i -E \
        -e "s#^MAIN_OP_NODE_GOSSIP_STATIC=.*#MAIN_OP_NODE_GOSSIP_STATIC={{gossip}}#" $dotenv

    sed -i -E \
        -e "s#^MAIN_OP_NODE_ENR=.*#MAIN_OP_NODE_ENR={{enr}}#" $dotenv

    sed -i -E \
        -e "s#^MAIN_OP_GETH_ENODE=.*#MAIN_OP_GETH_ENODE={{enode}}#" $dotenv

@_dump-chain-config dest=join(GATEWAY_DATA, "config"):
    mkdir -p {{dest}}
    # TODO: check for invalid json files (empty, ...)
    {{portal}} rollup > {{dest}}/rollup.json
    {{portal}} genesis > {{dest}}/genesis.json

@_link-data dest=BASED_OP_DATA_DIR:
    {{link-dir-if-exists-elsewhere}} BASED_OP_GETH_DATA_DIR "{{dest}}/geth"
    {{link-dir-if-exists-elsewhere}} BASED_OP_NODE_DATA_DIR "{{dest}}/node"
    {{link-dir-if-exists-elsewhere}} BASED_GATEWAY_DATA_DIR "{{dest}}/gateway"

# Create Gateway configuration
#
# Will take care of creating a private key for the gateway and setting up its .env 
create-config $dotenv=join(GATEWAY_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    GATEWAY_DATA=$(dirname $dotenv)
    echo "*** Setting up gateway in $GATEWAY_DATA"

    # If FORCE is set, reset the config
    if [ -n "$FORCE" ]; then
      echo "FORCE set, removing existing config..."
      rm -rf $GATEWAY_DATA || true
      {{wallet}} remove {{WALLET_NAME}}
    fi

    mkdir -p $GATEWAY_DATA 
     
    # Generate JWT if missing
    if [ ! -f $GATEWAY_DATA/jwt ]; then
      openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/'  > $GATEWAY_DATA/jwt
    fi

    # Ensure wallet is ready
    echo "Retrieving {{WALLET_NAME}} wallet"
    export GATEWAY_SEQUENCING_ADDRESS=$({{wallet}} ensure {{WALLET_NAME}})
    export GATEWAY_SEQUENCING_KEY=$({{wallet}} key {{WALLET_NAME}})
    
    echo "Retrieving OP Node bootstrap environment"
    gossip=$({{portal}} node_gossip_static)
    enr=$({{portal}} node_enr)
    enode=$({{portal}} geth_enode)
    
    # Generate .env if missing
    if [ ! -f $dotenv ]; then
      echo "Initializing gateway and follower op-node in $GATEWAY_DATA..."
      echo "Gateway Sequencing Private Key: $GATEWAY_SEQUENCING_KEY" 
      echo "Gateway Sequencing Wallet:      $GATEWAY_SEQUENCING_ADDRESS"

      {{self}} _generate-dotenv "$gossip" "$enr" "$enode" "$dotenv"
      {{self}} _dump-chain-config "$GATEWAY_DATA/config"
      {{self}} _link-data "$GATEWAY_DATA/data"
    else
      # Replace values of existing config
      {{self}} _update-dotenv "$gossip" "$enr" "$enode" "$dotenv"
    fi

    echo "*** Gateway setup completed!"

# Start the gateway service and the overseer TUI
#
# Will ensure the gateway is configured correctly before starting the service
start $dotenv=join(GATEWAY_DATA, ".env") network="based_op_node": (docker::create-network network) (create-config dotenv)
    #!/usr/bin/env bash
    set -euo pipefail

    GATEWAY_DATA=$(dirname $dotenv)
    
    # Retrieve IP and port from config
    ip=$(grep -m2 '^OP_NODE_GOSSIP_IP[[:space:]]*=' $dotenv | tail -n1 | cut -d= -f2)
    port=$(grep -m1 '^GATEWAY_PORT[[:space:]]*=' $dotenv | cut -d= -f2)

    address=$({{wallet}} key {{WALLET_NAME}})
    jwt=$(cat $GATEWAY_DATA/jwt) 

    echo "Registering gateway"
    {{self}} register-gateway "http://$ip:$port" "$address" "$jwt"

    # Start services
    echo "Starting gateway and monitoring services..."
    docker compose -f $GATEWAY_DATA/compose.yml up -d
    docker compose -f {{justfile_directory()}}/../monitoring/compose.yml

    echo "*** Starting overseer"
    just -f {{justfile_directory()}}/overseer.just start
    
stop $dotenv=join(GATEWAY_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    GATEWAY_DATA=$(dirname $dotenv)
    
    docker compose -f $GATEWAY_DATA/compose.yml down
    docker compose -f {{justfile_directory()}}/../monitoring/compose.yml down
