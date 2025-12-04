assets := canonicalize(join(justfile_directory(), "..", ".github", "assets"))

parent := "just -f " + join(justfile_directory(), "..", "Justfile")

# Prepare chain config from known values 
prepare:
    #!/usr/bin/env bash
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

run: (prepare)
    {{parent}} main-node start
    {{parent}} follower-node start-dev
