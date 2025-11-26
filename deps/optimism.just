repo_default := "https://github.com/gattaca-com/based-optimism"
ref_default := "based/develop"

repo := env("BASED_OP_GETH_REPO", repo_default)
ref := env("BASED_OP_GETH_REF", ref_default)

@fetch repo=repo ref=ref:
    REPO={{repo}} REF={{ref}} just -f source.just ensure optimism
    
@link source:
    LOCAL={{source}} just -f source.just ensure optimism 
 
# 🏗️ Build OP node from based-optimism
build-op-node:
    cd optimism && \
      IMAGE_TAGS=develop \
      docker buildx bake -f docker-bake.hcl \
      --set op-node.tags=local_based_op_node \
      --load op-node
 
# 🏗️ Build OP deployer from based-optimism
build-op-deployer:
    cd optimism && \
      IMAGE_TAGS=develop \
      docker buildx bake -f docker-bake.hcl \
      --set op-deployer.tags=local_based_op_deployer \
      --load op-deployer

build: build-op-node build-op-deployer
