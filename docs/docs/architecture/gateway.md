---
description: A fast pipelined sequencer
---

# Gateway

The gateway is a specialised, high-performance sequencer that supports fast execution preconfirmations via partial block gossip (frags). It's fully built from the ground-up to be optimised for sequencing, only re-using the bare minimum needed from a EL node, notably: 1) a database holding state, 2) the EVM implementation.

This setup requires the gateway have access to an external RPC node (such as [Geth](https://github.com/ethereum/go-ethereum) or [Reth](https://github.com/paradigmxyz/reth)) for syncing.

![Gateway Architecture](/img/architecture.png)

## High-Level Components

#### Actor Model

Gateway components are implemented using the [Actor](https://en.wikipedia.org/wiki/Actor_model) model.
Each component runs in an infinite loop, receiving messages from either other components or the outside world. Messages are then processed locally, potentially resulting in other messages being sent to other actors. The `Actor` trait is implemented by the `Sequencer`, `BlockFetcher`, `Gossiper`, and `Simulator`s.

#### Gateway Actor & State Machine

The central entry point of the gateway is the [Sequencer](https://github.com/gattaca-com/based-op/blob/main/based/crates/sequencer/src/lib.rs#L59). It implements the `Actor` trait, and its event loop is driven by incoming messages from the `Connections` pool.  

Internally, the sequencer holds two main items:
- `SequencerState`, that represents the current state (for example, `Syncing` or `Sorting`).
- `SequencerContext`, with the current block being built, connections, transaction pool, and timers.

#### Message Handling

The gateway processes the following categories of messages:
- [Engine API](https://specs.optimism.io/protocol/exec-engine.html) messages, including `getPayload` to trigger block sealing
- New transaction messages (e.g. user transactions, bundles).
- Simulation results messages (responses from separate `Simulator` actors that handle transaction simulation).
- Block sync messages (used when the gateway is out of sync or dealing with a reorg).

#### Block Building & Pipelining

Instead of building and sealing blocks only at the end of a block interval, the gateway pipelines its work throughout the block time:
- Transactions are continuously accepted, validated, and sorted into `Frag`s.
- `Frag`s are simulated, then broadcast via [p2p](/architecture/p2p) network before the block is finalised.
- On receiving a `getPayload` request, the gateway seals the last `Frag`, finalises the block, and sends a `Seal` message via p2p.

#### Database & State Commit
   The gateway uses an underlying Reth DB (accessible via `DatabaseRead` and `DatabaseWrite` traits) to hold state and chain data.
   - State commitment / revert is handled through the `BlockSync` struct.
   - If enabled, when the gateway is the sequencer, blocks can be immediately commited after `getPayload` is called.

## State Machine
The gateway adopts a [Finite-State Machine](https://en.wikipedia.org/wiki/Finite-state_machine) model: the system can only be in finite a set of `states` and reacts to `events` to transition to other states.

- **Syncing**: The gateway detects it is behind the chain tip and requests missing blocks. It processes them in bulk until fully up to date.

- **WaitingForNewPayload**: the default idle state. The gateway is up to date with the chain and is waiting for a new payload from the OP node or another incoming block.

- **WaitingForForkChoiceWithAttributes**: the gateway has just received a payload or block, and may be instructed in a separate forkchoice update to begin building the next block.

- **Sorting**: the gateway is actively constructing a new block. Transactions flow in, get simulated, and are packed into `Frag`s that are then broadcast in real-time. Once the system receives `getPayload` via the Engine API, the gateway seals any remaining transaction in a last `Frag` and finalises the block.

## Data Flows

- **Engine API → Sync or Build**: the OP node uses:
   - `newPayloadV3` to announce a new L2 block. The gateway verifies its local database and applies or fetches missing blocks as needed.
   - `forkchoiceUpdatedV3` to confirm the new chain head. If payload attributes are present, the gateway transitions into block production.

- **Transaction Ingestion**: User transactions arrive over RPC. If valid, they are added to a transaction pool. If the gateway is in the `Sorting` phase, they are also queued for simulation to be added in the next `Frag`.

- **Frag Building & Sealing**: As the gateway builds a block, it requests simulations from one or more `Simulator` actors. Simulated transactions are then included in `Frag`s that are continually broadcast. The final `Frag` is sealed when a `getPayload` call arrives from the OP node, and a `SealV0` message is then broadcast to confirm the full block.

## Sorting

When in the `Sorting` state:
1. Transactions are selected from the tx pool.  
2. Simulation tasks are dispatched to simulator threads.  
3. Results are gathered back and sorted. The best transaction is added to the next `Frag`, while others are re-simulated to account for the state change

