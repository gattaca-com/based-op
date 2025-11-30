mod deps

# Will error if these binaries are not present
jq := require("jq")
docker := require("docker")

# Verifies that system dependencies are present
@check:
    echo "jq: {{jq}}"
    echo "docker: {{docker}}"

@prepare:
    just deps::fetch
    cd docs && npm i

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

spamoor *args=("start" "./spamoor-config.yml"):
    just -f scripts/spamoor.just {{args}}
