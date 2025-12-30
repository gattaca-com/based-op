set dotenv-load := true

export LOCAL_DATA := canonicalize(env("LOCAL_DATA", shell('mkdir -p .local && realpath .local')))

# dev = use locally built images
# prod = use releases

export BASED_ENV := env("BASED_ENV", "prod")
self := "just -f " + justfile()
deps := "just -f " + join(justfile_directory(), "deps", "Justfile")

# Verifies that system dependencies are present
@check:
    echo "jq: {{ require('jq') }}"
    echo "docker: {{ require('docker') }}"
    echo "cast: {{ require('cast') }}"
    echo "rustup: {{ require('rustup') }}"
    echo "python: {{ require('python') }}"

# Prepare the local environment: fetch deps, build them, setup toolchains...
@prepare:
    {{ deps }} fetch
    cd docs && npm i
    cd based && rustup toolchain install

# 🏗️ Build
build:
    #!/usr/bin/env bash
    set -euo pipefail

    {{ deps }} build &
    just -f based/docker/Justfile all &

    wait

# 📚 Build local docs
docs:
    just -f docs/Justfile serve

# Build and link rabby in the configured output folder
rabby out="./dist":
    just -f deps/rabby.just build
    ln -s deps/rabby/dist {{ out }}/rabby
    ln -s deps/rabby/dist-mv2 {{ out }}/rabby-mv2

## Component access (component verb)

# Run recipes from scripts/spamoor.just
spamoor *args=("start ./spamoor-config.yml"):
    just -f scripts/spamoor.just {{ args }}

# Run recipes from scripts/overseer.just
overseer *args=("start"):
    just -f based/overseer.just {{ args }}

# Run recipes from based/portal.just
portal *args:
    just -f based/portal.just {{ args }}

# Run recipes from based/registry.just
registry *args:
    just -f based/registry.just {{ args }}

# Run recipes from based/main-node.just
main-node *args:
    just -f based/main-node.just {{ args }}

# Run recipes from based/follower-node.just
follower-node *args:
    just -f based/follower-node.just {{ args }}

# Run recipes from based/monitoring.just
monitoring *args:
    just -f scripts/monitoring.just {{ args }}

## Action access (verb component)

# View logs for the given service
logs name:
    just -f scripts/logs.just {{ name }}

# Start the given service
start name:
    {{ self }} {{ name }} start

# Stop the given service
stop name:
    {{ self }} {{ name }} stop

# Run a test recipe described in scripts/test.just
test name:
    just -f scripts/test.just {{ name }}

# Cleanup all the local state of the project
reset:
    {{ self }} main-node reset
    {{ self }} follower-node reset
    rm -rf $LOCAL_DATA

# Run a recipe from scripts/ci.just
ci *args:
    just -f scripts/ci.just {{ args }}

# TODO: consider some sort of interactive config if needed
quick-start:
    {{ self }} main-node config-with-deploy
    {{ self }} main-node start
    {{ self }} follower-node create-config
    {{ self }} follower-node start-dev

    echo "Waiting for 10 seconds before starting the overseer" && sleep 10
    {{ self }} overseer start

# Cleanup all local state
reset-and-start-full-stack-local:
    #!/usr/bin/env bash
    set -euo pipefail

    export PUBLIC_IP=127.0.0.1

    echo "Ensuring required environment variables are available..."
    echo 'OP_BATCHER_KEY={{ env("OP_BATCHER_KEY") }}'
    echo 'OP_PROPOSER_KEY={{ env("OP_PROPOSER_KEY") }}'
    echo 'OP_SEQUENCER_KEY={{ env("OP_SEQUENCER_KEY") }}'

    echo "Resetting configuration and deploying new L2 from scratch"
    {{ self }} reset || true

    {{ self }} main-node config-with-deploy
    {{ self }} main-node start
    {{ self }} follower-node create-config
    {{ self }} follower-node start-dev

    echo "Waiting for 15 seconds before triggering peering and starting the overseer" && sleep 15

    python peering.py
    # {{ self }} start overseer
