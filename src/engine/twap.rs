use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use rand::Rng;
use rand::rngs::StdRng;
use rand::{SeedableRng, distributions::Distribution};

use crate::lob::orderbook::{LimitOrderBook, Side, TICK_SIZE, MAX_SPREAD_TICKS, parse_price, format_fix_price};

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

pub struct TwapConfig {
    pub symbol: String,
    pub symbol_bytes: Vec<u8>,
    pub total_qty: u64,
    pub duration_secs: u64,
    pub side: Side,
    pub tick_size: i64,
    pub max_spread_ticks: u32,
    pub seed: u64,
}

pub struct TwapScheduler {
    config: TwapConfig,
    books: HashMap<[u8; 8], LimitOrderBook>,
    child_orders: Vec<TwapChildOrder>,
    rng: StdRng,
    sent_count: usize,
    paused: bool,
    override_forced: bool,
    next_order_id: u64,
    next_exec_id: u64,
}

#[derive(Clone, Debug)]
pub struct TwapChildOrder {
    pub qty: u64,
    pub scheduled_offset: Duration,
    pub price: i64,
    pub clord_id: String,
    pub sent: bool,
}

struct JitterDistribution;

impl Distribution<f64> for JitterDistribution {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        rng.gen_range(-1.0..=1.0)
    }
}

fn symbol_key(sym: &[u8]) -> [u8; 8] {
    let mut key = [0u8; 8];
    let len = sym.len().min(8);
    key[..len].copy_from_slice(&sym[..len]);
    key
}

