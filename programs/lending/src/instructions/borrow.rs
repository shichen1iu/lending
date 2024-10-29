use std::f64::consts::E;

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

use crate::constants::*;
use crate::error::ErrorCode;
use crate::state::*;

#[derive(Accounts)]
pub struct Borrow<'info> {
    #[account(
        mut,
        seeds = [mint.key().as_ref()],
        bump
    )]
    pub bank: Account<'info, Bank>,
    #[account(
        mut,
        seeds = [b"treasury",mint.key().as_ref()],
        bump
    )]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [payer.key().as_ref()],
        bump
    )]
    pub user: Account<'info, User>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = payer,
        associated_token::token_program = token_program
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    pub price_update: Account<'info, PriceUpdateV2>, //使用pyth oracles 来更新价格
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
    let bank = &mut ctx.accounts.bank;
    let user = &mut ctx.accounts.user;

    let price_update = &mut ctx.accounts.price_update;

    let total_collateral: u64;

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            // 这里的mint为usdc这表明用户想要质押sol换取usdc,所以就要计算的当前sol的价值
            let sol_feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID)?;
            let sol_price =
                price_update.get_price_no_older_than(&Clock::get()?, MAX_AGE, &sol_feed_id)?; //得到不stale于100s的价格
            let new_value = calculate_collateral_interest(
                user.deposited_sol,
                bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = sol_price.price as u64 * new_value;
        }
        _ => {
            let usdc_feed_id = get_feed_id_from_hex(USDC_USD_FEED_ID)?;
            let usdc_price =
                price_update.get_price_no_older_than(&Clock::get()?, MAX_AGE, &usdc_feed_id)?;
            let new_value = calculate_collateral_interest(
                user.deposited_usdc,
                bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = usdc_price.price as u64 * new_value;
        }
    }

    let borrowable_amount = total_collateral
        .checked_mul(bank.liquidation_threshold)
        .unwrap();

    require!(borrowable_amount >= amount, ErrorCode::OverBorrowableAmount);

    let transfer_cpx_account = TransferChecked {
        from: ctx.accounts.bank_token_account.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
        authority: ctx.accounts.bank_token_account.to_account_info(),
    };

    let mint_key = ctx.accounts.mint.to_account_info().key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury",
        mint_key.as_ref(),
        &[ctx.bumps.bank_token_account],
    ]];

    let transfer_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        transfer_cpx_account,
        signer_seeds,
    );

    transfer_checked(transfer_ctx, amount, ctx.accounts.mint.decimals)?;

    if bank.total_borrowed == 0 {
        bank.total_borrowed = amount;
        bank.total_borrowed_shares = amount;
    }

    let borrow_ratio = amount.checked_div(bank.total_borrowed).unwrap();
    let user_shares = bank
        .total_borrowed_shares
        .checked_mul(borrow_ratio)
        .unwrap();

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.borrowed_usdc += amount;
            user.borrowed_usdc_shares += user_shares;
        }
        _ => {
            user.borrowed_sol += amount;
            user.borrowed_sol_shares += user_shares;
        }
    }

    user.last_updated_borrow = Clock::get()?.unix_timestamp;

    Ok(())
}

fn calculate_collateral_interest(
    deposited: u64,
    interest_rate: u64,
    last_updated: i64,
) -> Result<u64> {
    let current_time = Clock::get()?.unix_timestamp;
    let time_diff = current_time - last_updated;
    let new_value = (deposited as f64 * E.powf(interest_rate as f64 * time_diff as f64)) as u64;
    Ok(new_value)
}
