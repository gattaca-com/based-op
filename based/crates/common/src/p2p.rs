use alloy_primitives::{Bytes, B256};
use alloy_signer::{Signature, SignerSync};
use alloy_signer_local::PrivateKeySigner;

pub struct Signed<T> {
    pub signature: Signature,
    pub inner: T,
}

/// A _fragment_ of a block, containing a sequenced set of transactions that will be eventually included in the next
/// block in this order
#[derive(Debug, Clone)]
pub struct Frag {
    /// Block in which this frag will be included
    pub block_number: u64,
    /// Index of this frag. Frags need to be applied sequentially by index from 0 to the end of sequence index
    pub seq: u64,
    /// Ordered list of RLP encoded transactions
    pub txs: Vec<Bytes>,
}

/// A message sealing a sequence of frags
#[derive(Debug, Clone)]
pub struct Sealed {
    /// The hash of the block being sequence
    pub hash: B256,
    /// Block number common to all frags
    pub block_number: u64,
    /// Last frag index
    pub seal_seq: u64,
}

#[derive(Debug, Clone)]
pub enum FragMessage {
    Frag(Frag),
    Sealed(Sealed),
}

/// TODO: import crate
pub trait TreeHash {
    fn tree_hash_root(&self) -> B256;
    fn do_sign(self, signer: &PrivateKeySigner) -> alloy_signer::Result<Signed<Self>>
    where
        Self: Sized, // Ensure that Self has a known size at compile time
    {
        let hash = self.tree_hash_root();
        let sig = signer.sign_hash_sync(&hash)?;
        Ok(Signed { signature: sig, inner: self })
    }
}

impl TreeHash for Frag {
    fn tree_hash_root(&self) -> B256 {
        todo!()
    }
}

impl TreeHash for Sealed {
    fn tree_hash_root(&self) -> B256 {
        todo!()
    }
}

pub type SignedFrag = Signed<Frag>;
pub type SignedSealed = Signed<Sealed>;
