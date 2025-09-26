use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use arcium_anchor::prelude::*;
use arcium_client::idl::arcium::types::CallbackAccount;

// Computation definition offsets for our MPC instructions
const COMP_DEF_OFFSET_INIT_ORDER_BOOK: u32 = comp_def_offset("init_order_book");
const COMP_DEF_OFFSET_SUBMIT_ORDER: u32 = comp_def_offset("submit_order");
const COMP_DEF_OFFSET_MATCH_ORDERS: u32 = comp_def_offset("match_orders");

declare_id!("5AVcTFBTCbR8CYcJYcqp7FgszwQMgEh5TySAUspb7y4E");

#[arcium_program]
pub mod conf_hide {
    use super::*;

    /// Initialize computation definition for order book initialization
    pub fn init_order_book_comp_def(ctx: Context<InitOrderBookCompDef>) -> Result<()> {
        init_comp_def(ctx.accounts, true, 0, None, None)?;
        Ok(())
    }

    /// Initialize computation definition for order submission
    pub fn init_submit_order_comp_def(ctx: Context<InitSubmitOrderCompDef>) -> Result<()> {
        init_comp_def(ctx.accounts, true, 0, None, None)?;
        Ok(())
    }

    /// Initialize computation definition for order matching
    pub fn init_match_orders_comp_def(ctx: Context<InitMatchOrdersCompDef>) -> Result<()> {
        init_comp_def(ctx.accounts, true, 0, None, None)?;
        Ok(())
    }

