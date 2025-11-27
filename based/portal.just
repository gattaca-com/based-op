export PORTAL := env("PORTAL", "http://0.0.0.0:8080")

@_rpc method params="[]":
    curl -s -X POST -H "Content-Type: application/json" \
      --data '{"jsonrpc": "2.0", "method": "{{method}}", "params": {{params}}, "id": 1}' \
      {{PORTAL}} | jq -r '.result'

node_gossip_static: (_rpc "portal_opNodeGossipStatic")
node_enr: (_rpc "portal_opNodeBootnodeEnr")
geth_enode: (_rpc "portal_opGethBootnodeEnode")
l2_chain_id: (_rpc "portal_l2ChainId")

rollup: (_rpc "portal_fileRollup")
genesis: (_rpc "portal_fileGenesis")

register-gateway url address jwt: (_rpc "registry_registerGateway" "[\"{{url}}\", \"{{address}}\", \"{{jwt}}\"]") 
