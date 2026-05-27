use crate::engine::projection;
use crate::models::*;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    pub project_root: Arc<RwLock<PathBuf>>,
    flush_generation: Arc<AtomicU64>,
    flush_tx: mpsc::UnboundedSender<()>,
    /// Set to true when context.md was successfully parsed (or user committed
    /// real data).  While false, flush_inner will NOT overwrite context.md —
    /// protecting old-format files from being nuked by empty-state writes.
    context_ok: Arc<AtomicBool>,
    decisions_ok: Arc<AtomicBool>,
    traps_ok: Arc<AtomicBool>,
}

// ── ADR Conflict Detection ────────────────────────────────────────────────

/// Compute a content hash for an ADR based on its title and decision fields.
fn content_hash(adr: &ADR) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    adr.title.trim().to_lowercase().hash(&mut h);
    adr.decision.trim().to_lowercase().hash(&mut h);
    h.finish()
}

/// Structured error variants for ADR conflict detection.
#[derive(Debug)]
pub enum AdrError {
    ActiveConflict {
        id: String,
        existing_title: String,
        new_title: String,
    },
    RejectedRepeat {
        id: String,
    },
}

impl std::fmt::Display for AdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdrError::ActiveConflict {
                id,
                existing_title,
                new_title,
            } => write!(
                f,
                "[CONFLICT] ADR {} conflict: an active ADR with this id already exists with different content. Existing title: \"{}\", New title: \"{}\"",
                id, existing_title, new_title
            ),
            AdrError::RejectedRepeat { id } => write!(
                f,
                "[CONFLICT] ADR {} was previously rejected with the same decision content. To re-submit, explain what material conditions have changed in the context field.",
                id
            ),
        }
    }
}

