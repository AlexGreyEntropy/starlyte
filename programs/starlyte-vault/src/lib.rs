use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Mint, Token, TokenAccount, Transfer},
    associated_token::AssociatedToken, 
};
use pyth_sdk_solana::state::PriceAccount; //oracol

declare_id!("lyteGiYQgaLZW2fNNUjYZRNRqjDgXGZAST1endGDxjKP");

#[program]
pub mod starlyte_vault {
    use super::*;

  /// Initializare vault cu SOL collateral
    pub fn initialize_vault(
        ctx: Context<InitializeVault>, //context
        deposit_amount: u64,
        mint_amount: u64,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault; //rezultat
        let clock = Clock::get()?; //timestap
/// Verificare 150% collateralization      
        let required_collateral = mint_amount  //suma colateral
            .checked_mul(150) //multiplu
            .and_then(|v| v.checked_div(100)) //formula
            .ok_or(ErrorCode::MathOverflow)?; //rezultat
        
        require!(deposit_amount >= required_collateral, ErrorCode::InsufficientCollateral); //logic necesar
    /// Initializare vault state
        vault.collateral_amount = deposit_amount; //depozit
        vault.minted_lyteusd = mint_amount; //mintare lyteUSD
        vault.created_at = clock.unix_timestamp; //timestamp
        vault.cooldown_end = 0; //cooldown
        vault.liquidated = false; //status lichidare
        vault.liquidation_start = 0; //status lichidare
        vault.bump = *ctx.bumps.get("vault").unwrap(); //bumps pentru pda
        
  /// Transfer JitoSOL collateral in vault
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(), //cpi la token accounts
                Transfer {
                    from: ctx.accounts.user_jitosol.to_account_info(),
                    to: ctx.accounts.vault_jitosol.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            deposit_amount,
        )?;

        /// Mint lyteUSD catre user
        token::mint_to(
            CpiContext::new_with_signer( //cpi again
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.lyteusd_mint.to_account_info(),
                    to: ctx.accounts.user_lyteusd.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&["vault".as_bytes(), &[vault.bump]]], //o mica problema rezolvata de vault slices si bump
            ),
            mint_amount,
        )?;

        Ok(())
    }

  /// Close vault si returnare de collateral
    pub fn close_vault(ctx: Context<CloseVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;

        require!(!vault.liquidated, ErrorCode::VaultLiquidated); //cerinte necesare
        require!(vault.cooldown_end > 0, ErrorCode::CooldownNotStarted); //cerinte necesare
        require!(clock.unix_timestamp >= vault.cooldown_end, ErrorCode::CooldownActive); //timestamp necesar pentru cooldown'ul de 7 zile

    /// Calculat instant unstaking fee de 0.1%
        let fee = vault.collateral_amount
            .checked_mul(0.1)
            .and_then(|v| v.checked_div(100))
            .ok_or(ErrorCode::MathOverflow)?;
        
        let remaining_collateral = vault.collateral_amount.checked_sub(fee).unwrap();

    // Transfer de collateral la user
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_jitosol.to_account_info(),
                    to: ctx.accounts.user_jitosol.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&["vault".as_bytes(), &[vault.bump]]], //m-au mancat astea
            ),
            remaining_collateral,
        )?;

// Transfer de fee'uri la treasury
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_jitosol.to_account_info(),
                    to: ctx.accounts.treasury_jitosol.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&["vault".as_bytes(), &[vault.bump]]], //nu ma mai complic
            ),
            fee,
        )?;

  // Burn lyteUSD pentru balanta
        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::Burn {
                    mint: ctx.accounts.lyteusd_mint.to_account_info(),
                    to: ctx.accounts.user_lyteusd.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            vault.minted_lyteusd,
        )?;

        Ok(())
    }

/// Trigger pentru procesul liquidare
    pub fn start_liquidation(ctx: Context<StartLiquidation>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        
        // Verify collateral ratio <= 110%
        let current_ratio = calculate_collateral_ratio(
            vault.collateral_amount,
            vault.minted_lyteusd,
            &ctx.accounts.price_account
        )?;
        
        require!(current_ratio <= 110, ErrorCode::LiquidationNotRequired); //aici aveam probleme
        require!(!vault.liquidated, ErrorCode::VaultLiquidated); //traiasca dev'ii mei

        vault.liquidation_start = clock.unix_timestamp;
        Ok(())
    }

