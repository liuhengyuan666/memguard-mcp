use crate::engine::projection;
use crate::models::*;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

// ── StateManager ───────────────────────────────────────────────────────────

/// Thread-safe state machine for MemGuard runtime.
///
/// Maintains the in-memory `RuntimeState`, `Vec<ADR>`, and `Vec<Trap>` under
/// `Arc<RwLock<...>>` for concurrent cross-agent access.  Writes to disk are
/// debounced (500 ms silence window) via an mpsc channel + spawned Tokio task.
pub struct StateManager {
    pub state: Arc<RwLock<RuntimeState>>,
    pub decisions: Arc<RwLock<Vec<ADR>>>,
    pub traps: Arc<RwLock<Vec<Trap>>>,
    pub project_root: PathBuf,
    flush_tx: mpsc::UnboundedSender<()>,
}

impl StateManager {
    /// Create a new StateManager, spawn the debounced flush background task.
    pub fn new(project_root: PathBuf) -> Self {
        let state = Arc::new(RwLock::new(RuntimeState {
            current_phase: String::new(),
            active_tasks: Vec::new(),
            constraints: Vec::new(),
        }));
        let decisions = Arc::new(RwLock::new(Vec::new()));
        let traps = Arc::new(RwLock::new(Vec::new()));
        let (flush_tx, mut flush_rx) = mpsc::unbounded_channel::<()>();

        // Spawn the debounced flush loop.  Clones are cheap (Arc bumps).
        let s = state.clone();
        let d = decisions.clone();
        let t = traps.clone();
        let root = project_root.clone();

        tokio::spawn(async move {
            loop {
                // Wait for the first flush signal.
                if flush_rx.recv().await.is_none() {
                    return; // channel closed — shut down
                }

                // Debounce window: collect additional signals for 500 ms.
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                            break; // silence — flush now
                        }
                        msg = flush_rx.recv() => {
                            if msg.is_none() {
                                return; // channel closed
                            }
                            // Another signal arrived; reset the timer.
                        }
                    }
                }

                flush_inner(&s, &d, &t, &root).await;
            }
        });

        Self {
            state,
            decisions,
            traps,
            project_root,
            flush_tx,
        }
    }

    // ── Bootstrap ──────────────────────────────────────────────────────

    /// Load existing state from `memory/` directory or initialize defaults.
    ///
    /// - If `memory/` exists: parse its Markdown files into memory.
    /// - If `memory/` does NOT exist: create it with empty defaults.
    /// - Always ensures `.memguard/` exists and writes cache files.
    pub async fn bootstrap(&self) -> Result<()> {
        let memory_dir = self.project_root.join("memory");
        let memguard_dir = self.project_root.join(".memguard");

        tokio::fs::create_dir_all(&memguard_dir)
            .await
            .context("Failed to create .memguard/ directory")?;

        if tokio::fs::try_exists(&memory_dir)
            .await
            .unwrap_or(false)
        {
            // ── Load existing memory ────────────────────────────────

            // context.md
            let ctx_path = memory_dir.join("context.md");
            if tokio::fs::try_exists(&ctx_path).await.unwrap_or(false) {
                let content = tokio::fs::read_to_string(&ctx_path)
                    .await
                    .context("Failed to read memory/context.md")?;
                let state = projection::parse_context(&content)
                    .context("Failed to parse memory/context.md")?;
                *self.state.write().await = state;
            }

            // decisions.md
            let dec_path = memory_dir.join("decisions.md");
            if tokio::fs::try_exists(&dec_path).await.unwrap_or(false) {
                let content = tokio::fs::read_to_string(&dec_path)
                    .await
                    .context("Failed to read memory/decisions.md")?;
                let adrs = projection::parse_decisions(&content)
                    .context("Failed to parse memory/decisions.md")?;
                *self.decisions.write().await = adrs;
            }

            // traps.md
            let trp_path = memory_dir.join("traps.md");
            if tokio::fs::try_exists(&trp_path).await.unwrap_or(false) {
                let content = tokio::fs::read_to_string(&trp_path)
                    .await
                    .context("Failed to read memory/traps.md")?;
                let traps = projection::parse_traps(&content)
                    .context("Failed to parse memory/traps.md")?;
                *self.traps.write().await = traps;
            }
        } else {
            // ── Greenfield: create memory/ with defaults ─────────────

            tokio::fs::create_dir_all(&memory_dir)
                .await
                .context("Failed to create memory/ directory")?;

            let default_ctx =
                "# Current Phase\n\n# Active Tasks\n\n# Constraints\n";
            tokio::fs::write(memory_dir.join("context.md"), default_ctx)
                .await
                .context("Failed to write default memory/context.md")?;
            tokio::fs::write(memory_dir.join("decisions.md"), "")
                .await
                .context("Failed to write default memory/decisions.md")?;
            tokio::fs::write(memory_dir.join("traps.md"), "")
                .await
                .context("Failed to write default memory/traps.md")?;
        }

        // ── Always write cache files ────────────────────────────────

        self.write_cache().await?;

        Ok(())
    }

    // ── Event processing ──────────────────────────────────────────────

    /// Apply a single `RuntimeEvent`, mutate in-memory state, and signal a
    /// debounced flush to disk.
    ///
    /// **Concurrency contract**: write locks are held ONLY for the mutation
    /// and dropped BEFORE the flush signal is sent.  This prevents deadlocks
    /// and ensures the lock is never held across an `.await` point.
    pub async fn apply_event(&self, event: RuntimeEvent) -> Result<()> {
        match event {
            RuntimeEvent::TaskUpdated {
                task_id,
                new_status,
            } => {
                let mut state = self.state.write().await;
                let task = state
                    .active_tasks
                    .iter_mut()
                    .find(|t| t.id == task_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Task not found: {}", task_id)
                    })?;
                task.status = new_status;
                // Lock dropped here (end of scope).
            }

            RuntimeEvent::AdrCommitted(adr) => {
                let mut decisions = self.decisions.write().await;
                // Supersede any existing ADR with the same id.
                if let Some(existing) =
                    decisions.iter_mut().find(|a| a.id == adr.id)
                {
                    existing.status = "superseded".to_string();
                }
                let mut adr = adr;
                adr.status = "active".to_string();
                decisions.push(adr);
            }

            RuntimeEvent::TrapRecorded(trap) => {
                let mut traps = self.traps.write().await;
                traps.push(trap);
            }

            RuntimeEvent::PhaseChanged(new_phase) => {
                let mut state = self.state.write().await;
                state.current_phase = new_phase;
            }
        }

        // Signal flush *after* all locks are released.
        let _ = self.flush_tx.send(());
        Ok(())
    }

    /// Manually trigger an immediate flush to disk (bypasses debounce).
    #[allow(dead_code)]
    pub async fn flush_now(&self) -> Result<()> {
        flush_inner(&self.state, &self.decisions, &self.traps, &self.project_root).await;
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Write cache files (runtime_state.json, search_index.json).
    async fn write_cache(&self) -> Result<()> {
        let memguard_dir = self.project_root.join(".memguard");

        // runtime_state.json
        {
            let state = self.state.read().await;
            let json = serde_json::to_string_pretty(&*state)
                .context("Failed to serialize RuntimeState")?;
            tokio::fs::write(memguard_dir.join("runtime_state.json"), json)
                .await
                .context("Failed to write .memguard/runtime_state.json")?;
        }

        // search_index.json
        {
            let decisions = self.decisions.read().await;
            let traps = self.traps.read().await;

            let adr_entries: Vec<serde_json::Value> = decisions
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "title": a.title,
                        "tags": a.tags,
                    })
                })
                .collect();

            let trap_entries: Vec<serde_json::Value> = traps
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "signature": t.error_signature,
                        "solution": t.solution,
                    })
                })
                .collect();

            let index = serde_json::json!({
                "adrs": adr_entries,
                "traps": trap_entries,
            });
            let json = serde_json::to_string_pretty(&index)
                .context("Failed to serialize search_index")?;
            tokio::fs::write(memguard_dir.join("search_index.json"), json)
                .await
                .context("Failed to write .memguard/search_index.json")?;
        }

        Ok(())
    }
}

