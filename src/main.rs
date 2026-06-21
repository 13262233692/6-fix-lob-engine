use fix_lob_engine::net::listener::NetworkLayer;
use fix_lob_engine::fix::scanner::FixMessage;
use fix_lob_engine::engine::matcher::MatchingEngine;
use std::io::{self, BufRead};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "--stdin" {
        run_stdin_mode();
    } else {
        let addr = if args.len() > 1 {
            args[1].clone()
        } else {
            "0.0.0.0:8088".to_string()
        };
        run_tcp_mode(addr).await;
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
