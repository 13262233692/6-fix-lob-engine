use std::collections::HashMap;
use crate::lob::orderbook::{self, Side, LimitOrderBook};
use crate::fix::scanner;
use crate::fix::scanner::FixMessage;

pub struct MatchingEngine {
    books: HashMap<[u8; 8], LimitOrderBook>,
    next_order_id: u64,
    next_exec_id: u64,
}

fn symbol_key(sym: &[u8]) -> [u8; 8] {
    let mut key = [0u8; 8];
    let len = sym.len().min(8);
    key[..len].copy_from_slice(&sym[..len]);
    key
}

impl MatchingEngine {
    pub fn new() -> Self {
        MatchingEngine {
            books: HashMap::new(),
            next_order_id: 1,
            next_exec_id: 1,
        }
    }

    #[allow(dead_code)]
    fn get_or_create_book(&mut self, symbol: &[u8]) -> &mut LimitOrderBook {
        let key = symbol_key(symbol);
        self.books.entry(key).or_insert_with(|| LimitOrderBook::new())
    }

    pub fn process_fix_message(&mut self, msg: &FixMessage) {
        let msg_type = match msg.msg_type {
            Some(mt) => mt,
            None => return,
        };

        if msg_type != b"D" {
            return;
        }

        let side = match msg.side {
            Some(s) if s.len() == 1 => match Side::from_byte(s[0]) {
                Some(s) => s,
                None => return,
            },
            _ => return,
        };

        let symbol = match msg.symbol {
            Some(s) => s,
            None => return,
        };

        let price_raw = match msg.price {
            Some(p) => p,
            None => return,
        };
        let price = orderbook::parse_price(price_raw);

        let qty = match msg.order_qty {
            Some(q) => match scanner::parse_u64(q) {
                Some(v) => v,
                None => return,
            },
            None => return,
        };

        let clord_id = match msg.clord_id {
            Some(c) => c,
            None => b"UNKNOWN",
        };

        let key = symbol_key(symbol);
        let (next_oid, next_eid) = (self.next_order_id, self.next_exec_id);
        let book = self.books.entry(key).or_insert_with(|| LimitOrderBook::new());
        book.next_order_id = next_oid;
        book.next_exec_id = next_eid;

        let fills = book.submit_order(clord_id, symbol, side, price, qty);
        self.next_order_id = book.next_order_id;
        self.next_exec_id = book.next_exec_id;

        if !fills.is_empty() {
            LimitOrderBook::process_fills(&fills);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lob::orderbook::parse_price;

    #[test]
    fn test_price_parsing() {
        assert_eq!(parse_price(b"100.50"), 100_5000_0000i64);
        assert_eq!(parse_price(b"100"), 100_0000_0000i64);
        assert_eq!(parse_price(b"0.01"), 100_0000i64);
        assert_eq!(parse_price(b"1500.25"), 1500_2500_0000i64);
    }

    #[test]
    fn test_single_fill() {
        let mut engine = MatchingEngine::new();
        let sell_msg = FixMessage::scan(b"35=D\x0154=2\x0155=AAPL\x0144=100.50\x0138=100\x0111=S001\x01");
        engine.process_fix_message(&sell_msg);
        let buy_msg = FixMessage::scan(b"35=D\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x0111=B001\x01");
        engine.process_fix_message(&buy_msg);
    }

    #[test]
    fn test_partial_fill() {
        let mut engine = MatchingEngine::new();
        let sell_msg = FixMessage::scan(b"35=D\x0154=2\x0155=AAPL\x0144=100.50\x0138=100\x0111=S001\x01");
        engine.process_fix_message(&sell_msg);
        let buy_msg = FixMessage::scan(b"35=D\x0154=1\x0155=AAPL\x0144=100.50\x0138=50\x0111=B001\x01");
        engine.process_fix_message(&buy_msg);
        let book = engine.books.get(&symbol_key(b"AAPL")).unwrap();
        assert_eq!(book.asks.arena.get(book.asks.min_key().unwrap()).total_qty, 50);
    }

    #[test]
    fn test_multi_symbol_isolation() {
        let mut engine = MatchingEngine::new();
        let sell_aapl = FixMessage::scan(b"35=D\x0154=2\x0155=AAPL\x0144=100\x0138=100\x0111=S001\x01");
        engine.process_fix_message(&sell_aapl);
        let sell_goog = FixMessage::scan(b"35=D\x0154=2\x0155=GOOG\x0144=1500\x0138=200\x0111=S002\x01");
        engine.process_fix_message(&sell_goog);

        let buy_goog = FixMessage::scan(b"35=D\x0154=1\x0155=GOOG\x0144=1500\x0138=200\x0111=B001\x01");
        engine.process_fix_message(&buy_goog);

        let aapl_book = engine.books.get(&symbol_key(b"AAPL")).unwrap();
        assert!(!aapl_book.asks.is_empty());

        let goog_book = engine.books.get(&symbol_key(b"GOOG")).unwrap();
        assert!(goog_book.asks.is_empty());
    }

    #[test]
    fn test_price_time_priority() {
        let mut book = LimitOrderBook::new();
        book.submit_order(b"S1", b"AAPL", Side::Sell, 100_0000_0000, 100);
        book.submit_order(b"S2", b"AAPL", Side::Sell, 100_0000_0000, 200);

        let fills = book.submit_order(b"B1", b"AAPL", Side::Buy, 100_0000_0000, 150);
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].fill_qty, 100);
        assert_eq!(fills[0].maker_leaves, 0);
        assert_eq!(fills[1].fill_qty, 50);
        assert_eq!(fills[1].maker_leaves, 150);
    }

    #[test]
    fn test_no_cross_no_fill() {
        let mut book = LimitOrderBook::new();
        book.submit_order(b"S1", b"AAPL", Side::Sell, 105_0000_0000, 100);
        let fills = book.submit_order(b"B1", b"AAPL", Side::Buy, 100_0000_0000, 100);
        assert!(fills.is_empty());
        assert!(!book.bids.is_empty());
        assert!(!book.asks.is_empty());
    }

    #[test]
    fn test_aggressive_buy_matches_multiple_levels() {
        let mut book = LimitOrderBook::new();
        book.submit_order(b"S1", b"AAPL", Side::Sell, 100_0000_0000, 50);
        book.submit_order(b"S2", b"AAPL", Side::Sell, 101_0000_0000, 50);

        let fills = book.submit_order(b"B1", b"AAPL", Side::Buy, 101_0000_0000, 100);
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].price, 100_0000_0000);
        assert_eq!(fills[0].fill_qty, 50);
        assert_eq!(fills[1].price, 101_0000_0000);
        assert_eq!(fills[1].fill_qty, 50);
    }
}
