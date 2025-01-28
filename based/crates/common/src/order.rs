use alloy_consensus::TxEnvelope;
use alloy_primitives::B256;

pub enum Order {
    Tx(Transaction),
}

impl Order {
    pub fn hash(&self) -> B256 {
        match self {
            Order::Tx(tx) => tx.hash(),
        }
    }
}

// TODO
pub struct Transaction {
    pub tx: TxEnvelope,
}

impl Transaction {
    pub fn new(tx: TxEnvelope) -> Self {
        Self { tx }
    }

    pub fn hash(&self) -> B256 {
        *self.tx.tx_hash()
    }
}

impl alloy_rlp::Decodable for Transaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let tx = TxEnvelope::decode(buf)?;
        Ok(Self { tx })
    }
}
