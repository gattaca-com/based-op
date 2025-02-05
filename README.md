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

### Available Commands

Run `make` to see the available commands:

```Shell
$ make
build                          🏗️ Build
build-op-node                  🏗️ Build OP node from optimistic directory
clean                          🧹 Clean
deps                           🚀 Install all dependencies
help                           📚 Show help for each of the Makefile recipes
logs                           📜 Show logs
restart                        🔄 Restart
run                            🚀 Run
```

### Restart

> [!WARNING]
> This will remove the based-op enclave.

Run the following to restart the project:

```
make restart
```

### Logging

To view the logs, run the following:

```
make logs
```

### Running multiple OP nodes

To run multiple OP nodes with kurtosis, edit the `config.yml` file adding more items to the `participants` vector. For example, you can run one OP node with reth and two with geth with the following config:

```yaml
optimism_package:
  chains:
    - participants:
        - el_type: op-reth
          cl_type: op-node
        - el_type: op-geth
          cl_type: op-node
        - el_type: op-geth
          cl_type: op-node
      additional_services:
        - blockscout
        - rollup-boost
```
