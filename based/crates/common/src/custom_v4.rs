use alloy_primitives::{B256, Bytes, U256};
use alloy_rpc_types::engine::BlobsBundleV1;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV4, OpExecutionPayloadV4};
use serde::{Deserialize, Deserializer};

pub fn null_as_empty_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// This structure maps for the return value of `engine_getPayload` of the beacon chain spec, for
/// V4.
///
/// See also:
/// [Optimism execution payload envelope v4] <https://github.com/ethereum-optimism/specs/blob/main/specs/protocol/exec-engine.md#engine_getpayloadv4>
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpExecutionPayloadEnvelopeV4Patch {
    /// Execution payload V4
    pub execution_payload: OpExecutionPayloadV4,
    /// The expected value to be received by the feeRecipient in wei
    pub block_value: U256,
    /// The blobs, commitments, and proofs associated with the executed payload.
    pub blobs_bundle: BlobsBundleV1,
    /// Introduced in V3, this represents a suggestion from the execution layer if the payload
    /// should be used instead of an externally provided one.
    pub should_override_builder: bool,
    /// Ecotone parent beacon block root
    pub parent_beacon_block_root: B256,
    /// A list of opaque [EIP-7685][eip7685] requests.
    ///
    /// [eip7685]: https://eips.ethereum.org/EIPS/eip-7685
    #[serde(default, deserialize_with = "null_as_empty_vec")]
    pub execution_requests: Vec<Bytes>,
}

impl OpExecutionPayloadEnvelopeV4Patch {
    /// Converts this envelope to the V4 format.
    pub fn into_v4(self) -> OpExecutionPayloadEnvelopeV4 {
        OpExecutionPayloadEnvelopeV4 {
            execution_payload: self.execution_payload,
            block_value: self.block_value,
            blobs_bundle: self.blobs_bundle,
            should_override_builder: self.should_override_builder,
            parent_beacon_block_root: self.parent_beacon_block_root,
            execution_requests: self.execution_requests,
        }
    }
}

mod tests {
    #[test]
    fn test_op_execution_payload_envelope_v4_patch() {
        use crate::custom_v4::OpExecutionPayloadEnvelopeV4Patch;
        let raw = r#"{"executionPayload":{"parentHash":"0x70fa255460001cc0e676a5d92e7aa2dfaa792db30e79f052bff2a87f06c21913","feeRecipient":"0x4200000000000000000000000000000000000011","stateRoot":"0x6157c83f4d5578eb78816916e988d7865edae5070f6e3e01a9c747478929ff0c","receiptsRoot":"0x6a8486e713e867135784dfe6399f31a7a12acb9118ca4206d0160cc2c2454323","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","prevRandao":"0xd73f0eaa07f8428105bb4edce3f120accf3dd99e688a3fb56e30d1cd8a2a368e","blockNumber":"0x5c6","gasLimit":"0x3938700","gasUsed":"0xcbfc","timestamp":"0x68a713bc","extraData":"0x00000000fa00000006","baseFeePerGas":"0x29eef4","blockHash":"0xbed80760570d9cb9250dc44825fe5c26348d0944c529cced1f246f00616c8648","transactions":["0x7ef8f8a08130f5ef2dab927d7f8caef91599d10c46032c3dbb88c6be91d66eb0df72815494deaddeaddeaddeaddeaddeaddeaddeaddead00019442000000000000000000000000000000000000158080830f424080b8a4440a5e20000f42400000000000000000000000000000000068a71370000000000089d3760000000000000000000000000000000000000000000000000000000001cdd2d500000000000000000000000000000000000000000000000000000000000000324e8d7cbe34dcb634d16cd8756eb3da5967d3138273762732998d50642cbebd6e000000000000000000000000f3c4fb7320a6795b9b1843e26fb925a2393aad84"],"withdrawals":[],"blobGasUsed":"0x0","excessBlobGas":"0x0","withdrawalsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"},"blockValue":"0x0","blobsBundle":{"commitments":[],"proofs":[],"blobs":[]},"executionRequests":null,"shouldOverrideBuilder":false,"parentBeaconBlockRoot":"0x6f3474c7332a629e53b75e154a46dc60833ae4fe213eab70d3c49ed867ef42a2"}"#;
        let envelope: OpExecutionPayloadEnvelopeV4Patch = serde_json::from_str(raw).unwrap();
        println!("{:#?}", envelope);
    }
}
