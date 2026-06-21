use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use crate::fix::ringbuffer::{LookaheadRingBuffer, ScanStatus};
use crate::fix::scanner::FixMessage;
use crate::engine::matcher::MatchingEngine;

const MAX_BUFFER: usize = 4096;
const RINGBUFFER_CAP: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 512;
const READ_TIMEOUT: Duration = Duration::from_secs(120);

pub struct NetworkLayer {
    addr: String,
}

pub struct IncomingFixMessage {
    pub raw_len: usize,
    pub data: [u8; MAX_BUFFER],
}

struct ConnectionGuard {
    peer: std::net::SocketAddr,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active.store(false, std::sync::atomic::Ordering::SeqCst);
        eprintln!(
            "[fix-lob-engine] connection closed (fd released): {}",
            self.peer
        );
    }
}

impl NetworkLayer {
    pub fn new(addr: String) -> Self {
        NetworkLayer { addr }
    }

    pub async fn run(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        eprintln!("[fix-lob-engine] listening on {}", self.addr);

        let (tx, mut rx) = mpsc::channel::<IncomingFixMessage>(8192);
        let mut connection_tasks = JoinSet::<()>::new();

        let active_connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let engine_handle = tokio::spawn(async move {
            let mut engine = MatchingEngine::new();
            while let Some(msg) = rx.recv().await {
                let fix_msg = FixMessage::scan(&msg.data[..msg.raw_len]);
                engine.process_fix_message(&fix_msg);
            }
            eprintln!("[fix-lob-engine] engine task shutting down");
        });

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((socket, peer)) => {
                            let current_count = active_connections.load(std::sync::atomic::Ordering::SeqCst);
                            if current_count >= MAX_CONNECTIONS {
                                eprintln!(
                                    "[fix-lob-engine] max connections reached ({}), rejecting {}",
                                    MAX_CONNECTIONS, peer
                                );
                                drop(socket);
                                continue;
                            }

                            eprintln!(
                                "[fix-lob-engine] accepted connection from {} (active: {}/{})",
                                peer, current_count + 1, MAX_CONNECTIONS
                            );

                            active_connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                            let tx = tx.clone();
                            let active_connections = active_connections.clone();
                            let active_flag = Arc::new(std::sync::atomic::AtomicBool::new(true));

                            connection_tasks.spawn(async move {
                                let guard = ConnectionGuard {
                                    peer,
                                    active: active_flag.clone(),
                                };

                                if let Err(e) = handle_connection(socket, tx, active_flag.clone()).await {
                                    eprintln!(
                                        "[fix-lob-engine] connection error from {}: {}",
                                        peer, e
                                    );
                                }

                                drop(guard);
                                active_connections.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            });
                        }
                        Err(e) => {
                            eprintln!("[fix-lob-engine] accept error: {}", e);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }

                Some(task_result) = connection_tasks.join_next() => {
                    match task_result {
                        Ok(()) => {}
                        Err(join_err) => {
                            if join_err.is_panic() {
                                eprintln!(
                                    "[fix-lob-engine] connection task panicked: {}",
                                    join_err
                                );
                            }
                        }
                    }
                }
            }

            if connection_tasks.is_empty() && engine_handle.is_finished() {
                break;
            }
        }

        eprintln!("[fix-lob-engine] shutting down, draining remaining connections");
        connection_tasks.shutdown().await;

        let _ = engine_handle.await;
        Ok(())
    }
}

async fn handle_connection(
    socket: TcpStream,
    tx: mpsc::Sender<IncomingFixMessage>,
    active: Arc<std::sync::atomic::AtomicBool>,
) -> std::io::Result<()> {
    let peer = socket.peer_addr()?;

    let (mut reader, mut writer) = socket.into_split();

    let mut ringbuf = LookaheadRingBuffer::new(RINGBUFFER_CAP);
    let mut read_buf = vec![0u8; 64 * 1024];
    let mut idle_since = std::time::Instant::now();

    loop {
        if !active.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        if idle_since.elapsed() > READ_TIMEOUT {
            eprintln!("[fix-lob-engine] connection idle timeout: {}", peer);
            break;
        }

        if ringbuf.available() == 0 {
            eprintln!(
                "[fix-lob-engine] ring buffer full for {}, skipping data",
                peer
            );
            ringbuf.clear();
            idle_since = std::time::Instant::now();
        }

        tokio::select! {
            read_result = async {
                let read_cap = ringbuf.available().min(read_buf.len());
                reader.read(&mut read_buf[..read_cap]).await
            } => {
                match read_result {
                    Ok(0) => {
                        eprintln!("[fix-lob-engine] client closed connection: {}", peer);
                        break;
                    }
                    Ok(n) => {
                        idle_since = std::time::Instant::now();
                        let written = ringbuf.extend_from_slice(&read_buf[..n]);
                        if written < n {
                            eprintln!(
                                "[fix-lob-engine] ring buffer overflow for {}, dropped {} bytes",
                                peer, n - written
                            );
                        }

                        loop {
                            match ringbuf.extract_complete_message() {
                                ScanStatus::Complete(msg_len) => {
                                    if msg_len > MAX_BUFFER {
                                        eprintln!(
                                            "[fix-lob-engine] message too large ({} bytes) from {}, discarding",
                                            msg_len, peer
                                        );
                                        ringbuf.consume(msg_len);
                                        continue;
                                    }

                                    let mut incoming = IncomingFixMessage {
                                        raw_len: msg_len,
                                        data: [0u8; MAX_BUFFER],
                                    };

                                    if let Some(msg_bytes) = ringbuf.peek_slice(0, msg_len) {
                                        incoming.data[..msg_len].copy_from_slice(&msg_bytes);
                                        ringbuf.consume(msg_len);

                                        match tx.try_send(incoming) {
                                            Ok(()) => {}
                                            Err(mpsc::error::TrySendError::Full(_)) => {
                                                eprintln!(
                                                    "[fix-lob-engine] engine channel full, dropping message from {}",
                                                    peer
                                                );
                                            }
                                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                                eprintln!(
                                                    "[fix-lob-engine] engine channel closed, shutting down connection {}",
                                                    peer
                                                );
                                                return Ok(());
                                            }
                                        }
                                    } else {
                                        eprintln!(
                                            "[fix-lob-engine] failed to peek {} bytes from ring buffer for {}",
                                            msg_len, peer
                                        );
                                        ringbuf.consume(msg_len);
                                    }
                                }
                                ScanStatus::Incomplete => {
                                    break;
                                }
                                ScanStatus::Invalid => {
                                    eprintln!(
                                        "[fix-lob-engine] invalid FIX message from {}, resetting buffer",
                                        peer
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut
                        {
                            continue;
                        }
                        if e.kind() == std::io::ErrorKind::ConnectionReset
                            || e.kind() == std::io::ErrorKind::ConnectionAborted
                        {
                            eprintln!(
                                "[fix-lob-engine] connection reset by peer {}: {}",
                                peer, e
                            );
                        } else {
                            eprintln!(
                                "[fix-lob-engine] read error from {}: {}",
                                peer, e
                            );
                        }
                        break;
                    }
                }
            }

            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if ringbuf.is_empty() {
                    continue;
                }
                if idle_since.elapsed() > Duration::from_secs(30) {
                    eprintln!(
                        "[fix-lob-engine] partial message timeout for {}, discarding {} bytes",
                        peer, ringbuf.len()
                    );
                    ringbuf.clear();
                }
            }
        }
    }

    let _ = writer.shutdown().await;
    drop(writer);
    drop(reader);

    Ok(())
}
