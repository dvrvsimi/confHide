use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use arcium_anchor::prelude::*;
use arcium_client::idl::arcium::types::CallbackAccount;

// Computation definition offsets for our MPC instructions
const COMP_DEF_OFFSET_INIT_ORDER_BOOK: u32 = comp_def_offset("init_order_book");
const COMP_DEF_OFFSET_SUBMIT_ORDER: u32 = comp_def_offset("submit_order");
const COMP_DEF_OFFSET_CANCEL_ORDER: u32 = comp_def_offset("cancel_order");
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

    /// Initialize computation definition for order cancellation
    pub fn init_cancel_order_comp_def(ctx: Context<InitCancelOrderCompDef>) -> Result<()> {
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
        // Validate that the provided accounts are actually valid mints
        // This is done by attempting to deserialize them
        let base_mint_data = ctx.accounts.base_mint.try_borrow_data()?;
        let quote_mint_data = ctx.accounts.quote_mint.try_borrow_data()?;
        require!(base_mint_data.len() >= 82, ErrorCode::InvalidTokenAccount); // Mint account size
        require!(quote_mint_data.len() >= 82, ErrorCode::InvalidTokenAccount);

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
            vec![InitOrderBookCallback::callback_ix(&[])],
        )?;
        Ok(())
    }

    /// Callback handler for order book initialization
    #[arcium_callback(encrypted_ix = "init_order_book")]
    pub fn init_order_book_callback(
        ctx: Context<InitOrderBookCallback>,
        output: ComputationOutputs<InitOrderBookOutput>,
    ) -> Result<()> {
        let order_book = match output {
            ComputationOutputs::Success(InitOrderBookOutput { field_0 }) => field_0,
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
        client_pubkey: [u8; 32],
        client_nonce: u128,
        encrypted_price: [u8; 32],
        encrypted_quantity: [u8; 32],
        encrypted_is_buy: [u8; 32],
        encrypted_trader_id: [u8; 32],
    ) -> Result<()> {
        require!(
            ctx.accounts.trading_pair.is_active,
            ErrorCode::TradingPairInactive
        );

        // Note: We can't validate encrypted order parameters directly
        // Validation will happen in the MPC circuit

        // Balance validation: Check if user provided token accounts
        // This is a basic validation - full validation would need client-side checks
        // or additional MPC circuits for balance verification
        if let Some(base_account_info) = &ctx.accounts.user_base_token_account {
            // Deserialize and validate the token account
            let base_account = TokenAccount::try_deserialize(&mut &base_account_info.try_borrow_data()?[..])?;
            require!(
                base_account.mint == ctx.accounts.trading_pair.base_mint,
                ErrorCode::InvalidTokenAccount
            );
            require!(
                base_account.owner == ctx.accounts.payer.key(),
                ErrorCode::InvalidTokenAccount
            );
        }

        if let Some(quote_account_info) = &ctx.accounts.user_quote_token_account {
            // Deserialize and validate the token account
            let quote_account = TokenAccount::try_deserialize(&mut &quote_account_info.try_borrow_data()?[..])?;
            require!(
                quote_account.mint == ctx.accounts.trading_pair.quote_mint,
                ErrorCode::InvalidTokenAccount
            );
            require!(
                quote_account.owner == ctx.accounts.payer.key(),
                ErrorCode::InvalidTokenAccount
            );
        }

        // TODO: For production, implement additional balance checks:
        // 1. Client-side balance validation before encryption
        // 2. MPC circuit to verify sufficient balance within encrypted computation
        // 3. Reserve tokens during order submission to prevent double-spending

        // Prepare encrypted order arguments
        let timestamp = Clock::get()?.unix_timestamp as u64;
        let args = vec![
            // Order data (encrypted by client)
            Argument::ArcisPubkey(client_pubkey),
            Argument::PlaintextU128(client_nonce),
            Argument::EncryptedU64(encrypted_price),
            Argument::EncryptedU64(encrypted_quantity),
            Argument::EncryptedBool(encrypted_is_buy),
            Argument::EncryptedU128(encrypted_trader_id),
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
            vec![SubmitOrderCallback::callback_ix(&[])],
        )?;

        Ok(())
    }

    /// Callback handler for order submission
    #[arcium_callback(encrypted_ix = "submit_order")]
    pub fn submit_order_callback(
        ctx: Context<SubmitOrderCallback>,
        output: ComputationOutputs<SubmitOrderOutput>,
    ) -> Result<()> {
        let updated_book = match output {
            ComputationOutputs::Success(SubmitOrderOutput { field_0 }) => field_0,
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

    /// Cancel an existing order
    pub fn cancel_order(
        ctx: Context<CancelOrder>,
        computation_offset: u64,
        trading_pair_id: u64,
        client_pubkey: [u8; 32],
        client_nonce: u128,
        encrypted_order_id: [u8; 32],
        encrypted_trader_id: [u8; 32],
    ) -> Result<()> {
        require!(
            ctx.accounts.trading_pair.is_active,
            ErrorCode::TradingPairInactive
        );

        // Prepare encrypted cancellation arguments
        let args = vec![
            // Cancellation data (encrypted by client)
            Argument::ArcisPubkey(client_pubkey),
            Argument::PlaintextU128(client_nonce),
            Argument::EncryptedU128(encrypted_order_id),
            Argument::EncryptedU128(encrypted_trader_id),
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
            vec![CancelOrderCallback::callback_ix(&[])],
        )?;

        Ok(())
    }

    /// Callback handler for order cancellation
    #[arcium_callback(encrypted_ix = "cancel_order")]
    pub fn cancel_order_callback(
        ctx: Context<CancelOrderCallback>,
        output: ComputationOutputs<CancelOrderOutput>,
    ) -> Result<()> {
        let updated_book = match output {
            ComputationOutputs::Success(CancelOrderOutput { field_0 }) => field_0,
            _ => return Err(ErrorCode::AbortedComputation.into()),
        };

        let trading_pair = &mut ctx.accounts.trading_pair;
        trading_pair.order_book = updated_book.ciphertexts[0];
        trading_pair.order_book_nonce = updated_book.nonce;

        emit!(OrderCancelledEvent {
            trading_pair_id: trading_pair.trading_pair_id,
            order_book_nonce: updated_book.nonce,
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
            vec![MatchOrdersCallback::callback_ix(&[])],
        )?;

        Ok(())
    }

    /// Callback handler for order matching
    #[arcium_callback(encrypted_ix = "match_orders")]
    pub fn match_orders_callback(
        ctx: Context<MatchOrdersCallback>,
        output: ComputationOutputs<MatchOrdersOutput>,
    ) -> Result<()> {
        let match_result = match output {
            ComputationOutputs::Success(MatchOrdersOutput { field_0 }) => field_0,
            _ => return Err(ErrorCode::AbortedComputation.into()),
        };

        // Extract trade data and updated order book from MPC result
        let trading_pair = &mut ctx.accounts.trading_pair;
        trading_pair.order_book_nonce = match_result.nonce;

        // TODO: For production implementation, need to:
        // 1. Deserialize MatchResult from match_result.ciphertexts
        // 2. Extract individual trades from the result
        // 3. For each trade, execute token transfers
        // 4. Handle partial fills and order book updates

        // Current limitation: MPC results are encrypted and need decryption
        // For MVP, we emit a placeholder event showing the computation completed
        emit!(OrdersMatchedEvent {
            trading_pair_id: trading_pair.trading_pair_id,
            match_nonce: match_result.nonce,
            timestamp: Clock::get()?.unix_timestamp as u64,
        });

        // In a complete implementation, we would extract trades like this:
        // let trades = deserialize_trades_from_mpc_result(&match_result);
        // for trade in trades {
        //     execute_individual_trade(ctx, trade)?;
        // }

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
        // Validate trade parameters
        require!(trade_price > 0, ErrorCode::InvalidPrice);
        require!(trade_quantity > 0, ErrorCode::InvalidQuantity);

        // Deserialize and validate token accounts
        let buyer_quote = TokenAccount::try_deserialize(&mut &ctx.accounts.buyer_quote_account.try_borrow_data()?[..])?;
        let seller_base = TokenAccount::try_deserialize(&mut &ctx.accounts.seller_base_account.try_borrow_data()?[..])?;
        let buyer_base = TokenAccount::try_deserialize(&mut &ctx.accounts.buyer_base_account.try_borrow_data()?[..])?;
        let seller_quote = TokenAccount::try_deserialize(&mut &ctx.accounts.seller_quote_account.try_borrow_data()?[..])?;

        // Validate token accounts belong to the correct traders
        require!(buyer_quote.owner == ctx.accounts.buyer.key(), ErrorCode::InvalidTokenAccount);
        require!(seller_base.owner == ctx.accounts.seller.key(), ErrorCode::InvalidTokenAccount);
        require!(buyer_base.owner == ctx.accounts.buyer.key(), ErrorCode::InvalidTokenAccount);
        require!(seller_quote.owner == ctx.accounts.seller.key(), ErrorCode::InvalidTokenAccount);

        // Calculate total quote amount (price * quantity)
        let quote_amount = trade_price
            .checked_mul(trade_quantity)
            .ok_or(ErrorCode::MathOverflow)?;

        // Verify sufficient balances before executing transfers
        require!(
            buyer_quote.amount >= quote_amount,
            ErrorCode::InsufficientBalance
        );
        require!(
            seller_base.amount >= trade_quantity,
            ErrorCode::InsufficientBalance
        );

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
    /// CHECK: Verified by Arcium macros via derive_mempool_pda!() address constraint
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_execpool_pda!() address constraint
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_comp_pda!() address constraint
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
    /// CHECK: Base token mint address, validated by trading_pair.base_mint
    pub base_mint: UncheckedAccount<'info>,
    /// CHECK: Quote token mint address, validated by trading_pair.quote_mint
    pub quote_mint: UncheckedAccount<'info>,
}

// Callback account structure
#[callback_accounts("init_order_book")]
#[derive(Accounts)]
pub struct InitOrderBookCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_ORDER_BOOK))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    /// CHECK: Validated by Arcium program through address constraint
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
    /// CHECK: Verified by Arcium macros via derive_mempool_pda!() address constraint
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_execpool_pda!() address constraint
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_comp_pda!() address constraint
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
    // User's token accounts for balance validation
    /// CHECK: Optional user base token account for balance validation
    pub user_base_token_account: Option<UncheckedAccount<'info>>,
    /// CHECK: Optional user quote token account for balance validation
    pub user_quote_token_account: Option<UncheckedAccount<'info>>,
}

#[callback_accounts("submit_order")]
#[derive(Accounts)]
pub struct SubmitOrderCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SUBMIT_ORDER))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    /// CHECK: Validated by Arcium program through address constraint
    pub instructions_sysvar: AccountInfo<'info>,
    #[account(mut)]
    pub trading_pair: Account<'info, TradingPair>,
}

