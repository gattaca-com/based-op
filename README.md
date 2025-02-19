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
> - [Kurtosis CLI](https://docs.kurtosis.com/install/) (installed later in the setup process)

### Quick Start

Run the following to download the dependencies, build, and run the project:

```Shell
make deps build run
```

All the components, including sequencer, gateway, portal, and follower nodes will start in a new kurtosis enclave. To test sending transactions, you can use `make test-tx`

### Available Commands

Run `make` to see the available commands:

```Shell
$ make
build-op-geth                  🏗️ Build OP geth from op-eth directory
build-op-node                  🏗️ Build OP node from optimism directory
build-portal                   🏗️ Build based portal from based directory
build-gateway                  🏗️ Build based gateway from based directory
build                          🏗️ Build
clean                          🧹 Clean
deps                           🚀 Install all dependencies
gateway                        🚀 Run the gateway
help                           📚 Show help for each of the Makefile recipes
logs                           📜 Show logs
restart                        🔄 Restart
run-follower                   🚀 Run a single follower node with RPC enabled.
run                            🚀 Run
```

#### Restart

> [!WARNING]
> This will remove the based-op enclave.

Run the following to restart the project:

```
make restart
```

#### Logging

To view the logs, run the following:

```Shell
make op-node-logs            // OP node logs
make op-reth-logs            // OP reth logs
make gateway-logs            // Based gateway logs
make portal-logs             // Based portal logs

make logs SERVICE=<service>  // Replace <service> with the service name
```

#### Docker Image Build

```Shell
make build-portal            // Build the local portal docker image, named `based_portal_local`
make build-gateway           // Build the local gateway docker image, named `based_gateway_local`
make build-op-geth           // Builds the modified op-geth image, named `based_op_geth`
make build-op-node           // Build the modified op-node image, named `based_op_node`
```

### Running multiple Follower Nodes

To run multiple OP nodes with kurtosis, edit the `config.yml` file adding more items to the `participants` vector:

```yaml
optimism_package:
  chains:
    - participants:
        # Vanilla Stack (OP-Node, OP-EL) for the Sequencer
        - el_type: op-reth
          cl_type: op-node
          cl_image: us-docker.pkg.dev/oplabs-tools-artifacts/images/op-node:latest
        # Follower Node Stack 1 (BOP-Node, BOP-EL)
        - el_type: op-geth
          el_image: based_op_geth
          el_extra_params:
            - --rollup.sequencerhttp
            - http://host.docker.internal:9997
          cl_type: op-node
          cl_image: based_op_node
          cl_extra_params:
            - --rpc.enable-based
        # Follower Node Stack 2 (BOP-Node, BOP-EL)
        - el_type: op-geth
          el_image: based_op_geth
          el_extra_params:
            - --rollup.sequencerhttp
            - http://host.docker.internal:9997
          cl_type: op-node
          cl_image: based_op_node
          cl_extra_params:
            - --rpc.enable-based
      mev_type: based-portal
      mev_params:
        based_portal_image: based_portal_local
        builder_host: "172.17.0.1"
        builder_port: "9997"
      additional_services:
        - blockscout

ethereum_package:
  participants:
    - el_type: geth
      # This is fixed because v1.15.0 (latest) introduces braking changes
      el_image: ethereum/client-go:v1.14.13

```

## Wallets

Wallets commonly use a high polling interval for the transaction receipt. To be able to see the preconfirmation speed, we modify Rabby to speed up that interval. You can test it compiling it:

```sh
make build-rabby
```

And importing it to your browser locally (see [Firefox](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/) or [Chrome](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world?hl=es-419#load-unpacked) references). The compiled extension directory is `rabby/dist`
