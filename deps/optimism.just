repo_default := "https://github.com/gattaca-com/based-optimism"
ref_default := "feat/portal-auth"

repo := env("BASED_OP_GETH_REPO", repo_default)
ref := env("BASED_OP_GETH_REF", ref_default)

suppress_output := env("QUIET", "true")
quiet := if suppress_output == "true" { "> /dev/null 2>&1" } else { "" }

export IMAGE_TAGS := "develop"
set quiet

@fetch repo=repo ref=ref:
    REPO={{repo}} REF={{ref}} just -f source.just ensure optimism
    
@link source:
    LOCAL={{source}} just -f source.just ensure optimism 
 
# 🏗️ Build OP node from based-optimism
[working-directory: 'optimism']
build-op-node:
    echo "Building based-op-node..."
    docker buildx bake -f docker-bake.hcl \
      --set op-node.tags=local_based_op_node \
      --load op-node {{quiet}}

# 🏗️ Build OP deployer from based-optimism
[working-directory: 'optimism']
build-op-deployer:
    echo "Building based-op-deployer..."
    docker buildx bake -f docker-bake.hcl \
      --set op-deployer.tags=local_based_op_deployer \
      --load op-deployer {{quiet}}

build: build-op-node build-op-deployer
