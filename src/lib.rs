mod accounts;

use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};
use solana_sdk::commitment_config::CommitmentConfig;
use std::str::FromStr;
use anchor_lang::AnchorDeserialize;
use crate::accounts::marinade::MarinadeState;
use crate::accounts::instructions::MarinadeFinanceInstruction;
use sha2::{Sha256, Digest};
use lazy_static::lazy_static;
use log::{debug, error};

const SOL_MINT_PUBKEY: &str = "So11111111111111111111111111111111111111112";
const MSOL_MINT_PUBKEY: &str = "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK1iNKhS3nZF";

// marinade program pubkey
const MARINADE_PROGRAM_KEY: &str = "MR2LqxoSbw831bNy68utpu5n4YqBH3AzDmddkgk9LQv";
// marinade staking program account pubkey
const MARINADE_STATE_PUBKEY: &str = "8szGkuLTAux9XMgZ2vtY39jVSowEcpBfFfD8hXSEqdGC";

// this is for testing
const DEPOSIT_DISCRIMINATOR: [u8; 8] = [0xf2, 0x23, 0xc6, 0x89, 0x52, 0xe1, 0xf2, 0xb6];

// TODO: figure out how to handle multiple types of state changing discriminators
lazy_static! {
    static ref STATE_MODIFYING_DISCRIMINATORS: Vec<[u8; 8]> = vec![
        DEPOSIT_DISCRIMINATOR,
        get_instruction_discriminator("initialize"),
        get_instruction_discriminator("change_authority"),
        get_instruction_discriminator("add_validator"),
        get_instruction_discriminator("remove_validator"),
        get_instruction_discriminator("set_validator_score"),
        get_instruction_discriminator("config_validator"),
        get_instruction_discriminator("deposit_stake_account"),
        get_instruction_discriminator("liquid_unstake"),
        get_instruction_discriminator("add_liquidity"),
        get_instruction_discriminator("remove_liquidity"),
        get_instruction_discriminator("set_lp_params"),
        get_instruction_discriminator("configure_delegated_stake"),
        get_instruction_discriminator("order_unstake"),
        get_instruction_discriminator("claim"),
        // Add any other state-modifying instructions here
    ];
}
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
fn fetch_account_data(rpc_client: &RpcClient, pubkey: &Pubkey) -> Option<Vec<u8>> {
    rpc_client.get_account_data(pubkey).ok()
}

/// retrieve the MarinadeState from the Solana RPC by querying the account data
fn find_and_parse_marinade_state(rpc_client: &RpcClient, pubkey: &Pubkey) -> Option<MarinadeState> {
    // we must fetch account data as its not included in the transaction 
    let account_data = fetch_account_data(rpc_client, pubkey)?;
    let mut data_slice = account_data.as_slice();
    MarinadeState::deserialize(&mut data_slice).ok()
}

// anchors discriminator values are the first 8 bytes of the sha256 sum of the instruction name and it makes me wanna vomit
fn get_instruction_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&result[..8]);
    discriminator
}

