use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::error::ErrorCode;
use crate::state::*;

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(
        mut,
        seeds = [mint.key().as_ref()],
        bump,
    )]
    pub bank: Account<'info, Bank>,
    #[account(
        mut,
        seeds = [b"treasury", mint.key().as_ref()],
        bump,
    )]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [payer.key().as_ref()],
        bump,
    )]
    pub user: Account<'info, User>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = user,
        associated_token::token_program = token_program,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
    let user = &mut ctx.accounts.user;

    // 使用deposited_value来用户存储的资产
    let deposited_value: u64;

    if ctx.accounts.mint.to_account_info().key() == user.usdc_address {
        deposited_value = user.deposited_usdc;
    } else {
        deposited_value = user.deposited_sol;
    }

    if amount > deposited_value {
        return Err(ErrorCode::InsufficientFunds.into());
    }

    let transfer_cpi_accounts = TransferChecked {
        from: ctx.accounts.bank_token_account.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        authority: ctx.accounts.bank_token_account.to_account_info(),
    };

    let mint_key = ctx.accounts.mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury",
        mint_key.as_ref(),
        &[ctx.bumps.bank_token_account],
    ]];

    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx = CpiContext::new_with_signer(cpi_program, transfer_cpi_accounts, signer_seeds);

    transfer_checked(cpi_ctx, amount, ctx.accounts.mint.decimals)?;

    let bank = &mut ctx.accounts.bank;

    let shares_to_remove =
        (amount as f64 / bank.total_depoists as f64) * bank.total_depoists as f64;

    let user = &mut ctx.accounts.user;

    if ctx.accounts.mint.to_account_info().key() == user.usdc_address {
        user.deposited_usdc -= amount;
        user.deposited_usdc_shares -= shares_to_remove as u64;
    } else {
        user.deposited_sol -= amount;
        user.deposited_sol_shares -= shares_to_remove as u64;
    }

    bank.total_depoists -= amount;
    bank.total_deposit_shares -= shares_to_remove as u64;
    Ok(())
}
