use std::io::{self, Write};

const SOH: u8 = 0x01;
const PRICE_SCALE: i64 = 100_000_000;
pub const TICK_SIZE: i64 = 1_000_000;
pub const MAX_SPREAD_TICKS: u32 = 5;

#[derive(Clone, Copy)]
pub enum Side {
    Buy = 1,
    Sell = 2,
}

impl Side {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'1' => Some(Side::Buy),
            b'2' => Some(Side::Sell),
            _ => None,
        }
    }

    pub fn to_fix_byte(self) -> u8 {
        match self {
            Side::Buy => b'1',
            Side::Sell => b'2',
        }
    }
}

#[derive(Clone, Copy)]
pub struct Order {
    pub order_id: u64,
    pub clord_id: [u8; 20],
    pub clord_id_len: u8,
    pub symbol: [u8; 8],
    pub symbol_len: u8,
    pub side: Side,
    pub price: i64,
    pub order_qty: u64,
    pub filled_qty: u64,
    pub timestamp: u64,
    pub next: u32,
    pub prev: u32,
}

impl Order {
    pub fn new() -> Self {
        Order {
            order_id: 0,
            clord_id: [0u8; 20],
            clord_id_len: 0,
            symbol: [0u8; 8],
            symbol_len: 0,
            side: Side::Buy,
            price: 0,
            order_qty: 0,
            filled_qty: 0,
            timestamp: 0,
            next: u32::MAX,
            prev: u32::MAX,
        }
    }

    #[inline(always)]
    pub fn leaves_qty(&self) -> u64 {
        self.order_qty.saturating_sub(self.filled_qty)
    }

    #[inline(always)]
    pub fn clord_id_slice(&self) -> &[u8] {
        &self.clord_id[..self.clord_id_len as usize]
    }

    #[inline(always)]
    pub fn symbol_slice(&self) -> &[u8] {
        &self.symbol[..self.symbol_len as usize]
    }
}

pub const ORDER_ARENA_CAP: usize = 262_144;
pub const LEVEL_ARENA_CAP: usize = 65_536;

pub struct LimitOrderBook {
    pub bids: crate::lob::rbtree::RbTree,
    pub asks: crate::lob::rbtree::RbTree,
    pub order_arena: crate::lob::arena::Arena<Order>,
    pub next_order_id: u64,
    pub next_exec_id: u64,
    pub seq: u64,
}

pub struct Fill {
    pub maker_order_id: u64,
    pub taker_clord_id: [u8; 20],
    pub taker_clord_id_len: u8,
    pub symbol: [u8; 8],
    pub symbol_len: u8,
    pub side: Side,
    pub price: i64,
    pub fill_qty: u64,
    pub maker_leaves: u64,
    pub taker_leaves: u64,
    pub taker_order_qty: u64,
    pub exec_id: u64,
}

fn parse_fix_price(s: &[u8]) -> i64 {
    let mut result: i64 = 0;
    let mut decimal = false;
    let mut scale: i32 = 8;
    for &b in s {
        if b == b'.' {
            decimal = true;
        } else if b >= b'0' && b <= b'9' {
            if decimal {
                if scale > 0 {
                    result = result * 10 + (b - b'0') as i64;
                    scale -= 1;
                }
            } else {
                result = result * 10 + (b - b'0') as i64;
            }
        }
    }
    while scale > 0 {
        result *= 10;
        scale -= 1;
    }
    result
}

pub(crate) fn format_fix_price(price: i64, buf: &mut [u8]) -> usize {
    let integer_part = price / PRICE_SCALE;
    let decimal_part = (price % PRICE_SCALE).unsigned_abs();
    let mut pos = 0;

    if integer_part == 0 {
        buf[pos] = b'0';
        pos += 1;
    } else {
        let mut tmp = [0u8; 20];
        let mut tpos = 0;
        let mut n = integer_part.unsigned_abs();
        while n > 0 {
            tmp[tpos] = (n % 10) as u8 + b'0';
            n /= 10;
            tpos += 1;
        }
        if integer_part < 0 {
            buf[pos] = b'-';
            pos += 1;
        }
        for i in (0..tpos).rev() {
            buf[pos] = tmp[i];
            pos += 1;
        }
    }

    if decimal_part > 0 {
        buf[pos] = b'.';
        pos += 1;
        let mut dp = decimal_part;
        let mut digits = [0u8; 8];
        let mut ndigits: usize = 0;
        let mut scale = 8u32;
        while scale > 0 {
            let divisor = 10u64.pow(scale - 1);
            let digit = (dp / divisor) as u8;
            dp %= divisor;
            digits[ndigits] = digit + b'0';
            ndigits += 1;
            scale -= 1;
        }
        let mut end = ndigits;
        while end > 1 && digits[end - 1] == b'0' {
            end -= 1;
        }
        for i in 0..end {
            buf[pos] = digits[i];
            pos += 1;
        }
    } else {
        buf[pos] = b'.';
        pos += 1;
        buf[pos] = b'0';
        pos += 1;
    }

    pos
}

