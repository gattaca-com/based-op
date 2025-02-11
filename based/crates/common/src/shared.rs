use std::sync::Arc;

use alloy_consensus::{Block, Header};
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use reth_optimism_primitives::OpTransactionSigned;
use revm_primitives::{BlockEnv, HashMap, B256};

use crate::db::DBFrag;

#[derive(Clone, Debug)]
struct LatestBlockInfo {
    hash: B256,
    block: Block<OpTransactionSigned>,
}

impl LatestBlockInfo {
    /// Create a new latest block info with a random block hash.
    pub fn new(block: Block<OpTransactionSigned>) -> Self {
        Self { hash: B256::random(), block }
    }
}

/// Shared state between Sequencer and RPC
/// Allows for access to the State and Receipts
/// Receipts and State are updated when a frag gets sealed
#[derive(Clone, Debug)]
pub struct SharedState<Db> {
    db: DBFrag<Db>,
    receipts: Arc<RwLock<HashMap<B256, OpTransactionReceipt>>>,
    /// This is a dummy block that is set while we are building frags.
    /// We use it so that we can return a block for the pending block that is getting built.
    /// Txs will always be empty and the "hash" header fields will be random.
    latest_block: Arc<RwLock<Option<LatestBlockInfo>>>,
}

impl<Db> SharedState<Db> {
    pub fn new(db: DBFrag<Db>) -> Self {
        Self { db, receipts: Arc::new(RwLock::new(Default::default())), latest_block: Arc::new(RwLock::new(None)) }
    }

    /// Resets the pending state its holding for live blocks that are being built.
    /// Will be called when the block is committed as this state will now be available in the EL node.
    pub fn reset(&mut self) {
        self.db.reset();
        self.receipts.write().clear();
        self.latest_block.write().take();
    }

    /// Should be called as soon as we have applied the first frag.
    pub fn initialise_for_new_block(&self, env: &BlockEnv, parent_hash: B256) {
        let mut guard = self.latest_block.write();
        debug_assert!(guard.is_none(), "latest block already set");

        let header = Header {
            parent_hash,
            beneficiary: env.coinbase,
            number: env.number.to(),
            gas_limit: env.gas_limit.to(),
            timestamp: env.timestamp.to(),
            base_fee_per_gas: Some(env.basefee.to()),

            // Default the rest of the values as they are either unused or not known yet.
            ..Default::default()
        };
        let block = Block { header, body: Default::default() };

        *guard = Some(LatestBlockInfo::new(block));
    }

    pub fn insert_receipt(&mut self, tx_hash: B256, receipt: OpTransactionReceipt) {
        self.receipts.write().insert(tx_hash, receipt);
    }

    pub fn get_receipt(&self, tx_hash: &B256) -> Option<OpTransactionReceipt> {
        self.receipts.read().get(tx_hash).cloned()
    }

    pub fn get_latest_block(&self) -> Option<Block<OpTransactionSigned>> {
        self.latest_block.read().as_ref().map(|info| info.block.clone())
    }

    pub fn get_latest_block_hash(&self) -> Option<B256> {
        self.latest_block.read().as_ref().map(|info| info.hash)
    }

    pub fn get_latest_block_number(&self) -> Option<u64> {
        self.latest_block.read().as_ref().map(|info| info.block.header.number)
    }

    /// Returns the latest block if its number matches the requested number.
    /// Returns None if the block number differs or no latest block exists.
    pub fn get_block_by_number(&self, number: u64) -> Option<Block<OpTransactionSigned>> {
        self.latest_block
            .read()
            .as_ref()
            .and_then(|info| (info.block.header.number == number).then_some(info.block.clone()))
    }

    /// Returns the latest block if its hash matches the requested hash.
    /// Returns None if the block hash differs or no latest block exists.
    pub fn get_block_by_hash(&self, hash: B256) -> Option<Block<OpTransactionSigned>> {
        self.latest_block.read().as_ref().and_then(|info| (info.hash == hash).then_some(info.block.clone()))
    }
}

impl<Db: Clone> From<&SharedState<Db>> for DBFrag<Db> {
    fn from(value: &SharedState<Db>) -> Self {
        value.db.clone()
    }
}

impl<Db> AsMut<DBFrag<Db>> for SharedState<Db> {
    fn as_mut(&mut self) -> &mut DBFrag<Db> {
        &mut self.db
    }
}

impl<Db> AsRef<DBFrag<Db>> for SharedState<Db> {
    fn as_ref(&self) -> &DBFrag<Db> {
        &self.db
    }
}
