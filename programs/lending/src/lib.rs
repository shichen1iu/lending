pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;
pub mod utils;
use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("8s8Bqp4G6QmFW6gXn3vCFyWYDwiWixAHUjo4wsr6dftc");

#[program]
pub mod lending {
    use super::*;
    pub fn init_bank(
        ctx: Context<InitBank>,
        liquidation_threshold: u64,
        max_ltv: u64,
        liquidation_close_factor: u64,
        liquidation_bonus: u64,
    ) -> Result<()> {
        process_init_bank(
            ctx,
            liquidation_threshold,
            liquidation_close_factor,
            liquidation_bonus,
            max_ltv,
        )
    }

    pub fn init_user(ctx: Context<InitUser>, usdc_address: Pubkey) -> Result<()> {
        process_init_user(ctx, usdc_address)
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        process_deposit(ctx, amount)
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        process_withdraw(ctx, amount)
    }

    pub fn borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
        process_borrow(ctx, amount)
    }

    pub fn repay(ctx: Context<Repay>, amount: u64) -> Result<()> {
        process_repay(ctx, amount)
    }

    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        process_liquidate(ctx)
    }
}
