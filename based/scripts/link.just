set quiet

export FORCE := env("FORCE", "")

# Links `src` to `dest`, if `src` is not `dest` and `src` exists, otherwise, created `src`
# 
# `src` is the result of evaluating `srcEnv` environment. 
link-dir-if-exists-elsewhere srcEnv $dest:
    #!/usr/bin/env bash
    set -euo pipefail
    
    src=${{srcEnv}}

    if [ -n "$FORCE" ]; then
      rm -rf $dest || true
    fi

    if [[ "$src" != "$dest" && -d "$src" && ! -d "$dest" ]]; then
      ln -s $src $dest
    else
      mkdir -p $dest
    fi
