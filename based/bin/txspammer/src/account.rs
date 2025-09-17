use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{B256, Bytes, U256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::AccessList;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;

pub struct AccountGenerator {
    current: U256,
}

impl AccountGenerator {
    pub fn new(start: U256) -> Self {
        Self { current: start }
    }

    pub fn next(&mut self) -> PrivateKeySigner {
        let temp = self.current;
        self.current += U256::from(1);
        PrivateKeySigner::from_slice(temp.as_le_slice()).expect("something wrong with U256 to bytes conversion")
    }
}

#[derive(Clone, Debug)]
pub struct TxSpec {
    pub chain_id: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
    pub value: U256,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub signer: PrivateKeySigner,
    pub nonce: u64,
    pub balance: U256,
}

impl Account {
    pub fn new(signer: PrivateKeySigner) -> Self {
        Self { signer, nonce: 0, balance: U256::ZERO }
    }

    pub async fn refresh_balance(&mut self, provider: &RootProvider) -> eyre::Result<U256> {
        self.balance = provider.get_balance(self.signer.address()).await?;
        Ok(self.balance)
    }

    pub async fn refresh_nonce(&mut self, provider: &RootProvider) -> eyre::Result<u64> {
        self.nonce = provider.get_transaction_count(self.signer.address()).await?;
        Ok(self.nonce)
    }

    pub async fn refresh(&mut self, provider: &RootProvider) -> eyre::Result<u64> {
        self.refresh_balance(provider).await?;
        self.refresh_nonce(provider).await?;
        Ok(self.nonce)
    }

    pub async fn transfer(
        &mut self,
        to: &mut Account,
        spec: &TxSpec,
        provider: &RootProvider,
        sequencer: &Option<RootProvider>,
    ) -> eyre::Result<B256> {
        let tx = TxEip1559 {
            chain_id: spec.chain_id,
            nonce: self.nonce,
            gas_limit: spec.gas_limit,
            max_fee_per_gas: spec.max_fee_per_gas,
            max_priority_fee_per_gas: spec.max_priority_fee_per_gas,
            to: to.signer.address().into(),
            value: spec.value,
            access_list: AccessList::default(),
            input: Bytes::new(),
        };

        let sig = self.signer.sign_hash_sync(&tx.signature_hash()).unwrap();
        let tx: TxEnvelope = tx.into_signed(sig).into();
        let encoded = tx.encoded_2718();
        let provider_to_use = sequencer.as_ref().unwrap_or(provider);
        let _pending_tx = provider_to_use.send_raw_transaction(&encoded).await.unwrap();
        self.nonce += 1;
        self.balance -= spec.value + U256::from(spec.gas_limit) * U256::from(spec.max_fee_per_gas);
        to.balance += spec.value;

        Ok(*tx.tx_hash())
    }

    pub async fn _self_transfer(
        &mut self,
        spec: &TxSpec,
        provider: &RootProvider,
        sequencer: &Option<RootProvider>,
    ) -> eyre::Result<B256> {
        let tx = TxEip1559 {
            chain_id: spec.chain_id,
            nonce: self.nonce,
            gas_limit: spec.gas_limit,
            max_fee_per_gas: spec.max_fee_per_gas,
            max_priority_fee_per_gas: spec.max_priority_fee_per_gas,
            to: self.signer.address().into(),
            value: spec.value,
            access_list: AccessList::default(),
            input: Bytes::new(),
        };

        let sig = self.signer.sign_hash_sync(&tx.signature_hash()).unwrap();
        let tx: TxEnvelope = tx.into_signed(sig).into();
        let encoded = tx.encoded_2718();
        let provider_to_use = sequencer.as_ref().unwrap_or(provider);
        let _pending_tx = provider_to_use.send_raw_transaction(&encoded).await.unwrap();
        self.nonce += 1;
        self.balance -= U256::from(spec.gas_limit) * U256::from(spec.max_fee_per_gas);

        Ok(*tx.tx_hash())
    }
}
