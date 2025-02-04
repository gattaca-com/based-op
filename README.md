# based-op

## Local Development

### Prerequisites

Before you start, make sure you have the following installed on your machine:

- [Go](https://golang.org/dl/)
- [Rust](https://www.rust-lang.org/tools/install)
- [Docker](https://docs.docker.com/get-docker/)
- [Make](https://www.gnu.org/software/make/)
- [Kurtosis CLI](https://docs.kurtosis.com/install/) (installed later in the setup process)

### Install Dependencies

Running the following will install the necessary dependencies for the project. For now it only installs the Kurtosis CLI, and Rust since Go needs to be installed manually, and Docker could need to be installed manually depending on your OS.

```Shell
make deps
```

### Secrets

If you do not have a secrets file, running the following will crate one in `$HOME/secrets/jwt.hex`.

```Shell
make secrets
```

### Build the Project

Running the following will build the project binaries.

```Shell
make build
```

### Run the Project

```Shell
make op-node
make op-geth
make mux
```