fn does_tx_affect_msol_value(tx: &EncodedConfirmedTransactionWithStatusMeta) -> bool {
    let decoded_transaction = match tx.transaction.transaction.decode() {
        Some(decoded) => decoded,
        None => {
            debug!("failed to decode transaction");
            return false;
        }
    };

    let marinade_program_pubkey = Pubkey::from_str(MARINADE_PROGRAM_KEY).unwrap();

    for (i, instruction) in decoded_transaction.message.instructions().iter().enumerate() {
        let program_id = instruction.program_id(decoded_transaction.message.static_account_keys());
        debug!("instruction {}: program ID: {:?}", i, program_id);
        
        if *program_id == marinade_program_pubkey {
            debug!("instruction {} matches Marinade program", i);
            if instruction.data.len() >= 8 {
                let ix_discriminator: [u8; 8] = instruction.data[..8].try_into().unwrap();
                debug!("instruction {} discriminator: {:?}", i, ix_discriminator);
                
                // TODO: switch this to an all encompassing state changing discriminator map or something
                if ix_discriminator == DEPOSIT_DISCRIMINATOR {
                    debug!("deposit instruction found!");
                    return true;
                }
                
                for (j, &disc) in STATE_MODIFYING_DISCRIMINATORS.iter().enumerate() {
                    debug!("comparing with discriminator {}: {:?}", j, disc);
                    if disc == ix_discriminator {
                        debug!("match found! Instruction affects mSOL value");
                        return true;
                    }
                }
            } else {
                debug!("instruction {} data too short: {}", i, instruction.data.len());
            }
        }
    }

    debug!("no state-modifying instructions found");
    false
}
/// analyze a tx to check if it affects the Marinade state and if so, convert the data into MintUnderlying and return
pub fn analyze_transaction(rpc_client: &RpcClient, tx: &EncodedConfirmedTransactionWithStatusMeta) -> Option<MintUnderlying> {
    let marinade_state_pubkey = Pubkey::from_str(MARINADE_STATE_PUBKEY).ok()?;

    // check if the transaction affects the Marinade state
    if !does_tx_affect_msol_value(tx) {
        let decoded_transaction = tx.transaction.transaction.decode()?;
        let signature = decoded_transaction.signatures.get(0)?;
        println!("tx hash {} does not modify msol state", signature);
        return None;
    }

    // tx affects marinade state, so get post-state account data and deserialize, returning post_state
    let post_state = find_and_parse_marinade_state(rpc_client, &marinade_state_pubkey)?;

    // per:https://github.com/marinade-finance/liquid-staking-program/blob/26147376b75d8c971963da458623e646f2795e15/programs/marinade-finance/src/instructions/crank/update.rs#L237
    // price is computed as:
    // total_active_balance + total_cooling_down + reserve - circulating_ticket_balance
    // DIVIDED by msol_supply
    // -----
    let sol_amount = post_state.validator_system.total_active_balance + post_state.emergency_cooling_down + post_state.available_reserve_balance - post_state.circulating_ticket_balance;
    let msol_value = sol_amount / post_state.msol_supply;

    let block_time = tx.block_time?;
    let mint_pubkey = MSOL_MINT_PUBKEY.to_string();
    let platform_program_pubkey = MARINADE_STATE_PUBKEY.to_string();


    // TODO: i think there's a better way to do this where we don't have to hardcode the SOL mint pubkey and
    // instead review the post_state object for all mints underlying the lst
    let mints = vec![SOL_MINT_PUBKEY.to_string()];
    let total_underlying_amounts = vec![sol_amount];

    Some(MintUnderlying {
        block_time,
        msol_value,
        mint_pubkey,
        platform_program_pubkey,
        mints,
        total_underlying_amounts,
    })
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
        let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let deposit_signature = "4uL95njGxnL7oPRBv6qb9ZKeWbTfKifbJgKe5zJ98FFyh7TJofUghQ2tcp4gR9fUHsX5exHayzcK9Zt1SR1Cwy7k";
        let expected_sol_deposit_value: f64 = 0.020890732;
        let expected_msol_returned_value: f64 = 0.017192933;

        let tx = fetch_transaction(deposit_signature).expect("failed to fetch deposit transaction");

        let result = analyze_transaction(&rpc_client, &tx);
        println!("result: {:?}", result);
        assert!(result.is_some(), "deposit transaction should produce a result");

        let mint_underlying = result.unwrap();

        println!("MintUnderlying: {:?}", mint_underlying);

        assert_eq!(mint_underlying.mint_pubkey, MSOL_MINT_PUBKEY);
        assert_eq!(mint_underlying.platform_program_pubkey, MARINADE_STATE_PUBKEY);
        assert_eq!(mint_underlying.mints, vec![MSOL_MINT_PUBKEY.to_string()]);

        let total_underlying_sol = mint_underlying.total_underlying_amounts[0];
        let expected_min = (expected_sol_deposit_value * 1_000_000_000.0_f64).round() as u64;
        let expected_max = expected_min + 10;

        assert!(
            total_underlying_sol >= expected_min && total_underlying_sol <= expected_max,
            "total underlying SOL is outside the expected range"
        );

        let msol_value = mint_underlying.msol_value;
        let expected_msol_min = (expected_msol_returned_value * 1_000_000_000.0_f64).round() as u64;
        let expected_msol_max = expected_msol_min + 10;

        assert!(
            msol_value >= expected_msol_min && msol_value <= expected_msol_max,
            "msol value is outside the expected range"
        );
    }

    #[test]
    fn test_does_tx_affect_msol_value() {
        env_logger::init();  // initialize logger

        let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let deposit_signature = "4uL95njGxnL7oPRBv6qb9ZKeWbTfKifbJgKe5zJ98FFyh7TJofUghQ2tcp4gR9fUHsX5exHayzcK9Zt1SR1Cwy7k";

        debug!("fetching transaction...");
        let tx = fetch_transaction(deposit_signature).expect("failed to fetch deposit transaction");

        debug!("analyzing transaction...");
        let result = does_tx_affect_msol_value(&tx);
        
        debug!("transaction affects msol value: {}", result);
        assert!(result, "transaction should affect msol value");
    }
}
