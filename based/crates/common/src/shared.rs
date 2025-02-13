use std::sync::Arc;

use alloy_consensus::Header;
use alloy_rpc_types::BlockTransactions;
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use revm_primitives::{BlockEnv, HashMap, B256};

use crate::{api::OpRpcBlock, db::DBFrag};

/// Shared state between Sequencer and RPC
/// Allows for access to the State and Receipts
/// Receipts and State are updated when a frag gets sealed
#[derive(Clone, Debug)]
pub struct SharedState<Db> {
    db: DBFrag<Db>,
    receipts: Arc<RwLock<HashMap<B256, OpTransactionReceipt>>>,
    /// This is a dummy block that is set while we are building frags.
    /// We use it so that we can return a block for the pending block that is getting built.
    /// The "hash" header fields will be random.
    /// 
    /// Note: this is just temporary while we serve state from the sequencer directly.
    latest_block: Arc<RwLock<Option<OpRpcBlock>>>,
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
    pub fn initialise_for_new_block(&self, env: &BlockEnv, parent_hash: B256, txs: Vec<OpTxEnvelope>) {
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
        let block = OpRpcBlock {
            header: alloy_rpc_types::Header { hash: B256::random(), inner: header, total_difficulty: None, size: None },
            transactions: BlockTransactions::Full(txs),
            ..Default::default()
        };

        *guard = Some(block);
    }

    pub fn insert_confirmed_tx(&mut self, tx: OpTxEnvelope, receipt: OpTransactionReceipt) {
        self.receipts.write().insert(tx.tx_hash(), receipt);
        
        let mut guard = self.latest_block.write();
        debug_assert!(guard.is_some(), "latest block must be set");

        let block = guard.as_mut().unwrap();
        match &mut block.transactions {
            BlockTransactions::Full(txs) => txs.push(tx),
            _ => {
                block.transactions = BlockTransactions::Full(vec![tx]);
            },
        }
    }

    pub fn get_receipt(&self, tx_hash: &B256) -> Option<OpTransactionReceipt> {
        self.receipts.read().get(tx_hash).cloned()
    }

    pub fn get_latest_block(&self, full: bool) -> Option<OpRpcBlock> {
        self.latest_block.read().as_ref().map(|block| {
            let mut block = block.clone();
            if !full {
                if let BlockTransactions::Full(items) = &mut block.transactions {
                    block.transactions = BlockTransactions::Hashes(items.iter().map(|tx| tx.tx_hash()).collect());
                }
            }
            block
        })
    }

    pub fn get_latest_block_hash(&self) -> Option<B256> {
        self.latest_block.read().as_ref().map(|block| block.header.hash)
    }

    pub fn get_latest_block_number(&self) -> Option<u64> {
        self.latest_block.read().as_ref().map(|info| info.header.number)
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
