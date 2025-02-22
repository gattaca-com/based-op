---
title: Scaling
---

# Scaling

When scaling throughput (increased gas limit / reduced block time), follower (non-sequencing) nodes are under increased pressure to process more and larger blocks. This directly impacts costs for node operators in terms of hardware, bandwidth, and devops.

Increased costs also result in reduced validation of the sequencer, as followers may struggle to keep up and fall behind. This forces users to either wait for the node to catch up or transact on an unvalidated state. In extreme cases, if a node takes longer to sync a new block than the block time, the replica will never catch up to the tip of the chain.

On Ethereum and many L2s, blocks are produced and processed at regular intervals, and nodes process them only once they have been sealed by the sequencer and propagated through the p2p. This means that for most of the block time, nodes are essentially idle, either waiting to produce a new block or waiting for incoming blocks.

## Pipelining

Our approach focuses on targeted changes that are incremental and **backwards compatible**. These improvements upgrade nodes while still allowing regular, non-modified nodes to follow the chain and process blocks as usual.

[Pipelining](https://www.techtarget.com/whatis/definition/pipelining) the block production and replay is a key unlock for scaling nodes. It’s a common concept in computing, and is fundamental in developing high-performance, scalable systems. Pipelining optimizes hardware utilization and increases overall system efficiency, unlocking large performance gains without increasing hardware costs.

### Block production

To increase performance, it is essential for the sequencer to transition to continuous block building, ensuring that useful work is performed throughout the entire block time rather than waiting until the end to create a block. Through pipelining, transactions are continuously simulated, cached, and pre-sorted in parallel. This ensures that when block sealing is triggered, a valid block is immediately available.

To further increase performance and throughput, block time is divided into several sub-slots, each of which is individually sorted. These block fragments (frags for short) are shared with the network before the block is fully built, enabling pipelining of the block replay (discussed below).

### Block replay

By sharing frags as the block is being built, the sequencer allows nodes to utilize block time more efficiently. Nodes can begin pre-processing incoming blocks, which greatly accelerates the processing of new payloads once the block is finalized and sealed. This approach directly impacts throughput by smoothing workloads over the entire block time, making more efficient use of hardware and thereby increasing capacity.

As the sequencer streams frags to the network, nodes process them by reconstructing local “partial blocks”, using an extended endpoint of the Engine API. Nodes are then upgraded to serve RPC calls on the “partial block” before the block is finalized and received, thus unlocking extremely fast transaction confirmation times, unconstrained by block time. 

### Propagation

Propagation initially occurs via the existing p2p by adding new message types support.

Subsequently, the p2p is upgraded to use a high-performance, leader-aware protocol that classifies peers as sequencing or non-sequencing and prioritizes fast sequencer-to-all communication. In a multi-sequencer environment, the gossip layer is aware of the sequencer schedule and optimizes transitions between the current and next sequencer.

### Data Availability

Ethereum imposes a fixed limit on how much blob data can be included in each block. As L2 throughput increases, this constraint becomes a bottleneck. When an L2 posts batches to Ethereum, each transaction, whether it succeeds or reverts, contributes to the overall blob size. In practice, a sizeable fraction of L2 traffic consists of “spam,” such as repeated arbitrage attempts. Because there is no concept of revert protection on L2s (as there is on Ethereum with MEV bundles), spam transactions that revert still occupy space. While these transactions do pay fees, they yield minimal utility once higher-value “real” user transactions start competing for available blob capacity.

By introducing more advanced building algorithms at the gateway layer, it becomes possible to filter or minimise reverting transactions, whether through concepts like ethereum MEV Bundles or outright removal of reverts. This frees up capacity for more meaningful transactions, preventing the fixed blob limit from being overwhelmed by revert spam.

As L2 transaction volume grows, the gateway-based model scales more gracefully by trimming DA waste and reserving space primarily for user transactions.