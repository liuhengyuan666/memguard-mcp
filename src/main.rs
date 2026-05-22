mod engine;
mod mcp;
mod models;

use crate::engine::state_manager::StateManager;
use crate::mcp::server::McpServer;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Resolve project root: first CLI arg, or current directory.
    let project_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().expect("Failed to get current directory")
        });

    eprintln!(
        "[memguard] Project root: {}",
        project_root.display()
    );

    let state_manager = Arc::new(StateManager::new(project_root.clone()));

    let server = McpServer::new(state_manager);

    if let Err(e) = server.run(&project_root.display().to_string()).await {
        eprintln!("[memguard] Fatal: {}", e);
        std::process::exit(1);
    }
}
