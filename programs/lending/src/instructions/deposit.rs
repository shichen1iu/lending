use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::*;

#[derive(Accounts)]
pub struct Deposit<'info> {
    pub mint: InterfaceAccount<'info, Mint>,
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
        mut,
        associated_token::mint = mint,
        associated_token::authority = payer,
        associated_token::token_program = token_program,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    let transfer_cpi_account = TransferChecked {
        from: ctx.accounts.user_token_account.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
        to: ctx.accounts.bank_token_account.to_account_info(),
        authority: ctx.accounts.payer.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx = CpiContext::new(cpi_program, transfer_cpi_account);

    let decimals = ctx.accounts.mint.decimals;
    transfer_checked(cpi_ctx, amount, decimals)?;

    let bank = &mut ctx.accounts.bank;

    let user_shares = if bank.total_depoists == 0 {
        // 首次存款，shares 等于存款金额
        bank.total_depoists = amount;
        bank.total_deposit_shares = amount;
        amount
    } else {
        // 计算新增份额：(amount * total_shares) / total_deposits
        amount
            .checked_mul(bank.total_deposit_shares)
            .unwrap()
            .checked_div(bank.total_depoists)
            .unwrap()
    };

    bank.total_depoists = bank.total_depoists.checked_add(amount).unwrap();
    bank.total_deposit_shares = bank.total_deposit_shares.checked_add(user_shares).unwrap();

    let user = &mut ctx.accounts.user;

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.deposited_usdc += amount;
            user.deposited_usdc_shares += user_shares;
        }
        _ => {
            user.deposited_sol += amount;
            user.deposited_sol_shares += user_shares;
        }
    }


    user.last_updated = Clock::get()?.unix_timestamp;
    Ok(())
}
