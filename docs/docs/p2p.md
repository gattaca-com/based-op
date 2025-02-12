# P2P upgrades

To enable block pipelining and fast preconfirmations, the sequencer shares fragments of blocks (`frags`), with the network as the block is being built. This approach is inspired by the Solana Turbine protocol and its shreds.

In the first iteration, the p2p gossip already connecting op-nodes via libp2p is extended with two new messages.

### Frag
This message contains a list of transactions just pre-confirmed by the sequencer.

```rust
struct FragV0 {
    /// Block in which this frag will be included
    block_number: u64,
    /// Index of this frag. Frags need to be applied sequentially by index, up to [`SealV0::total_frags`]
    seq: u64,
    /// Whether this is the last frag in the sequence
    is_last: bool,
    /// Ordered list of EIP-2718 encoded transactions
    txs: Transactions,
}
```
As the replica nodes receive frags, they start pre-validating and simulating transactions. Importantly, nodes also optimistically serve state off executed frags, assuming they will be eventually included in a block. Since the frag includes a sequence number, nodes can also request missing ones from other peers. 

### Seal
Once the sequencer receives a `GetPayload` call its `op-node`, it first sends off a last `Frag` message, and then starts sealing the block. Once the block is sealed the sequencer sends a `Seal` message via p2p and returns the sealed execution payload to the `op-node`.

```rust
struct SealV0 {
    /// How many frags for this block were in this sequence
    total_frags: u64,
    // Header fields
    block_number: u64,
    gas_used: u64,
    gas_limit: u64,
    parent_hash: B256,
    transactions_root: B256,
    receipts_root: B256,
    state_root: B256,
    block_hash: B256,
}
```

The full block will then be shared by the sequencer `op-node` via the pre-existing p2p and will be validated against the received `Seal` message by replica nodes.

[See also EL changes page]


### A faster gossip
While the first iteration leverages the existing libp2p gossip, we plan to upgrade the protocol to a leader-aware gossip which differentiates sequencing vs non-sequencing peers and is optimized for fast leader-to-all communication. Once multiple sequencers are enabled, the protocol will be aware of the leader schedule (lookahead), and also optimize for leader to next-leader communication.