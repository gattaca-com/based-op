set quiet := true

# Set these to control where deps are fetched from.
repo := env("REPO")
ref := env("REF", "main")

# Set these to instead link the dependency
local := env("LOCAL", "")
uses_local := if trim(local) != "" { "true" } else { "" }

force := env("FORCE", "")

# Fetch the given dependency from the configured remote
fetch $folder:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Fetching $folder from {{repo}} @{{ref}}"
    git clone --branch "{{ref}}" --depth 1 "{{repo}}" "$folder"

# Determine if the dependency needs sourcing
#
# If the folder is not present, or if force is set, the dependency will be sourced 
needs_sourcing $folder:
    #!/usr/bin/env bash
    set -euo pipefail
	 
    if [[ -e "./$folder" && -z "{{force}}" ]]; then
        echo "$folder already exists, set $FORCE to override" >&2
        exit 1
    fi

    exit 0

# Fetch or link the configured dependency in `folder`
ensure $folder:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Checking $folder..."

    just -f {{justfile()}} needs_sourcing $folder > /dev/null 2>&1 || {
    # No need to source the dependency
    exit 0
    }
    
    # Cleanup existing copy (only possible if FORCE is set)
    rm -rf $folder || true
        
    if [ -n "{{uses_local}}" ]; then
        local_path="{{local}}"
        echo "Linking local path $local_path -> $folder"
        ln -s "$local_path" "./$folder"
        exit 0
    fi
        
    # 3) Fetch from source
    echo "Fetching $folder from source"
    just -f {{justfile()}} fetch {{folder}}
