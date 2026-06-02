mod cli;
mod engine;
mod mcp;
mod models;
mod search;

use crate::engine::state_manager::StateManager;
use crate::mcp::server::McpServer;
use std::path::PathBuf;
use std::sync::Arc;

/// Resolve the project root using a tiered strategy:
///
/// 1. `MEMGUARD_PROJECT_ROOT` environment variable (explicit override)
/// 2. `std::env::current_dir()` — OpenCode spawns MCP servers with CWD set
///    to the project directory.  This is the default for global installs.
/// 3. CLI positional argument (argv[1]) — **DEPRECATED**.  Kept for backward
///    compatibility with single-project deploys, but prints a warning because
///    it is easily misconfigured (users often put the exe directory as the arg).
///
/// NOTE: No heuristic upward search.  The authoritative correction happens in
/// `handle_initialize()` via the MCP `workspaceFolders` / `rootUri` handshake.
fn resolve_project_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("Failed to get current directory");

    eprintln!("[memguard] ARGS = {:?}", std::env::args().collect::<Vec<_>>());
    eprintln!("[memguard] CWD detected: {}", cwd.display());

    // Tier 1: Environment variable (explicit user override, highest priority).
    if let Ok(env_root) = std::env::var("MEMGUARD_PROJECT_ROOT") {
        let path = PathBuf::from(&env_root);
        eprintln!(
            "[memguard] Using MEMGUARD_PROJECT_ROOT env override: {} (CWD was: {})",
            path.display(),
            cwd.display()
        );
        return path;
    }

    // Tier 2: CWD — OpenCode sets this to the project directory when spawning
    // the MCP server.  This is the default and most reliable for global installs.
    eprintln!(
        "[memguard] Using CWD as project root: {}",
        cwd.display()
    );

    // Tier 3 (DEPRECATED): CLI positional argument.
    // If the user still has an arg in their opencode.json, warn them so they
    // can remove it.  We do NOT return the arg path; CWD is authoritative.
    if let Some(arg) = std::env::args().nth(1) {
        let arg_path = PathBuf::from(&arg);
        eprintln!(
            "[memguard] WARNING: CLI arg '{}' is ignored for project root. \
             CWD ({}) is used instead. Remove args from opencode.json for global install mode. \
             If you must override, use MEMGUARD_PROJECT_ROOT env.",
            arg_path.display(),
            cwd.display()
        );
    }

    cwd
}

#[tokio::main]
async fn main() {
    // ── CLI subcommand routing ──────────────────────────────────────
    if std::env::args().nth(1).as_deref() == Some("cleanup") {
        let args = cli::cleanup::CleanupArgs::parse();
        if let Err(e) = cli::cleanup::run_cleanup(&args) {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let project_root = resolve_project_root();

    eprintln!(
        "[memguard] Final project root: {}",
        project_root.display()
    );

    let state_manager = Arc::new(StateManager::new(project_root.clone()));

    // Spawn a task to handle OS shutdown signals (SIGINT/SIGTERM on Unix,
    // Ctrl+C on Windows).  When signal received, flush all pending state
    // synchronously before the process is killed.
    let sm_for_signal = state_manager.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigint =
                signal(SignalKind::interrupt()).expect("SIGINT handler");
            let mut sigterm =
                signal(SignalKind::terminate()).expect("SIGTERM handler");
            tokio::select! {
                _ = sigint.recv() => {}
                _ = sigterm.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }

        eprintln!("[memguard] Shutdown signal received, flushing state...");
        if let Err(e) = sm_for_signal.flush_now().await {
            eprintln!("[memguard] ERROR during shutdown flush: {}", e);
        }
        std::process::exit(0);
    });

    let server = McpServer::new(state_manager.clone());

    if let Err(e) = server.run(&project_root.display().to_string()).await {
        eprintln!("[memguard] Fatal: {}", e);
        // Attempt one last flush before exiting on error.
        if let Err(e) = state_manager.flush_now().await {
            eprintln!("[memguard] ERROR during error-shutdown flush: {}", e);
        }
        std::process::exit(1);
    }
}