fn print_banner() {
    println!("{}", MAGENTA);
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║       ████████╗██╗    ██╗ █████╗ ██████╗     ███████╗███╗   ██╗      ║");
    println!("║       ╚══██╔══╝██║    ██║██╔══██╗██╔══██╗    ██╔════╝████╗  ██║      ║");
    println!("║          ██║   ██║ █╗ ██║███████║██████╔╝    █████╗  ██╔██╗ ██║      ║");
    println!("║          ██║   ██║███╗██║██╔══██║██╔═══╝     ██╔══╝  ██║╚██╗██║      ║");
    println!("║          ██║   ╚███╔███╔╝██║  ██║██║         ███████╗██║ ╚████║      ║");
    println!("║          ╚═╝    ╚══╝╚══╝ ╚═╝  ╚═╝╚═╝         ╚══════╝╚═╝  ╚═══╝      ║");
    println!("║                    TIME-WEIGHTED AVERAGE PRICE ENGINE                ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!("{}", RESET);
}

fn print_alert(
    symbol: &str,
    bid: Option<i64>,
    ask: Option<i64>,
    spread_ticks: Option<u32>,
    paused_qty: u64,
    max_spread_ticks: u32,
) {
    eprintln!();
    eprintln!("{}", RED);
    eprintln!("{}", BOLD);
    eprintln!("╔══════════════════════════════════════════════════════════════════════╗");
    eprintln!("║  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ║");
    eprintln!("║  ██╔═════════════════════════════════════════════════════════════██  ║");
    eprintln!("║  ██║  ⚠⚠⚠  EXTREME LIQUIDITY VACUUM DETECTED  ⚠⚠⚠               ██  ║");
    eprintln!("║  ██║  ─────────────────────────────────────────────────────────  ██  ║");
    eprintln!("║  ██║  SYMBOL: {:<52} ██  ║", symbol);
    match (bid, ask) {
        (Some(b), Some(a)) => {
            let mut bid_buf = [0u8; 32];
            let mut ask_buf = [0u8; 32];
            let bl = format_fix_price(b, &mut bid_buf);
            let al = format_fix_price(a, &mut ask_buf);
            eprintln!("{}  ║  ██║  BEST BID: {:<46}  ██  ║{}", RED, std::str::from_utf8(&bid_buf[..bl]).unwrap_or("N/A"), RESET);
            eprintln!("{}  ║  ██║  BEST ASK: {:<46}  ██  ║{}", RED, std::str::from_utf8(&ask_buf[..al]).unwrap_or("N/A"), RESET);
        }
        _ => {
            eprintln!("║  ██║  BEST BID: N/A                                                   ██  ║");
            eprintln!("║  ██║  BEST ASK: N/A                                                   ██  ║");
        }
    }
    match spread_ticks {
        Some(s) => {
            eprintln!("{}  ║  ██║  SPREAD: {} TICKS (MAX ALLOWED: {} TICKS)                 ██  ║{}", 
                RED, s, max_spread_ticks, RESET);
        }
        None => {
            eprintln!("║  ██║  SPREAD: N/A                                                    ██  ║");
        }
    }
    eprintln!("║  ██║  PAUSED QTY: {:<48}  ██  ║", paused_qty);
    eprintln!("║  ██║                                                     █████╗    ██  ║");
    eprintln!("║  ██║  ALL CHILD ORDERS HALTED                        ███╔══██╗   ██  ║");
    eprintln!("║  ██║  PENDING OPERATOR OVERRIDE...                   █████╔═╝   ██  ║");
    eprintln!("║  ██║                                                  ██╔══██╗   ██  ║");
    eprintln!("║  ██║  TYPE 'override-resume' TO UNLOCK               ██████╔╝   ██  ║");
    eprintln!("║  ██╚═══════════════════════════════════════════════════════════════██  ║");
    eprintln!("║  ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀  ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════════╝");
    eprintln!("{}", RESET);
    eprintln!();
}

fn print_resume() {
    eprintln!();
    eprintln!("{}", GREEN);
    eprintln!("{}", BOLD);
    eprintln!("╔══════════════════════════════════════════════════════════════════════╗");
    eprintln!("║  ✅✅✅  OPERATOR OVERRIDE RECEIVED - RESUMING EXECUTION  ✅✅✅    ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════════╝");
    eprintln!("{}", RESET);
    eprintln!();
}

fn print_child_order_sent(idx: usize, total: usize, qty: u64, price: i64, side: Side) {
    let mut price_buf = [0u8; 32];
    let pl = format_fix_price(price, &mut price_buf);
    let price_str = std::str::from_utf8(&price_buf[..pl]).unwrap_or("N/A");
    let side_char = if matches!(side, Side::Buy) { "B" } else { "S" };
    println!(
        "{}[TWAP]{} [{}/{}] Sending child order: side={} qty={:<8} price={:<12}",
        CYAN, RESET, idx + 1, total, side_char, qty, price_str
    );
}

fn print_summary(config: &TwapConfig, sent: usize, total: usize) {
    println!();
    println!("{}", GREEN);
    println!("{}", BOLD);
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                      TWAP EXECUTION COMPLETE                          ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!("║  Symbol:    {:<56} ║", config.symbol);
    println!("║  Side:      {:<56} ║", if matches!(config.side, Side::Buy) { "BUY" } else { "SELL" });
    println!("║  Total Qty: {:<56} ║", config.total_qty);
    println!("║  Duration:  {:<56} ║", format!("{} secs", config.duration_secs));
    println!("║  Child Ord: {:<56} ║", format!("{} sent / {} scheduled", sent, total));
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!("{}", RESET);
    println!();
}

impl TwapScheduler {
    pub fn new(config: TwapConfig) -> Self {
        let mut books = HashMap::new();
        books.insert(symbol_key(&config.symbol_bytes), LimitOrderBook::new());

        let rng = StdRng::seed_from_u64(config.seed);

        TwapScheduler {
            config,
            books,
            child_orders: Vec::new(),
            rng,
            sent_count: 0,
            paused: false,
            override_forced: false,
            next_order_id: 1,
            next_exec_id: 1,
        }
    }

    pub fn generate_schedule(&mut self) {
        let duration = Duration::from_secs(self.config.duration_secs);
        let num_orders = (self.config.duration_secs as f64 * 5.0) as usize;
        let qty_per_order = self.config.total_qty / num_orders.max(1) as u64;
        let remainder = self.config.total_qty % num_orders.max(1) as u64;

        let interval = duration.as_secs_f64() / num_orders.max(1) as f64;
        let jitter_scale = interval * 0.1;

        let mut accumulated_qty = 0u64;

        for i in 0..num_orders {
            let jitter: f64 = JitterDistribution.sample(&mut self.rng);
            let offset_secs = (i as f64) * interval + jitter * jitter_scale;
            let offset_secs = offset_secs.max(0.0).min(duration.as_secs_f64());

            let mut qty = qty_per_order;
            if (i as u64) < remainder {
                qty += 1;
            }
            accumulated_qty += qty;

            let price_jitter: f64 = JitterDistribution.sample(&mut self.rng);
            let price_tick_jitter = (price_jitter * 2.0).round() as i64;
            let base_price = 100 * TICK_SIZE * 100;
            let price = base_price + price_tick_jitter * TICK_SIZE;

            let clord_id = format!("TWAP-{:08}-{:04}", self.config.seed, i);

            self.child_orders.push(TwapChildOrder {
                qty,
                scheduled_offset: Duration::from_secs_f64(offset_secs),
                price,
                clord_id,
                sent: false,
            });
        }

        if accumulated_qty < self.config.total_qty {
            let diff = self.config.total_qty - accumulated_qty;
            if let Some(last) = self.child_orders.last_mut() {
                last.qty += diff;
            }
        }

        self.child_orders.sort_by_key(|o| o.scheduled_offset);

        eprintln!(
            "{}[TWAP]{} Generated {} child orders for {} qty over {} secs",
            YELLOW, RESET, self.child_orders.len(), self.config.total_qty, self.config.duration_secs
        );
    }

    fn get_book(&self) -> &LimitOrderBook {
        self.books.get(&symbol_key(&self.config.symbol_bytes)).unwrap()
    }

    fn get_book_mut(&mut self) -> &mut LimitOrderBook {
        self.books.get_mut(&symbol_key(&self.config.symbol_bytes)).unwrap()
    }

    pub fn seed_book(&mut self, seed_bid: i64, seed_ask: i64) {
        let sym_bytes = self.config.symbol_bytes.clone();
        let mut next_oid = self.next_order_id;
        let mut next_eid = self.next_exec_id;

        let book = self.get_book_mut();
        book.next_order_id = next_oid;
        book.next_exec_id = next_eid;

        book.submit_order(b"SEED-S1", &sym_bytes, Side::Sell, seed_ask, 10000);
        book.submit_order(b"SEED-B1", &sym_bytes, Side::Buy, seed_bid, 10000);

        next_oid = book.next_order_id;
        next_eid = book.next_exec_id;

        self.next_order_id = next_oid;
        self.next_exec_id = next_eid;
    }

    fn pre_send_check(&mut self) -> bool {
        if self.override_forced {
            return true;
        }

        let book = self.get_book();
        let spread = book.spread_ticks();
        let (bid, ask) = book.top_of_book();

        match spread {
            Some(s) if s > self.config.max_spread_ticks => {
                self.paused = true;
                let remaining: u64 = self.child_orders
                    .iter()
                    .filter(|o| !o.sent)
                    .map(|o| o.qty)
                    .sum();

                print_alert(&self.config.symbol, bid, ask, spread, remaining, self.config.max_spread_ticks);
                self.wait_for_override();
                self.override_forced = true;
                self.paused = false;
                print_resume();
                true
            }
            _ => true,
        }
    }

    fn wait_for_override(&self) {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut attempts = 0;

        loop {
            attempts += 1;
            if attempts > 1 {
                eprintln!(
                    "{}[INPUT]{} Type 'override-resume' to continue (attempt {}): ",
                    YELLOW, RESET, attempts
                );
            } else {
                eprintln!("{}[INPUT]{} Enter 'override-resume' to resume: ", YELLOW, RESET);
            }
            let _ = stdout.flush();

            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed == "override-resume" {
                        return;
                    } else if trimmed.eq_ignore_ascii_case("override-resume") {
                        return;
                    } else if !trimmed.is_empty() {
                        eprintln!(
                            "{}[ERROR]{} Invalid command. Expected 'override-resume', got '{}'",
                            RED, RESET, trimmed
                        );
                    }
                }
                Err(e) => {
                    eprintln!("{}[ERROR]{} Read error: {}", RED, RESET, e);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }

    pub async fn run(&mut self) {
        print_banner();
        self.generate_schedule();

        eprintln!(
            "{}[TWAP]{} Symbol: {} | Side: {} | Total Qty: {} | Duration: {}s",
            BLUE, RESET,
            self.config.symbol,
            if matches!(self.config.side, Side::Buy) { "BUY" } else { "SELL" },
            self.config.total_qty,
            self.config.duration_secs
        );

        let start_time = Instant::now();
        let total = self.child_orders.len();

        for i in 0..self.child_orders.len() {
            let scheduled = self.child_orders[i].scheduled_offset;

            loop {
                let elapsed = start_time.elapsed();
                if elapsed >= scheduled {
                    break;
                }
                let remaining = scheduled.saturating_sub(elapsed);
                tokio::time::sleep(remaining.min(Duration::from_millis(10))).await;
            }

            if !self.pre_send_check() {
                continue;
            }

            let (qty, price, clord_id, sym_bytes, side) = {
                let order = &mut self.child_orders[i];
                order.sent = true;
                (
                    order.qty,
                    order.price,
                    order.clord_id.clone(),
                    self.config.symbol_bytes.clone(),
                    self.config.side,
                )
            };

            print_child_order_sent(i, total, qty, price, side);

            let mut next_oid = self.next_order_id;
            let mut next_eid = self.next_exec_id;

            let book = self.get_book_mut();
            book.next_order_id = next_oid;
            book.next_exec_id = next_eid;

            let _fills = book.submit_order(
                clord_id.as_bytes(),
                &sym_bytes,
                side,
                price,
                qty,
            );

            next_oid = book.next_order_id;
            next_eid = book.next_exec_id;

            self.next_order_id = next_oid;
            self.next_exec_id = next_eid;

            self.sent_count += 1;

            if i < self.child_orders.len() - 1 {
                let next_offset = self.child_orders[i + 1].scheduled_offset;
                let elapsed = start_time.elapsed();
                if elapsed < next_offset {
                    tokio::time::sleep(next_offset - elapsed).await;
                }
            }
        }

        print_summary(&self.config, self.sent_count, total);
    }
}

pub fn parse_args(args: &[String]) -> Option<TwapConfig> {
    let mut symbol: Option<String> = None;
    let mut total_qty: Option<u64> = None;
    let mut duration: Option<u64> = None;
    let mut side = Side::Buy;
    let mut seed = 0xDEADBEEFCAFEBABEu64;
    let mut tick_size = TICK_SIZE;
    let mut max_spread_ticks = MAX_SPREAD_TICKS;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--symbol" | "-s" => {
                if i + 1 < args.len() {
                    symbol = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --symbol requires a value", RED, RESET);
                    return None;
                }
            }
            "--total-qty" | "-q" => {
                if i + 1 < args.len() {
                    total_qty = Some(args[i + 1].parse().ok()?);
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --total-qty requires a value", RED, RESET);
                    return None;
                }
            }
            "--duration" | "-d" => {
                if i + 1 < args.len() {
                    duration = Some(args[i + 1].parse().ok()?);
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --duration requires a value", RED, RESET);
                    return None;
                }
            }
            "--side" | "-S" => {
                if i + 1 < args.len() {
                    side = match args[i + 1].as_str() {
                        "BUY" | "buy" | "1" => Side::Buy,
                        "SELL" | "sell" | "2" => Side::Sell,
                        _ => {
                            eprintln!("{}[ERROR]{} Invalid side, use BUY or SELL", RED, RESET);
                            return None;
                        }
                    };
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --side requires a value", RED, RESET);
                    return None;
                }
            }
            "--seed" => {
                if i + 1 < args.len() {
                    seed = u64::from_str_radix(&args[i + 1].trim_start_matches("0x"), 16)
                        .or_else(|_| args[i + 1].parse())
                        .ok()?;
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --seed requires a value", RED, RESET);
                    return None;
                }
            }
            "--tick-size" => {
                if i + 1 < args.len() {
                    tick_size = parse_price(args[i + 1].as_bytes());
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --tick-size requires a value", RED, RESET);
                    return None;
                }
            }
            "--max-spread" => {
                if i + 1 < args.len() {
                    max_spread_ticks = args[i + 1].parse().ok()?;
                    i += 2;
                } else {
                    eprintln!("{}[ERROR]{} --max-spread requires a value", RED, RESET);
                    return None;
                }
            }
            _ => {
                eprintln!("{}[ERROR]{} Unknown argument: {}", RED, RESET, args[i]);
                return None;
            }
        }
    }

    match (symbol, total_qty, duration) {
        (Some(sym), Some(qty), Some(dur)) => {
            let symbol_bytes = sym.as_bytes().to_vec();
            if symbol_bytes.is_empty() || symbol_bytes.len() > 8 {
                eprintln!("{}[ERROR]{} Symbol must be 1-8 characters", RED, RESET);
                return None;
            }
            if qty == 0 {
                eprintln!("{}[ERROR]{} Total qty must be > 0", RED, RESET);
                return None;
            }
            if dur == 0 {
                eprintln!("{}[ERROR]{} Duration must be > 0", RED, RESET);
                return None;
            }
            Some(TwapConfig {
                symbol: sym,
                symbol_bytes,
                total_qty: qty,
                duration_secs: dur,
                side,
                tick_size,
                max_spread_ticks,
                seed,
            })
        }
        _ => {
            eprintln!("{}", RED);
            eprintln!("Missing required arguments. Usage:");
            eprintln!("  fix-engine twap --symbol <SYMBOL> --total-qty <QTY> --duration <SECS> [OPTIONS]");
            eprintln!();
            eprintln!("Required:");
            eprintln!("  --symbol, -s      Trading symbol (1-8 chars)");
            eprintln!("  --total-qty, -q   Total quantity to execute");
            eprintln!("  --duration, -d    Execution duration in seconds");
            eprintln!();
            eprintln!("Optional:");
            eprintln!("  --side, -S        Side: BUY (default) or SELL");
            eprintln!("  --seed            Random seed for jitter (hex or decimal)");
            eprintln!("  --tick-size       Tick size (default 0.01)");
            eprintln!("  --max-spread      Max allowed spread in ticks (default 5)");
            eprintln!("{}", RESET);
            None
        }
    }
}

pub fn print_help() {
    println!("{}", CYAN);
    println!("fix-engine twap - Time-Weighted Average Price Execution Engine");
    println!("{}", RESET);
    println!();
    println!("USAGE:");
    println!("    fix-engine twap --symbol <SYMBOL> --total-qty <QTY> --duration <SECS> [OPTIONS]");
    println!();
    println!("EXAMPLES:");
    println!("    fix-engine twap --symbol BTCUSDT --total-qty 10000 --duration 300");
    println!("    fix-engine twap -s ETHUSDT -q 50000 -d 60 --side SELL --seed 0xCAFEBABE");
}
