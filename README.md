# based-op

## Local Development

> [!IMPORTANT]
>
> **Prerequisites**
>
> Before you start, make sure you have the following installed on your machine:
>
> - [Go](https://golang.org/dl/)
> - [Rust](https://www.rust-lang.org/tools/install)
> - [Docker](https://docs.docker.com/get-docker/)
> - [Make](https://www.gnu.org/software/make/)


### Quick start
#### With existing OP chain
The following steps have been tested on Sepolia, with a previously deployed L2 chain (non-pectra) 
1. locate your `rollup.json`, `genesis.json` and `state.json` files
2. run `make config-main-node OP_NODE_DATA_DIR=<path/to/op-node/data> OP_GETH_DATA_DIR=<path/to/op-geth/data> ROLLUP_JSON=<path/to/rollup.json> GENESIS_JSON=<path/to/genesis.json> STATE_JSON=<path/to/state.json>`
3. there should be some files set up in `.local_main_node`
4. start sequencing the main chain with `make start-main-node OP_BATCHER_KEY=<op-batcher-private-key> OP_PROPOSER_KEY=<op-proposer-private-key> MAIN_KEY=<vault-key/main-sequencing key> L1_RPC_URL=<sepolia rpc url> L1_BEACON_RPC_URL=<sepolia beacon rpc url>`
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000` 
7a. `make stop-main-node` to stop all the sequencing services
7b. `make logs-main-node` to output logs of all the main services

#### Deploy a new l2 chain on Sepolia
1. To deploy a new chain on l2, make sure to have an address on Sepolia with some funds. This will be used as the `MAIN`/`vault` address.
2. create 2 more accounts, deposit ~0.2 eth in them. One will be used for the `op-batcher` one for the `op-proposer.
3. run `make deploy-chain OP_BATCHER_KEY=<op-batcher private key> OP_PROPOSER_KEY=<op-proposer private key> MAIN_KEY=<vault key> L1_RPC_URL=<l1 sepolia rpc url> L1_BEACON_RPC_URL=<l1 sepolia beacon rpc url>`
4. start sequencing the main chain with `make start-main-node OP_BATCHER_KEY=<op-batcher-private-key> OP_PROPOSER_KEY=<op-proposer-private-key> MAIN_KEY=<vault-key/main-sequencing key> L1_RPC_URL=<sepolia rpc url> L1_BEACON_RPC_URL=<sepolia beacon rpc url>`
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000` 
7a. `make stop-main-node` to stop all the sequencing services
7b. `make logs-main-node` to output logs of all the main services

#### Run a gateway
1. run `make start-gateway PORTAL=<portal rpc url> GATEWAY_SEQUENCING_KEY=<private key used to sequence with>`
2. to stop the sequencer run `make stop-gateway`
3. for logs `make logs-gateway`

## Wallets

Wallets commonly use a high polling interval for the transaction receipt. To be able to see the preconfirmation speed, we modify Rabby to speed up that interval. You can test it compiling it:

```sh
make build-rabby
```

And importing it to your browser locally (see [Firefox](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/) or [Chrome](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world?hl=es-419#load-unpacked) references). The compiled extension directory is `rabby/dist` for Google Chrome, and `rabby/dist-mv2` for Mozilla Firefox.

### Connecting your local wallet to your local follower node

> [!WARNING]
> You need to have our modified Rabby extension installed.

1. Open your Rabby extension.
2. Import, create a new wallet, or use your existing one.
3. Click on "More".
4. Scroll down to the "Settings" section.
5. Click on "Add Custom Network".
6. Fill the form with the following values:
   - Network Name: `Based-OP`
   - RPC URL: `http://localhost:8545`
   - Chain ID: `2151908`
   - Symbol: `ETH`
   - Block Explorer URL: `<TODO>`
7. Check the "This network supports preconfirmations" option.
8. Click on "Confirm".

Now, you have added your local follower node RPC as the custom network.

### Witnessing Preconfirmations

> [!WARNING]
> You need to have our modified Rabby extension installed and connected to your local follower node.

1. Open your Rabby extension.
2. Click on "Send".
3. Click on the "Chain" dropdown and select "Based-OP".
4. Fill the form.
5. Click on "Send".
6. Sign the transaction.
7. Enjoy.
