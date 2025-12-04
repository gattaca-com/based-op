assets := canonicalize(join(justfile_directory(), "..", ".github", "assets"))

parent := "just -f " + join(justfile_directory(), "..", "Justfile")

# Prepare chain configuration from known values
prepare:
    {{parent}} main-node create-config {{assets}}

run:
    {{parent}} main-node start
    {{parent}} follower-node create-config
    {{parent}} follower-node start-dev
