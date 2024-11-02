use crate::constants::*;
use crate::error::ErrorCode;
use crate::state::*;
use crate::utils::calculate_collateral_interest;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
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
    pub collateral_bank: Box<Account<'info, Bank>>,
    #[account(
        mut,
        seeds = [borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank: Box<Account<'info, Bank>>,
    #[account(
        mut,
        seeds = [b"treasury",collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_bank_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [b"treasury",borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = collateral_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_collateral_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = borrowed_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_borrowed_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [liquidator.key().as_ref()],
        bump
    )]
    pub user: Box<Account<'info, User>>,
    pub collateral_mint: InterfaceAccount<'info, Mint>,
    pub borrowed_mint: InterfaceAccount<'info, Mint>,
    pub sol_price_feed: Account<'info, PriceUpdateV2>,
    pub usdc_price_feed: Account<'info, PriceUpdateV2>,
    #[account(mut)]
    pub liquidator: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let collateral_bank = &mut ctx.accounts.collateral_bank;
    let borrowed_bank = &mut ctx.accounts.borrowed_bank;

    let user = &mut ctx.accounts.user;

    let sol_price_feed = &mut ctx.accounts.sol_price_feed;
    let usdc_price_feed = &mut ctx.accounts.usdc_price_feed;

    let sol_feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID)?;
    let usdc_feed_id = get_feed_id_from_hex(USDC_USD_FEED_ID)?;

    let sol_price =
        sol_price_feed.get_price_no_older_than(&Clock::get()?, MAX_AGE, &sol_feed_id)?;
    let usdc_price =
        usdc_price_feed.get_price_no_older_than(&Clock::get()?, MAX_AGE, &usdc_feed_id)?;

    //都是以usd为单位
    let total_collateral: u64;
    let total_borrowed_in_usd: u64;

    //总共借款的清算金额= 借款 + 利息
    let borrow_liquidation_amount: u64;

    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            let new_usdc = calculate_collateral_interest(
                user.deposited_usdc,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = usdc_price.price as u64 * new_usdc;
            //借款的金额
            borrow_liquidation_amount = calculate_collateral_interest(
                user.borrowed_sol,
                collateral_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed_in_usd = sol_price.price as u64 * borrow_liquidation_amount;
        }
        _ => {
            let new_sol = calculate_collateral_interest(
                user.deposited_sol,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = sol_price.price as u64 * new_sol;
            borrow_liquidation_amount = calculate_collateral_interest(
                user.borrowed_usdc,
                collateral_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed_in_usd = usdc_price.price as u64 * borrow_liquidation_amount;
        }
    }

    //通过同一单位usd 来计算 健康因子
    let health_factor = total_collateral
        .checked_mul(collateral_bank.liquidation_threshold)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(10_000)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(total_borrowed_in_usd)
        .ok_or(ErrorCode::MathOverflow)?;

    //如果health_factor >=1,那么就不需要被清算,反之需要被清算
    require!(health_factor < 1_u64, ErrorCode::NotUnderCollateralized);

    //当需要清算的时候,首先被清算的用户需要将借的钱根据liquidation_close_factor归还
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

    //计算借款的清算金额
    let borrow_liquidation_amount_liquidation_close_factor = borrow_liquidation_amount
        .checked_mul(borrowed_bank.liquidation_close_factor)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(10_000)
        .ok_or(ErrorCode::MathOverflow)?;

    transfer_checked(
        transfer_to_bank_cpi_ctx,
        borrow_liquidation_amount_liquidation_close_factor,
        ctx.accounts.borrowed_mint.decimals,
    )?;

    //计算借款的份额 并 更新信息
    let borrow_shares = borrow_liquidation_amount_liquidation_close_factor
        .checked_mul(borrowed_bank.total_borrowed_shares)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(borrowed_bank.total_borrowed)
        .ok_or(ErrorCode::MathOverflow)?;

    borrowed_bank.total_borrowed_shares -= borrow_shares;
    borrowed_bank.total_borrowed -= borrow_liquidation_amount_liquidation_close_factor;

    //将borrow_liquidation_amount_liquidation_close_factor 换算成usd
    //再将usd 换算成 collateral_liquidation_amount_liquidation_close_factor
    //同时 更新 user的借款信息
    let collateral_liquidation_amount_liquidation_close_factor: u64;
    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.borrowed_sol -= borrow_liquidation_amount_liquidation_close_factor;
            user.borrowed_sol_shares -= borrow_shares;

            collateral_liquidation_amount_liquidation_close_factor =
                borrow_liquidation_amount_liquidation_close_factor
                    .checked_mul(sol_price.price as u64)
                    .ok_or(ErrorCode::MathOverflow)?
                    .checked_div(usdc_price.price as u64)
                    .ok_or(ErrorCode::MathOverflow)?;
        }
        _ => {
            user.borrowed_usdc -= borrow_liquidation_amount_liquidation_close_factor;
            user.borrowed_usdc_shares -= borrow_shares;

            collateral_liquidation_amount_liquidation_close_factor =
                borrow_liquidation_amount_liquidation_close_factor
                    .checked_mul(usdc_price.price as u64)
                    .ok_or(ErrorCode::MathOverflow)?
                    .checked_div(sol_price.price as u64)
                    .ok_or(ErrorCode::MathOverflow)?;
        }
    }

    //collateral_liquidation_amount 是liquidator可以获得的质押代币数量:归还的钱加上清算奖金
    //即 collateral_liquidation_amount_liquidation_close_factor  + bonus
    let collateral_liquidation_amount = collateral_liquidation_amount_liquidation_close_factor
        .checked_mul(collateral_bank.liquidation_bonus)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(10_000)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_add(collateral_liquidation_amount_liquidation_close_factor)
        .ok_or(ErrorCode::MathOverflow)?;

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
        collateral_liquidation_amount,
        ctx.accounts.collateral_mint.decimals,
    )?;

    let collateral_shares = collateral_liquidation_amount
        .checked_mul(collateral_bank.total_deposit_shares)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(collateral_bank.total_depoists)
        .ok_or(ErrorCode::MathOverflow)?;

    collateral_bank.total_deposit_shares -= collateral_shares;
    collateral_bank.total_depoists -= collateral_liquidation_amount;

    // 更新清算了之后用户的信息
    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.deposited_usdc -= collateral_liquidation_amount;
            user.deposited_usdc_shares -= collateral_shares;
        }
        _ => {
            user.deposited_sol -= collateral_liquidation_amount;
            user.deposited_sol_shares -= collateral_shares;
        }
    }
    Ok(())
}
