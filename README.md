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
