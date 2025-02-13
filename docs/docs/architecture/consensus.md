---
description: Upgrades to the OP node
---

# OP Node

The OP node was slightly modified to support the new architecture. We have extended its communications protocols as we added a new namespace to the RPC server, a new capability to the P2P server, and extended the current engine API. The new namespace is called `based` and includes new methods to share new envs, frags, and seals with the root OP node, which will then broadcast them to the network. The new capability is the ability to send messages to the execution layer (EL) once the gateway's signature is verified. Lastly, the engine API was extended to include new methods to send the new messages to the EL.

![op-node](./architecture.png)

## Based RPC

A new namespace `based` is added to the RPC server, which includes new methods. These methods are called by the gateway to share new envs, frags, and seals with the root OP node, which will then broadcast them to the network.

### `based_env`

Receives an env envelope, which includes the env itself and the signature of the gateway.

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `env` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```
curl --request POST   --url <follower_node_host>:<op_node_port> --header 'Content-Type: application/json' \
--data '{ \
    "jsonrpc": "2.0", \
    "id": 1, \
    "method": "based_newFrag", \
    "params": [ \
        { \
            "signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
            "frag": { \
                "blockNumber": $(BLOCK_NUMBER), \
                "seq": 0, \
                "isLast": false, \
                "txs": [], \
                "version": 0 \
            } \
        } \
    ] \
}'
```

### `based_newFrag`

Receives a frag envelope, which includes the frag itself and the signature of the gateway.

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `frag` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```Shell
curl --request POST   --url <follower_node_host>:<op_node_port> --header 'Content-Type: application/json' \
--data '{ \
    "jsonrpc": "2.0", \
    "id": 1, \
    "method": "based_sealFrag", \
    "params": [ \
        { \
            "signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
            "seal": { \
                "totalFrags": 2, \
                "blockNumber": $(BLOCK_NUMBER), \
                "gasUsed": 0, \
                "gasLimit": 0, \
                "parentHash": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "transactionsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "receiptsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "stateRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "blockHash": "0x1234567890123456789012345678901234567890123456789012345678901234" \
            } \
        } \
    ] \
}'
```

### `based_sealFrag`

Receives a frag envelope, which includes the frag itself and the signature of the gateway.

#### Parameters

- `signature`: The signature of the gateway.
- `message`: The `seal` message.

#### Returns

- `OK` if the frag was successfully received and published.
- `ERROR` if the frag was not successfully received/published.

#### Example

```Shell
curl --request POST   --url <follower_node_host>:<op_node_port> --header 'Content-Type: application/json' \
--data '{ \
    "jsonrpc": "2.0", \
    "id": 1, \
    "method": "based_env", \
    "params": [ \
        { \
            "signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
            "env": { \
                "totalFrags": 2, \
                "number": 0, \
                "beneficiary": "0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a", \
                "timestamp": 2739281173, \
                "gasLimit": 0, \
                "baseFee": 0, \
                "difficulty": 0, \
                "prevrandao": "0x1234567890123456789012345678901234567890123456789012345678901234" \
                "parentHash": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "parentBeaconRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
                "extraData": "0x010203", \
            } \
        } \
    ] \
}'
```

## Based P2P Capability

The P2P server is extended to include a new capability to broadcast messages to other OP nodes of the network. For more information about the P2P upgrade, please refer to the [P2P documentation](./p2p.md).

## Based Engine API Upgrade

New methods in the namespace are added to enable OP node to send the new messages to the execution layer (EL). We only send the message from the original envelope once the gateway's signature is verified. These methods are:

- `engine_newFragV0`
- `engine_sealFragV0`
- `engine_envV0`
