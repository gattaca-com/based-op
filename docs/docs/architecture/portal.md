---
description: Unlocking external block production
---

# Portal

The based portal is a sequencer sidecar that enables seamless integration of based components with the existing stacks. It sits between the sequencer OP node and the EL, intercepting the [Engine API](https://specs.optimism.io/protocol/exec-engine.html) calls and multiplexing them to gateways.

The portal is based on an original design called [Rollup Boost](https://github.com/flashbots/rollup-boost), with added support for multiple gateways. Currently the schedule is static and gatewyas are picked in a round-robin fashion for sequencing for a set number of L2 blocks. Eventually, the schedule will be computed from an on-chain lookahead.

## Flows
Depending on the call, the portal will multiplex to one or multiple gateways, as well as always forwarding to the fallback EL for redundancy and verification: 
- `engine_forkchoiceupdatedv3`, if the call has payload attributes it triggers the start of a block building. The portal forwards this to a single elected gateway, as well as the fallback. If the call has no attributes the call is forwarded to all gateways in order to keep them synced with the tip of the chain
- `engine_getPayloadV3`, the portal polls the previously selected gateway for a new execution payload. The payload is then verified with the fallback EL as a sanity check and returned to the OP node if successful, to be added to the canonical chain. If the gateway doesn't return a payload or the payload is invalid, the payload from the fallback is used instead, ensuring liveness.
- `engine_newPayloadV3`, the portal forwards the calll to all gateways and the fallback. Gateways will optimistically apply blocks to process them faster once the next FCU is received.

### Note
Currently, the portal also mutliplexes `eth_` calls, sharing transactions with both fallback and gateways, and requesting state reading calls (e.g. `eth_getBalance`) from the gateway first. This is a temporary solution and we don't expect this to ship in production.
Eventually only transactions will be shared between the gateway and fallback. Users will make other RPC calls directly to follower nodes, or listening to frags in the p2p.

#### References
- [OP protocol: external Block Production](https://github.com/ethereum-optimism/design-docs/blob/main/protocol/external-block-production.md)
- [Rollup Boost](https://github.com/flashbots/rollup-boost)