/ Mintam lyteUSD daca collateral ratio >150%
    pub fn mint_surplus(ctx: Context<MintSurplus>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        
        let current_ratio = calculate_collateral_ratio( //calculator ratio
            vault.collateral_amount,
            vault.minted_lyteusd,
            &ctx.accounts.price_account
        )?;
        
        require!(current_ratio > 150, ErrorCode::NoSurplusAvailable); //verificam
        
    //Maximumul de mintat
        let max_mint = vault.collateral_amount
            .checked_mul(100)
            .and_then(|v| v.checked_div(150))
            .ok_or(ErrorCode::MathOverflow)?;
        
        let available_mint = max_mint.checked_sub(vault.minted_lyteusd).unwrap(); //cat putem minta
        require!(amount <= available_mint, ErrorCode::ExceedsSurplusLimit);

  // Mintam extra lyteUSD daca e nevoie
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.lyteusd_mint.to_account_info(),
                    to: ctx.accounts.user_lyteusd.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&["vault".as_bytes(), &[vault.bump]]],
            ),
            amount,
        )?;

        vault.minted_lyteusd = vault.minted_lyteusd.checked_add(amount).unwrap();
        
        Ok(())
    }
}

//functii de ajutor sa calculam ratio
fn calculate_collateral_ratio(
    collateral_amount: u64,
    minted_amount: u64,
    price_account: &PriceAccount,
) -> Result<u64> {
    let price = price_account.get_current_price().unwrap();
    let collateral_value = collateral_amount
        .checked_mul(price.price as u64)
        .and_then(|v| v.checked_div(10u64.pow(price.expo.unsigned_abs())))
        .ok_or(ErrorCode::MathOverflow)?;

    let ratio = collateral_value
        .checked_mul(100)
        .and_then(|v| v.checked_div(minted_amount))
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(ratio)
}

//accounts
#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(
        init,
        payer = user,
        space = 8 + Vault::INIT_SPACE,
        seeds = [b"vault", user.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,
    
    #[account(mut)]
    pub user: Signer<'info>,
    
    ///JitoSOL
    #[account(mut)]
    pub user_jitosol: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = jitosol_mint,
        associated_token::authority = vault
    )]
    pub vault_jitosol: Account<'info, TokenAccount>,
    
    //lyteUSD
    #[account(mut)]
    pub lyteusd_mint: Account<'info, Mint>,
    #[account(
        mut,
        associated_token::mint = lyteusd_mint,
        associated_token::authority = user
    )]
    pub user_lyteusd: Account<'info, TokenAccount>,
    
    // treasury
    #[account(mut)]
    pub treasury_jitosol: Account<'info, TokenAccount>,
    
    // Pyth oracle
    pub price_account: Account<'info, PriceAccount>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

//state'ul vaultului
#[account]
#[derive(InitSpace)]
pub struct Vault {
    pub collateral_amount: u64,      // JitoSOL
    pub minted_lyteusd: u64,         // debt
    pub created_at: i64,             // unix timestamp
    pub cooldown_end: i64,           // Cooldown
    pub liquidated: bool,            // liquidation status
    pub liquidation_start: i64,      // Liquidation start
    pub bump: u8,                    // PDA bump
}

// erori
#[error_code]
pub enum ErrorCode {
    #[msg("Insufficient collateral")]
    InsufficientCollateral,
    #[msg("Vault is in cooldown period")]
    CooldownActive,
    #[msg("Cooldown not started")]
    CooldownNotStarted,
    #[msg("Vault already liquidated")]
    VaultLiquidated,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Liquidation not required")]
    LiquidationNotRequired,
    #[msg("No surplus available")]
    NoSurplusAvailable,
    #[msg("Exceeds surplus limit")]
    ExceedsSurplusLimit,
}
