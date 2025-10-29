use anchor_lang::prelude::*;

declare_id!("EUR47AAUsA8aFEgoXdvaDDTXsqfnFpbTMyHe9UwbvmTm");

#[program]
pub mod amm_copilot {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
