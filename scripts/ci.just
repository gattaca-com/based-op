root := join(justfile_directory(), "..")
assets := canonicalize(join(root, ".github", "assets"))

parent := "just -f " + join(root, "Justfile")
based := "just -f " + join(root, "based", "docker", "Justfile")
deps := "just -f " + join(root, "deps", "Justfile")

# Upgrade the stored chain config with the one currently in use
_upgrade:
    #!/usr/bin/env bash
    set -euo pipefail

    {{parent}} main-node _is-configured || {
      echo "*** No active chain configuration!"
      exit 1
    }

    {{parent}} main-node backup-config {{assets}}

# Prepare chain config from known values 
prepare:
    #!/usr/bin/env bash
    set -euo pipefail

    # Ensure LOCAL_DATA folder is available
    mkdir -p $LOCAL_DATA
    
    # Populate wallets from backup
    cp -r --update=none {{join(assets, "wallets")}} {{join("$LOCAL_DATA", ".wallet")}} || true

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

# Run the configured chain
@run: (prepare)
    {{parent}} main-node start
    {{parent}} follower-node start-dev

# Run tests on the configured chain
test: 
    {{parent}} test tx

# Stop the configured chain
@stop:
    {{parent}} main-node stop
    {{parent}} follower-node stop
    
# Save the built images of the used services
export-images dest:
    docker save local_based_op_node | gzip > {{dest}}/based-op-node.tar.gz
    docker save local_based_op_deployer | gzip > {{dest}}/based-op-deployer.tar.gz
    docker save local_based_op_geth | gzip > {{dest}}/based-op-geth.tar.gz
    {{based}} export-all {{dest}}


