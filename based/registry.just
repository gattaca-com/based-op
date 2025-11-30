
# Invocation utils
main_node := "just -f " + justfile_directory() + "/main-node.just"
self := "just -f " + justfile()

service-name := "based-registry"

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