impl std::error::Error for AdrError {}

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

        // Parse-success guards: protect old-format files from being
        // overwritten by empty-state flushes.  Set to true when parsing
        // succeeds or when the user commits real data via events.
        let context_ok = Arc::new(AtomicBool::new(false));
        let decisions_ok = Arc::new(AtomicBool::new(false));
        let traps_ok = Arc::new(AtomicBool::new(false));

        // Spawn the debounced flush loop.  Clones are cheap (Arc bumps).
        let s = state.clone();
        let d = decisions.clone();
        let t = traps.clone();
        let root = Arc::new(RwLock::new(project_root));
        let root_for_task = root.clone();
        let flush_generation = Arc::new(AtomicU64::new(0));
        let flush_gen_for_task = flush_generation.clone();
        let ctx_ok = context_ok.clone();
        let dec_ok = decisions_ok.clone();
        let trp_ok = traps_ok.clone();

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

                // Snapshot root and generation atomically.
                // If generation changes before we finish, abort to avoid
                // writing wrong-project data to the wrong directory.
                let gen_snapshot = flush_gen_for_task.load(Ordering::Acquire);
                let root_path = root_for_task.read().await.clone();

                // Double-check generation after reading root.
                if flush_gen_for_task.load(Ordering::Acquire) != gen_snapshot {
                    eprintln!("[memguard] Flush aborted: project root changed mid-flush");
                    continue;
                }

                flush_inner(&s, &d, &t, &root_path, &ctx_ok, &dec_ok, &trp_ok).await.unwrap_or_else(|e| {
                    eprintln!("[memguard] flush error: {}", e);
                });

                // After writing, verify generation hasn't changed.
                // If it has, the next flush cycle will correct any stale data.
                if flush_gen_for_task.load(Ordering::Acquire) != gen_snapshot {
                    eprintln!("[memguard] WARNING: project switched during flush; next flush will correct.");
                }
            }
        });

        Self {
            state,
            decisions,
            traps,
            project_root: root,
            flush_generation,
            flush_tx,
            context_ok,
            decisions_ok,
            traps_ok,
        }
    }

    /// Switch to a different project root, loading its state from disk.
    ///
    /// Flushes all pending writes for the current project BEFORE switching,
    /// then bumps a generation counter to abort any in-flight flush tasks,
    /// preventing cross-project data leaks.
    ///
    /// The active `project_root` is updated only AFTER the new project's
    /// state is successfully loaded from disk — if parsing fails, the
    /// root is left unchanged so the caller can retry or fall back safely.
    ///
    /// State is loaded and swapped atomically (all three RwLocks held
    /// simultaneously) so no intermediate "empty" state is ever visible
    /// to the debounced flush task.
    pub async fn switch_project(&self, new_root: PathBuf) -> Result<()> {
        // 1. Flush pending data for the current project BEFORE switching.
        self.flush_now().await?;

        // 2. Bump generation to signal in-flight flush tasks to abort.
        self.flush_generation.fetch_add(1, Ordering::Release);

        // 3. Load new project state from disk into temporary variables.
        //    project_root is NOT updated yet — if loading fails (e.g.
        //    parse error), the active project reference remains unchanged
        //    and no cross-project state corruption occurs.
        let loaded = load_state_from_disk(&new_root).await?;
        let is_greenfield = loaded.is_greenfield;

        // 4. Update the active project root (only after load succeeds).
        *self.project_root.write().await = new_root;
        let project_root = self.project_root.read().await.clone();

        // 5. Atomic swap: acquire all three write locks in the globally
        //    consistent order (state → decisions → traps), perform the
        //    assignment, then drop all guards — no .await points between.
        {
            let mut s = self.state.write().await;
            let mut d = self.decisions.write().await;
            let mut t = self.traps.write().await;
            *s = loaded.state;
            *d = loaded.decisions;
            *t = loaded.traps;
        }

        // Track parse success for flush guard.
        self.context_ok.store(loaded.context_parsed, Ordering::Release);
        self.decisions_ok.store(loaded.decisions_parsed, Ordering::Release);
        self.traps_ok.store(loaded.traps_parsed, Ordering::Release);

        // 6. If the target project has no memory/ yet, seed defaults.
        if is_greenfield {
            let memory_dir = project_root.join("memory");
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

        // 7. Ensure .memguard/ exists, then write cache files.
        tokio::fs::create_dir_all(&project_root.join(".memguard"))
            .await
            .context("Failed to create .memguard/ directory")?;
        self.write_cache().await?;

        Ok(())
    }

    // ── Bootstrap ──────────────────────────────────────────────────────

    /// Load existing state from `memory/` directory or initialize defaults.
    ///
    /// - If `memory/` exists: parse its Markdown files into memory.
    /// - If `memory/` does NOT exist: create it with empty defaults.
    /// - Always ensures `.memguard/` exists and writes cache files.
    ///
    /// Uses atomic three-lock swap so no intermediate empty state is
    /// visible to the debounced flush task.
    pub async fn bootstrap(&self) -> Result<()> {
        let project_root = self.project_root.read().await.clone();
        let memguard_dir = project_root.join(".memguard");

        tokio::fs::create_dir_all(&memguard_dir)
            .await
            .context("Failed to create .memguard/ directory")?;

        let loaded = load_state_from_disk(&project_root).await?;

        // Atomic swap: acquire all three write locks in consistent order.
        {
            let mut s = self.state.write().await;
            let mut d = self.decisions.write().await;
            let mut t = self.traps.write().await;
            *s = loaded.state;
            *d = loaded.decisions;
            *t = loaded.traps;
        }

        // Track parse success for flush guard.
        self.context_ok.store(loaded.context_parsed, Ordering::Release);
        self.decisions_ok.store(loaded.decisions_parsed, Ordering::Release);
        self.traps_ok.store(loaded.traps_parsed, Ordering::Release);

        // If greenfield, write default files to disk.
        if loaded.is_greenfield {
            let memory_dir = project_root.join("memory");
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
                let new_hash = content_hash(&adr);

                // Check ALL existing ADRs with the same ID for conflict.
                // A single Vec may contain multiple entries for the same ID
                // (e.g., after bootstrap loads both decisions.md and archive).
                let mut active_adr: Option<&ADR> = None;
                let mut rejected_adr: Option<&ADR> = None;
                for existing in decisions.iter().filter(|a| a.id == adr.id) {
                    if existing.status == "active" && active_adr.is_none() {
                        active_adr = Some(existing);
                    }
                    if existing.status == "rejected" && rejected_adr.is_none() {
                        rejected_adr = Some(existing);
                    }
                }

                if let Some(active) = active_adr {
                    let active_hash = content_hash(active);
                    if new_hash == active_hash {
                        // Idempotent: same content, silently ignore.
                        drop(decisions);
                        let _ = self.flush_tx.send(());
                        return Ok(());
                    }
                    return Err(AdrError::ActiveConflict {
                        id: adr.id,
                        existing_title: active.title.clone(),
                        new_title: adr.title,
                    }
                    .into());
                }

                if let Some(rejected) = rejected_adr {
                    let rejected_hash = content_hash(rejected);
                    if new_hash == rejected_hash {
                        return Err(AdrError::RejectedRepeat { id: adr.id }.into());
                    }
                    // Different content: fall through to supersede + push.
                }

                // No conflict: mark all existing versions as superseded, then push new.
                for existing in decisions.iter_mut().filter(|a| a.id == adr.id) {
                    existing.status = "superseded".to_string();
                }
                let mut adr = adr;
                adr.status = "active".to_string();
                decisions.push(adr);
                self.decisions_ok.store(true, Ordering::Release);
            }

            RuntimeEvent::TrapRecorded(trap) => {
                let mut traps = self.traps.write().await;
                traps.push(trap);
                self.traps_ok.store(true, Ordering::Release);
            }

            RuntimeEvent::PhaseChanged(new_phase) => {
                let mut state = self.state.write().await;
                state.current_phase = new_phase;
                // Auto-unlock: user is actively using the new system,
                // so overwriting old-format context.md is now safe.
                self.context_ok.store(true, Ordering::Release);
            }
        }

        // Signal flush *after* all locks are released.
        let _ = self.flush_tx.send(());
        Ok(())
    }

    /// Manually trigger an immediate flush to disk (bypasses debounce).
    pub async fn flush_now(&self) -> Result<()> {
        let root = self.project_root.read().await.clone();
        flush_inner(
            &self.state,
            &self.decisions,
            &self.traps,
            &root,
            &self.context_ok,
            &self.decisions_ok,
            &self.traps_ok,
        )
        .await
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Write cache files (runtime_state.json, search_index.json).
    async fn write_cache(&self) -> Result<()> {
        let project_root = self.project_root.read().await.clone();
        let memguard_dir = project_root.join(".memguard");

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
                        "status": a.status,
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

/// Loaded runtime state from disk, with per-file parse-success flags.
struct LoadedFiles {
    state: RuntimeState,
    decisions: Vec<ADR>,
    traps: Vec<Trap>,
    is_greenfield: bool,
    context_parsed: bool,
    decisions_parsed: bool,
    traps_parsed: bool,
}

/// Load runtime state from `{root}/memory/*.md` files.
///
/// Returns `LoadedFiles` with per-file parse-success flags.  When a file
/// exists but can't be parsed (e.g. old-format), the parse flag is `false`
/// and empty defaults are returned — but the on-disk file is NOT touched
/// until the user explicitly commits new data.
///
/// **Error handling**: `try_exists` failures are propagated as errors
/// rather than silently treated as "doesn't exist" — preventing transient
/// filesystem issues from triggering the greenfield path which would
/// overwrite existing files with empty content.
async fn load_state_from_disk(
    root: &PathBuf,
) -> Result<LoadedFiles> {
    let memory_dir = root.join("memory");

    let exists = tokio::fs::try_exists(&memory_dir)
        .await
        .context("Failed to check memory/ existence")?;

    if !exists {
        return Ok(LoadedFiles {
            state: RuntimeState {
                current_phase: String::new(),
                active_tasks: Vec::new(),
                constraints: Vec::new(),
            },
            decisions: Vec::new(),
            traps: Vec::new(),
            is_greenfield: true,
            context_parsed: true,   // greenfield: nothing to parse, no risk
            decisions_parsed: true,
            traps_parsed: true,
        });
    }

    let mut context_parsed = false;
    let mut traps_parsed = false;
    let decisions_parsed;

    // ── Load context.md ─────────────────────────────────────────
    let state = {
        let ctx_path = memory_dir.join("context.md");
        if tokio::fs::try_exists(&ctx_path)
            .await
            .context("Failed to check memory/context.md existence")?
        {
            let content = tokio::fs::read_to_string(&ctx_path)
                .await
                .context("Failed to read memory/context.md")?;
            match projection::parse_context(&content) {
                Ok(s) => {
                    context_parsed = true;
                    s
                }
                Err(e) => {
                    eprintln!(
                        "[memguard] WARNING: failed to parse memory/context.md (old format?): {}",
                        e
                    );
                    eprintln!(
                        "[memguard] The file is preserved as-is. To migrate: have the LLM read the old file,",
                    );
                    eprintln!(
                        "[memguard] convert content to new format, and write it back. Then re-run bootstrap.",
                    );
                    RuntimeState {
                        current_phase: String::new(),
                        active_tasks: Vec::new(),
                        constraints: Vec::new(),
                    }
                }
            }
        } else {
            RuntimeState {
                current_phase: String::new(),
                active_tasks: Vec::new(),
                constraints: Vec::new(),
            }
        }
    };

    // ── Load decisions.md + decisions_archive.md ────────────────
    let mut decisions = Vec::new();
    let mut main_parsed = false;
    let mut archive_parsed = false;

    let dec_path = memory_dir.join("decisions.md");
    if tokio::fs::try_exists(&dec_path)
        .await
        .context("Failed to check memory/decisions.md existence")?
    {
        let content = tokio::fs::read_to_string(&dec_path)
            .await
            .context("Failed to read memory/decisions.md")?;
        match projection::parse_decisions(&content) {
            Ok(mut adrs) => {
                main_parsed = true;
                decisions.append(&mut adrs);
            }
            Err(e) => {
                eprintln!(
                    "[memguard] WARNING: failed to parse memory/decisions.md (old format?): {}",
                    e
                );
                eprintln!(
                    "[memguard] The file is preserved as-is. To migrate: have the LLM read the old file,",
                );
                eprintln!(
                    "[memguard] convert content to new format, and write it back. Then re-run bootstrap.",
                );
            }
        }
    }

    let archive_path = memory_dir.join("decisions_archive.md");
    if tokio::fs::try_exists(&archive_path)
        .await
        .context("Failed to check memory/decisions_archive.md existence")?
    {
        let content = tokio::fs::read_to_string(&archive_path)
            .await
            .context("Failed to read memory/decisions_archive.md")?;
        match projection::parse_decisions(&content) {
            Ok(mut adrs) => {
                archive_parsed = true;
                decisions.append(&mut adrs);
            }
            Err(e) => {
                eprintln!(
                    "[memguard] WARNING: failed to parse memory/decisions_archive.md (old format?): {}",
                    e
                );
            }
        }
    }

    // Deduplicate by ID: keep the entry with the highest-priority status.
    // This prevents duplicate ADR entries when bootstrap loads both
    // decisions.md (active) and decisions_archive.md (superseded).
    fn adr_status_priority(s: &str) -> u8 {
        match s {
            "active" => 5,
            "proposed" => 4,
            "rejected" => 3,
            "superseded" => 2,
            "deprecated" => 1,
            _ => 0,
        }
    }
    let mut best_by_id: std::collections::HashMap<String, ADR> = std::collections::HashMap::new();
    for adr in decisions {
        let priority = adr_status_priority(&adr.status);
        best_by_id
            .entry(adr.id.clone())
            .and_modify(|existing| {
                if priority > adr_status_priority(&existing.status) {
                    *existing = adr.clone();
                }
            })
            .or_insert(adr);
    }
    let mut deduped: Vec<ADR> = best_by_id.into_values().collect();
    // Sort by priority descending, then by ID ascending for deterministic order.
    deduped.sort_by(|a, b| {
        let pa = adr_status_priority(&a.status);
        let pb = adr_status_priority(&b.status);
        pb.cmp(&pa).then_with(|| a.id.cmp(&b.id))
    });
    decisions = deduped;

    decisions_parsed = main_parsed || archive_parsed;

    // ── Load traps.md ───────────────────────────────────────────
    let traps = {
        let trp_path = memory_dir.join("traps.md");
        if tokio::fs::try_exists(&trp_path)
            .await
            .context("Failed to check memory/traps.md existence")?
        {
            let content = tokio::fs::read_to_string(&trp_path)
                .await
                .context("Failed to read memory/traps.md")?;
            projection::parse_traps(&content)
                .context("Failed to parse memory/traps.md")?
        } else {
            Vec::new()
        }
    };
    if !traps.is_empty() {
        traps_parsed = true;
    }

    Ok(LoadedFiles {
        state,
        decisions,
        traps,
        is_greenfield: false,
        context_parsed,
        decisions_parsed,
        traps_parsed,
    })
}

/// Core flush routine: read-lock state, render all three Markdown files,
/// write to disk.  Used by both the debounced task and `flush_now()`.
///
/// **Parse-guard**: if a file's parse-success flag is `false` and the
/// rendered content is empty, the write is skipped.  This prevents
/// old-format files from being overwritten with empty skeletons when
/// the parser couldn't understand them.  The flag auto-clears when the
/// user commits real data via a corresponding `runtime_commit_event`.
///
/// **Graceful degradation with error reporting**: directory creation
/// failures are fatal (can't proceed without directories).  Individual
/// file write errors are collected — all files are attempted, then a
/// combined error is returned if any failed.
async fn flush_inner(
    state: &Arc<RwLock<RuntimeState>>,
    decisions: &Arc<RwLock<Vec<ADR>>>,
    traps: &Arc<RwLock<Vec<Trap>>>,
    project_root: &PathBuf,
    context_ok: &AtomicBool,
    decisions_ok: &AtomicBool,
    traps_ok: &AtomicBool,
) -> Result<()> {
    let memory_dir = project_root.join("memory");
    let memguard_dir = project_root.join(".memguard");

    // Directory creation is mandatory — bail early on failure.
    tokio::fs::create_dir_all(&memory_dir)
        .await
        .context("Failed to create memory/ directory")?;
    tokio::fs::create_dir_all(&memguard_dir)
        .await
        .context("Failed to create .memguard/ directory")?;

    let mut errors: Vec<String> = Vec::new();

    // ── Write memory/*.md files ─────────────────────────────────────

    // context.md
    {
        let s = state.read().await;
        let rendered = projection::render_context(&s);
        let is_empty = s.current_phase.is_empty()
            && s.active_tasks.is_empty()
            && s.constraints.is_empty();
        if !context_ok.load(Ordering::Acquire) && is_empty {
            eprintln!(
                "[memguard] Skipping context.md flush — file was not successfully parsed and state is empty. Old content preserved."
            );
        } else if let Err(e) =
            tokio::fs::write(memory_dir.join("context.md"), &rendered).await
        {
            errors.push(format!("memory/context.md: {}", e));
        }
    }

    // decisions.md + decisions_archive.md
    {
        let d = decisions.read().await;
        let (active_adrs, stale_adrs): (Vec<ADR>, Vec<ADR>) =
            d.iter().cloned().partition(|adr| adr.status == "active");

        let mut active_rendered = String::new();
        if !stale_adrs.is_empty() {
            active_rendered.push_str("> Historical decisions are in [decisions_archive.md](./decisions_archive.md)\n\n");
        }
        active_rendered.push_str(&projection::render_decisions(&active_adrs));

        if !decisions_ok.load(Ordering::Acquire) && d.is_empty() {
            eprintln!(
                "[memguard] Skipping decisions.md flush — file was not successfully parsed and decisions are empty. Old content preserved."
            );
        } else if let Err(e) =
            tokio::fs::write(memory_dir.join("decisions.md"), &active_rendered).await
        {
            errors.push(format!("memory/decisions.md: {}", e));
        }

        if !stale_adrs.is_empty() {
            let stale_rendered = projection::render_decisions(&stale_adrs);
            if let Err(e) =
                tokio::fs::write(memory_dir.join("decisions_archive.md"), &stale_rendered).await
            {
                errors.push(format!("memory/decisions_archive.md: {}", e));
            }
        }
    }

    // traps.md
    {
        let t = traps.read().await;
        let rendered = projection::render_traps(&t);
        if !traps_ok.load(Ordering::Acquire) && t.is_empty() {
            eprintln!(
                "[memguard] Skipping traps.md flush — file was not successfully parsed and traps are empty. Old content preserved."
            );
        } else if let Err(e) =
            tokio::fs::write(memory_dir.join("traps.md"), &rendered).await
        {
            errors.push(format!("memory/traps.md: {}", e));
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
                    errors.push(format!(".memguard/runtime_state.json: {}", e));
                }
            }
            Err(e) => {
                errors.push(format!(".memguard/runtime_state.json serialize: {}", e));
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
                    "status": a.status,
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
                    errors.push(format!(".memguard/search_index.json: {}", e));
                }
            }
            Err(e) => {
                errors.push(format!(".memguard/search_index.json serialize: {}", e));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "{} flush error(s): {}",
            errors.len(),
            errors.join("; ")
        ))
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
        let project_root = mgr.project_root.read().await.clone();
        let memory_dir = project_root.join("memory");
        assert!(memory_dir.join("context.md").exists());
        assert!(memory_dir.join("decisions.md").exists());
        assert!(memory_dir.join("traps.md").exists());

        // Verify .memguard/ cache was created.
        let cache_dir = project_root.join(".memguard");
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

        // Same id with different content now triggers a conflict error.
        let result = mgr.apply_event(RuntimeEvent::AdrCommitted(adr2)).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.starts_with("[CONFLICT]"));
        assert!(err_msg.contains("ADR-001"));
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
        let project_root = mgr.project_root.read().await.clone();
        let content =
            tokio::fs::read_to_string(project_root.join("memory/context.md"))
                .await
                .unwrap();
        assert!(content.contains("testing"));
        assert!(content.contains("Flush test"));
        assert!(content.contains("Must flush correctly"));
    }

    #[tokio::test]
    async fn test_adr_conflict_active_different_content() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let adr1 = ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        };
        mgr.apply_event(RuntimeEvent::AdrCommitted(adr1))
            .await
            .unwrap();

        let adr2 = ADR {
            id: "ADR-001".into(),
            title: "Use SQLite".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use SQLite for persistence".into(),
            tags: vec![],
        };
        let result = mgr.apply_event(RuntimeEvent::AdrCommitted(adr2)).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.starts_with("[CONFLICT]"));
        assert!(err_msg.contains("ADR-001"));
    }

    #[tokio::test]
    async fn test_adr_idempotent_same_content() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        let adr1 = ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        };
        mgr.apply_event(RuntimeEvent::AdrCommitted(adr1))
            .await
            .unwrap();

        let adr2 = ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        };
        mgr.apply_event(RuntimeEvent::AdrCommitted(adr2))
            .await
            .unwrap();

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].status, "active");
    }

    #[tokio::test]
    async fn test_adr_rejected_repeat() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut decisions = mgr.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Use Cassandra".into(),
                status: "rejected".into(),
                context: "".into(),
                decision: "Use Cassandra for persistence".into(),
                tags: vec![],
            });
        }

        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use Cassandra for persistence".into(),
            tags: vec![],
        };
        let result = mgr.apply_event(RuntimeEvent::AdrCommitted(adr)).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.starts_with("[CONFLICT]"));
        assert!(err_msg.contains("rejected"));
    }

    #[tokio::test]
    async fn test_adr_rejected_different_content_allowed() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut decisions = mgr.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Use Cassandra".into(),
                status: "rejected".into(),
                context: "".into(),
                decision: "Use Cassandra for persistence".into(),
                tags: vec![],
            });
        }

        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra with Sharding".into(),
            status: "".into(),
            context: "New scaling requirements".into(),
            decision: "Use Cassandra with consistent hashing".into(),
            tags: vec![],
        };
        mgr.apply_event(RuntimeEvent::AdrCommitted(adr))
            .await
            .unwrap();

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].status, "superseded");
        assert_eq!(decisions[1].status, "active");
        assert_eq!(decisions[1].title, "Use Cassandra with Sharding");
    }

    #[tokio::test]
    async fn test_adr_superseded_allowed() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut decisions = mgr.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Use MySQL".into(),
                status: "superseded".into(),
                context: "".into(),
                decision: "Use MySQL for persistence".into(),
                tags: vec![],
            });
        }

        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: "".into(),
            context: "".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        };
        mgr.apply_event(RuntimeEvent::AdrCommitted(adr))
            .await
            .unwrap();

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].status, "superseded");
        assert_eq!(decisions[1].status, "active");
        assert_eq!(decisions[1].title, "Use Postgres");
    }

    #[tokio::test]
    async fn test_flush_partitions_stale_adrs() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut decisions = mgr.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Active Choice".into(),
                status: "active".into(),
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec!["a".into()],
            });
            decisions.push(ADR {
                id: "ADR-002".into(),
                title: "Superseded Choice".into(),
                status: "superseded".into(),
                context: "ctx2".into(),
                decision: "dec2".into(),
                tags: vec![],
            });
            decisions.push(ADR {
                id: "ADR-003".into(),
                title: "Rejected Choice".into(),
                status: "rejected".into(),
                context: "ctx3".into(),
                decision: "dec3".into(),
                tags: vec![],
            });
        }

        mgr.flush_now().await.expect("flush should succeed");

        let project_root = mgr.project_root.read().await.clone();
        let memory_dir = project_root.join("memory");

        let dec_content = tokio::fs::read_to_string(memory_dir.join("decisions.md"))
            .await
            .unwrap();
        let archive_content =
            tokio::fs::read_to_string(memory_dir.join("decisions_archive.md"))
                .await
                .unwrap();

        // decisions.md should reference archive and contain only active ADRs
        assert!(dec_content.contains("> Historical decisions are in [decisions_archive.md](./decisions_archive.md)"));
        assert!(dec_content.contains("## ADR-001: Active Choice"));
        assert!(!dec_content.contains("## ADR-002:"));
        assert!(!dec_content.contains("## ADR-003:"));

        // archive should contain stale ADRs
        assert!(archive_content.contains("## ADR-002: Superseded Choice"));
        assert!(archive_content.contains("## ADR-003: Rejected Choice"));
        assert!(!archive_content.contains("## ADR-001:"));

        // parse round-trip
        let active_parsed = projection::parse_decisions(&dec_content).unwrap();
        let stale_parsed = projection::parse_decisions(&archive_content).unwrap();
        assert_eq!(active_parsed.len(), 1);
        assert_eq!(active_parsed[0].id, "ADR-001");
        assert_eq!(stale_parsed.len(), 2);
    }

    #[tokio::test]
    async fn test_load_merges_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory_dir = dir.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();

        let active_md = r##"## ADR-001: Active Choice

