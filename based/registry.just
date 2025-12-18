
# Invocation utils
main_node := "just -f " + justfile_directory() + "/main-node.just"
self := "just -f " + justfile()

service-name := "based-registry"

export REGISTRY_PORT := env("REGISTRY_PORT", "8082")
export REGISTRY := env("REGISTRY", "http://0.0.0.0:" + REGISTRY_PORT)
export JQ_ARGS := env("JQ_ARGS", "-e")

@address:
    echo $REGISTRY

@_rpc method params="[]":
    curl -s -X POST -H "Content-Type: application/json" \
      --data '{"jsonrpc": "2.0", "method": "{{method}}", "params": {{params}}, "id": 1}' \
      {{REGISTRY}} | jq -r $JQ_ARGS '.result'

@register-gateway url address:
    # The registry returns null on success so we remove the -e flag
    JQ_ARGS="" {{self}} _rpc "registry_registerGateway" \
      "[[\"{{url}}\", \"{{address}}\"]]"

# Start the registry service and view logs (for main sequencing node)
start:
    #!/usr/bin/env bash
    set -euo pipefail

    data=$(dirname $({{main_node}} _is-configured))

    docker compose -f $data/compose.yml up -d {{service-name}} 
    {{self}} logs

# View the registry service logs
logs:
    docker logs {{service-name}} --tail 100 -f
    
# Stop the registry service
stop:
    #!/usr/bin/env bash
    set -euo pipefail

    data=$(dirname $({{main_node}} _is-configured))

    docker compose -f $data/compose.yml down {{service-name}} 
