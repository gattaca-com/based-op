import 'portal.just'

mod docker

# Override where the local data for the gateway will be stored
default_local_data := canonicalize(join(justfile_directory(), ".local"))
export LOCAL_DATA := canonicalize(env("LOCAL_DATA", default_local_data))
export DATA_FOLDER := env("GATEWAY_DATA_FOLDER", "gateway_and_follower")

export GATEWAY_DATA := join(LOCAL_DATA, DATA_FOLDER)

export BASED_OP_DATA_DIR := join(GATEWAY_DATA, "data")
export BASED_OP_GETH_DATA_DIR := env("BASED_OP_GETH_DATA_DIR", join(BASED_OP_DATA_DIR, "geth"))
export BASED_OP_NODE_DATA_DIR := env("BASED_OP_NODE_DATA_DIR", join(BASED_OP_DATA_DIR, "node"))
export BASED_GATEWAY_DATA_DIR := env("BASED_GATEWAY_DATA_DIR", join(BASED_OP_DATA_DIR, "gateway"))

# Override follower node source dir
export FOLLOWER_NODE_DIR := join(justfile_directory(), "..", "follower_node")

export FORCE := env("FORCE", "")

WALLET_NAME := env("GATEWAY_WALLET", "gateway")

# Easily access the wallet.just scripts
@wallet *args:
    just -f {{justfile_directory()}}/scripts/wallet.just {{args}}

_generate-dotenv gossip enr enode $dotenv=join(GATEWAY_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail
    
    cp $FOLLOWER_NODE_DIR/env_example $dotenv
    cp $FOLLOWER_NODE_DIR/compose* $(dirname $dotenv) 

    l2_chain_id=$(just -f {{justfile()}} l2_chain_id)

    PUBLIC_IP=$(curl -s ifconfig.me)

    echo "PORTAL={{PORTAL}}" >> $dotenv
    echo "OP_NODE_GOSSIP_IP=$PUBLIC_IP" >> $dotenv
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

@_dump_chain_config dest=join(GATEWAY_DATA, "config"):
    mkdir -p {{dest}}
    just -f {{justfile()}} rollup > {{dest}}/rollup.json
    just -f {{justfile()}} genesis > {{dest}}/genesis.json

# Links `src` to `dest`, if `src` is not `dest` and `src` exists, otherwise, created `src`
# 
# `src` is the result of evaluating `srcEnv` environment. 
_link-dir-if-exists-elsewhere srcEnv $dest:
    #!/usr/bin/env bash
    src=${{srcEnv}}

    if [ -n "$FORCE" ]; then
      rm -rf $dest || true
    fi

    if [ "$src" != "$dest" && -d "$src" && ! -d "$dest"]; then
      ln -s $src $dest
    else
      mkdir -p $dest
    fi
    
_link-data dest=join(GATEWAY_DATA, "data"):
    just -f {{justfile()}} _link-dir-if-exists-elsewhere BASED_OP_GETH_DATA_DIR "{{dest}}/geth"
    just -f {{justfile()}} _link-dir-if-exists-elsewhere BASED_OP_NODE_DATA_DIR "{{dest}}/node"
    just -f {{justfile()}} _link-dir-if-exists-elsewhere BASED_GATEWAY_DATA_DIR "{{dest}}/gateway"

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
      just -f {{justfile()}} wallet remove {{WALLET_NAME}}
    fi

    mkdir -p $GATEWAY_DATA 
     
    # Generate JWT if missing
    if [ ! -f $GATEWAY_DATA/jwt ]; then
      openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/'  > $GATEWAY_DATA/jwt
    fi

    # Ensure wallet is ready
    echo "Retrieving {{WALLET_NAME}} wallet"
    just -f {{justfile()}} wallet ensure {{WALLET_NAME}}
    export GATEWAY_SEQUENCING_KEY=$(just -f {{justfile()}} wallet key {{WALLET_NAME}})
    export GATEWAY_SEQUENCING_ADDRESS=$(just -f {{justfile()}} wallet address {{WALLET_NAME}})
    
    echo "Retrieving OP Node bootstrap environment"
    gossip=$(just -f {{justfile()}} node_gossip_static)
    enr=$(just -f {{justfile()}} node_enr)
    enode=$(just -f {{justfile()}} geth_enode)
    
    # Generate .env if missing
    if [ ! -f $dotenv ]; then
      echo "Initializing gateway and follower op-node in $GATEWAY_DATA..."
      echo "Gateway Sequencing Private Key: $GATEWAY_SEQUENCING_KEY" 
      echo "Gateway Sequencing Wallet:      $GATEWAY_SEQUENCING_ADDRESS"

      just -f {{justfile()}} _generate-dotenv "$gossip" "$enr" "$enode" "$dotenv"
      just -f {{justfile()}} _dump_chain_config "$GATEWAY_DATA/config"
    else
      # Replace values of existing config
      just -f {{justfile()}} _update-dotenv "$gossip" "$enr" "$enode" "$dotenv"
    fi

    echo "*** Gateway setup completed!"

# Start the gateway service and the overseer TUI
#
# Will ensure the gateway is configured correctly before starting the service
start: docker::create-network create-config
    #!/usr/bin/env bash
    
    ip=$(grep -m2 '^OP_NODE_GOSSIP_IP[[:space:]]*=' {{GATEWAY_DATA}}/.env | tail -n1 | cut -d= -f2)
    port=$(grep -m1 '^GATEWAY_PORT[[:space:]]*=' {{GATEWAY_DATA}}/.env | cut -d= -f2)

    address=$(just -f {{justfile()}} wallet key {{WALLET_NAME}})
    jwt=$(cat {{GATEWAY_DATA}}/jwt)

    echo "Registering gateway"
    just -f {{justfile()}} register-gateway "http://$ip:$port" "$address" "$jwt"

    # Start services
    echo "Starting gateway and monitoring services..."
    docker compose -f {{GATEWAY_DATA}}/compose.yml up -d
    docker compose -f {{justfile_directory()}}/../monitoring/compose.yml

    echo "*** Starting overseer"
    just -f {{justfile_directory()}}/overseer.just start
    
