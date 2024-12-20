use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

use crate::constants::*;
use crate::error::ErrorCode;
use crate::state::*;
use crate::utils::calculate_collateral_interest;
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
    pub sol_or_usdc_price_feed: Account<'info, PriceUpdateV2>, //使用pyth oracles 来更新价格
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
    let bank = &mut ctx.accounts.bank;
    let user = &mut ctx.accounts.user;

    let sol_or_usdc_price_feed = &mut ctx.accounts.sol_or_usdc_price_feed;

    //抵押物的价值 (以usd为单位)
    let total_collateral_in_usd: u64;
    // 用户想要借出的金额(以usd为单位)
    let amount_in_usd: u64;

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            // 这里的mint为usdc这表明用户想要用deposited的sol换取usdc,所以就要计算的当前sol的价值
            let sol_feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID)?;
            let sol_price = sol_or_usdc_price_feed.get_price_no_older_than(
                &Clock::get()?,
                MAX_AGE,
                &sol_feed_id,
            )?; //得到不stale于100s的价格
            let new_value = calculate_collateral_interest(
                user.deposited_sol,
                bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral_in_usd = sol_price.price as u64 * new_value;
            amount_in_usd = amount * sol_price.price as u64;
        }
        _ => {
            let usdc_feed_id = get_feed_id_from_hex(USDC_USD_FEED_ID)?;
            let usdc_price = sol_or_usdc_price_feed.get_price_no_older_than(
                &Clock::get()?,
                MAX_AGE,
                &usdc_feed_id,
            )?;
            let new_value = calculate_collateral_interest(
                user.deposited_usdc,
                bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral_in_usd = usdc_price.price as u64 * new_value;
            amount_in_usd = amount * usdc_price.price as u64;
        }
    }

    msg!("total_collateral_in_usd: {}", total_collateral_in_usd);
    msg!("bank.max_ltv: {}", bank.max_ltv);
    // 通过抵押物的价值计算出用户可以借出的最大金额(usd为单位)
    let borrowable_amount_in_usd = total_collateral_in_usd
        .checked_div(10_000)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_mul(bank.max_ltv)
        .ok_or(ErrorCode::MathOverflow)?;

    require!(
        borrowable_amount_in_usd >= amount_in_usd,
        ErrorCode::OverBorrowableAmount
    );

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

    bank.total_borrowed_shares += user_shares;
    bank.total_borrowed += amount;

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