    /// Initialize a new trading pair with empty order book
    pub fn initialize_trading_pair(
        ctx: Context<InitializeTradingPair>,
        computation_offset: u64,
        trading_pair_id: u64,
        mxe_nonce: u128,
    ) -> Result<()> {
        let trading_pair = &mut ctx.accounts.trading_pair;
        trading_pair.bump = ctx.bumps.trading_pair;
        trading_pair.trading_pair_id = trading_pair_id;
        trading_pair.base_mint = ctx.accounts.base_mint.key();
        trading_pair.quote_mint = ctx.accounts.quote_mint.key();
        trading_pair.order_book = [0; 32]; // Will be populated by MPC callback
        trading_pair.order_book_nonce = 0;
        trading_pair.is_active = true;
        trading_pair.total_orders = 0;

        // Queue MPC computation to initialize empty order book
        let args = vec![Argument::PlaintextU128(mxe_nonce)];

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            None,
            vec![InitOrderBookCallback::callback_ix(&[
                CallbackAccount {
                    pubkey: ctx.accounts.trading_pair.key(),
                    is_writable: true,
                },
            ])],
        )?;
        Ok(())
    }

    /// Callback handler for order book initialization
    #[arcium_callback(encrypted_ix = "init_order_book")]
    pub fn init_order_book_callback(
        ctx: Context<InitOrderBookCallback>,
        output: ComputationOutputs<MXEEncryptedStruct<1>>,
    ) -> Result<()> {
        let order_book = match output {
            ComputationOutputs::Success(order_book_data) => order_book_data,
            _ => return Err(ErrorCode::AbortedComputation.into()),
        };

        let trading_pair = &mut ctx.accounts.trading_pair;
        trading_pair.order_book = order_book.ciphertexts[0];
        trading_pair.order_book_nonce = order_book.nonce;

        emit!(TradingPairInitializedEvent {
            trading_pair_id: trading_pair.trading_pair_id,
            order_book_nonce: order_book.nonce,
        });
        Ok(())
    }

    /// Submit a new order to the trading pair
    pub fn submit_order(
        ctx: Context<SubmitOrder>,
        computation_offset: u64,
        trading_pair_id: u64,
        price: u64,
        quantity: u64,
        is_buy: bool,
        client_pubkey: [u8; 32],
        client_nonce: u128,
    ) -> Result<()> {
        require!(
            ctx.accounts.trading_pair.is_active,
            ErrorCode::TradingPairInactive
        );

        // Validate order parameters
        require!(price > 0, ErrorCode::InvalidPrice);
        require!(quantity > 0, ErrorCode::InvalidQuantity);

        // For MVP, we'll skip balance validation and implement it later
        // In production, would check user has sufficient tokens

        // Prepare encrypted order arguments
        let timestamp = Clock::get()?.unix_timestamp as u64;
        let args = vec![
            // Order data
            Argument::ArcisPubkey(client_pubkey),
            Argument::PlaintextU128(client_nonce),
            Argument::EncryptedU64([0; 32]), // price (encrypted)
            Argument::EncryptedU64([0; 32]), // quantity (encrypted)
            Argument::EncryptedBool([0; 32]), // is_buy (encrypted)
            Argument::EncryptedU128([0; 32]), // trader_id (encrypted)
            Argument::PlaintextU64(timestamp),
            // Current order book
            Argument::PlaintextU128(ctx.accounts.trading_pair.order_book_nonce),
            Argument::Account(ctx.accounts.trading_pair.key(), 8, 32), // order book data
        ];

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            None,
            vec![SubmitOrderCallback::callback_ix(&[
                CallbackAccount {
                    pubkey: ctx.accounts.trading_pair.key(),
                    is_writable: true,
                },
            ])],
        )?;

        Ok(())
    }

    /// Callback handler for order submission
    #[arcium_callback(encrypted_ix = "submit_order")]
    pub fn submit_order_callback(
        ctx: Context<SubmitOrderCallback>,
        output: ComputationOutputs<MXEEncryptedStruct<1>>,
    ) -> Result<()> {
        let updated_book = match output {
            ComputationOutputs::Success(order_book_data) => order_book_data,
            _ => return Err(ErrorCode::AbortedComputation.into()),
        };

        let trading_pair = &mut ctx.accounts.trading_pair;
        trading_pair.order_book = updated_book.ciphertexts[0];
        trading_pair.order_book_nonce = updated_book.nonce;
        trading_pair.total_orders += 1;

        emit!(OrderSubmittedEvent {
            trading_pair_id: trading_pair.trading_pair_id,
            order_book_nonce: updated_book.nonce,
            total_orders: trading_pair.total_orders,
        });

        Ok(())
    }

    /// Match orders in the trading pair (batch auction)
    pub fn match_orders(
        ctx: Context<MatchOrders>,
        computation_offset: u64,
        trading_pair_id: u64,
    ) -> Result<()> {
        require!(
            ctx.accounts.trading_pair.is_active,
            ErrorCode::TradingPairInactive
        );

        let timestamp = Clock::get()?.unix_timestamp as u64;
        let args = vec![
            // Current order book
            Argument::PlaintextU128(ctx.accounts.trading_pair.order_book_nonce),
            Argument::Account(ctx.accounts.trading_pair.key(), 8, 32),
            // Timestamp for trades
            Argument::PlaintextU64(timestamp),
        ];

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            None,
            vec![MatchOrdersCallback::callback_ix(&[
                CallbackAccount {
                    pubkey: ctx.accounts.trading_pair.key(),
                    is_writable: true,
                },
            ])],
        )?;

        Ok(())
    }

    /// Callback handler for order matching
    #[arcium_callback(encrypted_ix = "match_orders")]
    pub fn match_orders_callback(
        ctx: Context<MatchOrdersCallback>,
        output: ComputationOutputs<MXEEncryptedStruct<1>>,
    ) -> Result<()> {
        let match_result = match output {
            ComputationOutputs::Success(match_data) => match_data,
            _ => return Err(ErrorCode::AbortedComputation.into()),
        };

        // Extract trade data and updated order book
        // For MVP, we'll emit events about trades but not execute token transfers yet
        // In production, would parse trades and execute token transfers

        let trading_pair = &mut ctx.accounts.trading_pair;
        // Update order book with post-matching state
        // Note: This is simplified - in production would extract from MatchResult

        emit!(OrdersMatchedEvent {
            trading_pair_id: trading_pair.trading_pair_id,
            match_nonce: match_result.nonce,
            timestamp: Clock::get()?.unix_timestamp as u64,
        });

        Ok(())
    }

    /// Execute token transfers for matched trades
    /// Called after MPC reveals matched trades
    pub fn execute_trade(
        ctx: Context<ExecuteTrade>,
        buyer_id: u128,
        seller_id: u128,
        trade_price: u64,
        trade_quantity: u64,
    ) -> Result<()> {
        // For MVP: Simple SOL for Token swap
        // Buyer pays: trade_price * trade_quantity in quote token (USDC)
        // Seller pays: trade_quantity in base token (SOL equivalent)

        // Transfer quote tokens from buyer to seller
        let cpi_accounts = Transfer {
            from: ctx.accounts.buyer_quote_account.to_account_info(),
            to: ctx.accounts.seller_quote_account.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        let quote_amount = trade_price
            .checked_mul(trade_quantity)
            .ok_or(ErrorCode::MathOverflow)?;

        token::transfer(cpi_ctx, quote_amount)?;

        // Transfer base tokens from seller to buyer
        let cpi_accounts = Transfer {
            from: ctx.accounts.seller_base_account.to_account_info(),
            to: ctx.accounts.buyer_base_account.to_account_info(),
            authority: ctx.accounts.seller.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);

        token::transfer(cpi_ctx, trade_quantity)?;

        emit!(TradeExecutedEvent {
            buyer_id,
            seller_id,
            price: trade_price,
            quantity: trade_quantity,
            timestamp: Clock::get()?.unix_timestamp as u64,
        });

        Ok(())
    }
}

