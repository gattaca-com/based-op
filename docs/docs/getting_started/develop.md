# Develop

## Prerequisites

Before you start, make sure you have the following installed on your machine:

- [Docker](https://docs.docker.com/get-docker/)
- [Make](https://www.gnu.org/software/make/)

For local development, you will also need:
- [Go](https://golang.org/dl/)
- [Rust](https://www.rust-lang.org/tools/install)

## Quick Start

### With existing OP chain
The following steps have been tested on Sepolia, with a previously deployed L2 chain (non-pectra) 
1. locate your `rollup.json`, `genesis.json` and `state.json` files
2. run `make config-main-node OP_NODE_DATA_DIR=<path/to/op-node/data> OP_GETH_DATA_DIR=<path/to/op-geth/data> ROLLUP_JSON=<path/to/rollup.json> GENESIS_JSON=<path/to/genesis.json> STATE_JSON=<path/to/state.json>`
3. there should be some files set up in `.local_main_node`
4. start sequencing the main chain with `make start-main-node OP_BATCHER_KEY=<op-batcher-private-key> OP_PROPOSER_KEY=<op-proposer-private-key> MAIN_KEY=<vault-key/main-sequencing key> L1_RPC_URL=<sepolia rpc url> L1_BEACON_RPC_URL=<sepolia beacon rpc url>`
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000` 
7a. `make stop-main-node` to stop all the sequencing services
7b. `make logs-main-node` to output logs of all the main services

### Deploy a new l2 chain on Sepolia
1. To deploy a new chain on l2, make sure to have an address on Sepolia with some funds. This will be used as the `MAIN`/`vault` address.
2. create 2 more accounts, deposit ~0.2 eth in them. One will be used for the `op-batcher` one for the `op-proposer.
3. run `make deploy-chain OP_BATCHER_KEY=<op-batcher private key> OP_PROPOSER_KEY=<op-proposer private key> MAIN_KEY=<vault key> L1_RPC_URL=<l1 sepolia rpc url> L1_BEACON_RPC_URL=<l1 sepolia beacon rpc url>`
4. start sequencing the main chain with `make start-main-node OP_BATCHER_KEY=<op-batcher-private-key> OP_PROPOSER_KEY=<op-proposer-private-key> MAIN_KEY=<vault-key/main-sequencing key> L1_RPC_URL=<sepolia rpc url> L1_BEACON_RPC_URL=<sepolia beacon rpc url>`
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000` 
7a. `make stop-main-node` to stop all the sequencing services
7b. `make logs-main-node` to output logs of all the main services

### Run a gateway
1. run `make start-gateway PORTAL=<portal rpc url> GATEWAY_SEQUENCING_KEY=<private key used to sequence with>`
2. to stop the sequencer run `make stop-gateway`
3. for logs `make logs-gateway`

You can now test sending a transaction with `make test-tx`.
The transaction will be sent to the Portal, and forwarded to the gateway, which will sequence the transaction in a new Frag, and broacast it via p2p to follower nodes.
The Portal is only temporarily acting as a multiplexer for `eth_` calls, but we don't expect this to be in the final design.

### Logging

To view the logs, run the following:

```shell
make logs-main-node          // Full main node logs
make logs-gateway            // Based gateway and follower node logs
make logs-portal             // Based portal logs
make op-node-logs            // OP node logs
make op-reth-logs            // OP reth logs
```
