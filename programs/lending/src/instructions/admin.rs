use crate::state::Bank;
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

#[derive(Accounts)]
pub struct InitBank<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + Bank::INIT_SPACE,
        seeds = [mint.key().as_ref()],
        bump,
    )]
    pub bank: Account<'info, Bank>, //每种mint都有一个自己的bank
    #[account(
        init,
        seeds = [b"treasury", mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = bank_token_account,
        payer = payer,
    )]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn process_init_bank(
    ctx: Context<InitBank>,
    liquidation_threshold: u64,
    max_ltv: u64,
) -> Result<()> {
    let bank = &mut ctx.accounts.bank;
    bank.mint_address = ctx.accounts.mint.key();
    bank.authority = ctx.accounts.payer.key();
    bank.liquidation_threshold = liquidation_threshold;
    bank.max_ltv = max_ltv;
    Ok(())
}
