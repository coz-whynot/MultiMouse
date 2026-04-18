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
