root := join(justfile_directory(), "..")
assets := canonicalize(join(root, ".github", "assets"))

parent := "just -f " + join(root, "Justfile")

# Prepare chain config from known values 
prepare:
    #!/usr/bin/env bash
    set -euo pipefail
    
    # Populate wallets from backup
    cp -r --update=none {{join(assets, "wallets")}} {{join(env("LOCAL_DATA"), ".wallet")}} || true

    {{parent}} main-node _is-configured || {
      echo "*** Using chain config at {{assets}} to configure Main Node..."
      {{parent}} main-node create-config {{assets}}
    }

    {{parent}} follower-node _is-configured || {
      echo "*** Starting Main Node to configure Follower Node..."
      {{parent}} start main-node
      {{parent}} follower-node create-config
      {{parent}} stop main-node
    }

    echo "*** Main Node and Follower Node configured! Use \`run\` to start both"

@run: (prepare)
    {{parent}} main-node start
    {{parent}} follower-node start-dev
