use std::sync::Arc;

use alloy_primitives::{Address, U256};
use op_revm::OpHaltReason;
use revm::{
    context::result::{ExecResultAndState, ExecutionResult, ResultVecAndState},
    state::EvmState,
};
use uuid::Uuid;

use crate::{
    telemetry::Telemetry,
    time::Nanos,
    transaction::{SimulatedTx, SimulatedTxList, Transaction, TxList},
};

pub mod bundle;
pub use bundle::{SimulatedBundle, ValidatedBundle};

/// An order is either a transaction or an atomic bundle of transactions.
#[derive(Debug, Clone)]
pub enum Order {
    Tx(Arc<Transaction>),
    Bundle(Arc<ValidatedBundle>),
}

impl From<Transaction> for Order {
    fn from(tx: Transaction) -> Self {
        Order::Tx(Arc::new(tx))
    }
}

impl From<Arc<Transaction>> for Order {
    fn from(tx: Arc<Transaction>) -> Self {
        Order::Tx(tx)
    }
}

impl From<Arc<ValidatedBundle>> for Order {
    fn from(bundle: Arc<ValidatedBundle>) -> Self {
        Order::Bundle(bundle)
    }
}

impl From<ValidatedBundle> for Order {
    fn from(bundle: ValidatedBundle) -> Self {
        Order::Bundle(Arc::new(bundle))
    }
}

