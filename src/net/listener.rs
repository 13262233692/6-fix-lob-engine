use tokio::net::TcpListener;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use crate::fix::scanner;
use crate::fix::scanner::FixMessage;
use crate::engine::matcher::MatchingEngine;

const MAX_BUFFER: usize = 4096;

pub struct NetworkLayer {
    addr: String,
}

pub struct IncomingFixMessage {
    pub raw_len: usize,
    pub data: [u8; MAX_BUFFER],
}

impl NetworkLayer {
    pub fn new(addr: String) -> Self {
        NetworkLayer { addr }
    }

    pub async fn run(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        eprintln!("[fix-lob-engine] listening on {}", self.addr);

        let (tx, mut rx) = mpsc::channel::<IncomingFixMessage>(4096);

        let _engine_handle = tokio::spawn(async move {
            let mut engine = MatchingEngine::new();
            while let Some(msg) = rx.recv().await {
                let fix_msg = FixMessage::scan(&msg.data[..msg.raw_len]);
                engine.process_fix_message(&fix_msg);
            }
        });

        loop {
            let (socket, peer) = listener.accept().await?;
            eprintln!("[fix-lob-engine] accepted connection from {}", peer);
            let tx = tx.clone();
            tokio::spawn(async move {
                let (mut reader, _writer) = socket.into_split();
                let mut buf = vec![0u8; MAX_BUFFER * 4];
                let mut buf_used = 0usize;

                loop {
                    match reader.read(&mut buf[buf_used..]).await {
                        Ok(0) => {
                            eprintln!("[fix-lob-engine] connection closed from {}", peer);
                            break;
                        }
                        Ok(n) => {
                            buf_used += n;
                            while buf_used > 0 {
                                match scanner::extract_complete_message(&buf[..buf_used]) {
                                    Some(msg_end) => {
                                        let mut incoming = IncomingFixMessage {
                                            raw_len: 0,
                                            data: [0u8; MAX_BUFFER],
                                        };
                                        let copy_len = msg_end.min(MAX_BUFFER);
                                        incoming.raw_len = copy_len;
                                        incoming.data[..copy_len]
                                            .copy_from_slice(&buf[..copy_len]);
                                        if tx.send(incoming).await.is_err() {
                                            break;
                                        }
                                        buf.copy_within(msg_end..buf_used, 0);
                                        buf_used -= msg_end;
                                    }
                                    None => break,
                                }
                            }
                            if buf_used >= buf.len() {
                                let start = find_last_message_start(&buf[..buf_used]);
                                if start > 0 {
                                    let remaining = buf_used - start;
                                    buf.copy_within(start..buf_used, 0);
                                    buf_used = remaining;
                                } else {
                                    buf_used = 0;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[fix-lob-engine] read error from {}: {}", peer, e);
                            break;
                        }
                    }
                }
            });
        }
    }
}

fn find_last_message_start(buf: &[u8]) -> usize {
    let pattern = b"8=FIX";
    for i in (0..buf.len().saturating_sub(pattern.len())).rev() {
        if buf[i..].starts_with(pattern) {
            return i;
        }
    }
    0
}
