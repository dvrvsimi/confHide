# confHide MVP Implementation Todo List

## **MVP Scope (Simplified)**
Focus on **core private order book functionality** - users can submit encrypted orders that get matched privately and executed publicly. This demonstrates the key innovation of MEV-protected trading through Arcium's MPC.

## **End-to-End Implementation Tasks**

### **1. Project Setup** ✅
- [x] Initialize confHide project using Arcium CLI
- [x] Move todo.md to proper project location
- [ ] Examine generated project structure and dependencies

### **2. Core MPC Instructions (encrypted-ixs)**
- [ ] Create basic Order and OrderBook structs in encrypted-ixs
  - Simple Order struct: price, quantity, side (buy/sell), trader_id
  - OrderBook struct: arrays for buy_orders and sell_orders
  - Keep MVP simple - no complex order types initially

- [ ] Implement submit_order MPC instruction
  - Takes encrypted order from user
  - Adds to encrypted order book
  - Returns updated encrypted order book state

- [ ] Implement simple order matching algorithm in MPC
  - Basic price-time priority matching
  - Match highest buy with lowest sell when prices cross
  - Return list of matched trades and updated order book
  - Keep algorithm simple for MVP - just basic matching

### **3. Solana Program Implementation**
- [ ] Create Solana program structure with basic accounts
  - TradingPair account (stores encrypted order book state)
  - User account (tracks balances and open orders)
  - Trade execution accounts

- [ ] Implement order submission endpoint in Solana program
  - Validate user has sufficient balance
  - Queue MPC computation for order insertion
  - Handle user authentication and input validation

- [ ] Implement order matching callback handler in Solana program
  - Process MPC results (matched trades)
  - Execute token transfers for successful matches
  - Update user balances and positions
  - Emit trade execution events

- [ ] Add basic token transfer logic for trade execution
  - Simple SOL/USDC trading pair for MVP
  - Transfer tokens between matched traders
  - Update account balances after execution

### **4. Comprehensive Testing (Critical for MVP)**
- [ ] Write comprehensive unit tests for MPC instructions
  - Test order insertion with various scenarios
  - Test matching algorithm with crossing prices
  - Test edge cases: empty book, no matches, partial fills
  - Test encrypted data handling and privacy preservation

- [ ] Write integration tests for Solana program endpoints
  - Test complete order submission flow
  - Test callback processing with mock MPC results
  - Test error handling and edge cases
  - Test account state management

- [ ] Test end-to-end order flow from submission to execution
  - Submit orders from multiple mock users
  - Verify MPC matching produces correct results
  - Verify token transfers execute correctly
  - Verify privacy is maintained throughout

- [ ] Deploy and test on Solana devnet with sample orders
  - Deploy both encrypted-ixs and Solana program
  - Create test scenarios with real encrypted orders
  - Measure performance and latency
  - Validate privacy guarantees in live environment

## **MVP Success Criteria**
- **Core Functionality**: Users can submit buy/sell orders that get matched privately
- **Privacy**: Order details remain encrypted until execution
- **Performance**: Sub-10 second order matching (reasonable for MVP)
- **Security**: No order leakage, proper authentication
- **Testing**: 90%+ test coverage with comprehensive edge case handling
- **Demo Ready**: Working devnet deployment with sample trades

## **Production-Level Improvements (Post-MVP)**
- [ ] **Advanced Order Matching Algorithm**
  - Implement price-time priority (FIFO at same price level)
  - Add pro-rata allocation for large orders at same price
  - Implement self-trade prevention (same trader ID)
  - Add minimum order size enforcement
  - Implement maximum trade limits per matching round
  - Order book sorting by best price, then timestamp
  - Support for order expiration timestamps
  - Iceberg orders (hidden quantity)
  - Time-in-force rules (IOC, FOK, GTC)

- [ ] **Scalability & Performance**
  - Increase order book capacity beyond 10 orders per side
  - Optimize matching algorithm complexity
  - Batch processing for high-volume periods
  - Multi-level order book with price levels

- [ ] **Risk Management**
  - Position limits per trader
  - Circuit breakers for extreme price movements
  - Pre-trade risk checks (margin, collateral)
  - Post-trade settlement risk monitoring

## **Deliberately Excluded from MVP**
- UI/Frontend (focus on backend functionality)
- Advanced order types (limit/stop/conditional)
- Leverage and perpetuals
- Token launch/bonding curves
- Multiple trading pairs (start with SOL/USDC only)
- Complex fee structures
- Liquidity provider features

## **Generated Project Structure**
```
conf_hide/
├── Anchor.toml                  # Anchor configuration
├── Arcium.toml                  # Arcium configuration
├── Cargo.toml                   # Workspace configuration
├── todo.md                      # This file
├── encrypted-ixs/               # MPC instructions
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs              # Order matching logic in MPC
├── programs/
│   └── conf_hide/              # Solana program
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs          # State management & callbacks
├── app/                        # Client SDK (future)
├── tests/                      # Integration tests
└── migrations/                 # Deployment scripts
```

### **Core Data Structures**
```rust
// In encrypted-ixs/src/lib.rs
pub struct Order {
    price: u64,        // Encrypted price
    quantity: u64,     // Encrypted quantity
    side: bool,        // Buy = true, Sell = false
    trader_id: u128,   // Anonymous trader ID
}

pub struct OrderBook {
    buy_orders: Vec<Order>,    // Encrypted buy side
    sell_orders: Vec<Order>,   // Encrypted sell side
}

// Key MPC Instructions
#[instruction]
pub fn submit_order(order: Enc<Shared, Order>, book: Enc<Mxe, OrderBook>) -> Enc<Mxe, OrderBook>

#[instruction]
pub fn match_orders(book: Enc<Mxe, OrderBook>) -> (Vec<Trade>, Enc<Mxe, OrderBook>)
```

This streamlined approach focuses on proving the core innovation - **private order matching with public settlement** - which is the fundamental breakthrough needed for MEV-protected trading.