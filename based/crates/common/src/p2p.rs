use alloy_primitives::{Address, Bytes, B256, U256};
use revm_primitives::BlockEnv;
use serde::{Deserialize, Serialize};
use ssz_types::{typenum, VariableList};
use strum_macros::AsRefStr;
use tree_hash::TreeHash;
use tree_hash_derive::TreeHash;

use crate::{signing::ECDSASigner, transaction::Transaction as BuilderTransaction};

#[derive(Debug, Clone, PartialEq, Eq, TreeHash, Serialize, Deserialize, AsRefStr)]
#[tree_hash(enum_behaviour = "union")]
#[serde(untagged)]
#[non_exhaustive]
pub enum VersionedMessage {
    FragV0(FragV0),
    SealV0(SealV0),
    EnvV0(EnvV0),
}

impl From<FragV0> for VersionedMessage {
    fn from(value: FragV0) -> Self {
        Self::FragV0(value)
    }
}

impl From<SealV0> for VersionedMessage {
    fn from(value: SealV0) -> Self {
        Self::SealV0(value)
    }
}

impl From<EnvV0> for VersionedMessage {
    fn from(value: EnvV0) -> Self {
        Self::EnvV0(value)
    }
}

pub type MaxExtraDataSize = typenum::U256;
pub type ExtraData = VariableList<u8, MaxExtraDataSize>;

/// Initial message to set the block environment for the current block
#[derive(Debug, Clone, PartialEq, Eq, TreeHash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvV0 {
    number: u64,
    parent_hash: B256,
    beneficiary: Address,
    timestamp: u64,
    gas_limit: u64,
    basefee: u64,
    difficulty: U256,
    prevrandao: B256,
    #[serde(with = "ssz_types::serde_utils::hex_var_list")]
    extra_data: ExtraData,
    parent_beacon_block_root: B256,
}

impl EnvV0 {
    pub fn new(env: &BlockEnv, parent_hash: B256, extra_data: &Bytes, parent_beacon_block_root: B256) -> Self {
        Self {
            number: env.number.to(),
            parent_hash,
            beneficiary: env.coinbase,
            timestamp: env.timestamp.to(),
            gas_limit: env.gas_limit.to(),
            basefee: env.basefee.to(),
            difficulty: env.difficulty,
            prevrandao: env.prevrandao.unwrap_or_default(),
            extra_data: ExtraData::from(extra_data.to_vec()),
            parent_beacon_block_root,
        }
    }
}

pub type MaxBytesPerTransaction = typenum::U1073741824;
pub type MaxTransactionsPerPayload = typenum::U1048576;
pub type Transaction = VariableList<u8, MaxBytesPerTransaction>;
pub type Transactions = VariableList<Transaction, MaxTransactionsPerPayload>;

/// A _fragment_ of a block, containing a sequenced set of transactions that will be eventually included in the next
/// block in this order
#[derive(Debug, Clone, PartialEq, Eq, TreeHash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FragV0 {
    /// Block in which this frag will be included
    block_number: u64,
    /// Index of this frag. Frags need to be applied sequentially by index, up to [`SealV0::total_frags`]
    seq: u64,
    /// Whether this is the last frag in the sequence
    pub is_last: bool,
    /// Ordered list of EIP-2718 encoded transactions
    #[serde(with = "ssz_types::serde_utils::list_of_hex_var_list")]
    txs: Transactions,
}

impl FragV0 {
    pub fn new<'a>(
        block_number: u64,
        seq: u64,
        builder_txs: impl Iterator<Item = &'a BuilderTransaction>,
        is_last: bool,
    ) -> Self {
        let txs = builder_txs.map(|tx| tx.encode().to_vec()).map(Transaction::from).collect::<Vec<_>>();
        Self { block_number, seq, txs: Transactions::from(txs), is_last }
    }
}

/// A message sealing a sequence of frags, with fields from the block header
#[derive(Debug, Clone, PartialEq, Eq, TreeHash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealV0 {
    /// How many frags for this block were in this sequence
    pub total_frags: u64,

    // Header fields
    pub block_number: u64,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub parent_hash: B256,
    pub transactions_root: B256,
    pub receipts_root: B256,
    pub state_root: B256,
    pub block_hash: B256,
}

impl VersionedMessage {
    pub fn to_json(&self, signer: &ECDSASigner) -> serde_json::Value {
        let signature = signer.sign_message(self.tree_hash_root()).expect("couldn't sign message");
        let signature = Bytes::from(signature.as_bytes());

        let method = match &self {
            VersionedMessage::FragV0(_) => "based_newFrag",
            VersionedMessage::SealV0(_) => "based_sealFrag",
            VersionedMessage::EnvV0(_) => "based_env",
        };

        serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": [{"signature": signature, "message": self}],
            "id": 1
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{address, b256};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_env_v0() {
        let env = EnvV0 {
            number: 1,
            beneficiary: address!("1234567890123456789012345678901234567890"),
            timestamp: 2,
            gas_limit: 3,
            basefee: 4,
            difficulty: U256::from(5),
            prevrandao: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            parent_hash: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            extra_data: ExtraData::from(vec![1, 2, 3]),
            parent_beacon_block_root: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
        };

        let message = VersionedMessage::from(env);
        let hash = message.tree_hash_root();
        assert_eq!(hash, b256!("fa09df7670737568ba783dfd934e19b06e6681e367a866a5647449bd4e5ca324"));
    }

    #[test]
    fn test_env_v0_2() {
        let env = EnvV0 {
            number: 97,
            parent_hash: b256!("1fcb07f5b2c4783244ae53fa10362d931e81cdde3ee6c63992df5a3e7ffe8e88"),
            beneficiary: address!("4200000000000000000000000000000000000011"),
            timestamp: 1741894329,
            gas_limit: 60000000,
            basefee: 679116901,
            difficulty: U256::ZERO,
            prevrandao: b256!("75ed449926e893872b74e7c73e207710b042d04d055b6234728b009f2bc70635"),
            extra_data: ExtraData::from(vec![0x00, 0x00, 0x00, 0x00, 0xfa, 0x00, 0x00, 0x00, 0x06]),
            parent_beacon_block_root: b256!("44188e8e70142a7897d952a999f784a546b80e83078c864f7d9fbaba32f0a5b2"),
        };

        let message = VersionedMessage::from(env);
        let hash = message.tree_hash_root();
        assert_eq!(hash, b256!("779172e004a01f01249038ed576516345ee9ce46a5e678b61945f33eff7448bb"));
    }

    #[test]
    fn test_frag_v0() {
        let tx = Transaction::from(vec![1, 2, 3]);
        let txs = Transactions::from(vec![tx]);

        let frag = FragV0 { block_number: 1, seq: 0, is_last: true, txs };

        let message = VersionedMessage::from(frag);
        let hash = message.tree_hash_root();
        assert_eq!(hash, b256!("2a5ebad20a81878e5f229928e5c2043580051673b89a7a286008d30f62b10963"));
    }

    #[test]
    fn test_seal_v0() {
        let sealed = SealV0 {
            total_frags: 8,
            block_number: 123,
            gas_used: 25_000,
            gas_limit: 1_000_000,
            parent_hash: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            transactions_root: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            receipts_root: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            state_root: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
            block_hash: b256!("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758"),
        };

        let message = VersionedMessage::from(sealed);
        let hash = message.tree_hash_root();
        assert_eq!(hash, b256!("e86afda21ddc7338c7e84561681fde45e2ab55cce8cde3163e0ae5f1c378439e"));
    }
}
