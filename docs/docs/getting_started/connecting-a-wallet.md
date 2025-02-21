# Connecting a Wallet

Wallets commonly use a high polling interval for the transaction receipt. To be able to see the preconfirmation speed, we've modified Rabby to speed up that interval.

## Build and Install Modified Rabby Wallet

You can test it by compiling it:

```sh
make build-rabby
```

And importing it to your browser locally (see [Firefox](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/) or [Chrome](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world?hl=es-419#load-unpacked) references). The compiled extension directory is `rabby/dist` for Google Chrome, and `rabby/dist-mv2` for Mozilla Firefox.

## Adding the devnet as a Rabby custom network

To access our devnet, you will need to add the network to your modified wallet.

To manually add the network, follow these steps:

1. Log into your Rabby wallet, click on the **More** button near the bottom right corner.

![1](../../static/img/wallet_tutorial/step_1.png)

2. Click on the **Add Custom Network** below the **Settings** section.

![2](../../static/img/wallet_tutorial/step_2.png)

3. Click in **Add Custom Network**

![3](../../static/img/wallet_tutorial/step_3.png)

4. Fill in the form with the following values:
    - Chain ID: `2151908`
    - Network Name: `Based-OP`
    - RPC URL: `https://rpc-based.gattaca.com`
    - Currency Symbol: `ETH`
    - Block Explorer URL: `https://blockscout.com/poa/xdai`

![4](../../static/img/wallet_tutorial/step_4.png)

5. Toggle the **This network supports preconfirmations** option.

![5](../../static/img/wallet_tutorial/step_5.png)

6. Click on **Confirm**.

![6](../../static/img/wallet_tutorial/step_6.png)

You can now use the wallet to interact with the Based-OP devnet. Read the section below to learn how to witness preconfirmations.
