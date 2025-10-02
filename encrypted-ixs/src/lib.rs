use arcis_imports::*;

#[encrypted]
mod circuits {
    use arcis_imports::*;

    #[derive(Copy, Clone)]
    pub struct Order {
        pub order_id: u128,
        pub price: u64,
        pub quantity: u64,
        pub side: bool,
        pub trader_id: u128,
        pub timestamp: u64,
    }

    pub struct OrderBook {
        pub buy_orders: [Order; 10],
        pub buy_count: u8,
        pub sell_orders: [Order; 10],
        pub sell_count: u8,
        pub next_order_id: u128,
    }

    #[derive(Copy, Clone)]
    pub struct Trade {
        pub buyer_id: u128,
        pub seller_id: u128,
        pub price: u64,
        pub quantity: u64,
        pub timestamp: u64,
    }

    pub struct MatchResult {
        pub trades: [Trade; 5],
        pub trade_count: u8,
        pub order_book: OrderBook,
    }

    impl Order {
        pub fn new() -> Self {
            Order {
                order_id: 0,
                price: 0,
                quantity: 0,
                side: false,
                trader_id: 0,
                timestamp: 0,
            }
        }
    }

    impl Trade {
        pub fn new() -> Self {
            Trade {
                buyer_id: 0,
                seller_id: 0,
                price: 0,
                quantity: 0,
                timestamp: 0,
            }
        }
    }

    impl OrderBook {
        pub fn new() -> Self {
            OrderBook {
                buy_orders: [Order::new(); 10],
                buy_count: 0,
                sell_orders: [Order::new(); 10],
                sell_count: 0,
                next_order_id: 1,
            }
        }

        fn add_buy_order(&mut self, mut order: Order) -> bool {
            let can_add = self.buy_count < 10;
            if can_add {
                order.order_id = self.next_order_id;
                self.next_order_id += 1;
                let idx = self.buy_count as usize;
                self.buy_orders[idx] = order;
                self.buy_count += 1;
            }
            can_add
        }

        fn add_sell_order(&mut self, mut order: Order) -> bool {
            let can_add = self.sell_count < 10;
            if can_add {
                order.order_id = self.next_order_id;
                self.next_order_id += 1;
                let idx = self.sell_count as usize;
                self.sell_orders[idx] = order;
                self.sell_count += 1;
            }
            can_add
        }

        fn cancel_order(&mut self, order_id: u128, trader_id: u128) -> bool {
            let mut found = false;

            // Try to find and remove the order from buy orders
            // Must use constant loop bounds for MPC compilation
            for i in 0..10 {
                let idx = i as usize;
                let order_exists = i < self.buy_count;
                let is_target = self.buy_orders[idx].order_id == order_id && self.buy_orders[idx].trader_id == trader_id;

                if order_exists && is_target && !found {
                    // Shift remaining orders left to fill the gap
                    for j in i..9 {
                        let j_idx = j as usize;
                        self.buy_orders[j_idx] = self.buy_orders[j_idx + 1];
                    }
                    self.buy_count -= 1;
                    found = true;
                }
            }

            // Try to find and remove the order from sell orders if not found in buy orders
            for i in 0..10 {
                let idx = i as usize;
                let order_exists = i < self.sell_count;
                let is_target = self.sell_orders[idx].order_id == order_id && self.sell_orders[idx].trader_id == trader_id;

                if order_exists && is_target && !found {
                    // Shift remaining orders left to fill the gap
                    for j in i..9 {
                        let j_idx = j as usize;
                        self.sell_orders[j_idx] = self.sell_orders[j_idx + 1];
                    }
                    self.sell_count -= 1;
                    found = true;
                }
            }

            found
        }
    }

    /// Initialize an empty order book
    #[instruction]
    pub fn init_order_book(mxe: Mxe) -> Enc<Mxe, OrderBook> {
        let order_book = OrderBook::new();
        mxe.from_arcis(order_book)
    }

    #[instruction]
    pub fn submit_order(
        order_ctxt: Enc<Shared, Order>,
        book_ctxt: Enc<Mxe, OrderBook>,
    ) -> Enc<Mxe, OrderBook> {
        let order = order_ctxt.to_arcis();
        let mut book = book_ctxt.to_arcis();

        let _success = if order.side {
            book.add_buy_order(order)
        } else {
            book.add_sell_order(order)
        };

        book_ctxt.owner.from_arcis(book)
    }

