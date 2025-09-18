## About

Chain replication provides a testground where it possible to replay a range of
past L2 blocks. The goal is to assess whether the based-op stack is able to
handle consistently some load, and end up with the same state as the target
chain it is replaying.

### Chain support

The chain supported is based-op-sepolia. While some of the components are
chain-agnostic, currently full-support for other op-stack chains would require
either some specific credentials (like the sequencer key), or making some of the
gateway and chain replication internals compatible with past version of Engine
API messages.

## Running it

### OS and Docker compatibility

In brief:

1. Run this testground on a Linux system
2. [Rootless Docker](https://docs.docker.com/engine/security/rootless/) is
   required for it to work.

**Rationale**

While developing this testground on a MacOS laptop, I've been fairly surprised
in seeing my Gateway service silently killed by the Linux kernel with a `SIGBUS`
error. After many attempts at troubleshooting, I narrowed it down to some
incompatibilities in volume mounting between MacOS and Linux of the Reth (and
underlying MDBX) database uses by the Gateway.

Rootless Docker is required because the binary interacts with some of the
volumes mounted in Docker compose. The binary needs to have read-access to the
Gateway database to check its synchronization or rollback status and wait
accordingly. If the Docker daemon is run as root, volumes like the database can
only be read by root.

### Starting it up

To get started, make a copy of the `.env.example` file:

```sh
cp .env.example .env
```

Edit your `.env` file if needed. This environment variable file configures both
the main binary and all the containers that are spawn. What you'll want to edit
mainly is the `BOP_REPLAY_BLOCKS_RANGE` variable, which targets which blocks we
want to replay in the test.

Then, we have to generate a custom `prometheus.yml` with the contents of the
`.env` file. _In a separate terminal_, run:

```bash
export BOP_REPLAY_HOST_IP=$(curl -4 ifconfig.me)
set -a && source ./.env && set +a && envsubst < ./monitoring/prometheus/prometheus.yml.tmpl > ./monitoring/prometheus/prometheus.yml
```

Then, start the binary with `cargo run`.

When the test completes, in case of either failure or success, containers are
not stopped nor removed. You have to do it manually. The reason is, in both
cases you might want to keep your services up to inspect them and see their
logs, or keep taking a look at the provided Grafana dashboard.

To shut everything down, run:

```sh
docker compose --file ./monitoring/compose.yml --env-file ./.env down
docker compose --file ../.local_gateway_and_follower_based-op-sepolia/compose --env-file ../.local_gateway_and_follower_based-op-sepolia/.env down
```

## How it works

Internally, the replication is achieve with two main components: a _spawner_,
and a _driver_, that run in the same binary.

**Spawner**

The spawner, as the name suggests, is responsible of spinning up all (or most)
of the follower node services and their configuration requirements, including:
`based-op-geth`, `based-op-node`, `based-registry`, `based-gateway`.

Moreover, the spawner duty is to make sure all the components are at the right
head of the L2 chain we want to replay. As such, it sends either synchronization
or rollback signals to the required components.

At the time of writing, the spawner and its docker compose file is able to spawn
two pairs of follower nodes, so that also frag propagation is tested.

To spawn these components, the binary will runs small child processes with
commands to spin up dedicated docker compose services.

**Driver**

The driver is the long lived service that runs in the binary and drives the
chain replication testing until completeness. To achieve so, such component
behaves as an arbiter, or coordinator, between the different components of the
testground.

In particular, the driver is responsible to move the Gateway to the different
stages of block production. As such, the driver sends proper Engine API messages
and runs along with a [Kona](https://github.com/op-rs/kona) sequencer node to
send unsafe L2 payloads. This setup is crucial to achieve the functionality of a
main node while being very flexible for our use-case.

## Limitations

Chain replication has some natural limitations due to Ethereum L1 and L2 clients
not made with this purpose in mind. In no particular order we have:

- Geth by defaults supports rolling back the chain down to the
  [`FullImmutabilityThreshold`](https://github.com/gattaca-com/based-op-geth/blob/3d1a8f60a54660607157dda514984844b41dbd88/params/network_params.go#L31-L35)
  of `90_000` blocks. This means jumping between different segments of the chain
  for chain replication purposes can be limited. However, this value can be
  extended in a fork of such codebase.

- Geth rolling back speed is quite low, so we can expect some waiting times when
  rolling back a large segment of the chain

- Rolling back the chain on Geth is an unsafe procedure that must not be
  interrupted while in process, otherwise there is a high chance of corrupting the
  database, leading to a wipe-out and re-sync in order to be able to perform such
  tests.

- To support chain replication on `op-node` it has been necessary to disable the
  derivation pipeline and other L1-driven events so that to avoid interfering of
  batches with past L2 blocks.