**Status:** active

### Context
ctx

### Decision
dec

**Tags:** a
"##;
        let archive_md = r##"## ADR-002: Old Choice

**Status:** superseded

### Context
old ctx

### Decision
old dec
"##;

        tokio::fs::write(memory_dir.join("decisions.md"), active_md)
            .await
            .unwrap();
        tokio::fs::write(memory_dir.join("decisions_archive.md"), archive_md)
            .await
            .unwrap();

        let mgr = StateManager::new(dir.path().to_path_buf());
        mgr.bootstrap().await.expect("bootstrap should succeed");

        let decisions = mgr.decisions.read().await;
        assert_eq!(decisions.len(), 2);
        let ids: Vec<&str> = decisions.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"ADR-001"));
        assert!(ids.contains(&"ADR-002"));
    }

    #[tokio::test]
    async fn test_bootstrap_deduplicates_by_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory_dir = dir.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();

        // decisions.md: ADR-001 active
        let main_md = r##"## ADR-001: Active Choice

**Status:** active

### Context
ctx

### Decision
dec

**Tags:** a
"##;
        // archive: ADR-001 superseded (duplicate ID!)
        let archive_md = r##"## ADR-001: Old Choice

**Status:** superseded

### Context
old ctx

### Decision
old dec

**Tags:** old
"##;

        tokio::fs::write(memory_dir.join("decisions.md"), main_md)
            .await
            .unwrap();
        tokio::fs::write(memory_dir.join("decisions_archive.md"), archive_md)
            .await
            .unwrap();

        let mgr = StateManager::new(dir.path().to_path_buf());
        mgr.bootstrap().await.expect("bootstrap should succeed");

        let decisions = mgr.decisions.read().await;
        // Should deduplicate ADR-001 to a single entry (active has priority)
        assert_eq!(decisions.len(), 1, "ADR-001 should be deduplicated");
        assert_eq!(decisions[0].status, "active", "higher-priority status should win");
    }

    #[tokio::test]
    async fn test_archive_not_created_when_empty() {
        let (_dir, mgr) = setup_temp_dir();
        mgr.bootstrap().await.unwrap();

        {
            let mut decisions = mgr.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Only Active".into(),
                status: "active".into(),
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            });
        }

        mgr.flush_now().await.expect("flush should succeed");

        let project_root = mgr.project_root.read().await.clone();
        let memory_dir = project_root.join("memory");

        assert!(memory_dir.join("decisions.md").exists());
        assert!(!memory_dir.join("decisions_archive.md").exists());
    }

    #[tokio::test]
    async fn test_adr_conflict_checks_all_matching_ids() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());

        // Manually inject duplicate ADR-001 entries:
        // first superseded (simulating archive load), then active.
        {
            let mut decisions = sm.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Old".into(),
                status: "superseded".into(),
                context: "old".into(),
                decision: "old".into(),
                tags: vec![],
            });
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Current".into(),
                status: "active".into(),
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            });
        }

        // Attempt to commit a new ADR-001 with different content.
        let event = RuntimeEvent::AdrCommitted(ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: "active".into(),
            context: "new ctx".into(),
            decision: "new dec".into(),
            tags: vec![],
        });
        let result = sm.apply_event(event).await;

        assert!(
            result.is_err(),
            "should fail with ActiveConflict even when first match is superseded"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.starts_with("[CONFLICT]"), "error should be ActiveConflict: {}", err_msg);
    }
}
