---
description: Upgrades to the P2P layer
---

# P2P

To enable block pipelining and fast preconfirmations, the leader gateway shares fragments of blocks (`frags`), with the network as the block is being built. This approach is inspired by the Solana [Turbine](https://www.helius.dev/blog/turbine-block-propagation-on-solana) protocol and its [shreds](https://github.com/solana-foundation/specs/blob/bff3757c9b7ef3e027105b4c3679e3310592db18/p2p/shred.md). All messages are signed by the current gateway, and nodes will reject any message with an invalid signature (i.e. not corresponding to the current gateway identity).

Currently, these new messages are broadcast leveraging the libp2p gossip already connecting OP nodes, extending it with new message types. In a second moment we plan to add a new faster and leader-aware protocol, that prioritizes leader-to-all and leader-to-next-leader communication.

### Env

This message is shared by the gaetway as soon as it starts building a new block. It contains necessary parameters to set the initial block environment.

```rust
struct EnvV0 {
    number: u64,
    parent_hash: B256,
    beneficiary: Address,
    timestamp: u64,
    gas_limit: u64,
    basefee: u64,
    difficulty: U256,
    prevrandao: B256,
    extra_data: ExtraData,
    parent_beacon_block_root: B256,
}
```

### Frag

As the gateway builds the block, it progressively shares `Frag` messages with the network. 

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
As the follower nodes receive frags, they start pre-validating and simulating transactions. Importantly, nodes can also optimistically serve state off executed frags, since they can reliably assume these transactions be eventually included in the block. Since the frag includes a sequence number, nodes can also request missing ones from peers. 

### Seal

Once the gateway receives an `engine_getPayloadV3` call from the OP node, it first sends off a last `Frag` message with the remaining transactions, and then starts sealing the block. Once the block is sealed the gateway sends a `Seal` message and returns the execution payload to the OP node.

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

If validated, the new block will be shared by the OP node via the pre-existing p2p and will be validated against the received `Seal` message by follower nodes.