/// MultiMouse Relay Server
/// Pairs two clients via a 6-character room code and relays TCP traffic between them.
/// Deploy anywhere with: cargo build --release && ./relay
/// Default port: 57173  (override with RELAY_PORT env var)
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use rand::Rng;

type RoomMap = Arc<Mutex<HashMap<String, oneshot::Sender<TcpStream>>>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let port: u16 = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(57173);

    let rooms: RoomMap = Arc::new(Mutex::new(HashMap::new()));
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await.expect("bind failed");
    tracing::info!("MultiMouse relay listening on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let rooms = rooms.clone();
                tokio::spawn(async move {
                    handle(stream, peer, rooms).await;
                });
            }
            Err(e) => tracing::error!("accept error: {}", e),
        }
    }
}

async fn handle(mut stream: TcpStream, addr: SocketAddr, rooms: RoomMap) {
    let line = match read_line(&mut stream).await {
        Some(l) => l,
        None => return,
    };

    if line == "CREATE" {
        let code = generate_code();
        tracing::info!("[{}] Created room {}", addr, code);

        if stream.write_all(format!("{}\n", code).as_bytes()).await.is_err() {
            return;
        }

        let (tx, rx) = oneshot::channel::<TcpStream>();
        rooms.lock().await.insert(code.clone(), tx);

        match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
            Ok(Ok(joiner)) => {
                tracing::info!("[{}] Room {} paired", addr, code);
                if stream.write_all(b"READY\n").await.is_err() {
                    return;
                }
                splice(stream, joiner).await;
            }
            _ => {
                rooms.lock().await.remove(&code);
                let _ = stream.write_all(b"TIMEOUT\n").await;
                tracing::info!("[{}] Room {} timed out", addr, code);
            }
        }
    } else if let Some(code) = line.strip_prefix("JOIN ") {
        let code = code.trim().to_uppercase();
        tracing::info!("[{}] Joining room {}", addr, code);

        let tx = rooms.lock().await.remove(&code);
        match tx {
            Some(tx) => {
                if stream.write_all(b"OK\n").await.is_err() {
                    return;
                }
                let _ = tx.send(stream);
            }
            None => {
                let _ = stream.write_all(b"NOT_FOUND\n").await;
            }
        }
    } else {
        let _ = stream.write_all(b"ERR unknown command\n").await;
    }
}

async fn read_line(stream: &mut TcpStream) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::with_capacity(16);
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.ok()?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            buf.push(byte[0]);
        }
        if buf.len() > 64 {
            return None; // protect against malformed input
        }
    }
    String::from_utf8(buf).ok()
}

async fn splice(mut a: TcpStream, mut b: TcpStream) {
    let (mut ar, mut aw) = a.split();
    let (mut br, mut bw) = b.split();
    tokio::join!(
        tokio::io::copy(&mut ar, &mut bw),
        tokio::io::copy(&mut br, &mut aw),
    );
}

fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".chars().collect();
    (0..6).map(|_| chars[rng.gen_range(0..chars.len())]).collect()
}
