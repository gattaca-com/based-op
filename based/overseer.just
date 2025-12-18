import './scripts/consts.just'

parent := "just -f " + join(justfile_directory(), "..", "Justfile")

# Start overseer
@start:
    docker exec -it based-gateway overseer \
      --portal-url $({{parent}} portal address) \
      --registry-url $({{parent}} registry address) \
      --rich-wallet-key {{DUMMY_RICH_WALLET_PRIVATE_KEY}}
