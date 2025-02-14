---
slug: /
---

# Introduction

![Overview](/img/overview.png)

We're excited to present a reference implementation of a [Phase 1](/overview#1-initial-deployment) based OP stack. The implementation is fully backwards compatible, and overlays the current stack with high performance components to unlock fast execution preconfs and improve scalability.

The key components are:
- a [Portal](/architecture/portal), upgrading existing sequencers with external block production
- a [Gateway](/architecture/gateway), a new sequencing entity that provides execution preconfs to the rollup on behalf of L1 proposers
- upgraded [OP node](/architecture/consensus) and [EL](/architecture/execution) changes, including an extended [P2P](/architecture/p2p) network, that enable nodes to pipeline the block processing and serve preconfs before the block time has fully elapsed


Designed by [Gattaca](https://gattaca.com/) and developed in collaboration with [Lambda Class](https://lambdaclass.com/), this implementation provides a reference for future development that we hope will help bootstrap based rollups and high-performance gateway solutions.
