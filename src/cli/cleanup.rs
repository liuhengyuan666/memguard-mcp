use crate::engine::projection;
use crate::models::*;
use std::io::Write;
use std::path::{Path, PathBuf};

// ── Argument parsing ───────────────────────────────────────────────────────

pub struct CleanupArgs {
    pub dry_run: bool,
    pub project_root: PathBuf,
    pub no_backup: bool,
}

impl CleanupArgs {
    /// Parse CLI args from `std::env::args()`.
    /// Expects argv[1] to already be consumed as "cleanup".
    /// Supports: `--dry-run`, `--project-root <path>`.
    pub fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut dry_run = false;
        let mut no_backup = false;
        let mut project_root =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let mut i = 2; // skip program name and "cleanup"
        while i < args.len() {
            match args[i].as_str() {
                "--dry-run" => dry_run = true,
                "--no-backup" => no_backup = true,
                "--project-root" if i + 1 < args.len() => {
                    project_root = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }

        CleanupArgs { dry_run, project_root, no_backup }
    }
}

// ── Issue detection ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CleanupIssues {
    /// Done tasks found in active_tasks
    pub completed_tasks: Vec<Task>,
    /// Duplicate ADRs: (keep, supersede)
    pub duplicate_pairs: Vec<(ADR, ADR)>,
    /// ADRs with stale status found in decisions.md
    pub stale_adrs_in_main: Vec<ADR>,
    /// Warnings about ADRs with identical content_hash but different IDs
    pub content_hash_warnings: Vec<String>,
}

impl CleanupIssues {
    pub fn is_empty(&self) -> bool {
        self.completed_tasks.is_empty()
            && self.duplicate_pairs.is_empty()
            && self.stale_adrs_in_main.is_empty()
            && self.content_hash_warnings.is_empty()
    }
}

fn adr_status_priority(s: &AdrStatus) -> u8 {
    match s {
        AdrStatus::Accepted => 5,
        AdrStatus::Proposed => 4,
        AdrStatus::Rejected => 3,
        AdrStatus::Superseded => 2,
        AdrStatus::Archived => 1,
    }
}

/// Analyze loaded memory state for cleanup opportunities.
pub fn analyze(
    state: &RuntimeState,
    main_adrs: &[ADR],
    archive_adrs: &[ADR],
) -> CleanupIssues {
    use crate::engine::validators::content_hash;

    let mut issues = CleanupIssues {
        completed_tasks: state
            .active_tasks
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Done | TaskStatus::Superseded | TaskStatus::Cancelled
                )
            })
            .cloned()
            .collect(),
        ..Default::default()
    };

    // ── 2. Duplicate ADRs: same title + same decision content ─────
    let all_adrs: Vec<&ADR> = main_adrs.iter().chain(archive_adrs.iter()).collect();
    for i in 0..all_adrs.len() {
        for j in (i + 1)..all_adrs.len() {
            let a = all_adrs[i];
            let b = all_adrs[j];
            if a.title == b.title && a.decision == b.decision && a.id != b.id {
                // Keep the one with higher status priority.
                let (keep, supersede) =
                    if adr_status_priority(&a.status) >= adr_status_priority(&b.status) {
                        ((*a).clone(), (*b).clone())
                    } else {
                        ((*b).clone(), (*a).clone())
                    };
                issues.duplicate_pairs.push((keep, supersede));
            }
        }
    }

    // ── 2b. Content-hash warnings: same hash, different ID ────────
    let all_adrs: Vec<&ADR> = main_adrs.iter().chain(archive_adrs.iter()).collect();
    for i in 0..all_adrs.len() {
        for j in (i + 1)..all_adrs.len() {
            let a = all_adrs[i];
            let b = all_adrs[j];
            if a.id != b.id && content_hash(a) == content_hash(b) {
                issues.content_hash_warnings.push(format!(
                    "ADR {} and {} have identical content_hash",
                    a.id, b.id
                ));
            }
        }
    }

    // ── 3. Stale ADRs in main decisions.md ────────────────────────
    issues.stale_adrs_in_main = main_adrs
        .iter()
        .filter(|a| matches!(a.status, AdrStatus::Superseded | AdrStatus::Rejected | AdrStatus::Archived))
        .cloned()
        .collect();

    issues
}

// ── Report rendering ───────────────────────────────────────────────────────

