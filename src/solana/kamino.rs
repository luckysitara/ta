use anyhow::Result;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};
use std::str::FromStr;

/// Kamino Lending (klend) Program ID on Solana Mainnet
pub const KAMINO_PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";

/// Kamino Main Lending Market
pub const KAMINO_MAIN_MARKET: &str = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";

/// Kamino SOL Reserve on Main Market
pub const KAMINO_SOL_RESERVE: &str = "d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q";

/// Kamino SOL Reserve Liquidity Supply Vault
pub const KAMINO_SOL_SUPPLY_VAULT: &str = "GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U";

/// Kamino SOL Reserve Fee Vault
pub const KAMINO_SOL_FEE_VAULT: &str = "3JNof8s453bwG5UqiXBLJc77NRQXezYYEBbk3fqnoKph";

/// Native Wrapped SOL Mint
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// SPL Token Program
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Instruction Discriminators (Anchor 8-byte sighashes)
pub const FLASH_BORROW_DISCRIMINATOR: [u8; 8] = [0x87, 0xe7, 0x34, 0xa7, 0x07, 0x34, 0xd4, 0xc1];
pub const FLASH_REPAY_DISCRIMINATOR: [u8; 8] = [185, 117, 0, 203, 96, 245, 180, 186];

/// Derives the Kamino Lending Market Authority PDA: PDA(["lma", lending_market], klend_program)
pub fn derive_lending_market_authority(lending_market: &Pubkey) -> Pubkey {
    let klend = Pubkey::from_str(KAMINO_PROGRAM_ID).unwrap();
    let (authority, _) = Pubkey::find_program_address(&[b"lma", lending_market.as_ref()], &klend);
    authority
}

/// Builds the `flash_borrow_reserve_liquidity` instruction for Kamino Lending.
///
/// Accounts expected by Kamino klend (10 non-optional accounts):
/// 0. `user_transfer_authority` [signer]
/// 1. `lending_market_authority` []
/// 2. `lending_market` []
/// 3. `reserve` [writable]
/// 4. `reserve_liquidity_mint` []
/// 5. `reserve_source_liquidity` [writable] (reserve supply vault)
/// 6. `user_destination_liquidity` [writable] (borrower's WSOL ATA)
/// 7. `reserve_liquidity_fee_receiver` [writable] (reserve fee vault)
/// 8. `sysvar_info` [] (sysvar::instructions::ID)
/// 9. `token_program` [] (SPL Token Program)
pub fn build_kamino_flash_borrow_ix(
    user_authority: &Pubkey,
    user_destination_wsol_ata: &Pubkey,
    borrow_lamports: u64,
) -> Result<Instruction> {
    let klend = Pubkey::from_str(KAMINO_PROGRAM_ID)?;
    let market = Pubkey::from_str(KAMINO_MAIN_MARKET)?;
    let market_auth = derive_lending_market_authority(&market);
    let reserve = Pubkey::from_str(KAMINO_SOL_RESERVE)?;
    let supply_vault = Pubkey::from_str(KAMINO_SOL_SUPPLY_VAULT)?;
    let fee_vault = Pubkey::from_str(KAMINO_SOL_FEE_VAULT)?;
    let wsol_mint = Pubkey::from_str(WSOL_MINT)?;
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID)?;
    let sysvar_instructions = sysvar::instructions::ID;

    let accounts = vec![
        AccountMeta::new_readonly(*user_authority, true),
        AccountMeta::new_readonly(market_auth, false),
        AccountMeta::new_readonly(market, false),
        AccountMeta::new(reserve, false),
        AccountMeta::new_readonly(wsol_mint, false),
        AccountMeta::new(supply_vault, false),
        AccountMeta::new(*user_destination_wsol_ata, false),
        AccountMeta::new(fee_vault, false),
        AccountMeta::new_readonly(sysvar_instructions, false),
        AccountMeta::new_readonly(token_program, false),
    ];

    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&FLASH_BORROW_DISCRIMINATOR);
    data.extend_from_slice(&borrow_lamports.to_le_bytes());

    Ok(Instruction {
        program_id: klend,
        accounts,
        data,
    })
}

