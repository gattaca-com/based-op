# The Gateway: Technical Architecture and Data Flow

This repository serves as the Gateway for a Based rollup. It implements a specialised, high-performance gateway (or sequencer) that supports fast execution preconfirmations via partial block gossip (Frag). Below is a high-level breakdown of its architecture, the core state machine, and the data flow that underpins the system. 

The Gateway acts as the minimum requirement for a sequencer. Essentially, all a gateway needs is an up-to-date state to simulate transactions on—everything else that an EL node has is extra processing that is not strictly necessary here. Hence, the node is stripped down to use just two main sections from an EL node:
1. The database holding the state.  
2. The EVM implementation.

This setup requires the gateway to run alongside a fully operational EL node (such as [Geth](https://github.com/ethereum/go-ethereum) or [Reth](https://github.com/paradigmxyz/reth)) to notify the gateway when it should update its state with a new block. Other than that, the gateway is fully built from scratch to be optimised for building block sequences.

---

## Architecture Diagram
![Gateway Architecture](/img/architecture.png)
---

## High-Level Components

1. **Gateway Actor & State Machine**  
   The central entry point of the gateway is the “Sequencer” struct (see crates/sequencer/src/lib.rs). It implements the “Actor” trait, and its run loop is driven by incoming messages from the “Connections” pool.  
   Internally, the sequencer holds two main items:  
   • A “SequencerState” enum that represents the current phase (for example, “Syncing” or “Sorting”).  
   • A “SequencerContext” struct with the current “Frag” state, connections, transaction pool, and timers.

2. **Gateway Message Handling**  
   The gateway processes the following categories of messages:
   • Engine API messages (forkchoice updates, new payload notifications from the consensus side, or get-payload requests).  
   • New transaction messages (e.g. from user mempool, to be added to the “TxPool”).  
   • Simulation results messages (responses from separate Simulator actors that handle transaction simulation).  
   • Block sync messages (bulk-fetched blocks, used when the gateway is out of sync or dealing with a reorg).

4. **Block Building & Pipelining**  
   Instead of building and sealing blocks only at the end of a block interval, this gateway pipelines its work:
   • Transactions are continuously accepted, validated, and sorted into smaller “Frag”s.  
   • “Frag”s are fully simulated, then broadcast to the network before the block is finalised.  
   • On receiving a get-payload request, the gateway seals the last “Frag”, finalises the block, and sends a “Seal” message to the op-node p2p network.

   Note: Only transaction simulation is currently pipelined. Further pipelining of the sealing process (e.g. pre-building Merkle tries incrementally) may be added later.

5. **P2P Extensions**  
   In addition to invoking “GetPayload”, the gateway uses three p2p message types for broadcasting partial blocks:
   • “EnvV0”: The first message of a new block, enabling follower nodes to set up the block environment before “Frag”s arrive.  
   • “FragV0”: A partial sequence of transactions the gateway has pre-confirmed for the current block.  
   • “SealV0”: A final message confirming the block has been sealed, including data such as the block hash and gas usage.

   Replica nodes process these messages immediately, simulating and pre-validating transactions in parallel. By the time the final block is broadcast via normal means, replicas have already simulated (and can commit) the block, needing only to confirm it matches the “SealV0” data.

6. **Database & State Commit**  
   The gateway uses an underlying Reth DB (accessible via “DatabaseRead” and “DatabaseWrite” traits) to hold state and chain data.
   • State commitment/ revert is handled through the "BlockSync" struct.
   • If enabled, when the gateway is the sequencer, blocks can be immediately commited after "getPayload" is called.

---

## Gateway State Machine

1. **Syncing**  
   The gateway detects it is behind the chain tip and requests missing blocks. It processes them in bulk until fully up to date.

2. **WaitingForNewPayload**  
   The default idle state. The gateway is up to date with the chain and is waiting for a new payload from the consensus layer or another incoming block.

3. **WaitingForForkChoiceWithAttributes**  
   The gateway has just received a payload or block, and may be instructed in a separate forkchoice update to begin building the next block.

4. **Sorting (FragSequence)**  
   The gateway is actively constructing a new block. Transactions flow in, get simulated, and are packaged into “Frag”s that are broadcast in near real-time. Once the system receives “GetPayload” from the consensus layer, the gateway seals any remaining fragment and finalises the block.

These transitions are implemented in `handle_new_payload_engine_api`, `handle_fork_choice_updated_engine_api`, `handle_get_payload_engine_api`, and related functions in `crates/sequencer/src/lib.rs`.

---

## Key Gateway Data Flows

1. **Engine API → Sync or Build**  
   The consensus layer (e.g. an op-node) uses:  
   • “newPayloadV3” to announce a new L2 block. The gateway checks its local database and applies or fetches missing blocks as needed.  
   • “forkchoiceUpdatedV3” to confirm the new chain head. If block-building attributes are present, the gateway transitions into block production.

2. **Transaction Ingestion**  
   User transactions arrive over RPC or other comms. If valid, they are added to the transaction pool. If the gateway is in the “Sorting” phase, they are also queued for simulation in the next “Frag”.

3. **Frag Building & Sealing**  
   As the gateway builds a block, it requests simulations from one or more Simulator actors. Simulated transactions are then included in “Frag”s that are continually broadcast. The final “Frag” is sealed when “GetPayload” arrives, and a “SealV0” message is broadcast to confirm the full block.

---

## Block Sorting

When in the “Sorting” state:
1. Transactions are selected from the tx pool.  
2. Simulation tasks are dispatched to simulator threads.  
3. Results are gathered back into a “SortingData” struct.  
4. The gateway continuously forms new “Frag”s for broadcast.

A block only becomes final and committed once the consensus layer requests “GetPayload”, prompting the gateway to seal the last “Frag” and finalise.
