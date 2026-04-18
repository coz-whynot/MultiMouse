use serde::{Deserialize, Serialize};

pub const MULTIMOUSE_PORT: u16 = 57172;
pub const TRANSFER_PORT: u16 = 57174;
pub const MULTIMOUSE_SERVICE: &str = "_multimouse._tcp.local.";
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Hello {
        device_id: String,
        device_name: String,
        version: u32,
    },
    PinRequest {
        pin: String,
    },
    SessionAuth {
        key: String,
    },
    PinResponse {
        accepted: bool,
        session_key: Option<String>,
    },
    FocusAcquired,
    FocusReleased,
    ScreenSize {
        width: f64,
        height: f64,
    },
    Input(InputEvent),
    ClipboardText {
        text: String,
    },
    Ping {
        ts: u64,
    },
    Pong {
        ts: u64,
    },
    Bye,
}

/// Separate protocol for file transfers (used on TRANSFER_PORT)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
pub enum TransferMessage {
    Auth {
        session_key: String,
        device_id: String,
    },
    AuthOk,
    AuthFail,
    FileOffer {
        id: String,
        name: String,
        size: u64,
    },
    FileAccept {
        id: String,
    },
    FileReject {
        id: String,
    },
    FileChunk {
        id: String,
        offset: u64,
        data: String, // base64 encoded
    },
    FileComplete {
        id: String,
    },
    FileError {
        id: String,
        reason: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum InputEvent {
    MouseMove { x: f64, y: f64 },
    MouseButton { button: u8, pressed: bool },
    MouseScroll { dx: i64, dy: i64 },
    Key { key: String, pressed: bool },
}

/// Commands sent from input capture / commands layer to the network writer task.
#[derive(Debug, Clone)]
pub enum NetCommand {
    Input(InputEvent),
    FocusAcquired,
    FocusReleased,
    ClipboardText(String),
    Ping(u64),
    Disconnect,
}

pub async fn read_message(stream: &mut tokio::net::TcpStream) -> Option<Message> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.ok()?;
    serde_json::from_slice(&buf).ok()
}

pub async fn send_message(stream: &mut tokio::net::TcpStream, msg: &Message) -> bool {
    use tokio::io::AsyncWriteExt;
    let data = match serde_json::to_vec(msg) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let len = data.len() as u32;
    if stream.write_all(&len.to_be_bytes()).await.is_err() {
        return false;
    }
    stream.write_all(&data).await.is_ok()
}

pub async fn read_transfer_msg(stream: &mut tokio::net::TcpStream) -> Option<TransferMessage> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 256 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.ok()?;
    serde_json::from_slice(&buf).ok()
}

pub async fn send_transfer_msg(stream: &mut tokio::net::TcpStream, msg: &TransferMessage) -> bool {
    use tokio::io::AsyncWriteExt;
    let data = match serde_json::to_vec(msg) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let len = data.len() as u32;
    if stream.write_all(&len.to_be_bytes()).await.is_err() {
        return false;
    }
    stream.write_all(&data).await.is_ok()
}
