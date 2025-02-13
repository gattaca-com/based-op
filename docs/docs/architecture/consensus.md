---
description: Upgrades to the OP node
---

# OP Node

The OP node was upgraded to support the new based architecture and process `Frag`s. 

The main changes include:
- a new capability in the P2P server, to support the [new](/architecture/p2p.md) messages
- a new `based_` RPC namespace, used by the local gateway to make the initial push of p2p messages
- an extended `EngineAPI` to enable processing of `Frag` in the execution layer after signature verification

Importantly, all these changes are incremental and backwards compatible. Non-modified nodes will still be able to join and receive new blocks, they just won't be able to process `Frag`s and provide preconfirmations.

![op-node](../../static/img/architecture_consensus.png)

## P2P Capability

The P2P server is extended to include a new capability to broadcast messages to other OP nodes of the network. The current implementation simply extends the existing libp2p protocol already used by nodes, while in the future we plan to a new faster leader-aware gossip.

## RPC

To avoid re-implementing the full p2p in the gateway, a new `based_` namespace is introduced, used by the gateway to make the initial push of p2p messages, which will then be broacast to the rest of the network.

### `based_env`

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `EnvV0` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```json
{
    "id": 1,
    "jsonrpc": "2.0",
    "method": "based_env",
    "params": [{
        "message": {
            "basefee": 4,
            "beneficiary": "0x1234567890123456789012345678901234567890",
            "difficulty": "0x5",
            "extraData": "0x010203",
            "gasLimit": 3,
            "number": 1,
            "parentBeaconBlockRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "parentHash": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "prevrandao": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "timestamp": 2
        }],
        "signature": "0x4fc733cc2f0b680e15452db40b9453412ccb25507582b192c1ea4fc4deaf709845002ab44af42327ed4b8b12943412810a8d9984ea1609dfc6f77338f8c395b41c"
    }
}
```

### `based_newFrag`

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `FragV0` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```json
{
    "id": 1,
    "jsonrpc": "2.0",
    "method": "based_newFrag",
    "params": [{
        "message": {
            "blockNumber": 1,
            "isLast": true,
            "seq": 0,
            "txs": [
                "0x010203"
            ]
        }],
        "signature": "0xa47da12abd5563f45332e637d1de946c3576902a245511d86826743c8af1f1e2093d4f5efd5b9630c0acc5f2bb23f236b4f7bdbe0d21d281b2bd2ff60c6cf1861b"
    }
}
```

### `based_sealFrag`

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `SealV0` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```json
{
    "id": 1,
    "jsonrpc": "2.0",
    "method": "based_sealFrag",
    "params": [{
        "message": {
            "blockHash": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "blockNumber": 123,
            "gasLimit": 1000000,
            "gasUsed": 25000,
            "parentHash": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "receiptsRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "stateRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            "totalFrags": 8,
            "transactionsRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"
        }],
        "signature": "0x090f69ccf02e0f468cac96f71bbf4b7732c63f3d50a4881f8665c1718570928e4497706eac2fe7da8b47ce355482ada8763614a3575a1af066ad06320b707c531b"
    }
}

```

## Engine API Upgrade

New methods in the `based_` namespace are added to enable the OP node to send the `Frag`s payloads to the execution layer (EL). Messages are only sent after singature verification of the gateway identity. The new methods are:

- `engine_newFragV0`
- `engine_sealFragV0`
- `engine_envV0`

the processing is detailed in the [next](/architecture/execution.md) section.