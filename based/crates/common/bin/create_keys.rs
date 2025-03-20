use bop_common::signing::ECDSASigner;
use revm_primitives::{Address, B256};

/// Use this bin to generate all private keys and addresses for a standard op deployment
fn generate_key_address() -> (B256, Address) {
    let key = B256::random();
    let signer =
        ECDSASigner::try_from_secret(key.as_ref()).expect(&format!("somehow couldn't create a signer for key {key}"));
    (key, signer.address)
}
fn main() {
    let (key_admin, address_admin) = generate_key_address();
    let (key_batcher, address_batcher) = generate_key_address();
    let (key_proposer, address_proposer) = generate_key_address();
    let (key_sequencer, address_sequencer) = generate_key_address();
    let (key_proxy_admin_owner, address_proxy_admin_owner) = generate_key_address();
    let (key_protocol_versions_owner, address_protocol_versions_owner) = generate_key_address();
    let (key_guardian, address_guardian) = generate_key_address();
    let (key_base_fee_vault_recipient, address_base_fee_vault_recipient) = generate_key_address();
    let (key_l1_fee_vault_recipient, address_l1_fee_vault_recipient) = generate_key_address();
    let (key_l1_proxy_admin_owner, address_l1_proxy_admin_owner) = generate_key_address();
    let (key_l2_proxy_admin_owner, address_l2_proxy_admin_owner) = generate_key_address();
    let (key_system_config_owner, address_system_config_owner) = generate_key_address();
    let (key_unsafe_block_signer, address_unsafe_block_signer) = generate_key_address();
    let (key_challenger, address_challenger) = generate_key_address();
    println!(
        "# for .envrc

## GS_ADMIN
export GS_ADMIN_PRIVATE_KEY={key_admin}
export GS_ADMIN_ADDRESS={address_admin}

## GS_BATCHER
export GS_BATCHER_PRIVATE_KEY={key_batcher}
export GS_BATCHER_ADDRESS={address_batcher}

## GS_PROPOSER
export GS_PROPOSER_PRIVATE_KEY={key_proposer}
export GS_PROPOSER_ADDRESS={address_proposer}

## GS_SEQUENCER
export GS_SEQUENCER_PRIVATE_KEY={key_sequencer}
export GS_SEQUENCER_ADDRESS={address_sequencer}

# for intents.toml
proxyAdminOwner            = \"{address_proxy_admin_owner}\" key={key_proxy_admin_owner}
protocolVersionsOwner      = \"{address_protocol_versions_owner}\" key={key_protocol_versions_owner}
guardian                   = \"{address_guardian}\" key={key_guardian}

baseFeeVaultRecipient      = \"{address_base_fee_vault_recipient}\" key={key_base_fee_vault_recipient}
l1FeeVaultRecipient        = \"{address_l1_fee_vault_recipient}\" key={key_l1_fee_vault_recipient}
sequencerFeeVaultRecipient = \"{address_sequencer}\" key={key_sequencer}

l1ProxyAdminOwner          = \"{address_l1_proxy_admin_owner}\" key={key_l1_proxy_admin_owner}
l2ProxyAdminOwner          = \"{address_l2_proxy_admin_owner}\" key={key_l2_proxy_admin_owner}
systemConfigOwner          = \"{address_system_config_owner}\" key={key_system_config_owner}
unsafeBlockSigner          = \"{address_unsafe_block_signer}\" key={key_unsafe_block_signer}
batcher                    = \"{address_batcher}\" key={key_batcher}
proposer                   = \"{address_proposer}\" key={key_proposer}
challenger                 = \"{address_challenger}\" key={key_challenger}
"
    );
}
