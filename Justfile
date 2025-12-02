export LOCAL_DATA := canonicalize(env("LOCAL_DATA", shell('mkdir -p .local && echo ".local"')))

mod deps

# Verifies that system dependencies are present
@check:
    echo "jq: {{require('jq')}}"
    echo "docker: {{require('docker')}}"
    echo "cast: {{require('cast')}}"
    echo "rustup: {{require('rustup')}}"

# Prepare the local environment: fetch deps, build them, setup toolchains...
@prepare:
    just deps::fetch
    cd docs && npm i
    cd based && rustup toolchain install 

# 🏗️ Build
@build: deps::build
    just -f based/docker/Justfile all
 
# 📚 Build local docs
docs:
    just -f docs/Justfile serve

# Build and link rabby in the configured output folder
rabby out="./dist":
    just -f deps/rabby.just build
    ln -s deps/rabby/dist {{out}}/rabby
    ln -s deps/rabby/dist-mv2 {{out}}/rabby-mv2

## Component access (component verb)

# Run recipes from scripts/spamoor.just
spamoor *args=("start ./spamoor-config.yml"):
    just -f scripts/spamoor.just {{args}}

# Run recipes from scripts/overseer.just
overseer *args=("start"):
    just -f based/overseer.just {{args}}

# Run recipes from based/portal.just
portal *args:
    just -f based/portal.just {{args}}

# Run recipes from based/registry.just
registry *args:
    just -f based/registry.just {{args}}

# Run recipes from based/main-node.just
main-node *args:
    just -f based/main-node.just {{args}}

# Run recipes from based/follower-node.just
follower-node *args:
    just -f based/follower-node.just {{args}}

# Run recipes from based/monitoring.just
monitoring *args:
    just -f scripts/monitoring.just {{args}}

## Action access (verb component)

# View logs for the given service
logs name:
    just -f scripts/logs.just {{name}}

# Start the given service 
start name:
    just -f {{justfile()}} {{name}} start
 
# Stop the given service 
stop name:
    just -f {{justfile()}} {{name}} stop

# Run a test recipe described in scripts/test.just
test name:
    just -f scripts/test.just {{name}}

# TODO: setup main node, start main node, setup follower-node, start follower-node (& gateway) 
# 
# TODO: consider some sort of interactive config if needed
quick-start:
    exit 1
