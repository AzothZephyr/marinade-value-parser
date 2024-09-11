mod accounts;

use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_account_decoder::UiAccountEncoding;
use std::str::FromStr;
use log::{debug, error};
use crate::accounts::marinade::{MarinadeState, parse_marinade_state};

const SOL_MINT_PUBKEY: &str = "So11111111111111111111111111111111111111112";
const MSOL_MINT_PUBKEY: &str = "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK1iNKhS3nZF";

// marinade staking program account pubkey
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

/// fetch account data for given a public key
fn fetch_account_data(rpc_client: &RpcClient, pubkey: &Pubkey, slot: Option<u64>) -> Option<Vec<u8>> {
    debug!("entering fetch_account_data");
    debug!("pubkey: {:?}, slot: {:?}", pubkey, slot);

    let config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::processed()),
        data_slice: None,
        min_context_slot: slot,
    };

    let response = rpc_client.get_account_with_config(pubkey, config);
    
    match response {
        Ok(account_data) => {
            match account_data.value {
                Some(account) => {
                    debug!("account data fetched successfully, length: {}", account.data.len());
                    Some(account.data)
                },
                None => {
                    error!("account data is None");
                    None
                }
            }
        },
        Err(e) => {
            error!("error fetching account data: {}", e);
            None
        }
    }
}
/// 
fn find_and_parse_marinade_state(rpc_client: &RpcClient, pubkey: &Pubkey, slot: Option<u64>) -> Option<MarinadeState> {
    debug!("entering find_and_parse_marinade_state");
    debug!("pubkey: {:?}, slot: {:?}", pubkey, slot);

    // Fetch account data, passing the optional slot
    let account_data = match fetch_account_data(rpc_client, pubkey, slot) {
        Some(data) => {
            debug!("account data fetched successfully, length: {}", data.len());
            data
        },
        None => {
            error!("failed to fetch account data");
            return None;
        }
    };

    // Log the first few bytes of the account data
    debug!("first 16 bytes of account data: {:?}", &account_data.get(..16).unwrap_or(&[]));

    match parse_marinade_state(&account_data) {
        Ok(state) => Some(state),
        Err(e) => {
            error!("failed to parse Marinade state: {:?}", e);
            None
        }
    }
}

/// analyze a tx to check if it affects the Marinade state and if so, convert the data into MintUnderlying and return
pub fn analyze_transaction(rpc_client: &RpcClient, tx: &EncodedConfirmedTransactionWithStatusMeta) -> Option<MintUnderlying> {
    debug!("starting analyze_transaction");
    let marinade_state_pubkey = match Pubkey::from_str(MARINADE_STATE_PUBKEY) {
        Ok(pubkey) => pubkey,
        Err(e) => {
            error!("failed to parse MARINADE_STATE_PUBKEY: {}", e);
            return None;
        }
    };

    let slot = tx.slot;
    debug!("tx slot: {}", slot);

    debug!("fetching Marinade state for slot: {}", slot);
    let post_state = match find_and_parse_marinade_state(rpc_client, &marinade_state_pubkey, Some(slot)) {
        Some(state) => state,
        None => {
            error!("Failed to find and parse Marinade state");
            return None;
        }
    };
    debug!("marinade state fetched successfully");

    let sol_amount = post_state.validator_system.total_active_balance + post_state.emergency_cooling_down + post_state.available_reserve_balance - post_state.circulating_ticket_balance;
    let msol_value = sol_amount / post_state.msol_supply;

    debug!("calculated sol_amount: {}", sol_amount);
    debug!("calculated msol_value: {}", msol_value);

    let block_time = match tx.block_time {
        Some(time) => time,
        None => {
            error!("tx block time is None");
            return None;
        }
    };

    let mu = MintUnderlying {
        block_time,
        msol_value,
        mint_pubkey: MSOL_MINT_PUBKEY.to_string(),
        platform_program_pubkey: MARINADE_STATE_PUBKEY.to_string(),
        mints: vec![SOL_MINT_PUBKEY.to_string()],
        total_underlying_amounts: vec![sol_amount],
    };
    debug!("created MintUnderlying: {:?}", mu);
    Some(mu)
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
    use env_logger;

    #[test]
    fn test_deposit_transaction() {
        env_logger::init();  // Initialize logger

        debug!("starting test_deposit_transaction");
        let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let deposit_signature = "4uL95njGxnL7oPRBv6qb9ZKeWbTfKifbJgKe5zJ98FFyh7TJofUghQ2tcp4gR9fUHsX5exHayzcK9Zt1SR1Cwy7k";
        let expected_sol_deposit_value: f64 = 0.020890732;
        let expected_msol_returned_value: f64 = 0.017192933;

        debug!("fetching transaction with signature: {}", deposit_signature);
        let tx = fetch_transaction(deposit_signature).expect("failed to fetch deposit transaction");
        debug!("transaction fetched successfully");

        debug!("analyzing transaction");
        let result = analyze_transaction(&rpc_client, &tx);
        debug!("analysis result: {:?}", result);
        assert!(result.is_some(), "deposit transaction should produce a result");

        let mint_underlying = result.unwrap();
        debug!("MintUnderlying: {:?}", mint_underlying);

        assert_eq!(mint_underlying.mint_pubkey, MSOL_MINT_PUBKEY);
        assert_eq!(mint_underlying.platform_program_pubkey, MARINADE_STATE_PUBKEY);
        assert_eq!(mint_underlying.mints, vec![MSOL_MINT_PUBKEY.to_string()]);

        let total_underlying_sol = mint_underlying.total_underlying_amounts[0];
        let expected_min = (expected_sol_deposit_value * 1_000_000_000.0_f64).round() as u64;
        let expected_max = expected_min + 10;

        debug!("total underlying SOL: {}", total_underlying_sol);
        debug!("expected range: {} to {}", expected_min, expected_max);
        assert!(
            total_underlying_sol >= expected_min && total_underlying_sol <= expected_max,
            "total underlying SOL is outside the expected range"
        );

        let msol_value = mint_underlying.msol_value;
        let expected_msol_min = (expected_msol_returned_value * 1_000_000_000.0_f64).round() as u64;
        let expected_msol_max = expected_msol_min + 10;

        debug!("msol value: {}", msol_value);
        debug!("expected msol range: {} to {}", expected_msol_min, expected_msol_max);
        assert!(
            msol_value >= expected_msol_min && msol_value <= expected_msol_max,
            "msol value is outside the expected range"
        );

        debug!("test_deposit_transaction completed successfully");
    }
}
