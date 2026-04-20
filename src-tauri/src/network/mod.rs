pub mod client;
pub mod discovery;
pub mod protocol;
pub mod relay;
pub mod server;
pub mod trackpad;
pub mod transfer;

use std::sync::Arc;
use tauri::AppHandle;
use crate::state::AppState;

pub async fn start_all_services(app: AppHandle, state: Arc<AppState>) {
    tokio::join!(
        discovery::start_discovery(app.clone(), state.clone()),
        server::start_server(app.clone(), state.clone()),
        transfer::start_transfer_server(app.clone(), state.clone()),
    );
}

/// Read the local log file, keep only the last 500 lines, then cap the
/// result at `protocol::LOG_SHARE_MAX` bytes (dropping leading lines —
/// the diagnostic signal is at the END of a log, not the start). Returns
/// an empty string if file logging wasn't set up this run.
///
/// Used by both sides of the cross-device log-pull flow — either side can
/// be the one answering a LogRequest, so this helper lives at network/mod
/// level rather than duplicated in server/client.
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