impl Order {
    pub fn tx(&self) -> Option<&Arc<Transaction>> {
        match self {
            Order::Tx(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn bundle(&self) -> Option<&Arc<ValidatedBundle>> {
        match self {
            Order::Bundle(bundle) => Some(bundle),
            _ => None,
        }
    }

    pub fn uuid(&self) -> Uuid {
        match self {
            Order::Tx(tx) => tx.uuid,
            Order::Bundle(bundle) => bundle.uuid(),
        }
    }

    /// Returns the pool telemetry update.
    pub fn pool_telemetry(&self) -> Vec<Telemetry> {
        match self {
            Order::Tx(tx) => vec![tx.to_added_to_pool_telemetry()],
            Order::Bundle(bundle) => bundle.transactions.iter().map(|tx| tx.to_added_to_pool_telemetry()).collect(),
        }
    }

    pub fn ty(&self) -> &'static str {
        match self {
            Order::Tx(_) => "tx",
            Order::Bundle(_) => "bundle",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SimulatedOrder {
    Tx(SimulatedTx),
    Bundle(SimulatedBundle),
}

impl SimulatedOrder {
    pub fn uuid(&self) -> Uuid {
        match self {
            SimulatedOrder::Tx(tx) => tx.uuid,
            SimulatedOrder::Bundle(bundle) => bundle.validated_ref().uuid(),
        }
    }

    pub fn payment(&self) -> Option<U256> {
        match self {
            SimulatedOrder::Tx(tx) => Some(tx.payment),
            SimulatedOrder::Bundle(bundle) => bundle.payment(),
        }
    }

    pub fn gas_used(&self) -> u64 {
        match self {
            SimulatedOrder::Tx(tx) => tx.gas_used(),
            SimulatedOrder::Bundle(bundle) => bundle.gas_used(),
        }
    }

    /// Returns an iterator over the senders of the transactions in the order.
    pub fn senders(&self) -> impl Iterator<Item = Address> {
        match self {
            SimulatedOrder::Tx(tx) => either::Either::Left(std::iter::once(tx.sender())),
            SimulatedOrder::Bundle(bundle) => either::Either::Right(bundle.senders()),
        }
    }

    /// Returns the result and state of the order, if available.
    pub fn result_and_state<'a>(&'a self) -> Option<ResultAndState<'a>> {
        match self {
            SimulatedOrder::Tx(tx) => Some(ResultAndState::Single(&tx.result_and_state)),
            SimulatedOrder::Bundle(bundle) => bundle.result_and_state().map(ResultAndState::Many),
        }
    }

    pub fn sim_time(&self) -> Option<Nanos> {
        match self {
            SimulatedOrder::Tx(tx) => Some(tx.sim_time),
            SimulatedOrder::Bundle(bundle) => bundle.sim_time(),
        }
    }

    /// Returns the estimated DA size of the order, if available.
    pub fn estimated_da(&self) -> u64 {
        match self {
            SimulatedOrder::Tx(tx) => tx.tx.estimated_tx_compressed_size(),
            SimulatedOrder::Bundle(bundle) => bundle.estimated_da(),
        }
    }

    pub fn included_telemetry(&self, frag: Uuid, id_in_frag: usize) -> Vec<Telemetry> {
        match self {
            SimulatedOrder::Tx(tx) => vec![tx.to_included_telemetry(frag, id_in_frag, None)],
            SimulatedOrder::Bundle(bundle) => bundle
                .transactions()
                .iter()
                .enumerate()
                // Make sure to increment the id_in_frag for each transaction in the bundle.
                .map(|(i, tx)| tx.to_included_telemetry(frag, id_in_frag + i, Some(bundle.id())))
                .collect(),
        }
    }
}

/// An order that is ready to be executed in the next block.
#[derive(Debug, Clone)]
pub enum PendingOrder {
    Tx(SimulatedTxList),
    Bundle(SimulatedBundle),
}

impl From<Order> for PendingOrder {
    fn from(order: Order) -> Self {
        match order {
            Order::Tx(tx) => PendingOrder::Tx(SimulatedTxList::new(None, &TxList::from(tx.clone()))),
            Order::Bundle(bundle) => PendingOrder::Bundle(SimulatedBundle::new(bundle)),
        }
    }
}

impl PendingOrder {
    pub fn as_tx_list(&self) -> Option<&SimulatedTxList> {
        match self {
            PendingOrder::Tx(list) => Some(list),
            _ => None,
        }
    }

    pub fn as_tx_list_mut(&mut self) -> Option<&mut SimulatedTxList> {
        match self {
            PendingOrder::Tx(list) => Some(list),
            _ => None,
        }
    }

    pub fn as_bundle(&self) -> Option<&SimulatedBundle> {
        match self {
            PendingOrder::Bundle(bundle) => Some(bundle),
            _ => None,
        }
    }

    /// Returns the weight of the order, in this case an estimated payment value. If the order has been simulated,
    /// the payment will be accurate, otherwise it's an estimate based on the priority fee.
    pub fn weight(&self) -> U256 {
        match self {
            PendingOrder::Tx(list) => list.weight(),
            PendingOrder::Bundle(bundle) => bundle.weight(),
        }
    }

    /// Returns the simulated payment of the order, if available.
    pub fn payment(&self) -> Option<U256> {
        match self {
            PendingOrder::Tx(list) => list.payment(),
            PendingOrder::Bundle(bundle) => bundle.payment(),
        }
    }

    /// Returns the gas limit of the order, if available.
    pub fn gas_limit(&self) -> Option<u64> {
        match self {
            PendingOrder::Tx(list) => list.gas_limit(),
            PendingOrder::Bundle(bundle) => Some(bundle.gas_limit()),
        }
    }

    /// Returns the estimated DA size of the order, if available.
    pub fn estimated_da(&self) -> Option<u64> {
        match self {
            PendingOrder::Tx(list) => list.estimated_da(),
            PendingOrder::Bundle(bundle) => Some(bundle.estimated_da()),
        }
    }
}

impl From<SimulatedOrder> for PendingOrder {
    fn from(order: SimulatedOrder) -> Self {
        match order {
            SimulatedOrder::Tx(tx) => PendingOrder::Tx(SimulatedTxList::from(tx)),
            SimulatedOrder::Bundle(bundle) => PendingOrder::Bundle(bundle),
        }
    }
}

/// The result and state of a single transaction.
pub type ResultAndStateSingle = ExecResultAndState<ExecutionResult<OpHaltReason>, EvmState>;

/// The results and state of many transactions.
pub type ResultAndStateMany = ResultVecAndState<ExecutionResult<OpHaltReason>, EvmState>;

/// The result and state of a single or many transactions.
#[derive(Debug)]
pub enum ResultAndState<'a> {
    Single(&'a ResultAndStateSingle),
    Many(&'a ResultAndStateMany),
}

impl<'a> ResultAndState<'a> {
    pub fn state(&self) -> &EvmState {
        match self {
            ResultAndState::Single(single) => &single.state,
            ResultAndState::Many(many) => &many.state,
        }
    }

    pub fn gas_used(&self) -> u64 {
        match self {
            ResultAndState::Single(single) => single.result.gas_used(),
            ResultAndState::Many(many) => many.result.iter().map(|result| result.gas_used()).sum(),
        }
    }
}