// ── Free functions ─────────────────────────────────────────────────────────

/// Core flush routine: read-lock state, render all three Markdown files,
/// write to disk.  Used by both the debounced task and `flush_now()`.
///
/// **Graceful degradation**: if a single file write fails, the error is
/// logged and the remaining writes are attempted.  The `.memguard/` cache
/// is always written last so it can serve as a recovery anchor.
async fn flush_inner(
    state: &Arc<RwLock<RuntimeState>>,
    decisions: &Arc<RwLock<Vec<ADR>>>,
    traps: &Arc<RwLock<Vec<Trap>>>,
    project_root: &PathBuf,
) {
    let memory_dir = project_root.join("memory");
    let memguard_dir = project_root.join(".memguard");

    // Ensure directories exist (best-effort).
    let _ = tokio::fs::create_dir_all(&memory_dir).await;
    let _ = tokio::fs::create_dir_all(&memguard_dir).await;

    // ── Write memory/*.md files ─────────────────────────────────────

    // context.md
    {
        let s = state.read().await;
        let rendered = projection::render_context(&s);
        if let Err(e) =
            tokio::fs::write(memory_dir.join("context.md"), &rendered).await
        {
            eprintln!(
                "[memguard] ERROR writing memory/context.md: {}",
                e
            );
        }
    }

    // decisions.md
    {
        let d = decisions.read().await;
        let rendered = projection::render_decisions(&d);
        if let Err(e) =
            tokio::fs::write(memory_dir.join("decisions.md"), &rendered).await
        {
            eprintln!(
                "[memguard] ERROR writing memory/decisions.md: {}",
                e
            );
        }
    }

    // traps.md
    {
        let t = traps.read().await;
        let rendered = projection::render_traps(&t);
        if let Err(e) =
            tokio::fs::write(memory_dir.join("traps.md"), &rendered).await
        {
            eprintln!(
                "[memguard] ERROR writing memory/traps.md: {}",
                e
            );
        }
    }

    // ── Write .memguard/ cache files ────────────────────────────────

    // runtime_state.json
    {
        let s = state.read().await;
        match serde_json::to_string_pretty(&*s) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(
                    memguard_dir.join("runtime_state.json"),
                    json,
                )
                .await
                {
                    eprintln!(
                        "[memguard] ERROR writing .memguard/runtime_state.json: {}",
                        e
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "[memguard] ERROR serializing runtime_state: {}",
                    e
                );
            }
        }
    }

    // search_index.json
    {
        let d = decisions.read().await;
        let t = traps.read().await;

        let adr_entries: Vec<serde_json::Value> = d
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "title": a.title,
                    "tags": a.tags,
                })
            })
            .collect();

        let trap_entries: Vec<serde_json::Value> = t
            .iter()
            .map(|tr| {
                serde_json::json!({
                    "signature": tr.error_signature,
                    "solution": tr.solution,
                })
            })
            .collect();

        match serde_json::to_string_pretty(&serde_json::json!({
            "adrs": adr_entries,
            "traps": trap_entries,
        })) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(
                    memguard_dir.join("search_index.json"),
                    json,
                )
                .await
                {
                    eprintln!(
                        "[memguard] ERROR writing .memguard/search_index.json: {}",
                        e
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "[memguard] ERROR serializing search_index: {}",
                    e
                );
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_temp_dir() -> (TempDir, StateManager) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mgr = StateManager::new(dir.path().to_path_buf());
        (dir, mgr)
    }

    #[tokio::test]
    async fn test_bootstrap_greenfield() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.expect("bootstrap should succeed");

        // Verify memory/ was created with default files.
        let memory_dir = mgr.project_root.join("memory");
        assert!(memory_dir.join("context.md").exists());
        assert!(memory_dir.join("decisions.md").exists());
        assert!(memory_dir.join("traps.md").exists());

        // Verify .memguard/ cache was created.
        let cache_dir = mgr.project_root.join(".memguard");
        assert!(cache_dir.join("runtime_state.json").exists());
        assert!(cache_dir.join("search_index.json").exists());

        // Verify state is empty defaults.
        let state = mgr.state.read().await;
        assert!(state.current_phase.is_empty());
        assert!(state.active_tasks.is_empty());
        assert!(state.constraints.is_empty());
    }

    #[tokio::test]
    async fn test_apply_task_updated() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        // Pre-populate a task.
        {
            let mut state = mgr.state.write().await;
            state.active_tasks.push(Task {
                id: "TASK-000".into(),
                description: "Test task".into(),
                status: TaskStatus::Todo,
            });
        }

        // Update the task.
        mgr.apply_event(RuntimeEvent::TaskUpdated {
            task_id: "TASK-000".into(),
            new_status: TaskStatus::Done,
        })
        .await
        .expect("apply should succeed");

        let state = mgr.state.read().await;
        assert!(matches!(
            state.active_tasks[0].status,
            TaskStatus::Done
        ));
    }

    #[tokio::test]
    async fn test_apply_task_not_found() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let result = mgr
            .apply_event(RuntimeEvent::TaskUpdated {
                task_id: "nonexistent".into(),
                new_status: TaskStatus::Done,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_apply_adr_committed() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let adr = ADR {
            id: "ADR-001".into(),
            title: "Test ADR".into(),
            status: "Proposed".into(),
            context: "Test context".into(),
            decision: "Test decision".into(),
            tags: vec!["test".into()],
        };

        mgr.apply_event(RuntimeEvent::AdrCommitted(adr))
            .await
            .expect("apply should succeed");

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].id, "ADR-001");
        assert_eq!(decisions[0].status, "active"); // forced to active
    }

    #[tokio::test]
    async fn test_apply_adr_supersedes_existing() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let adr1 = ADR {
            id: "ADR-001".into(),
            title: "First".into(),
            status: "".into(),
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };

        mgr.apply_event(RuntimeEvent::AdrCommitted(adr1))
            .await
            .unwrap();

        let adr2 = ADR {
            id: "ADR-001".into(),
            title: "Second".into(),
            status: "".into(),
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };

        mgr.apply_event(RuntimeEvent::AdrCommitted(adr2))
            .await
            .unwrap();

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].status, "superseded");
        assert_eq!(decisions[1].status, "active");
        assert_eq!(decisions[1].title, "Second");
    }

    #[tokio::test]
    async fn test_apply_phase_changed() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        mgr.apply_event(RuntimeEvent::PhaseChanged(
            "deployment".into(),
        ))
        .await
        .expect("apply should succeed");

        let state = mgr.state.read().await;
        assert_eq!(state.current_phase, "deployment");
    }

    #[tokio::test]
    async fn test_apply_trap_recorded() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let trap = Trap {
            error_signature: "NPE".into(),
            context: "Null pointer".into(),
            solution: "Add check".into(),
        };

        mgr.apply_event(RuntimeEvent::TrapRecorded(trap))
            .await
            .expect("apply should succeed");

        let traps = mgr.traps.read().await;
        assert_eq!(traps.len(), 1);
        assert_eq!(traps[0].error_signature, "NPE");
    }

    #[tokio::test]
    async fn test_flush_now_writes_files() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut state = mgr.state.write().await;
            state.current_phase = "testing".to_string();
            state.active_tasks.push(Task {
                id: "TASK-000".into(),
                description: "Flush test".into(),
                status: TaskStatus::InProgress,
            });
            state
                .constraints
                .push("Must flush correctly".into());
        }

        mgr.flush_now().await.expect("flush should succeed");

        // Read back context.md and verify.
        let content =
            tokio::fs::read_to_string(mgr.project_root.join("memory/context.md"))
                .await
                .unwrap();
        assert!(content.contains("testing"));
        assert!(content.contains("Flush test"));
        assert!(content.contains("Must flush correctly"));
    }
}
