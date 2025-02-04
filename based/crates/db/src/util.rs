use alloy_primitives::{map::HashMap, Address};
use revm::db::BundleState;
use revm_primitives::{db::DatabaseRef, Account};

/// Converts cached state in a `CachedDB` into `BundleState`
pub fn state_changes_to_bundle_state<D: DatabaseRef>(
    db: &D,
    changes: HashMap<Address, Account>,
) -> Result<BundleState, D::Error> {
    let mut bundle_state = BundleState::builder(0..=2);

    for (address, account) in changes {
        if let Some(original_account_info) = db.basic_ref(address)? {
            bundle_state = bundle_state.state_original_account_info(address, original_account_info);
        }
        bundle_state = bundle_state.state_present_account_info(address, account.info);
        bundle_state = bundle_state.state_storage(
            address,
            account.storage.into_iter().map(|(i, s)| (i, (s.original_value, s.present_value))).collect(),
        );
    }
    Ok(bundle_state.build())
}