/// Generate a human-readable report from CleanupIssues.
pub fn render_report(issues: &CleanupIssues) -> String {
    let mut report = String::new();

    report.push_str("Found:\n");
    if !issues.completed_tasks.is_empty() {
        report.push_str(&format!(
            "  {} completed task{}\n",
            issues.completed_tasks.len(),
            if issues.completed_tasks.len() == 1 { "" } else { "s" }
        ));
    }
    if !issues.duplicate_pairs.is_empty() {
        report.push_str(&format!(
            "  {} duplicate ADR{}\n",
            issues.duplicate_pairs.len(),
            if issues.duplicate_pairs.len() == 1 { "" } else { "s" }
        ));
    }
    if !issues.stale_adrs_in_main.is_empty() {
        report.push_str(&format!(
            "  {} stale ADR{} in decisions.md\n",
            issues.stale_adrs_in_main.len(),
            if issues.stale_adrs_in_main.len() == 1 { "" } else { "s" }
        ));
    }

    if issues.is_empty() {
        report.push_str("  No issues found.\n");
        return report;
    }

    if !issues.content_hash_warnings.is_empty() {
        report.push_str("\nWarnings:\n");
        for w in &issues.content_hash_warnings {
            report.push_str(&format!("  {}\n", w));
        }
    }

    report.push_str("\nSuggested:\n");
    if !issues.completed_tasks.is_empty() {
        report.push_str(&format!(
            "  Archive {} completed task{}\n",
            issues.completed_tasks.len(),
            if issues.completed_tasks.len() == 1 { "" } else { "s" }
        ));
    }
    for (keep, supersede) in &issues.duplicate_pairs {
        report.push_str(&format!(
            "  Supersede {} (keep {})\n",
            supersede.id, keep.id
        ));
    }
    for adr in &issues.stale_adrs_in_main {
        report.push_str(&format!("  Move {} to archive\n", adr.id));
    }

    report
}

// ── Interactive confirmation ───────────────────────────────────────────────

