# Overview

For a more in-depth overview, check out our EthResearch [post](https://ethresear.ch/t/becoming-based-a-path-towards-decentralised-sequencing/21733).

## Status quo
- Users currently enjoy an excellent UX on L2s, enabled by soft preconfirmations issued by centralized sequencers.
- By holding the exclusive right to advance the L2 state, these sequencers can give guarantees about the execution of these transactions.
- L2s avoid the overhead of consensus mechanisms, unlocking a design space that achieves performance and throughput far beyond even the most efficient L1s. 

Currently, most L2s rely on  centralized sequencers that require user trust, raising concerns about censorship resistance and liveness. While some L2s include an "escape hatch" mechanism, it often comes at the cost of degraded user experience due to significant delays.

## Why Based Sequencing

- **Based sequencing** enables Ethereum L1 proposers, the most decentralized and credibly neutral set of actors, to provide sequencing services for L2s. This unlocks shared sequencing across multiple rollups and, in turn, synchronous composability across the ecosystem.
- **Preconfirmations** backed by strong economic guarantees can mitigate the delays caused by long L1 slot times. This ensures L2 users maintain an excellent UX provided by fast transactions while benefiting from enhanced economic security and mitigating risks such as liveness failures or regulatory challenges posed by centralized sequencers.

## How to get there

- For Based sequencing to fully leverage the L1 proposer set, **all proposers including solo stakers** must be able to opt in and participate. Inclusivity ensures equitable access to yield, which is critical for maintaining decentralization of the L1 proposer set.
- **Gateways**, staked third-party entities providing specialized sequencing services, will be available to proposers lacking bandwidth and technical capacity to support high-throughput L2s. Gateways are held accountable through strict performance monitoring and are subject to slashing conditions, ensuring reliability and alignment with the network’s security guarantees.

## A phased approach

Achieving fully decentralized, Based sequencing is the ultimate goal. However, it's important to recognize that existing rollups with centralized sequencers may prefer to adopt a more gradual approach to decentralization. Our proposed framework allows for a progressive transition to Based sequencing while minimizing risks and ensuring operational stability and full backwards compatibility.

#### 1. Initial deployment
The existing centralized sequencer is upgraded to a single centralized gateway, using the existing rollup stack as fallback. Additional enhancements such as pipelined block production and replay are available for replica nodes to upgrade.

#### 2. Whitelisted gateways
Additional gateways are introduced via a whitelist enforced by the rollup operator. Gateways are selected in a round-robin fashion and maintain an SLA-like agreement with the rollup operator. In a second step, the lookahead is introduced so that L1 proposers can opt in and delegate to gateways. Slashing is not yet enabled at this step.

#### 3. Permissionless gateways
Gateways can permissionlessly join the sequencer set, and proposers are able to delegate sequencing responsibilities to gateways without any restrictions. Proposers are expected to monitor the performance and reliability of their delegates. At this stage, slashing mechanisms are activated to address safety and liveness faults. Further research is needed to ensure that full decentralization doesn't compromise liveness and UX of the rollup.

