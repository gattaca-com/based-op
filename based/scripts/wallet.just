# Override where the wallet data will be stored
# 
# The resulting path for a given wallet will be $LOCAL_DATA/$WALLET_DATA_FOLDER/{{name}} 
export LOCAL_DATA := env("LOCAL_DATA")
export DATA_FOLDER := join(LOCAL_DATA, env("WALLET_DATA_FOLDER", ".wallet"))

export FORCE := env("FORCE", "")

tool_image := env("WALLET_TOOL_IMAGE", "ghcr.io/gattaca-com/based-op/key-to-address:latest")

self := "just -f " + justfile()

[no-exit-message]
needs_create name:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ -e "{{join(DATA_FOLDER, name)}}" && -z "$FORCE" ]]; then
      echo "Wallet {{name}} already exists, set \$FORCE to override" >&2
      exit 1
    fi

    exit 0

# Creates the wallet `name` at `$DATA_FOLDER/{{name}}`
#
# Will create 2 files, key and address, holding the private key and the address of the wallet
#
# Outputs the address of the generated wallet
create name override="":
    #!/usr/bin/env sh
    set -euo pipefail

    wallet=$(docker run --rm -i {{tool_image}} {{override}})
    key=$(echo "$wallet" | head -n1)
    address=$(echo "$wallet" | head -n2 | tail -n1)

    mkdir -p {{join(DATA_FOLDER, name)}}
    echo "$key" > {{join(DATA_FOLDER, name, "key")}}
    echo "$address" > {{join(DATA_FOLDER, name, "adrress")}}
    echo "$address"

# Remove the wallet `name`
[no-exit-message]
@remove name:
    rm -rf {{join(DATA_FOLDER, "name")}}

# Ensures the wallet `name` exists, and returns the corresponding address
ensure name override="":
    #!/usr/bin/env sh
    set -euo pipefail
    
    mkdir -p $DATA_FOLDER 

    {{self}} needs_create {{name}} || {
        # No need to create the wallet 
        exit 0 
    }

    # Cleanup existing copy (only possible if FORCE is set)
    {{self}} remove {{name}} || true

    # Create the wallet
    {{self}} create {{name}} {{override}}

# Returns the key of the wallet `name`
@key name:
    cat {{join(DATA_FOLDER, name, "key")}}

# Returns the address of the wallet `name`
@address name:
    cat {{join(DATA_FOLDER, name, "address")}}

# Backup the named wallet at `dest`
backup name dest:
    mkdir -p {{join(invocation_directory(), dest, name)}}
    cp -r {{join(DATA_FOLDER, name)}}/* {{join(invocation_directory(), dest, name)}}
