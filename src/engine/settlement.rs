use crate::lob::orderbook::{Fill, Side};

pub struct SettlementReport {
    pub fill_price: i64,
    pub fill_qty: u64,
    pub side: Side,
    pub buyer_order_id: u64,
    pub seller_order_id: u64,
}

pub struct SettlementEngine;

impl SettlementEngine {
    pub fn new() -> Self {
        SettlementEngine
    }

    pub fn settle_fills(&self, fills: &[Fill]) -> Vec<SettlementReport> {
        fills
            .iter()
            .map(|fill| SettlementReport {
                fill_price: fill.price,
                fill_qty: fill.fill_qty,
                side: fill.side,
                buyer_order_id: if matches!(fill.side, Side::Buy) {
                    fill.maker_order_id
                } else {
                    0
                },
                seller_order_id: if matches!(fill.side, Side::Sell) {
                    fill.maker_order_id
                } else {
                    0
                },
            })
            .collect()
    }
}
