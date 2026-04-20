pub mod client;
pub mod discovery;
pub mod protocol;
pub mod relay;
pub mod server;
pub mod trackpad;
pub mod transfer;

use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use crate::state::AppState;

pub async fn start_all_services(app: AppHandle, state: Arc<AppState>) {
    tokio::join!(
        discovery::start_discovery(app.clone(), state.clone()),
        server::start_server(app.clone(), state.clone()),
        transfer::start_transfer_server(app.clone(), state.clone()),
    );
}

/// Enable TCP keepalive on a session socket. Called from BOTH server-side
/// (right after `listener.accept`) and client-side (right after the
/// `TcpStream::connect` succeeds), before `into_split()` since the split
/// halves don't expose the file descriptor.
///
/// Timing: 30s idle → first probe, 5s between probes, 3 probes → ~45s to
/// detect a dead peer. That's the direct fix for the "silent half-open
/// socket" incident in v0.3.7 — before this, the kernel defaults (~2h)
/// governed detection.
///
/// Cross-platform notes:
/// - macOS: `TCP_KEEPALIVE` (the "idle" timer) is always supported.
///   `TCP_KEEPINTVL` and `TCP_KEEPCNT` were added in 10.15; on older mac
///   OSes `socket2` silently no-ops them and keepalive remains functional
///   at a coarser cadence.
/// - Linux: all three options supported; behaves as specified.
/// - Windows: backed by `WSAIoctl(SIO_KEEPALIVE_VALS)` which doesn't
///   expose a retry count; `with_retries` becomes a no-op. Idle + interval
///   still apply.
///
/// Failures (returning Err) are logged and then ignored — an undetected
/// half-open socket is still a worse outcome than a harder-to-debug
/// socket-config error, but a session missing keepalive just falls back
/// to the pre-v0.3.8 behaviour.
/// Read the local log file, keep only the last 500 lines, then cap the
/// result at `protocol::LOG_SHARE_MAX` bytes (dropping leading lines —
/// the diagnostic signal is at the END of a log, not the start). Returns
/// an empty string if file logging wasn't set up this run.
///
/// Used by both sides of the Phase F cross-device log-pull flow — either
/// side can be the one answering a LogRequest, so this helper lives at
/// network/mod level rather than duplicated in server/client.
pub(crate) async fn read_log_tail_capped() -> String {
    let Some(path) = crate::log_file_path() else { return String::new() };
    let raw = tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&path).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let lines: Vec<&str> = raw.lines().collect();
    let start = lines.len().saturating_sub(500);
    let tail = lines[start..].join("\n");
    let cap = protocol::LOG_SHARE_MAX;
    if tail.len() > cap {
        // Cut from the start, but align to the next newline so we don't
        // ship a truncated log line that's hard to read.
        let cut_bytes = tail.len() - cap;
        let safe = tail[cut_bytes..].find('\n').map(|i| cut_bytes + i + 1).unwrap_or(cut_bytes);
        tail[safe..].to_string()
    } else {
        tail
    }
}

pub(crate) fn tune_session_socket(stream: &tokio::net::TcpStream) {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(5))
        .with_retries(3);
    let sock_ref = socket2::SockRef::from(stream);
    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        tracing::warn!(err = ?e, "tune_session_socket: set_tcp_keepalive failed; session continues without keepalive");
    }
}
