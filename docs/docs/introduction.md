---
slug: /
---

# Introduction

![Overview](/img/overview.png)

We present here a reference implementation of a [based](https://ethresear.ch/t/based-rollups-superpowers-from-l1-sequencing/15016) backwards-compatible OP stack.

It comprises several components, notably:
- a [Portal](/architecture/portal), upgrading existing sequencers with external block production
- a [Gateway](/architecture/gateway), a new sequencing entity that provides execution preconfs to the rollup
- upgraded [OP node](/architecture/consensus) and [EL](/architecture/execution) changes, including an extended [P2P](/architecture/p2p) network, that enable nodes to pipeline the block processing and serve preconfs before the block time has fully passed

These components were designed by Gattaca and developed in a joint effort by [Gattaca](https://gattaca.com/) and [Lambda Class](https://lambdaclass.com/).