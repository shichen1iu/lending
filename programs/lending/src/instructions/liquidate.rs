use crate::constants::*;
use crate::error::ErrorCode;
use crate::state::*;
use crate::utils::calculate_collateral_interest;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::spl_token_2022::solana_zk_token_sdk::instruction::transfer,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};
#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(
        mut,
        seeds = [collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_bank: Account<'info, Bank>,
    #[account(
        mut,
        seeds = [borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank: Account<'info, Bank>,
    #[account(
        mut,
        seeds = [b"treasury",collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_bank_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"treasury",borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = collateral_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_collateral_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = borrowed_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_borrowed_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [liquidator.key().as_ref()],
        bump
    )]
    pub user: Account<'info, User>,
    pub collateral_mint: InterfaceAccount<'info, Mint>,
    pub borrowed_mint: InterfaceAccount<'info, Mint>,
    pub price_update: Account<'info, PriceUpdateV2>,
    #[account(mut)]
    pub liquidator: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let collateral_bank = &mut ctx.accounts.collateral_bank;
    let user = &mut ctx.accounts.user;
    let borrowed_bank = &mut ctx.accounts.borrowed_bank;

    let price_update = &mut ctx.accounts.price_update;

    let sol_feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID)?;
    let usdc_feed_id = get_feed_id_from_hex(USDC_USD_FEED_ID)?;

    let sol_price = price_update.get_price_no_older_than(&Clock::get()?, MAX_AGE, &sol_feed_id)?;
    let usdc_price =
        price_update.get_price_no_older_than(&Clock::get()?, MAX_AGE, &usdc_feed_id)?;

    let total_collateral: u64;
    let total_borrowed: u64;

    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            let new_usdc = calculate_collateral_interest(
                user.deposited_usdc,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = usdc_price.price as u64 * new_usdc;
            let new_sol = calculate_collateral_interest(
                user.borrowed_sol,
                collateral_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed = sol_price.price as u64 * new_sol;
        }
        _ => {
            let new_sol = calculate_collateral_interest(
                user.deposited_sol,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = sol_price.price as u64 * new_sol;
            let new_usdc = calculate_collateral_interest(
                user.borrowed_usdc,
                collateral_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed = usdc_price.price as u64 * new_usdc;
        }
    }

    let health_factor = (total_collateral as f64 * collateral_bank.liquidation_threshold as f64)
        / total_borrowed as f64;

    //如果health_factor >=1,那么就不需要被清算,反之需要被清算
    require!(health_factor < 1.0, ErrorCode::NotUnderCollateralized);

    //当需要清算的时候,首先被清算的用户需要将借的钱归还
    let transfer_to_bank = TransferChecked {
        from: ctx
            .accounts
            .liquidator_borrowed_token_account
            .to_account_info(),
        to: ctx.accounts.borrowed_bank_token_account.to_account_info(),
        authority: ctx.accounts.liquidator.to_account_info(),
        mint: ctx.accounts.borrowed_mint.to_account_info(),
    };
    let transfer_to_bank_cpi_ctx = CpiContext::new(
        ctx.accounts.system_program.to_account_info(),
        transfer_to_bank,
    );

    //计算清算金额
    let liquidation_amount = total_borrowed
        .checked_mul(borrowed_bank.liquidation_close_factor)
        .unwrap();

    transfer_checked(
        transfer_to_bank_cpi_ctx,
        liquidation_amount,
        ctx.accounts.borrowed_mint.decimals,
    )?;

    //liquidator_amount是liquidator可以获得的质押代币数量:清算金额加上清算奖金
    let liquidator_amount =
        (liquidation_amount * collateral_bank.liquidation_bonus) + liquidation_amount;

    let transfer_to_liquidator = TransferChecked {
        from: ctx.accounts.collateral_bank_token_account.to_account_info(),
        to: ctx
            .accounts
            .liquidator_collateral_token_account
            .to_account_info(),
        authority: ctx.accounts.collateral_bank_token_account.to_account_info(),
        mint: ctx.accounts.collateral_mint.to_account_info(),
    };

    let mint_key = ctx.accounts.collateral_mint.to_account_info().key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury",
        mint_key.as_ref(),
        &[ctx.bumps.collateral_bank_token_account],
    ]];

    let transfer_to_liquidator_ctx_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        transfer_to_liquidator,
        signer_seeds,
    );
    //将加上清算奖励的抵押贷币数量转账给liquidator
    transfer_checked(
        transfer_to_liquidator_ctx_ctx,
        liquidator_amount,
        ctx.accounts.collateral_mint.decimals,
    )?;
    Ok(())
}
