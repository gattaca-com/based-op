import 'based/scripts/consts.just'

port := env("SPAMOOR_DAEMON_UI_PORT", "8075")
portal := "just -f " + join(justfile_directory(), "..", "Justfile") + " portal"

# Start overseer
start:
    docker exec -it based-gateway overseer \
      --portal-url $({{portal}} address)
      --rich-wallet-key {{DUMMY_RICH_WALLET_PRIVATE_KEY}}
