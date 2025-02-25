# Get Started

## Prerequisites

Before you start, make sure you have the following installed on your machine:

- [Docker](https://docs.docker.com/get-docker/)
- [Make](https://www.gnu.org/software/make/)
- [Kurtosis CLI](https://docs.kurtosis.com/install/) (installed later in the setup process)

For local development, you will also need:
- [Go](https://golang.org/dl/)
- [Rust](https://www.rust-lang.org/tools/install)

### Quick Start

Clone the repo at: https://github.com/gattaca-com/based-op and use the following command to download the dependencies, build, and run the project:

```shell
make deps build run
```

All the components, including sequencer, gateway, portal, and follower nodes will start in a new kurtosis enclave.
You can now test sending a transaction with `make test-tx`.
The transaction will be sent to the Portal, and forwarded to the gateway, which will sequence the transaction in a new Frag, and broacast it via p2p to follower nodes.
The Portal is only temporarily acting as a multiplexer for `eth_` calls, but we don't expect this to be in the final design.

#### Logging

To view the logs, run the following:

```shell
make gateway-logs            // Based gateway logs
make portal-logs             // Based portal logs
make op-node-logs            // OP node logs
make op-reth-logs            // OP reth logs
```

## Join our devnet

If you want to run a follower node for an existing network, just edit the values in `follower-node/.env` with you custom configuration and run `make run-follower`. Default values are set for connecting to Gattaca's devnet.
