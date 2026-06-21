use fix_lob_engine::net::listener::NetworkLayer;
use fix_lob_engine::fix::scanner::FixMessage;
use fix_lob_engine::engine::matcher::MatchingEngine;
use std::io::{self, BufRead};

const _RED: &str = "\x1b[31m";
const _GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() <= 1 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "twap" => {
            run_twap_command(&args).await;
        }
        "--stdin" => {
            run_stdin_mode();
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        addr => {
            run_tcp_mode(addr.to_string()).await;
        }
    }
}

fn print_usage() {
    println!("{}", CYAN);
    println!("{}fix-lob-engine{} - Low Latency FIX Limit Order Book Engine", BOLD, RESET);
    println!("{}", CYAN);
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                               COMMANDS                               ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!("║                                                                      ║");
    println!("║  {}TWAP ALGORITHMIC EXECUTION{}                                        ║", YELLOW, CYAN);
    println!("║    fix-engine twap --symbol <SYM> --total-qty <QTY> --duration <S>  ║");
    println!("║                                                                      ║");
    println!("║  {}NETWORK LISTENER{}                                                  ║", YELLOW, CYAN);
    println!("║    fix-engine [ADDR]         Listen for FIX stream on TCP addr       ║");
    println!("║                              Default: 0.0.0.0:8088                   ║");
    println!("║                                                                      ║");
    println!("║  {}STDIN MODE{}                                                        ║", YELLOW, CYAN);
    println!("║    fix-engine --stdin          Read FIX messages from STDIN          ║");
    println!("║                                                                      ║");
    println!("║  {}OPTIONS{}                                                           ║", YELLOW, CYAN);
    println!("║    -h, --help                 Show this help message                 ║");
    println!("║                                                                      ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!("{}", RESET);
    println!();
    println!("{}TWAP SUBCOMMAND OPTIONS:{}", BOLD, RESET);
    println!("  {}--symbol, -s{}      Trading symbol (1-8 chars, required)", YELLOW, RESET);
    println!("  {}--total-qty, -q{}   Total quantity to execute (required)", YELLOW, RESET);
    println!("  {}--duration, -d{}    Execution duration in seconds (required)", YELLOW, RESET);
    println!("  {}--side, -S{}        Side: BUY (default) or SELL", YELLOW, RESET);
    println!("  {}--seed{}            Random seed for time jitter", YELLOW, RESET);
    println!("  {}--tick-size{}       Tick size (default: 0.01)", YELLOW, RESET);
    println!("  {}--max-spread{}      Max allowed spread in ticks (default: 5)", YELLOW, RESET);
    println!();
    println!("{}EXAMPLES:{}", BOLD, RESET);
    println!("  fix-engine twap --symbol BTCUSDT --total-qty 10000 --duration 300");
    println!("  fix-engine twap -s ETHUSDT -q 50000 -d 60 --side SELL");
    println!("  fix-engine 0.0.0.0:9888");
    println!("  echo \"8=FIX.4.4|35=D|54=1|55=AAPL|44=100.5|38=100|\" | fix-engine --stdin");
    println!();
}

async fn run_twap_command(args: &[String]) {
    match fix_lob_engine::engine::twap::parse_args(args) {
        Some(config) => {
            let mut scheduler = fix_lob_engine::engine::twap::TwapScheduler::new(config);
            scheduler.seed_book(100_0000_0000, 100_0500_0000);
            scheduler.run().await;
        }
        None => {
            std::process::exit(1);
        }
    }
}

fn run_stdin_mode() {
    eprintln!("[fix-lob-engine] running in stdin mode");
    eprintln!("[fix-lob-engine] use | as SOH delimiter, e.g.: 8=FIX.4.4|35=D|54=1|55=AAPL|44=100.50|38=100|");
    let mut engine = MatchingEngine::new();
    let stdin = io::stdin();

    for line in stdin.lock().lines() {
        match line {
            Ok(l) => {
                if l.is_empty() {
                    continue;
                }
                let normalized = l.replace('|', "\x01");
                let data = normalized.as_bytes();

                let msg = FixMessage::scan(data);
                engine.process_fix_message(&msg);
            }
            Err(_) => break,
        }
    }
}

async fn run_tcp_mode(addr: String) {
    let network = NetworkLayer::new(addr);
    if let Err(e) = network.run().await {
        eprintln!("[fix-lob-engine] error: {}", e);
    }
}
