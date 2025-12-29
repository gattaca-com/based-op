import 'based/scripts/consts.just'

tool_image := env("SPAMOOR_TOOL_IMAGE", "ghcr.io/chainbound/spamoor-op-geth")

port := env("SPAMOOR_DAEMON_UI_PORT", "8075")

# Start spamooor as a foreground process
start config:
    docker run --pull always --network host \
     -v {{canonicalize(config)}}:/etc/spamoor-config.yml \
     {{tool_image}} \
     run /etc/spamoor-config.yml \
     --privkey {{DUMMY_RICH_WALLET_PRIVATE_KEY}}
     --rpchost http://localhost:{{BASED_OP_GETH_PORT}}

# Start spamoor as a daemon with UI on port `SPAMOOR_DAEMON_UI_PORT` (default 8075)
daemon config="../spamoor-daemon": #TODO: where is this folder?
    docker run --rm --network host --entrypoint {{config}} \
    {{tool_image}} --privkey {{DUMMY_RICK_WALLET_PRIVATE_KEY}} \
    --rpchost http://localhost:{{BASED_OP_GETH_PORT}} \
    --db /tmp/spamoor.db --port {{port}}
