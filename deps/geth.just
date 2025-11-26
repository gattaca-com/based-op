repo_default := "https://github.com/gattaca-com/based-op-geth"
ref_default := "based/develop"

repo := env("BASED_OP_GETH_REPO", repo_default)
ref := env("BASED_OP_GETH_REF", ref_default)

@fetch repo=repo ref=ref:
    REPO={{repo}} REF={{ref}} just -f source.just ensure geth 
    
@link source:
    LOCAL={{source}} just -f source.just ensure geth

# 🏗️ Build OP geth from based-op-geth 
build:
    cd geth && docker build -t local_based_op_geth .
    
