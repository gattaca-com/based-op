# based-op

## Local Development/Quick start

### With existing OP chain

The following steps have been tested on Sepolia, with a previously deployed L2 chain

1. locate your `rollup.json`, `genesis.json` and `state.json` files
2. run `just main-node create-config /path/to/folder/with/{rollup.json, genesis.json, state.json}`
3. there should be some files set up in `.local/local_main_node`
4. start sequencing the main chain with `just start main-node`, ensuring `OP_PROPOSER_KEY`, `OP_BATCHER_KEY` and `OP_SEQUENCER_KEY` environment variables are set with the private key of the corresponding entity for the chain
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000`
   7a. `just stop main-node` to stop all the sequencing services
   7b. `just stop monitoring` to stop all monitoring services (Grafana, Prometheus, etc)
   7c. `just logs main-node` to output logs of all the main services

### Deploy a new l2 chain on Sepolia

1. To deploy a new chain on l2, make sure to have an address on Sepolia with some funds. This will be used as the `MAIN`/`vault` address.
2. create 2 more accounts, deposit ~0.2 eth in them. One will be used for the `op-batcher` one for the `op-proposer`.
3. run `just main-node config-with-deploy`, ensuring `OP_BATCHER_KEY`, `OP_PROPOSER_KEY` and `OP_SEQUENCER_KEY` environment variables are set with the private key of the corresponding entity for your new chain
4. start sequencing the main chain with `just main-node start`, ensuring the same environment variables as set
5. Normally you should see some logs starting
6. `blockscout` should be up and running at `http://0.0.0.0:4000`
   7a. `just stop main-node` to stop all the sequencing services
   7b. `just stop monitoring` to stop all monitoring services (Grafana, Prometheus, etc)
   7c. `just logs main-node` to output logs of all the main services

### Run a Based Gateway

In the following, all defaults are set up to target the [`Based OP` testnet](https://based-explorer.gattaca.com), deployed on top of mainnet Sepolia.
With that default config, a new `private-key` and `wallet` combo will be generated which your `Gateway` will use to sign `Frags`.
The `wallet` is communicated back to the `Portal` to be gossiped around to the rest of the network for signature verification.

The following single command sets up everything and will start your `Gateway`, `Based OP-node`, `Based OP-geth`:
`git clone https://github.com/gattaca-com/based-op && cd based-op && just start follower-node`

If everything went well, you should see a terminal UI appear, called the `Overseer`:

![ ](./docs/static/img/based_op.gif)

This shows you the status of your local `Gateway` and a general overview of the chain.
You can press left and right keys to cycle between the different tabs and explore all the information!

The configuration that was generated can be found in `based-op/.local/gateway_and_follower`, mainly the `.env` and `compose.yml` files.

When you [spam some transactions with `based-bmf`](https://based-bmf.gattaca.com), you should see them appear in the `Transaction Pool` of your `Gateway`.

A couple of commands tend to come in handy (from the top `based-op` directory):

- `just stop follower-node`
- `just start follower-node`
- `just start overseer`
- `just logs follower-node`
- `just logs based-op-node`
- `just logs based-op-geth`

### Add/Update based-gateways to Registry

When a based-gateway is started with `just follower-node start`, it will register itself to the Registry behind the `PORTAL`. For now, the Registry is kept in a simple json file in `.local_main_node/config/registry.json`. You can add/update/remove gateways there, the Registry and Portal will pick up on the changes every minute.

If you have started both the main sequencing node and the gateway on the same machine, you might need to change the ip to `0.0.0.0`, by default `curl ifconfig.me` is used to populate
your url.

### Send your first tx

You can now test sending a transaction with `just test tx`.
The transaction will be sent to the Portal, and forwarded to the gateway, which will sequence the transaction in a new Frag, and broacast it via p2p to follower nodes.

> [!IMPORTANT]
>
> **The following is experimental**

## Wallets

Wallets commonly use a high polling interval for the transaction receipt. To be able to see the preconfirmation speed, we modify Rabby to speed up that interval. You can test it compiling it:

```sh
just rabby 
```

And importing it to your browser locally (see [Firefox](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/) or [Chrome](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world?hl=es-419#load-unpacked) references). The compiled extension directory is `dist/rabby` for Google Chrome, and `dist/rabby-mv2` for Mozilla Firefox.

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
