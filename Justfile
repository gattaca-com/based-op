mod deps

@prepare:
    just deps::fetch

build:
    just deps::build

rabby:
    just -f deps/rabby.just build
    ln -s deps/rabby/dist dist/rabby
    ln -s deps/rabby/dist-mv2 dist/rabby-mv2
