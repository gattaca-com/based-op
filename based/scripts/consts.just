dummy_rich_wallet_default := "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" 

export DUMMY_RICH_WALLET_PRIVATE_KEY := env("DUMMY_RICH_WALLET_PRIVATE_KEY", dummy_rich_wallet_default)

export L1_RPC_URL := env("L1_RPC_URL", "https://ethereum-sepolia-rpc.publicnode.com")
export L1_BEACON_RPC_URL := env("L1_BEACON_RPC_URL", "https://ethereum-sepolia-beacon-api.publicnode.com")

export BASED_OP_GETH_PORT := env("BASED_OP_GETH_PORT", "8645")
export FOLLOWER_NODE_HOST := env("FOLLOWER_NODE_HOST", "http://localhost")
