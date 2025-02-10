use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signature as ECDSASignature;
use revm_primitives::BlockEnv;
use ssz_types::{typenum, VariableList};
use tree_hash_derive::TreeHash;

use crate::transaction::Transaction as BuilderTransaction;

#[derive(Debug, Clone, PartialEq, Eq, TreeHash)]
#[tree_hash(enum_behaviour = "union")]
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

/// Initial message to set the block environment for the current block
#[derive(Debug, Clone, PartialEq, Eq, TreeHash)]
pub struct EnvV0 {
    number: U256,
    coinbase: Address,
    timestamp: U256,
    gas_limit: U256,
    basefee: U256,
    difficulty: U256,
    prevrandao: B256,
}

impl From<&BlockEnv> for EnvV0 {
    fn from(env: &BlockEnv) -> Self {
        Self {
            number: env.number,
            coinbase: env.coinbase,
            timestamp: env.timestamp,
            gas_limit: env.gas_limit,
            basefee: env.basefee,
            difficulty: env.difficulty,
            prevrandao: env.prevrandao.unwrap_or_default(),
        }
    }
}

pub type MaxBytesPerTransaction = typenum::U1073741824;
pub type MaxTransactionsPerPayload = typenum::U1048576;
pub type Transaction = VariableList<u8, MaxBytesPerTransaction>;
pub type Transactions = VariableList<Transaction, MaxTransactionsPerPayload>;

/// A _fragment_ of a block, containing a sequenced set of transactions that will be eventually included in the next
/// block in this order
#[derive(Debug, Clone, PartialEq, Eq, TreeHash)]
pub struct FragV0 {
    /// Block in which this frag will be included
    block_number: u64,
    /// Index of this frag. Frags need to be applied sequentially by index, up to [`SealV0::total_frags`]
    seq: u64,
    /// Whether this is the last frag in the sequence
    pub is_last: bool,
    /// Ordered list of EIP-2718 encoded transactions
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
#[derive(Debug, Clone, PartialEq, Eq, TreeHash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedMessage {
    pub signature: ECDSASignature,
    pub message: VersionedMessage,
}

#[cfg(test)]
mod tests {
    use revm_primitives::b256;
    use tree_hash::TreeHash;

    use super::*;

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
