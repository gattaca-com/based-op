import '../based/scripts/consts.just'

export BOP_EL_POR := env("BOP_EL_PORT") # TODO: Determine default or how to extract this 
export BOP_NODE_PORT := env("BOP_NODE_PORT") # TODO: Determine default or how to extract this 
export SEQ := env("SEQ") # TODO: Determine default or how to extract this

# Invocation utils
self := "just -f " + justfile()

tx:
    cast publish --rpc-url http://0.0.0.0:$BASED_OP_GETH_PORT {{shell(self + " _dummy-tx")}}

_block-number:
    cast block-number --rpc-url http://localhost:$BOP_EL_PORT

_dummy-tx:
    cast mktx --rpc-url http://0.0.0.0:$BASED_OP_GETH_PORT --private-key $DUMMY_RICH_WALLET_PRIVATE_KEY \
      --value 1 0x7DDcC7c49D562997A68C98ae7Bb62eD1E8E4488a

@_rpc method params="[]":
    curl -X POST --url $FOLLOWER_NODE_HOST:BASED_OP_GETH_PORT -H "Content-Type: application/json" \
     --data '{"jsonrpc", "2.0", method: "{{method}}", "params": {{params}}, "id": 1}'

frag:
    {{self}} _rpc based_newFrag \
      [{ \
        "signature": "0xa47da12abd5563f45332e637d1de946c3576902a245511d86826743c8af1f1e2093d4f5efd5b9630c0acc5f2bb23f236b4f7bdbe0d21d281b2bd2ff60c6cf1861b", \
        "message": { \
          "blockNumber": {{shell(self + " _block-number")}}, \
          "seq": {{SEQ}}, \
          "isLast": true, \
          "txs": [\"{{shell(self + " _dummy-tx | xxd -r -p | base64")}}\"],
          "version": 0,
        } \
      }]

seal:
    {{self}} _rpc based_sealFrag \
      [{ \
        "signature": "0x4fc733cc2f0b680e15452db40b9453412ccb25507582b192c1ea4fc4deaf709845002ab44af42327ed4b8b12943412810a8d9984ea1609dfc6f77338f8c395b41c",
        "message": { \
          "totalFrags": 8, \
          "blockNumber": {{shell(self + " _block-number")}}, \
          "gasUsed": 43806, \
          "gasLimit": 60000000, \
          "parentHash": "0x3d0f61f441af7d1640cb15cd7250bae72d8b334e27245ea44b536407892ec57c", \
          "transactionsRoot": "0x783425e75723ac77ea7f0f47fb4a7858f63deceb80137a0e53fa09703f477cc0", \
          "receiptsRoot": "0x6ff8f783179faedd1aef7e55889a1017ec700504ba6bedffd826a28a47b1a5a2", \
          "stateRoot": "0xc6a987cccdd0665f4d38c730dc05fb8b69497d45094b2b3615954686ff765f87", \
          "blockHash": "0xf3b170b6aee95faa665f77ad1ed0efe7bd29553aa2402e35de7ba3ce55d6974f", \
        } \
      }]


env:
    {{self}} _rpc based_env \
      [{ \
        "signature": "0x4fc733cc2f0b680e15452db40b9453412ccb25507582b192c1ea4fc4deaf709845002ab44af42327ed4b8b12943412810a8d9984ea1609dfc6f77338f8c395b41c",
        "message": { \
          "totalFrags": 2, \
          "number": {{shell(self + " _block-number")}}, \
          "beneficiary": "0x1234567890123456789012345678901234567890", \
          "timestamp": 2739281173, \
          "gasLimit": 3, \
          "baseFee": 4, \
          "difficulty": "0x5", \
          "prevrandao": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
          "parentHash": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
          "parentBeaconRoot": "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758", \
          "extraData": "0x010203", \
        } \
      }]
