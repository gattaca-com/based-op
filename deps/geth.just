repo_default := "https://github.com/gattaca-com/based-op-geth"
ref_default := "based/develop"

repo := env("BASED_OP_GETH_REPO", repo_default)
ref := env("BASED_OP_GETH_REF", ref_default)

suppress_output := env("QUIET", "true")
quiet := if suppress_output == "true" { "--quiet > /dev/null 2>&1 " } else { "" }

set quiet

@fetch repo=repo ref=ref:
    REPO={{repo}} REF={{ref}} just -f source.just ensure geth 
    
@link source:
    LOCAL={{source}} just -f source.just ensure geth

# 🏗️ Build OP geth from based-op-geth 
[working-directory: 'geth']
build:
    echo "Building based-op-geth..."
    docker build -t local_based_op_geth . {{quiet}}