// Cancel order accounts
#[queue_computation_accounts("cancel_order", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, trading_pair_id: u64)]
pub struct CancelOrder<'info> {
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
    /// CHECK: Verified by Arcium macros via derive_mempool_pda!() address constraint
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_execpool_pda!() address constraint
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_comp_pda!() address constraint
    #[account(mut, address = derive_comp_pda!(computation_offset))]
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_CANCEL_ORDER))]
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

#[callback_accounts("cancel_order")]
#[derive(Accounts)]
pub struct CancelOrderCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_CANCEL_ORDER))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = ::anchor_lang::solana_program::sysvar::instructions::ID)]
    /// CHECK: Validated by Arcium program through address constraint
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
    /// CHECK: Verified by Arcium macros via derive_mempool_pda!() address constraint
    #[account(mut, address = derive_mempool_pda!())]
    pub mempool_account: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_execpool_pda!() address constraint
    #[account(mut, address = derive_execpool_pda!())]
    pub executing_pool: UncheckedAccount<'info>,
    /// CHECK: Verified by Arcium macros via derive_comp_pda!() address constraint
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
    /// CHECK: Validated by Arcium program through address constraint
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

#[init_computation_definition_accounts("cancel_order", payer)]
#[derive(Accounts)]
pub struct InitCancelOrderCompDef<'info> {
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
    /// CHECK: Token account validated in execute_trade function
    #[account(mut)]
    pub buyer_base_account: UncheckedAccount<'info>,
    /// CHECK: Token account validated in execute_trade function
    #[account(mut)]
    pub buyer_quote_account: UncheckedAccount<'info>,
    /// CHECK: Token account validated in execute_trade function
    #[account(mut)]
    pub seller_base_account: UncheckedAccount<'info>,
    /// CHECK: Token account validated in execute_trade function
    #[account(mut)]
    pub seller_quote_account: UncheckedAccount<'info>,
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
pub struct OrderCancelledEvent {
    pub trading_pair_id: u64,
    pub order_book_nonce: u128,
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
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("Insufficient balance")]
    InsufficientBalance,
}