    #[instruction]
    pub fn cancel_order(
        order_id: Enc<Shared, u128>,
        trader_id: Enc<Shared, u128>,
        book_ctxt: Enc<Mxe, OrderBook>,
    ) -> Enc<Mxe, OrderBook> {
        let order_id_val = order_id.to_arcis();
        let trader_id_val = trader_id.to_arcis();
        let mut book = book_ctxt.to_arcis();

        let _cancelled = book.cancel_order(order_id_val, trader_id_val);

        book_ctxt.owner.from_arcis(book)
    }

    /// Helper function to remove filled orders and compact the order arrays
    fn compact_orders(book: &mut OrderBook, buy_filled: &[bool; 10], sell_filled: &[bool; 10]) {
        // Compact buy orders - remove filled orders and shift remaining ones
        let mut write_idx = 0u8;
        for read_idx in 0..10 {
            let should_keep = read_idx < book.buy_count &&
                             !buy_filled[read_idx as usize] &&
                             book.buy_orders[read_idx as usize].quantity > 0;

            if should_keep {
                if write_idx != read_idx {
                    book.buy_orders[write_idx as usize] = book.buy_orders[read_idx as usize];
                }
                write_idx += 1;
            }
        }
        book.buy_count = write_idx;

        // Compact sell orders - remove filled orders and shift remaining ones
        write_idx = 0;
        for read_idx in 0..10 {
            let should_keep = read_idx < book.sell_count &&
                             !sell_filled[read_idx as usize] &&
                             book.sell_orders[read_idx as usize].quantity > 0;

            if should_keep {
                if write_idx != read_idx {
                    book.sell_orders[write_idx as usize] = book.sell_orders[read_idx as usize];
                }
                write_idx += 1;
            }
        }
        book.sell_count = write_idx;
    }

    #[instruction]
    pub fn match_orders(
        book_ctxt: Enc<Mxe, OrderBook>,
        timestamp: u64,
    ) -> Enc<Mxe, MatchResult> {
        let mut book = book_ctxt.to_arcis();
        let mut trades = [Trade::new(); 5];
        let mut trade_count = 0u8;

        // Track which orders have been fully filled
        let mut buy_filled = [false; 10];
        let mut sell_filled = [false; 10];

        // Iterate through buy orders - match each buy against all sells
        for buy_idx in 0..10 {
            let should_process_buy = buy_idx < book.buy_count && trade_count < 5;

            if should_process_buy {
                let mut buy_order = book.buy_orders[buy_idx as usize];
                let buy_is_active = !buy_filled[buy_idx as usize] && buy_order.quantity > 0;

                if buy_is_active {
                    // Find matching sell orders
                    for sell_idx in 0..10 {
                        let should_process_sell = sell_idx < book.sell_count &&
                                                 trade_count < 5 &&
                                                 buy_order.quantity > 0;

                        if should_process_sell {
                            let mut sell_order = book.sell_orders[sell_idx as usize];
                            let sell_is_active = !sell_filled[sell_idx as usize] && sell_order.quantity > 0;

                            // Price match condition: buy price >= sell price
                            let prices_match = buy_order.price >= sell_order.price;

                            if sell_is_active && prices_match {
                                // Determine trade quantity (minimum of buy and sell quantities)
                                let trade_quantity = if buy_order.quantity < sell_order.quantity {
                                    buy_order.quantity
                                } else {
                                    sell_order.quantity
                                };

                                // Use sell price (provides price improvement for buyer)
                                let trade_price = sell_order.price;

                                // Record the trade
                                trades[trade_count as usize] = Trade {
                                    buyer_id: buy_order.trader_id,
                                    seller_id: sell_order.trader_id,
                                    price: trade_price,
                                    quantity: trade_quantity,
                                    timestamp,
                                };
                                trade_count += 1;

                                // Update order quantities after match
                                buy_order.quantity -= trade_quantity;
                                sell_order.quantity -= trade_quantity;

                                // Mark orders as filled if quantity reaches zero
                                if buy_order.quantity == 0 {
                                    buy_filled[buy_idx as usize] = true;
                                }
                                if sell_order.quantity == 0 {
                                    sell_filled[sell_idx as usize] = true;
                                }

                                // Update orders in the book
                                book.buy_orders[buy_idx as usize] = buy_order;
                                book.sell_orders[sell_idx as usize] = sell_order;
                            }
                        }
                    }
                }
            }
        }

        // Remove filled orders from the book and compact arrays
        compact_orders(&mut book, &buy_filled, &sell_filled);

        let result = MatchResult {
            trades,
            trade_count,
            order_book: book,
        };

        book_ctxt.owner.from_arcis(result)
    }
}