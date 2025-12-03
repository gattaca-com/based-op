export PORTAL := env("PORTAL", "http://0.0.0.0:8080")

# Normally JQ doesn't return error if its return was empty or null. -e fixes that
export JQ_ARGS := env("JQ_ARGS", "-e")

service-name := "based-portal"

# Invocation utils
main_node := "just -f " + justfile_directory() + "/main-node.just"
self := "just -f " + justfile()

@address:
    echo $PORTAL

@_rpc method params="[]":
    curl -s -X POST -H "Content-Type: application/json" \
      --data '{"jsonrpc": "2.0", "method": "{{method}}", "params": {{params}}, "id": 1}' \
      {{PORTAL}} | jq -r $JQ_ARGS '.result'

node_gossip_static: (_rpc "portal_opNodeGossipStatic")
node_enr: (_rpc "portal_opNodeBootnodeEnr")
geth_enode: (_rpc "portal_opGethBootnodeEnode")
l2_chain_id: (_rpc "portal_l2ChainId")

rollup: (_rpc "portal_fileRollup")
genesis: (_rpc "portal_fileGenesis")

@register-gateway url address jwt: 
    # The portal returns null on success so we remove the -e flag
    JQ_ARGS="" {{self}} _rpc "registry_registerGateway" \
      "[[\"{{url}}\", \"{{address}}\", \"{{jwt}}\"]]"

# Start the portal service and view logs (for main sequencing node)
start:
    #!/usr/bin/env bash
    set -euo pipefail

    cfg=$({{main_node}} _is-configured)
    data=$(dirname $cfg)
    
    # TODO: $(MAKE) fix-compose # Unable to find it
    docker compose -f $data/compose.yml up -d {{service-name}}
    {{self}} logs

# View the portal service logs
logs:
    docker logs {{service-name}} --tail 100 -f

# Stop the portal service
stop:
    #!/usr/bin/env bash
    set -euo pipefail

    data=$(dirname $({{main_node}} _is-configured))

    docker compose -f $data/compose.yml down {{service-name}} 