/// Builds the `flash_repay_reserve_liquidity` instruction for Kamino Lending.
///
/// Accounts expected by Kamino klend:
/// 0. `user_transfer_authority` [signer]
/// 1. `lending_market_authority` []
/// 2. `lending_market` []
/// 3. `reserve` [writable]
/// 4. `reserve_liquidity_mint` []
/// 5. `reserve_destination_liquidity` [writable] (reserve supply vault)
/// 6. `user_source_liquidity` [writable] (borrower's WSOL ATA holding repayment + fee)
/// 7. `reserve_liquidity_fee_receiver` [writable] (reserve fee vault)
/// 8. `sysvar_info` [] (sysvar::instructions::ID)
/// 9. `token_program` [] (SPL Token Program)
///
/// Args:
/// - `liquidity_amount`: original borrowed lamports
/// - `borrow_instruction_index`: instruction index in the transaction where flash_borrow occurred (typically 0 or 1)
pub fn build_kamino_flash_repay_ix(
    user_authority: &Pubkey,
    user_source_wsol_ata: &Pubkey,
    repay_lamports: u64,
    borrow_instruction_index: u8,
) -> Result<Instruction> {
    let klend = Pubkey::from_str(KAMINO_PROGRAM_ID)?;
    let market = Pubkey::from_str(KAMINO_MAIN_MARKET)?;
    let market_auth = derive_lending_market_authority(&market);
    let reserve = Pubkey::from_str(KAMINO_SOL_RESERVE)?;
    let supply_vault = Pubkey::from_str(KAMINO_SOL_SUPPLY_VAULT)?;
    let fee_vault = Pubkey::from_str(KAMINO_SOL_FEE_VAULT)?;
    let wsol_mint = Pubkey::from_str(WSOL_MINT)?;
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID)?;
    let sysvar_instructions = sysvar::instructions::ID;

    let accounts = vec![
        AccountMeta::new_readonly(*user_authority, true),
        AccountMeta::new_readonly(market_auth, false),
        AccountMeta::new_readonly(market, false),
        AccountMeta::new(reserve, false),
        AccountMeta::new_readonly(wsol_mint, false),
        AccountMeta::new(supply_vault, false),
        AccountMeta::new(*user_source_wsol_ata, false),
        AccountMeta::new(fee_vault, false),
        AccountMeta::new_readonly(sysvar_instructions, false),
        AccountMeta::new_readonly(token_program, false),
    ];

    let mut data = Vec::with_capacity(17);
    data.extend_from_slice(&FLASH_REPAY_DISCRIMINATOR);
    data.extend_from_slice(&repay_lamports.to_le_bytes());
    data.push(borrow_instruction_index);

    Ok(Instruction {
        program_id: klend,
        accounts,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kamino_authority_pda() {
        let market = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
        let auth = derive_lending_market_authority(&market);
        assert!(!auth.to_bytes().is_empty());
    }

    #[test]
    fn test_build_kamino_instructions() {
        let user = Pubkey::new_unique();
        let user_wsol = Pubkey::new_unique();
        let amount = 10_000_000_000u64; // 10 SOL

        let borrow_ix = build_kamino_flash_borrow_ix(&user, &user_wsol, amount).unwrap();
        assert_eq!(borrow_ix.accounts.len(), 10);
        assert_eq!(borrow_ix.data[0..8], FLASH_BORROW_DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(borrow_ix.data[8..16].try_into().unwrap()),
            amount
        );

        let repay_ix = build_kamino_flash_repay_ix(&user, &user_wsol, amount, 0).unwrap();
        assert_eq!(repay_ix.accounts.len(), 10);
        assert_eq!(repay_ix.data[0..8], FLASH_REPAY_DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(repay_ix.data[8..16].try_into().unwrap()),
            amount
        );
        assert_eq!(repay_ix.data[16], 0);
    }
}
