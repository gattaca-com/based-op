set quiet

# Invocation utils
self := "just -f " + justfile()
parent := "just -f " + join(justfile_directory(), "..", "Justfile")

logs := self + " docker-logs"

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

gateway:
    {{parent}} follower-node logs-gateway

main-node:
    {{parent}} main-node logs

portal:
    {{parent}} portal logs