/// Trading pair account storing encrypted order book state
#[account]
#[derive(InitSpace)]
pub struct TradingPair {
    /// Unique identifier for this trading pair
    pub trading_pair_id: u64,
    /// Base token mint (e.g., SOL)
    pub base_mint: Pubkey,
    /// Quote token mint (e.g., USDC)
    pub quote_mint: Pubkey,
    /// Encrypted order book data
    pub order_book: [u8; 32],
    /// Nonce for order book encryption
    pub order_book_nonce: u128,
    /// Whether trading is active
    pub is_active: bool,
    /// Total orders submitted
    pub total_orders: u64,
    /// PDA bump
    pub bump: u8,
}

// Account validation structures for initialization
#[queue_computation_accounts("init_order_book", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, trading_pair_id: u64)]
pub struct InitializeTradingPair<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, SignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset))]
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_ORDER_BOOK))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Account<'info, Cluster>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
    #[account(
        init,
        payer = payer,
        space = 8 + TradingPair::INIT_SPACE,
        seeds = [b"trading_pair", trading_pair_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub trading_pair: Account<'info, TradingPair>,
    pub base_mint: Account<'info, Mint>,
    pub quote_mint: Account<'info, Mint>,
}

// Callback account structure
#[callback_accounts("init_order_book")]
#[derive(Accounts)]
pub struct InitOrderBookCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_ORDER_BOOK))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: AccountInfo<'info>,
    #[account(mut)]
    pub trading_pair: Account<'info, TradingPair>,
}

// Submit order accounts
#[queue_computation_accounts("submit_order", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, trading_pair_id: u64)]
pub struct SubmitOrder<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, SignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset))]
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SUBMIT_ORDER))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Account<'info, Cluster>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
    #[account(
        mut,
        seeds = [b"trading_pair", trading_pair_id.to_le_bytes().as_ref()],
        bump = trading_pair.bump,
    )]
    pub trading_pair: Account<'info, TradingPair>,
}

#[callback_accounts("submit_order")]
#[derive(Accounts)]
pub struct SubmitOrderCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SUBMIT_ORDER))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: AccountInfo<'info>,
    #[account(mut)]
    pub trading_pair: Account<'info, TradingPair>,
}

// Match orders accounts
#[queue_computation_accounts("match_orders", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, trading_pair_id: u64)]
pub struct MatchOrders<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, SignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset))]
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_MATCH_ORDERS))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Account<'info, Cluster>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
    #[account(
        mut,
        seeds = [b"trading_pair", trading_pair_id.to_le_bytes().as_ref()],
        bump = trading_pair.bump,
    )]
    pub trading_pair: Account<'info, TradingPair>,
}

#[callback_accounts("match_orders")]
#[derive(Accounts)]
pub struct MatchOrdersCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_MATCH_ORDERS))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: AccountInfo<'info>,
    #[account(mut)]
    pub trading_pair: Account<'info, TradingPair>,
}

// Computation definition initialization accounts
#[init_computation_definition_accounts("init_order_book", payer)]
#[derive(Accounts)]
pub struct InitOrderBookCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    pub comp_def_account: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[init_computation_definition_accounts("submit_order", payer)]
#[derive(Accounts)]
pub struct InitSubmitOrderCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    pub comp_def_account: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[init_computation_definition_accounts("match_orders", payer)]
#[derive(Accounts)]
pub struct InitMatchOrdersCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    pub comp_def_account: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// Trade execution accounts
#[derive(Accounts)]
#[instruction(buyer_id: u128, seller_id: u128, trade_price: u64, trade_quantity: u64)]
pub struct ExecuteTrade<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut)]
    pub seller: Signer<'info>,
    #[account(mut)]
    pub buyer_base_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub buyer_quote_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub seller_base_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub seller_quote_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

// Events
#[event]
pub struct TradingPairInitializedEvent {
    pub trading_pair_id: u64,
    pub order_book_nonce: u128,
}

#[event]
pub struct OrderSubmittedEvent {
    pub trading_pair_id: u64,
    pub order_book_nonce: u128,
    pub total_orders: u64,
}

#[event]
pub struct OrdersMatchedEvent {
    pub trading_pair_id: u64,
    pub match_nonce: u128,
    pub timestamp: u64,
}

#[event]
pub struct TradeExecutedEvent {
    pub buyer_id: u128,
    pub seller_id: u128,
    pub price: u64,
    pub quantity: u64,
    pub timestamp: u64,
}

// Error codes
#[error_code]
pub enum ErrorCode {
    #[msg("The computation was aborted")]
    AbortedComputation,
    #[msg("Trading pair is inactive")]
    TradingPairInactive,
    #[msg("Invalid price")]
    InvalidPrice,
    #[msg("Invalid quantity")]
    InvalidQuantity,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Cluster not set")]
    ClusterNotSet,
}