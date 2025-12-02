set dotenv-load
mod docker

# Override where the local data for the gateway will be stored
default_local_data := canonicalize(join(justfile_directory(), ".local"))
export LOCAL_DATA := canonicalize(env("LOCAL_DATA", default_local_data))
export DATA_FOLDER := env("FOLLOWER_NODE_DATA_FOLDER", "gateway_and_follower")

FOLLOWER_NODE_DATA := join(LOCAL_DATA, DATA_FOLDER)

# Override these to change where geth, the main node and the gateway store their data
export BASED_OP_DATA_DIR := join(FOLLOWER_NODE_DATA, "data")
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

WALLET_NAME := env("FOLLOWER_NODE_WALLET", "gateway")

# Invocation utils
self := "just -f " + justfile()
parent := "just -f " + join(justfile_directory(), "..", "Justfile")
portal := "just -f " + join(justfile_directory(), "portal.just")
wallet := "just -f " + join(justfile_directory(), "scripts", "wallet.just")
link := "just -f " + join(justfile_directory(), "scripts", "link.just")

_generate-dotenv gossip enr enode $dotenv=join(FOLLOWER_NODE_DATA, ".env"):
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

_update-dotenv gossip enr enode $dotenv=join(FOLLOWER_NODE_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    sed -i -E \
        -e "s#^MAIN_OP_NODE_GOSSIP_STATIC=.*#MAIN_OP_NODE_GOSSIP_STATIC={{gossip}}#" $dotenv

    sed -i -E \
        -e "s#^MAIN_OP_NODE_ENR=.*#MAIN_OP_NODE_ENR={{enr}}#" $dotenv

    sed -i -E \
        -e "s#^MAIN_OP_GETH_ENODE=.*#MAIN_OP_GETH_ENODE={{enode}}#" $dotenv

@_dump-chain-config dest=join(FOLLOWER_NODE_DATA, "config"):
    mkdir -p {{dest}}
    # TODO: check for invalid json files (empty, ...)
    {{portal}} rollup > {{dest}}/rollup.json
    {{portal}} genesis > {{dest}}/genesis.json

@_link-data dest=BASED_OP_DATA_DIR:
    {{link}} link-dir-if-exists-elsewhere BASED_OP_GETH_DATA_DIR "{{dest}}/geth"
    {{link}} link-dir-if-exists-elsewhere BASED_OP_NODE_DATA_DIR "{{dest}}/node"
    {{link}} link-dir-if-exists-elsewhere BASED_GATEWAY_DATA_DIR "{{dest}}/gateway"

# Create Gateway configuration
#
# Will take care of creating a private key for the gateway and setting up its .env 
create-config $dotenv=join(FOLLOWER_NODE_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    FOLLOWER_NODE_DATA=$(dirname $dotenv)
    echo "*** Setting up gateway in $FOLLOWER_NODE_DATA"

    # If FORCE is set, reset the config
    if [ -n "$FORCE" ]; then
      echo "FORCE set, removing existing config..."
      rm -rf $FOLLOWER_NODE_DATA || true
      {{wallet}} remove {{WALLET_NAME}}
    fi

    mkdir -p $FOLLOWER_NODE_DATA 
     
    # Generate JWT if missing
    if [ ! -f $FOLLOWER_NODE_DATA/jwt ]; then
      openssl rand -hex 32 | tr -d '\n' | sed 's/^/0x/'  > $FOLLOWER_NODE_DATA/jwt
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
      echo "Initializing gateway and follower op-node in $FOLLOWER_NODE_DATA..."
      echo "Gateway Sequencing Private Key: $GATEWAY_SEQUENCING_KEY" 
      echo "Gateway Sequencing Wallet:      $GATEWAY_SEQUENCING_ADDRESS"

      {{self}} _generate-dotenv "$gossip" "$enr" "$enode" "$dotenv"
      {{self}} _dump-chain-config "$FOLLOWER_NODE_DATA/config"
      {{self}} _link-data "$FOLLOWER_NODE_DATA/data"
    else
      # Replace values of existing config
      {{self}} _update-dotenv "$gossip" "$enr" "$enode" "$dotenv"
    fi

    echo "*** Gateway setup completed!"

_is-configured $cfg=FOLLOWER_NODE_DATA:
    #!/usr/bin/env bash
    set -euo pipefail

    if  [ ! -d $cfg ]; then
      echo "❌ Gateway and Follower Node does not seem to have been configured at $cfg. run `create-config` to configure"
      exit 1
    fi

    echo $cfg

# Start the gateway service and the overseer TUI
#
# Will ensure the gateway is configured correctly before starting the service
start $dotenv=join(FOLLOWER_NODE_DATA, ".env") network="based_op_node": (docker::create-network network) (create-config dotenv)
    #!/usr/bin/env bash
    set -euo pipefail

    FOLLOWER_NODE_DATA=$(dirname $dotenv)
    
    # Retrieve IP and port from config
    ip=$(grep -m2 '^OP_NODE_GOSSIP_IP[[:space:]]*=' $dotenv | tail -n1 | cut -d= -f2)
    port=$(grep -m1 '^GATEWAY_PORT[[:space:]]*=' $dotenv | cut -d= -f2)

    address=$({{wallet}} key {{WALLET_NAME}})
    jwt=$(cat $FOLLOWER_NODE_DATA/jwt) 

    echo "Registering gateway"
    {{self}} register-gateway "http://$ip:$port" "$address" "$jwt"

    # Start services
    echo "Starting gateway and monitoring services..."
    docker compose -f $FOLLOWER_NODE_DATA/compose.yml up -d
    {{parent}} monitoring start

    echo "*** Starting overseer"
    just -f {{justfile_directory()}}/overseer.just start
    
stop $dotenv=join(FOLLOWER_NODE_DATA, ".env"):
    #!/usr/bin/env bash
    set -euo pipefail

    FOLLOWER_NODE_DATA=$(dirname $dotenv)
    
    docker compose -f $FOLLOWER_NODE_DATA/compose.yml down

logs-gateway:
    docker logs based-gateway --tail 100 -f

logs $dotenv=join(FOLLOWER_NODE_DATA, ".env"):
    docker compose -f $(dirname $FOLLOWER_NODE_DATA)/compose.yml logs --tail 100 -f

proxy network="kt-based-op" port="8545":
    docker run --rm network {{network}} -p {{host}}:8545 cars10/simprox simprox --skip-ssl-verify=true -l 127.0.0.1:8545 -t op-el-2-op-geth-op-node-op-kurtosis:8545