fn prompt_confirm() -> bool {
    print!("\nContinue? (y/N): ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Compute today's date as YYYY-MM-DD using Howard Hinnant's civil date algorithm.
fn today_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

// ── Backup ───────────────────────────────────────────────────────────────────

/// Generate a compact backup timestamp: YYYYMMDD-HHMMSS.
fn backup_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Reuse today_date() for YYYY-MM-DD, compact to YYYYMMDD
    let date = today_date();
    let date_compact = date.replace('-', "");
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;
    format!("{}-{:02}{:02}{:02}", date_compact, hh, mm, ss)
}

/// Create a backup snapshot of memory/*.md and .memguard/*.json files
/// into `.memguard/backups/<timestamp>/`. Returns the backup directory path.
fn create_backup(project_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let timestamp = backup_timestamp();
    let backup_dir = project_root
        .join(".memguard")
        .join("backups")
        .join(&timestamp);
    std::fs::create_dir_all(&backup_dir)?;

    // Copy memory/*.md files that exist
    let memory_dir = project_root.join("memory");
    for filename in &[
        "context.md",
        "decisions.md",
        "traps.md",
        "tasks_archive.md",
        "decisions_archive.md",
    ] {
        let src = memory_dir.join(filename);
        if src.exists() {
            std::fs::copy(&src, backup_dir.join(filename))?;
        }
    }

    // Copy .memguard/*.json files that exist
    let memguard_dir = project_root.join(".memguard");
    for filename in &["runtime_state.json", "search_index.json"] {
        let src = memguard_dir.join(filename);
        if src.exists() {
            std::fs::copy(&src, backup_dir.join(filename))?;
        }
    }

    // Compute time-of-day for the manifest iso-ish timestamp
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    let manifest = serde_json::json!({
        "created_at": format!("{}T{:02}:{:02}:{:02}Z", today_date(), hh, mm, ss),
        "version": "memguard-v4",
        "reason": "cleanup"
    });
    std::fs::write(
        backup_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    Ok(backup_dir)
}

// ── Apply changes ──────────────────────────────────────────────────────────

/// Write cleanup changes to disk. Assumes confirmation has already been obtained.
fn apply_changes(
    project_root: &Path,
    state: &RuntimeState,
    main_adrs: &[ADR],
    archive_adrs: &[ADR],
    issues: &CleanupIssues,
) -> Result<(), Box<dyn std::error::Error>> {
    let memory_dir = project_root.join("memory");

    // 1. Clean context.md: remove Done tasks from active_tasks.
    {
        let cleaned = RuntimeState {
            current_phase: state.current_phase.clone(),
            active_tasks: state
                .active_tasks
                .iter()
                .filter(|t| !matches!(t.status, TaskStatus::Done))
                .cloned()
                .collect(),
            done_tasks: Vec::new(),
            constraints: state.constraints.clone(),
        };
        let rendered = projection::render_context(&cleaned);
        std::fs::write(memory_dir.join("context.md"), &rendered)?;
    }

    // 2. Archive completed tasks to tasks_archive.md.
    if !issues.completed_tasks.is_empty() {
        let archive_path = memory_dir.join("tasks_archive.md");
        let existing = if archive_path.exists() {
            std::fs::read_to_string(&archive_path).unwrap_or_default()
        } else {
            String::new()
        };
        let appended =
            projection::append_tasks_archive(&existing, &issues.completed_tasks, &today_date());
        std::fs::write(&archive_path, &appended)?;
    }

    // 3. Re-render decisions.md + decisions_archive.md with corrected statuses.
    {
        let mut all_adrs: Vec<ADR> = main_adrs.to_vec();
        // Remove stale ADRs from main (they are already identified in issues).
        all_adrs.retain(|a| matches!(a.status, AdrStatus::Accepted | AdrStatus::Proposed));

        let mut stale_adrs: Vec<ADR> = issues.stale_adrs_in_main.clone();
        // Start with existing archive ADRs, but exclude any that we are
        // superseding (their replacement is in active_adrs).
        for a in archive_adrs {
            let is_superseded = issues
                .duplicate_pairs
                .iter()
                .any(|(_, sup)| sup.id == a.id && sup.title == a.title);
            if !is_superseded {
                stale_adrs.push(a.clone());
            }
        }

        // Add superseded duplicates to stale.
        for (_keep, supersede) in &issues.duplicate_pairs {
            // Only add if not already present.
            if !stale_adrs.iter().any(|a| a.id == supersede.id && a.title == supersede.title) {
                let mut sup = supersede.clone();
                sup.status = AdrStatus::Superseded;
                stale_adrs.push(sup);
            }
        }

        // Write decisions.md with active ADRs.
        let mut active_md = String::new();
        if !stale_adrs.is_empty() {
            active_md.push_str(
                "> Historical decisions are in [decisions_archive.md](./decisions_archive.md)\n\n",
            );
        }
        active_md.push_str(&projection::render_decisions(&all_adrs));
        std::fs::write(memory_dir.join("decisions.md"), &active_md)?;

        // Write decisions_archive.md with stale ADRs.
        if !stale_adrs.is_empty() {
            let stale_md = projection::render_decisions(&stale_adrs);
            std::fs::write(memory_dir.join("decisions_archive.md"), &stale_md)?;
        }
    }

    Ok(())
}

// ── Main entry point ───────────────────────────────────────────────────────

/// Run the full cleanup pipeline. `force_confirm` overrides interactive prompt:
/// `Some(true)` auto-confirms, `Some(false)` auto-cancels, `None` uses stdin.
pub fn run_cleanup_inner(
    args: &CleanupArgs,
    force_confirm: Option<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = &args.project_root;
    let memory_dir = root.join("memory");

    // ── Load files ─────────────────────────────────────────────────
    let (context_md, context_existed) = read_opt(&memory_dir.join("context.md"))?;
    let state = if context_existed {
        projection::parse_context(&context_md)?
    } else {
        println!("No issues found.\n(memory/context.md does not exist)");
        return Ok(());
    };

    let (decisions_md, _) = read_opt(&memory_dir.join("decisions.md"))?;
    let main_adrs = projection::parse_decisions(&decisions_md)?;

    let (archive_md, _) = read_opt(&memory_dir.join("decisions_archive.md"))?;
    let archive_adrs = projection::parse_decisions(&archive_md)?;

    let (traps_md, _) = read_opt(&memory_dir.join("traps.md"))?;
    let traps = projection::parse_traps(&traps_md)?;

    // ── Analyze ────────────────────────────────────────────────────
    let issues = analyze(&state, &main_adrs, &archive_adrs);

    // ── Report ─────────────────────────────────────────────────────
    print!("{}", render_report(&issues));

    // ── Dry run exit ───────────────────────────────────────────────
    if args.dry_run {
        println!("\n[Dry-run mode] No changes written.");
        return Ok(());
    }

    // Even when no issues are found, rebuild cache files to ensure they stay
    // in sync with markdown (e.g. after a format upgrade).
    if issues.is_empty() {
        rebuild_cache_files(root, &state, memory_dir, &traps)?;
        return Ok(());
    }

    // ── Confirm ────────────────────────────────────────────────────
    let confirmed = match force_confirm {
        Some(v) => v,
        None => prompt_confirm(),
    };

    if !confirmed {
        println!("Cancelled. No changes made.");
        return Ok(());
    }

    // ── Apply ──────────────────────────────────────────────────────
    // Backup snapshot before making destructive changes
    if !args.no_backup {
        match create_backup(root) {
            Ok(dir) => println!("\nBackup created at {}", dir.display()),
            Err(e) => eprintln!("\nWARNING: Backup failed: {}", e),
        }
    }

    println!("\nApplying changes...");
    apply_changes(root, &state, &main_adrs, &archive_adrs, &issues)?;
    println!("Done.");

    rebuild_cache_files(root, &state, memory_dir, &traps)?;

    Ok(())
}

/// Public entry point with interactive confirmation.
pub fn run_cleanup(args: &CleanupArgs) -> Result<(), Box<dyn std::error::Error>> {
    run_cleanup_inner(args, None)
}

/// Rebuild runtime_state.json and search_index.json from cleaned markdown.
/// Called both when issues are found (after apply) and when issues are empty
/// (to ensure cache stays in sync).
fn rebuild_cache_files(
    root: &Path,
    state: &RuntimeState,
    memory_dir: PathBuf,
    traps: &[Trap],
) -> Result<(), Box<dyn std::error::Error>> {
    let memguard_dir = root.join(".memguard");
    std::fs::create_dir_all(&memguard_dir)?;

    // 1. Rebuild runtime_state.json from cleaned state
    let cleaned_state = RuntimeState {
        current_phase: state.current_phase.clone(),
        active_tasks: state
            .active_tasks
            .iter()
            .filter(|t| !matches!(t.status, TaskStatus::Done))
            .cloned()
            .collect(),
        done_tasks: Vec::new(),
        constraints: state.constraints.clone(),
    };
    let runtime_json = serde_json::to_string_pretty(&cleaned_state)?;
    std::fs::write(memguard_dir.join("runtime_state.json"), &runtime_json)?;
    println!("Rebuilt runtime_state.json");

    // 2. Rebuild search_index.json from updated ADRs + traps
    let updated_main = projection::parse_decisions(
        &std::fs::read_to_string(memory_dir.join("decisions.md"))?,
    )?;
    let archive_md =
        std::fs::read_to_string(memory_dir.join("decisions_archive.md")).unwrap_or_default();
    let updated_archive = projection::parse_decisions(&archive_md)?;
    let all_adrs: Vec<ADR> =
        updated_main.into_iter().chain(updated_archive.into_iter()).collect();
    let index = crate::search::index::SearchIndex::build(&all_adrs, traps);
    let index_json = index.to_index_json(&all_adrs, traps);
    std::fs::write(
        memguard_dir.join("search_index.json"),
        serde_json::to_string_pretty(&index_json)?,
    )?;
    println!("Rebuilt search_index.json");

    // Concurrent MCP warning (still relevant because in-memory state may differ)
    eprintln!(
        "\n[INFO] If MemGuard MCP is currently running, restart it to pick up cleaned state."
    );

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Read file contents, returning (contents, existed).
fn read_opt(path: &PathBuf) -> Result<(String, bool), std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok((s, true)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), false)),
        Err(e) => Err(e),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Unit tests for analyze() ───────────────────────────────────────

    fn make_adr(id: &str, title: &str, status: AdrStatus, decision: &str) -> ADR {
        ADR {
            id: id.to_string(),
            title: title.to_string(),
            status,
            context: "test context".to_string(),
            decision: decision.to_string(),
            tags: vec![],
        }
    }

    fn make_task(id: &str, desc: &str, status: TaskStatus) -> Task {
        Task { id: id.to_string(), description: desc.to_string(), status, superseded_by: None }
    }

    #[test]
    fn test_analyze_no_issues() {
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![make_task("TASK-001", "Active", TaskStatus::Todo)],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[], &[]);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_analyze_completed_tasks() {
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![
                make_task("TASK-001", "Active", TaskStatus::Todo),
                make_task("TASK-002", "Done 1", TaskStatus::Done),
                make_task("TASK-003", "Done 2", TaskStatus::Done),
            ],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[], &[]);
        assert_eq!(issues.completed_tasks.len(), 2);
        assert_eq!(issues.completed_tasks[0].id, "TASK-002");
        assert_eq!(issues.completed_tasks[1].id, "TASK-003");
    }

    #[test]
    fn test_analyze_duplicate_adrs_same_title_decision() {
        let adr1 = make_adr("ADR-001", "Use Rust", AdrStatus::Accepted, "We decided to use Rust.");
        let adr2 = make_adr("ADR-005", "Use Rust", AdrStatus::Proposed, "We decided to use Rust.");
        // ADR-001 (Accepted priority=5) vs ADR-005 (Proposed priority=4)
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[adr1], &[adr2]);
        assert_eq!(issues.duplicate_pairs.len(), 1);
        let (keep, supersede) = &issues.duplicate_pairs[0];
        assert_eq!(keep.id, "ADR-001"); // higher priority kept
        assert_eq!(supersede.id, "ADR-005");
    }

    #[test]
    fn test_analyze_duplicate_adrs_same_priority() {
        let adr1 = make_adr("ADR-001", "Use Rust", AdrStatus::Proposed, "Rust.");
        let adr2 = make_adr("ADR-005", "Use Rust", AdrStatus::Proposed, "Rust.");
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[adr1], &[adr2]);
        assert_eq!(issues.duplicate_pairs.len(), 1);
        let (keep, _) = &issues.duplicate_pairs[0];
        assert_eq!(keep.id, "ADR-001"); // first in list kept when equal
    }

    #[test]
    fn test_analyze_no_duplicate_if_different_decision() {
        let adr1 = make_adr("ADR-001", "Use Rust", AdrStatus::Accepted, "Rust with Tokio.");
        let adr2 = make_adr("ADR-005", "Use Rust", AdrStatus::Accepted, "Rust with Actix.");
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[adr1], &[adr2]);
        assert!(issues.duplicate_pairs.is_empty());
    }

    #[test]
    fn test_analyze_stale_adrs_in_main() {
        let adr1 = make_adr("ADR-001", "Use Rust", AdrStatus::Accepted, "Rust.");
        let adr2 = make_adr("ADR-002", "Old decision", AdrStatus::Superseded, "Old stuff.");
        let adr3 = make_adr("ADR-003", "Rejected", AdrStatus::Rejected, "Bad idea.");
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        // main_adrs has the stale ones, archive_adrs is empty
        let issues = analyze(&state, &[adr1, adr2, adr3], &[]);
        assert_eq!(issues.stale_adrs_in_main.len(), 2);
        assert_eq!(issues.stale_adrs_in_main[0].id, "ADR-002");
        assert_eq!(issues.stale_adrs_in_main[1].id, "ADR-003");
    }

    #[test]
    fn test_analyze_archive_adrs_not_flagged_as_stale() {
        let active = make_adr("ADR-001", "Use Rust", AdrStatus::Accepted, "Rust.");
        let archived = make_adr("ADR-002", "Old stuff", AdrStatus::Superseded, "Old.");
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        // archived ADR is in archive, not in main — should not be flagged
        let issues = analyze(&state, &[active.clone()], &[archived]);
        assert!(issues.stale_adrs_in_main.is_empty());
    }

    #[test]
    fn test_analyze_content_hash_warning() {
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        let adr1 = make_adr("ADR-032", "Same content", AdrStatus::Accepted, "Decision A");
        let adr2 = make_adr("ADR-033", "Same content", AdrStatus::Accepted, "Decision A");
        let issues = analyze(&state, &[adr1, adr2], &[]);
        assert_eq!(issues.content_hash_warnings.len(), 1);
        assert!(issues.content_hash_warnings[0].contains("ADR-032"));
        assert!(issues.content_hash_warnings[0].contains("ADR-033"));
    }

    #[test]
    fn test_analyze_stale_explicit_only() {
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        };
        let accepted = make_adr("ADR-001", "Accepted", AdrStatus::Accepted, "Decision.");
        let proposed = make_adr("ADR-002", "Proposed", AdrStatus::Proposed, "Decision.");
        let superseded = make_adr("ADR-003", "Superseded", AdrStatus::Superseded, "Decision.");
        let rejected = make_adr("ADR-004", "Rejected", AdrStatus::Rejected, "Decision.");
        let archived = make_adr("ADR-005", "Archived", AdrStatus::Archived, "Decision.");
        let main_adrs = vec![accepted, proposed, superseded, rejected, archived];
        let issues = analyze(&state, &main_adrs, &[]);
        // Only Superseded, Rejected, Archived should be flagged
        assert_eq!(issues.stale_adrs_in_main.len(), 3);
        let stale_ids: Vec<&str> = issues.stale_adrs_in_main.iter().map(|a| a.id.as_str()).collect();
        assert!(stale_ids.contains(&"ADR-003"));
        assert!(stale_ids.contains(&"ADR-004"));
        assert!(stale_ids.contains(&"ADR-005"));
        assert!(!stale_ids.contains(&"ADR-001"));
        assert!(!stale_ids.contains(&"ADR-002"));
    }

    #[test]
    fn test_analyze_blocked_tasks_not_flagged() {
        let state = RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![
                make_task("TASK-001", "Blocked task", TaskStatus::Blocked),
                make_task("TASK-002", "In progress", TaskStatus::InProgress),
            ],
            done_tasks: vec![],
            constraints: vec![],
        };
        let issues = analyze(&state, &[], &[]);
        assert!(issues.completed_tasks.is_empty());
    }

    // ── Report rendering tests ───────────────────────────────────────

    #[test]
    fn test_render_report_no_issues() {
        let issues = CleanupIssues::default();
        let report = render_report(&issues);
        assert!(report.contains("No issues found"));
    }

    #[test]
    fn test_render_report_with_issues() {
        let mut issues = CleanupIssues::default();
        issues.completed_tasks = vec![make_task("TASK-001", "done", TaskStatus::Done)];
        issues.duplicate_pairs = vec![(
            make_adr("ADR-001", "Keep", AdrStatus::Accepted, "dec"),
            make_adr("ADR-002", "Keep", AdrStatus::Proposed, "dec"),
        )];
        let report = render_report(&issues);
        assert!(report.contains("Found:"));
        assert!(report.contains("1 completed task"));
        assert!(report.contains("1 duplicate ADR"));
        assert!(report.contains("Supersede ADR-002 (keep ADR-001)"));
    }

    #[test]
    fn test_render_report_plural() {
        let mut issues = CleanupIssues::default();
        issues.completed_tasks = vec![
            make_task("TASK-001", "a", TaskStatus::Done),
            make_task("TASK-002", "b", TaskStatus::Done),
        ];
        issues.stale_adrs_in_main = vec![
            make_adr("ADR-001", "S1", AdrStatus::Superseded, "d1"),
            make_adr("ADR-002", "S2", AdrStatus::Rejected, "d2"),
        ];
        let report = render_report(&issues);
        assert!(report.contains("2 completed tasks"));
        assert!(report.contains("2 stale ADRs in decisions.md"));
    }

    // ── Integration tests: dry-run with temp directory ─────────────────

    fn setup_memory_dir(dir: &std::path::Path) {
        let memory = dir.join("memory");
        std::fs::create_dir_all(&memory).unwrap();

        // context.md with a Done task
        let context_md = concat!(
            "# Current Phase\nplan\n\n",
            "# Active Tasks\n",
            "- [Todo] [TASK-001] Active task\n",
            "- [Done] [TASK-002] Completed task\n\n",
            "# Constraints\n",
            "- Keep it simple\n"
        );
        std::fs::write(memory.join("context.md"), context_md).unwrap();

        // decisions.md with one active and one stale ADR
        let decisions_md = concat!(
            "## ADR-001: Use Rust\n\n",
            "**Status:** Accepted\n\n",
            "### Context\nTest.\n\n",
            "### Decision\nUse Rust.\n\n",
            "**Tags:** rust\n\n",
            "## ADR-002: Old stuff\n\n",
            "**Status:** Superseded\n\n",
            "### Context\nOld.\n\n",
            "### Decision\nOld decision.\n\n",
        );
        std::fs::write(memory.join("decisions.md"), decisions_md).unwrap();
    }

    #[test]
    fn test_cleanup_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());

        let args = CleanupArgs {
            dry_run: true,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        // Capture stdout to check report
        let result = run_cleanup_inner(&args, Some(true));
        assert!(result.is_ok());

        // Verify context.md still has the Done task (dry run, no changes)
        let context_md =
            std::fs::read_to_string(dir.path().join("memory").join("context.md")).unwrap();
        assert!(context_md.contains("[Done] [TASK-002]"));

        // Verify decisions.md still has ADR-002 (stale ADR)
        let decisions_md =
            std::fs::read_to_string(dir.path().join("memory").join("decisions.md")).unwrap();
        assert!(decisions_md.contains("ADR-002: Old stuff"));

        // Verify no archive files were created
        assert!(!dir.path().join("memory").join("tasks_archive.md").exists());
    }

    #[test]
    fn test_cleanup_cancel() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        // force_confirm = Some(false) simulates user typing "n"
        let result = run_cleanup_inner(&args, Some(false));
        assert!(result.is_ok());

        // Verify content unchanged
        let context_md =
            std::fs::read_to_string(dir.path().join("memory").join("context.md")).unwrap();
        assert!(context_md.contains("[Done] [TASK-002]"));

        assert!(!dir.path().join("memory").join("tasks_archive.md").exists());
    }

    #[test]
    fn test_cleanup_apply() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        // force_confirm = Some(true) auto-confirms
        let result = run_cleanup_inner(&args, Some(true));
        assert!(result.is_ok());

        // Verify Done task removed from context.md
        let context_md =
            std::fs::read_to_string(dir.path().join("memory").join("context.md")).unwrap();
        assert!(!context_md.contains("[Done] [TASK-002]"));
        assert!(context_md.contains("[Todo] [TASK-001]"));

        // Verify Done task archived
        let archive_md =
            std::fs::read_to_string(dir.path().join("memory").join("tasks_archive.md")).unwrap();
        assert!(archive_md.contains("[Done] [TASK-002] Completed task"));

        // Verify stale ADR moved from decisions.md to archive
        let main_md =
            std::fs::read_to_string(dir.path().join("memory").join("decisions.md")).unwrap();
        assert!(!main_md.contains("ADR-002: Old stuff"));
        assert!(main_md.contains("ADR-001: Use Rust"));

        let archive_dec =
            std::fs::read_to_string(dir.path().join("memory").join("decisions_archive.md")).unwrap();
        assert!(archive_dec.contains("ADR-002: Old stuff"));
    }

    #[test]
    fn test_cleanup_no_memory_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Don't create memory/ directory

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        let result = run_cleanup_inner(&args, Some(true));
        // Should succeed gracefully, reporting no issues
        assert!(result.is_ok());
    }

    // ── Arg parsing tests ───────────────────────────────────────────
    // Note: these cannot directly test parse() since it reads real env::args.
    // The logic is tested implicitly through the integration tests above.

    // ── Idempotency ──────────────────────────────────────────────────

    #[test]
    fn test_cleanup_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        // First cleanup: archives the Done task
        let result = run_cleanup_inner(&args, Some(true));
        assert!(result.is_ok());

        // Verify Done task removed from context
        let context_md =
            std::fs::read_to_string(dir.path().join("memory").join("context.md")).unwrap();
        assert!(!context_md.contains("[Done] [TASK-002]"));

        // Verify Done task archived once
        let archive1 =
            std::fs::read_to_string(dir.path().join("memory").join("tasks_archive.md")).unwrap();
        let count1 = archive1.matches("[Done] [TASK-002]").count();
        assert_eq!(count1, 1, "Done task should appear exactly once after first cleanup");

        // Second cleanup: no issues, no changes
        let result2 = run_cleanup_inner(&args, Some(true));
        assert!(result2.is_ok());

        // Verify archive hasn't changed (no duplicate append)
        let archive2 =
            std::fs::read_to_string(dir.path().join("memory").join("tasks_archive.md")).unwrap();
        assert_eq!(
            archive1, archive2,
            "Second cleanup should not modify tasks_archive.md"
        );
    }

    // ── No resurrection ──────────────────────────────────────────────

    #[test]
    fn test_cleanup_no_resurrection() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        run_cleanup_inner(&args, Some(true)).unwrap();

        // Re-parse context.md — Done task must NOT be in active_tasks
        let context_md =
            std::fs::read_to_string(dir.path().join("memory").join("context.md")).unwrap();
        let state = projection::parse_context(&context_md).unwrap();
        assert!(
            !state.active_tasks.iter().any(|t| t.id == "TASK-002"),
            "Done task should not be resurrected in active_tasks"
        );

        // Verify Done task IS in archive
        let archive_md =
            std::fs::read_to_string(dir.path().join("memory").join("tasks_archive.md")).unwrap();
        assert!(
            archive_md.contains("[Done] [TASK-002]"),
            "Done task should be present in tasks_archive.md"
        );
    }

    // ── Backup tests ─────────────────────────────────────────────────

    fn setup_memguard_dir(dir: &std::path::Path) {
        let memguard = dir.join(".memguard");
        std::fs::create_dir_all(&memguard).unwrap();
        // Write dummy cache files
        std::fs::write(
            memguard.join("runtime_state.json"),
            r#"{"version":"test"}"#,
        )
        .unwrap();
        std::fs::write(
            memguard.join("search_index.json"),
            r#"{"index":"test"}"#,
        )
        .unwrap();
    }

    #[test]
    fn test_cleanup_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());
        setup_memguard_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: false,
        };

        run_cleanup_inner(&args, Some(true)).unwrap();

        // Verify backup directory exists
        let backups_dir = dir.path().join(".memguard").join("backups");
        assert!(backups_dir.exists(), "backups directory should exist");

        // Find the timestamped backup subdirectory
        let entries: Vec<_> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one backup directory should exist");

        let backup_dir = entries[0].path();
        let dir_name = backup_dir.file_name().unwrap().to_str().unwrap();
        // Verify format: YYYYMMDD-HHMMSS
        assert!(
            dir_name.len() == 15 && dir_name.contains('-') && dir_name[8..9] == *"-",
            "backup dir name should be YYYYMMDD-HHMMSS format, got: {}",
            dir_name
        );

        // Verify manifest.json exists and has correct fields
        let manifest_path = backup_dir.join("manifest.json");
        assert!(manifest_path.exists(), "manifest.json should exist");
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["version"], "memguard-v4");
        assert_eq!(manifest["reason"], "cleanup");
        assert!(manifest["created_at"].as_str().unwrap().contains('T'));

        // Verify context.md backup matches original (before cleanup modified it)
        // We can't compare perfectly since the original has been modified, but
        // the backup should contain the original Done task line
        let backup_context = backup_dir.join("context.md");
        assert!(backup_context.exists(), "context.md backup should exist");
        let backup_content = std::fs::read_to_string(&backup_context).unwrap();
        assert!(
            backup_content.contains("[Done] [TASK-002]"),
            "backup context.md should preserve original Done task"
        );
    }

    #[test]
    fn test_cleanup_no_backup_flag() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());
        setup_memguard_dir(dir.path());

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        run_cleanup_inner(&args, Some(true)).unwrap();

        // Verify NO backup directory created
        let backups_dir = dir.path().join(".memguard").join("backups");
        assert!(
            !backups_dir.exists(),
            "backups directory should NOT exist when --no-backup is set"
        );
    }

    // ── Cache deletion ───────────────────────────────────────────────

    #[test]
    fn test_cleanup_rebuilds_cache_files() {
        let dir = tempfile::tempdir().unwrap();
        setup_memory_dir(dir.path());
        setup_memguard_dir(dir.path());

        // Verify cache files exist before cleanup
        let runtime_path = dir.path().join(".memguard").join("runtime_state.json");
        let index_path = dir.path().join(".memguard").join("search_index.json");
        assert!(runtime_path.exists(), "runtime_state.json should exist before cleanup");
        assert!(index_path.exists(), "search_index.json should exist before cleanup");

        let args = CleanupArgs {
            dry_run: false,
            project_root: dir.path().to_path_buf(),
            no_backup: true,
        };

        run_cleanup_inner(&args, Some(true)).unwrap();

        // Verify cache files are rebuilt (not deleted)
        assert!(
            runtime_path.exists(),
            "runtime_state.json should be rebuilt after cleanup"
        );
        assert!(
            index_path.exists(),
            "search_index.json should be rebuilt after cleanup"
        );

        // Verify rebuilt runtime_state.json has no Done tasks
        let runtime_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&runtime_path).unwrap()).unwrap();
        let active_tasks = runtime_json["active_tasks"].as_array().unwrap();
        assert!(
            active_tasks.iter().all(|t| t["status"] != "Done"),
            "rebuilt runtime_state should not contain Done tasks"
        );

        // Verify rebuilt search_index.json has valid terms
        let index_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
        assert!(index_json.get("terms").is_some(), "rebuilt index should have terms");
        assert!(
            index_json.get("metadata").is_some(),
            "rebuilt index should have metadata"
        );
    }
}
