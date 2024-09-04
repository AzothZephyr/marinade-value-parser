mod accounts; 

use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_transaction_status::option_serializer::OptionSerializer;
use std::str::FromStr;
use anchor_lang::AnchorDeserialize;

use crate::accounts::marinade::MarinadeState;

const MSOL_MINT_PUBKEY: &str = "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK1iNKhS3nZF";
const MARINADE_STATE_PUBKEY: &str = "8szGkuLTAux9XMgZ2vtY39jVSowEcpBfFfD8hXSEqdGC";

#[derive(Debug, Clone)]
pub struct MintUnderlying {
    pub block_time: i64,
    pub msol_value: u64,
    pub mint_pubkey: String,
    pub platform_program_pubkey: String,
    pub mints: Vec<String>,
    pub total_underlying_amounts: Vec<u64>,
}

fn find_and_parse_marinade_state(tx: &EncodedConfirmedTransactionWithStatusMeta, pubkey: &Pubkey, pre: bool) -> Option<MarinadeState> {
    let meta = tx.transaction.meta.as_ref()?;
    let versioned_tx = tx.transaction.transaction.decode()?;
    let account_keys = versioned_tx.message.static_account_keys();

    let token_balances = if pre {
        &meta.pre_token_balances
    } else {
        &meta.post_token_balances
    };

    let account_index = account_keys.iter().position(|key| key == pubkey)
        .or_else(|| {
            match &meta.loaded_addresses {
                OptionSerializer::Some(loaded_addresses) => {
                    loaded_addresses.writable.iter()
                        .chain(loaded_addresses.readonly.iter())
                        .position(|key| *key == pubkey.to_string())
                        .map(|pos| pos + account_keys.len())
                },
                _ => None,
            }
        })?;

    let account_data = match token_balances {
        OptionSerializer::Some(balances) => {
            balances.iter()
                .find(|balance| balance.account_index as usize == account_index)
                .and_then(|balance| balance.ui_token_amount.amount.parse::<u64>().ok())
        },
        _ => None,
    }?;
    let temp_account_data = account_data.to_le_bytes(); // necessary account data value is temporary and dropped when borrowed
    let mut data_slice = temp_account_data.as_slice();
    MarinadeState::deserialize(&mut data_slice).ok()
}

pub fn analyze_transaction(tx: &EncodedConfirmedTransactionWithStatusMeta) -> Option<MintUnderlying> {
    // let msol_mint_pubkey = Pubkey::from_str(MSOL_MINT_PUBKEY).ok()?;
    let marinade_state_pubkey = Pubkey::from_str(MARINADE_STATE_PUBKEY).ok()?;

    let pre_state = find_and_parse_marinade_state(tx, &marinade_state_pubkey, true)?;
    let post_state = find_and_parse_marinade_state(tx, &marinade_state_pubkey, false)?;

    if !does_tx_affect_msol_value(&pre_state, &post_state) {
        let decoded_transaction = tx.transaction.transaction.decode()?;
        let signature = decoded_transaction.signatures.get(0)?;
        println!("tx hash {} does not modify msol state", signature);
        return None;
    }

    let msol_value = calculate_msol_value(&post_state);
    let total_underlying_sol = post_state.available_reserve_balance + post_state.emergency_cooling_down;

    Some(MintUnderlying {
        block_time: tx.block_time?,
        msol_value: msol_value,
        mint_pubkey: MSOL_MINT_PUBKEY.to_string(),
        platform_program_pubkey: MARINADE_STATE_PUBKEY.to_string(),
        mints: vec![MSOL_MINT_PUBKEY.to_string()],
        total_underlying_amounts: vec![total_underlying_sol],
    })
}

fn does_tx_affect_msol_value(pre_state: &MarinadeState, post_state: &MarinadeState) -> bool {
    pre_state.available_reserve_balance != post_state.available_reserve_balance ||
    pre_state.emergency_cooling_down != post_state.emergency_cooling_down ||
    pre_state.msol_supply != post_state.msol_supply
}

fn calculate_msol_value(state: &MarinadeState) -> u64 {
    state.msol_price
}

pub fn fetch_transaction(signature: &str) -> Result<EncodedConfirmedTransactionWithStatusMeta, Box<dyn std::error::Error>> {
    let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
    let tx_data = rpc_client.get_transaction_with_config(
        &Signature::from_str(signature)?,
        solana_client::rpc_config::RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            max_supported_transaction_version: Some(0),
        },
    )?;

    Ok(tx_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposit_transaction() {
        let deposit_signature = "4uL95njGxnL7oPRBv6qb9ZKeWbTfKifbJgKe5zJ98FFyh7TJofUghQ2tcp4gR9fUHsX5exHayzcK9Zt1SR1Cwy7k";
        let expected_sol_deposit_value: f64 = 0.020890732; // explicitly define the type as f64
        let expected_msol_returned_value: f64 = 0.017192933; // same as above

        let tx = fetch_transaction(deposit_signature).expect("failed to fetch deposit transaction");

        println!("tx fetched: {:?}", tx);

        let result = analyze_transaction(&tx);

        // Adding more information to understand why the transaction might fail
        if result.is_none() {
            println!("no MintUnderlying result was returned from analyze_transaction");
        }

        assert!(result.is_some(), "deposit transaction should produce a result");

        let mint_underlying = result.unwrap();

        println!("MintUnderlying: {:?}", mint_underlying);

        // validate the mints and platform details
        assert_eq!(mint_underlying.mint_pubkey, MSOL_MINT_PUBKEY);
        assert_eq!(mint_underlying.platform_program_pubkey, MARINADE_STATE_PUBKEY);
        assert_eq!(mint_underlying.mints, vec![MSOL_MINT_PUBKEY.to_string()]);

        // validate that the total underlying amounts match expected sol deposits
        assert!(!mint_underlying.total_underlying_amounts.is_empty(), "total underlying amounts should not be empty");

        // assert that the underlying SOL amount is within an expected range (within a reasonable tolerance)
        let total_underlying_sol = mint_underlying.total_underlying_amounts[0];
        let expected_min = (expected_sol_deposit_value * 1_000_000_000.0_f64).round() as u64; // Convert to lamports
        let expected_max = expected_min + 10; // Allowing some margin of error

        assert!(
            total_underlying_sol >= expected_min && total_underlying_sol <= expected_max,
            "total underlying SOL is outside the expected range"
        );

        // assert msol_value similarly:
        let msol_value = mint_underlying.msol_value;
        let expected_msol_min = (expected_msol_returned_value * 1_000_000_000.0_f64).round() as u64;
        let expected_msol_max = expected_msol_min + 10; // Margin of error

        assert!(
            msol_value >= expected_msol_min && msol_value <= expected_msol_max,
            "msol value is outside the expected range"
        );
    }
}

