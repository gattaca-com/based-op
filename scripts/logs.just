set quiet

# Invocation utils
self := "just -f " + justfile()
parent := "just -f " + join(justfile_directory(), "Justfile")

logs := self + " docker-logs"

follower-node:
    #!/usr/bin/env bash
    set -euo pipefail

    GATEWAY_DATA=$({{parent}} gateway _is-configured)
    docker compose -f $GATEWAY_DATA/compose.yml logs --tail 100 -f

docker-logs container:
    docker logs {{container}} --tail 100 -f

based-op-node:
    {{logs}} based-op-node

based-op-geth:
    {{logs}} based-op-geth

op-node:
    {{logs}} op-node

op-geth:
    {{logs}} op-geth

batcher:
    {{logs}} op-batcher

proposer:
    {{logs}} op-proposer
    
## The following are here to provide a consistent interface

follower-node:
    {{parent}} follower-node logs

main-node:
    {{parent}} main-node logs
