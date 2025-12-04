
export LOCAL_DATA := env("LOCAL_DATA")
export DATA_FOLDER := join(LOCAL_DATA, env("PEERING_DATA_FOLDER", ".peering"))

export OP_GETH_RPC := env("OP_GETH_RPC", "http://localhost:8545")
export BOP_GETH_RPC := env("BOP_GETH_RPC", "http://localhost:8645")

self := "just -f " + justfile()

@block-number rpc:
    curl -s -X POST -H "Content-Type: application/json" \
      --data '{"jsonrpc": "2.0", "method": "eth_blockNumber", "params": [], "id": 1}' {{rpc}}

_check-block-number:
    #!/usr/bin/env bash
    set -euo pipefail

    bop=$({{self}} block-number $BOP_GETH_RPC | jq -re '.result')
    op=$({{self}} block-number $OP_GETH_RPC | jq -re '.result')

    if [ "$bop" == "$op" ]; then
      exit 0
    fi

    echo "Based OP Geth and OP Geth not synced!"
    exit 1

_pyenv:
    #!/usr/bin/env bash
    set -euo pipefail 

    python3 -m venv {{DATA_FOLDER}}
    source {{DATA_FOLDER}}/bin/activate

    pip install requests

peering:
    #!/usr/bin/env bash
    set -euo pipefail

    {{self}} _check-block-number || {
      {{self}} _pyenv
      python3 {{join(justfile_directory(), "..", "peering.py")}}
    }
