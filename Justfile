mod deps

# Will error if these binaries are not present
jq := require("jq")
docker := require("docker")

# Verifies that system dependencies are present
@doctor:
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

rabby:
    just -f deps/rabby.just build
    ln -s deps/rabby/dist dist/rabby
    ln -s deps/rabby/dist-mv2 dist/rabby-mv2