impl LimitOrderBook {
    pub fn new() -> Self {
        LimitOrderBook {
            bids: crate::lob::rbtree::RbTree::new(LEVEL_ARENA_CAP),
            asks: crate::lob::rbtree::RbTree::new(LEVEL_ARENA_CAP),
            order_arena: crate::lob::arena::Arena::new(ORDER_ARENA_CAP),
            next_order_id: 1,
            next_exec_id: 1,
            seq: 0,
        }
    }

    #[inline(always)]
    pub fn best_bid(&self) -> Option<i64> {
        self.bids.max_key().map(|idx| self.bids.arena.get(idx).key)
    }

    #[inline(always)]
    pub fn best_ask(&self) -> Option<i64> {
        self.asks.min_key().map(|idx| self.asks.arena.get(idx).key)
    }

    pub fn top_of_book(&self) -> (Option<i64>, Option<i64>) {
        (self.best_bid(), self.best_ask())
    }

    pub fn spread_ticks(&self) -> Option<u32> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => {
                if ask > bid {
                    let diff = (ask - bid).unsigned_abs();
                    Some((diff / TICK_SIZE as u64) as u32)
                } else {
                    Some(0)
                }
            }
            _ => None,
        }
    }

    pub fn is_spread_too_wide(&self) -> bool {
        matches!(self.spread_ticks(), Some(s) if s > MAX_SPREAD_TICKS)
    }

    pub fn submit_order(
        &mut self,
        clord_id: &[u8],
        symbol: &[u8],
        side: Side,
        price: i64,
        qty: u64,
    ) -> Vec<Fill> {
        let mut fills = Vec::new();
        let order_id = self.next_order_id;
        self.next_order_id += 1;
        self.seq += 1;
        let timestamp = self.seq;

        let mut remaining = qty;

        match side {
            Side::Buy => {
                while remaining > 0 {
                    let best_ask = self.asks.min_key();
                    match best_ask {
                        Some(ask_node_idx) => {
                            let best_ask_price = self.asks.arena.get(ask_node_idx).key;
                            if price < best_ask_price {
                                break;
                            }
                            let mut head = self.asks.arena.get(ask_node_idx).order_head;
                            while head != u32::MAX && remaining > 0 {
                                let (fill_qty, next_order, prev_order, maker_order_id, maker_leaves) = {
                                    let order = self.order_arena.get_mut(head);
                                    let available = order.leaves_qty();
                                    let fill_qty = available.min(remaining);
                                    order.filled_qty += fill_qty;
                                    remaining -= fill_qty;
                                    (fill_qty, order.next, order.prev, order.order_id, order.leaves_qty())
                                };

                                let fill = Fill {
                                    maker_order_id,
                                    taker_clord_id: {
                                        let mut a = [0u8; 20];
                                        let len = clord_id.len().min(20);
                                        a[..len].copy_from_slice(&clord_id[..len]);
                                        a
                                    },
                                    taker_clord_id_len: clord_id.len().min(20) as u8,
                                    symbol: {
                                        let mut a = [0u8; 8];
                                        let len = symbol.len().min(8);
                                        a[..len].copy_from_slice(&symbol[..len]);
                                        a
                                    },
                                    symbol_len: symbol.len().min(8) as u8,
                                    side: Side::Sell,
                                    price: best_ask_price,
                                    fill_qty,
                                    maker_leaves,
                                    taker_leaves: remaining,
                                    taker_order_qty: qty,
                                    exec_id: self.next_exec_id,
                                };
                                self.next_exec_id += 1;
                                fills.push(fill);

                                if maker_leaves == 0 {
                                    if prev_order != u32::MAX {
                                        self.order_arena.get_mut(prev_order).next = next_order;
                                    } else {
                                        self.asks.arena.get_mut(ask_node_idx).order_head = next_order;
                                    }
                                    if next_order != u32::MAX {
                                        self.order_arena.get_mut(next_order).prev = prev_order;
                                    } else {
                                        self.asks.arena.get_mut(ask_node_idx).order_tail = prev_order;
                                    }
                                    self.asks.arena.get_mut(ask_node_idx).total_qty -= fill_qty;
                                    let dead = head;
                                    head = next_order;
                                    self.order_arena.dealloc(dead);
                                } else {
                                    self.asks.arena.get_mut(ask_node_idx).total_qty -= fill_qty;
                                    head = next_order;
                                }
                            }

                            if self.asks.arena.get(ask_node_idx).order_head == u32::MAX {
                                self.asks.delete(ask_node_idx);
                            }
                        }
                        None => break,
                    }
                }

                if remaining > 0 {
                    self.insert_bid(order_id, clord_id, symbol, side, price, remaining, qty, timestamp);
                }
            }
            Side::Sell => {
                while remaining > 0 {
                    let best_bid = self.bids.max_key();
                    match best_bid {
                        Some(bid_node_idx) => {
                            let best_bid_price = self.bids.arena.get(bid_node_idx).key;
                            if price > best_bid_price {
                                break;
                            }
                            let mut head = self.bids.arena.get(bid_node_idx).order_head;
                            while head != u32::MAX && remaining > 0 {
                                let (fill_qty, next_order, prev_order, maker_order_id, maker_leaves) = {
                                    let order = self.order_arena.get_mut(head);
                                    let available = order.leaves_qty();
                                    let fill_qty = available.min(remaining);
                                    order.filled_qty += fill_qty;
                                    remaining -= fill_qty;
                                    (fill_qty, order.next, order.prev, order.order_id, order.leaves_qty())
                                };

                                let fill = Fill {
                                    maker_order_id,
                                    taker_clord_id: {
                                        let mut a = [0u8; 20];
                                        let len = clord_id.len().min(20);
                                        a[..len].copy_from_slice(&clord_id[..len]);
                                        a
                                    },
                                    taker_clord_id_len: clord_id.len().min(20) as u8,
                                    symbol: {
                                        let mut a = [0u8; 8];
                                        let len = symbol.len().min(8);
                                        a[..len].copy_from_slice(&symbol[..len]);
                                        a
                                    },
                                    symbol_len: symbol.len().min(8) as u8,
                                    side: Side::Buy,
                                    price: best_bid_price,
                                    fill_qty,
                                    maker_leaves,
                                    taker_leaves: remaining,
                                    taker_order_qty: qty,
                                    exec_id: self.next_exec_id,
                                };
                                self.next_exec_id += 1;
                                fills.push(fill);

                                if maker_leaves == 0 {
                                    if prev_order != u32::MAX {
                                        self.order_arena.get_mut(prev_order).next = next_order;
                                    } else {
                                        self.bids.arena.get_mut(bid_node_idx).order_head = next_order;
                                    }
                                    if next_order != u32::MAX {
                                        self.order_arena.get_mut(next_order).prev = prev_order;
                                    } else {
                                        self.bids.arena.get_mut(bid_node_idx).order_tail = prev_order;
                                    }
                                    self.bids.arena.get_mut(bid_node_idx).total_qty -= fill_qty;
                                    let dead = head;
                                    head = next_order;
                                    self.order_arena.dealloc(dead);
                                } else {
                                    self.bids.arena.get_mut(bid_node_idx).total_qty -= fill_qty;
                                    head = next_order;
                                }
                            }

                            if self.bids.arena.get(bid_node_idx).order_head == u32::MAX {
                                self.bids.delete(bid_node_idx);
                            }
                        }
                        None => break,
                    }
                }

                if remaining > 0 {
                    self.insert_ask(order_id, clord_id, symbol, side, price, remaining, qty, timestamp);
                }
            }
        }

        fills
    }

    fn insert_bid(
        &mut self,
        order_id: u64,
        clord_id: &[u8],
        symbol: &[u8],
        side: Side,
        price: i64,
        remaining: u64,
        order_qty: u64,
        timestamp: u64,
    ) {
        let node_idx = match self.bids.find(price) {
            Some(idx) => idx,
            None => self.bids.insert(price).unwrap(),
        };

        let mut order = Order::new();
        order.order_id = order_id;
        let clen = clord_id.len().min(20);
        order.clord_id[..clen].copy_from_slice(&clord_id[..clen]);
        order.clord_id_len = clen as u8;
        let slen = symbol.len().min(8);
        order.symbol[..slen].copy_from_slice(&symbol[..slen]);
        order.symbol_len = slen as u8;
        order.side = side;
        order.price = price;
        order.order_qty = order_qty;
        order.filled_qty = order_qty - remaining;
        order.timestamp = timestamp;
        order.next = u32::MAX;
        order.prev = self.bids.arena.get(node_idx).order_tail;

        let order_idx = self.order_arena.alloc(order).unwrap();

        let tail = self.bids.arena.get(node_idx).order_tail;
        if tail != u32::MAX {
            self.order_arena.get_mut(tail).next = order_idx;
        } else {
            self.bids.arena.get_mut(node_idx).order_head = order_idx;
        }
        self.bids.arena.get_mut(node_idx).order_tail = order_idx;
        self.bids.arena.get_mut(node_idx).total_qty += remaining;
    }

    fn insert_ask(
        &mut self,
        order_id: u64,
        clord_id: &[u8],
        symbol: &[u8],
        side: Side,
        price: i64,
        remaining: u64,
        order_qty: u64,
        timestamp: u64,
    ) {
        let node_idx = match self.asks.find(price) {
            Some(idx) => idx,
            None => self.asks.insert(price).unwrap(),
        };

        let mut order = Order::new();
        order.order_id = order_id;
        let clen = clord_id.len().min(20);
        order.clord_id[..clen].copy_from_slice(&clord_id[..clen]);
        order.clord_id_len = clen as u8;
        let slen = symbol.len().min(8);
        order.symbol[..slen].copy_from_slice(&symbol[..slen]);
        order.symbol_len = slen as u8;
        order.side = side;
        order.price = price;
        order.order_qty = order_qty;
        order.filled_qty = order_qty - remaining;
        order.timestamp = timestamp;
        order.next = u32::MAX;
        order.prev = self.asks.arena.get(node_idx).order_tail;

        let order_idx = self.order_arena.alloc(order).unwrap();

        let tail = self.asks.arena.get(node_idx).order_tail;
        if tail != u32::MAX {
            self.order_arena.get_mut(tail).next = order_idx;
        } else {
            self.asks.arena.get_mut(node_idx).order_head = order_idx;
        }
        self.asks.arena.get_mut(node_idx).order_tail = order_idx;
        self.asks.arena.get_mut(node_idx).total_qty += remaining;
    }

    pub fn format_execution_report(fill: &Fill) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        let body = {
            let mut body_buf = Vec::with_capacity(200);
            Self::append_tag(&mut body_buf, 35, b"8");
            Self::append_tag_u64(&mut body_buf, 37, fill.maker_order_id);
            Self::append_tag_bytes(&mut body_buf, 11, &fill.taker_clord_id[..fill.taker_clord_id_len as usize]);
            Self::append_tag_u64(&mut body_buf, 17, fill.exec_id);
            let exec_type = if fill.taker_leaves == 0 { b"2" } else { b"1" };
            Self::append_tag(&mut body_buf, 150, exec_type);
            Self::append_tag(&mut body_buf, 39, exec_type);
            Self::append_tag_bytes(&mut body_buf, 55, &fill.symbol[..fill.symbol_len as usize]);
            Self::append_tag_byte(&mut body_buf, 54, fill.side.to_fix_byte());
            let mut price_buf = [0u8; 32];
            let price_len = format_fix_price(fill.price, &mut price_buf);
            Self::append_tag_bytes(&mut body_buf, 44, &price_buf[..price_len]);
            Self::append_tag_u64(&mut body_buf, 38, fill.taker_order_qty);
            Self::append_tag_bytes(&mut body_buf, 6, &price_buf[..price_len]);
            let cum_qty = fill.taker_order_qty - fill.taker_leaves;
            Self::append_tag_u64(&mut body_buf, 14, cum_qty);
            Self::append_tag_u64(&mut body_buf, 151, fill.taker_leaves);
            body_buf
        };

        Self::append_tag(&mut buf, 8, b"FIX.4.4");
        let body_len_str = format!("{}", body.len());
        Self::append_tag(&mut buf, 9, body_len_str.as_bytes());
        buf.extend_from_slice(&body);

        let mut checksum: u8 = 0;
        for &b in &buf {
            checksum = checksum.wrapping_add(b);
        }
        let cksum = format!("10={:03}", checksum);
        buf.extend_from_slice(cksum.as_bytes());
        buf.push(SOH);

        buf
    }

    fn append_tag(buf: &mut Vec<u8>, tag: u32, value: &[u8]) {
        let tag_str = format!("{}=", tag);
        buf.extend_from_slice(tag_str.as_bytes());
        buf.extend_from_slice(value);
        buf.push(SOH);
    }

    fn append_tag_u64(buf: &mut Vec<u8>, tag: u32, value: u64) {
        let s = format!("{}={}\x01", tag, value);
        buf.extend_from_slice(s.as_bytes());
    }

    fn append_tag_bytes(buf: &mut Vec<u8>, tag: u32, value: &[u8]) {
        let tag_str = format!("{}=", tag);
        buf.extend_from_slice(tag_str.as_bytes());
        buf.extend_from_slice(value);
        buf.push(SOH);
    }

    fn append_tag_byte(buf: &mut Vec<u8>, tag: u32, value: u8) {
        let s = format!("{}={}\x01", tag, value as char);
        buf.extend_from_slice(s.as_bytes());
    }

    pub fn process_fills(fills: &[Fill]) {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        for fill in fills {
            let report = Self::format_execution_report(fill);
            let _ = lock.write_all(&report);
            let _ = lock.write_all(b"\n");
        }
        let _ = lock.flush();
    }
}

pub fn parse_price(s: &[u8]) -> i64 {
    parse_fix_price(s)
}
