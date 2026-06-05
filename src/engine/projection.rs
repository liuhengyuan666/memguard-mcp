use crate::models::*;
use anyhow::{anyhow, Result};
use regex::Regex;
use std::collections::HashSet;

// ── context.md ↔ RuntimeState ──────────────────────────────────────────────

/// Parse context.md content into RuntimeState.
///
/// Supports both new format (English H1) and legacy format (Chinese H2
/// under an outer H1).  Render always outputs the new format.
///
/// New format:
/// ```markdown
/// # Current Phase
/// {phase}
///
/// # Active Tasks
/// - [Todo] [TASK-XXX] {description}
/// - [InProgress|Done] [TASK-XXX] {description}
///
/// # Constraints
/// - {constraint}
/// ```
///
/// Legacy format (v2 skill):
/// ```markdown
/// # Current Context
///
/// ## 当前阶段
/// ...
///
/// ## 关键任务
/// ...
///
/// ## 当前约束
/// ...
/// ```
pub fn parse_context(md: &str) -> Result<RuntimeState> {
    let mut current_phase: Option<String> = None;
    let mut tasks: Vec<Task> = Vec::new();
    let mut constraints: Vec<String> = Vec::new();

    // Try H1 sections first (new format).  If no H1 sections are found,
    // fall back to H2 sections (legacy format with outer H1 wrapper).
    let h1_sections: Vec<_> = md.split("\n# ").collect();
    let h2_sections: Vec<_> = md.split("\n## ").collect();

    let sections: Vec<_> = if h1_sections.len() > 1 {
        h1_sections.into_iter().collect()
    } else if h2_sections.len() > 1 {
        h2_sections.into_iter().collect()
    } else {
        vec![md]
    };

    for section in sections {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }

        // Match section name against English and Chinese aliases.
        // `match_section_title` strips the title and returns the body text.
        if let Some(rest) = match_section_title(section, &["Current Phase", "当前阶段", "阶段"]) {
            current_phase = Some(canonicalize_phase(&extract_section_body(rest)));
        } else if let Some(rest) = match_section_title(section, &[
            "Active Tasks", "关键任务", "当前任务", "任务",
        ]) {
            tasks = parse_task_lines(rest);
        } else if let Some(rest) = match_section_title(section, &[
            "Constraints", "当前约束", "约束条件", "约束",
        ]) {
            constraints = parse_bullet_list(rest);
        }
    }

    let current_phase = current_phase.ok_or_else(|| {
        anyhow!(
            "Missing 'Current Phase' / '当前阶段' section in context.md"
        )
    })?;

    Ok(RuntimeState {
        current_phase,
        active_tasks: tasks,
        done_tasks: Vec::new(),
        constraints,
    })
}

/// Try to strip a section title (with optional leading "# " or "## ") from the
/// beginning of `section`.  Returns the remaining text after the title.
fn match_section_title<'a>(section: &'a str, titles: &[&str]) -> Option<&'a str> {
    let after_header = section
        .strip_prefix("## ")
        .or_else(|| section.strip_prefix("# "))
        .unwrap_or(section);
    for t in titles {
        if let Some(rest) = after_header.strip_prefix(t) {
            return Some(rest);
        }
    }
    None
}

/// Render RuntimeState back to context.md Markdown.
pub fn render_context(state: &RuntimeState) -> String {
    let mut md = String::new();

    md.push_str("# Current Phase\n");
    md.push_str(&state.current_phase);
    md.push_str("\n\n");

    md.push_str("# Active Tasks\n");
    if state.active_tasks.is_empty() {
        md.push('\n');
    } else {
        for task in &state.active_tasks {
            if matches!(task.status, TaskStatus::Done) {
                continue;
            }
            let status = match task.status {
                TaskStatus::Todo => "Todo",
                TaskStatus::InProgress => "InProgress",
                TaskStatus::Done => "Done",
                TaskStatus::Blocked => "Blocked",
                TaskStatus::Superseded => "Superseded",
                TaskStatus::Cancelled => "Cancelled",
            };
            md.push_str(&format!("- [{}] [{}] {}\n", status, task.id, task.description));
        }
        md.push('\n');
    }

    md.push_str("# Constraints\n");
    if state.constraints.is_empty() {
        md.push('\n');
    } else {
        for c in &state.constraints {
            md.push_str(&format!("- {}\n", c));
        }
        md.push('\n');
    }

    md
}

/// Scan the entire markdown for already-archived task IDs.
///
/// Parses lines matching `- [Done] [<id>]` and collects the IDs into a
/// HashSet so callers can globally deduplicate across all date sections.
fn extract_task_ids(md: &str) -> HashSet<String> {
    let re = Regex::new(r"^-\s*\[Done\]\s*\[([A-Za-z0-9_-]+)\]").unwrap();
    md.lines()
        .filter_map(|line| re.captures(line).map(|cap| cap[1].to_string()))
        .collect()
}

/// Categorise a terminal task into its archive section name.
fn archive_section(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Done => "Completed",
        TaskStatus::Superseded => "Superseded",
        TaskStatus::Cancelled => "Cancelled",
        _ => "Completed",
    }
}

/// Render a single archived task line (and optional Superseded metadata).
fn render_archived_task(t: &Task) -> String {
    let status_label = match t.status {
        TaskStatus::Done => "Done",
        TaskStatus::Superseded => "Superseded",
        TaskStatus::Cancelled => "Cancelled",
        _ => "Done",
    };
    let mut out = format!("- [{}] [{}] {}\n", status_label, t.id, t.description);
    if let Some(info) = &t.superseded_by {
        let ref_str = match &info.reference {
            Reference::Task(id) => format!("Task {}", id),
            Reference::Adr(id) => format!("ADR {}", id),
        };
        out.push_str(&format!("  Superseded by: {}\n", ref_str));
        if !info.reason.is_empty() {
            out.push_str(&format!("  Reason: {}\n", info.reason));
        }
    }
    out
}

/// Append terminal-status tasks to tasks_archive.md with three top-level sections.
///
/// ```markdown
/// # Archived Tasks
///
/// ## Completed
/// ### 2026-06-02
/// - [Done] [TASK-001] desc
///
/// ## Superseded
/// ### 2026-06-02
/// - [Superseded] [TASK-011] desc
///   Superseded by: ADR-053
///   Reason: Ground Truth generation redesigned
///
/// ## Cancelled
/// ### 2026-06-02
/// - [Cancelled] [TASK-028] desc
/// ```
///
/// Global deduplication by task ID across all sections.
/// If the existing file uses the old flat format (no `## Completed` header),
/// it is transparently migrated on first write.
pub fn append_tasks_archive(
    existing: &str,
    new_tasks: &[Task],
    today_date: &str,
) -> String {
    let existing_ids = extract_task_ids(existing);

    let tasks_to_add: Vec<&Task> = new_tasks
        .iter()
        .filter(|t| !existing_ids.contains(&t.id))
        .collect();

    if tasks_to_add.is_empty() && !existing.is_empty() {
        return existing.to_string();
    }

    // ── Parse existing content into sections ─────────────────────────
    // If the file is empty or uses the old flat format, start from scratch.
    let mut completed = String::new();
    let mut superseded = String::new();
    let mut cancelled = String::new();

    if !existing.trim().is_empty() && existing.contains("## Completed") {
        // New-format file — split into three sections.
        // We don't need to parse deeply; we'll keep the existing text
        // and append new entries into the right sections.
        // For simplicity, we rebuild the file from scratch each time,
        // preserving existing entries by re-inserting them.
        // This is acceptable because the archive file is small.
        let mut current_section: Option<&str> = None;
        for line in existing.lines() {
            if line.trim() == "## Completed" {
                current_section = Some("completed");
                continue;
            } else if line.trim() == "## Superseded" {
                current_section = Some("superseded");
                continue;
            } else if line.trim() == "## Cancelled" {
                current_section = Some("cancelled");
                continue;
            }
            match current_section {
                Some("completed") => completed.push_str(&format!("{}\n", line)),
                Some("superseded") => superseded.push_str(&format!("{}\n", line)),
                Some("cancelled") => cancelled.push_str(&format!("{}\n", line)),
                _ => {}
            }
        }
    } else if !existing.trim().is_empty() {
        // Old-format file — migrate all existing entries to Completed.
        completed = existing.to_string();
    }

    // ── Append new tasks into the correct section buffers ────────────
    for t in &tasks_to_add {
        let section = match t.status {
            TaskStatus::Done => &mut completed,
            TaskStatus::Superseded => &mut superseded,
            TaskStatus::Cancelled => &mut cancelled,
            _ => &mut completed,
        };

        // Check if today's date subsection already exists in this section.
        let today_header = format!("### {}", today_date);
        if section.contains(&today_header) {
            // Find the end of today's subsection and insert before it.
            // Simple heuristic: append at the end of the section buffer
            // (date ordering is not strictly required for V4.1).
            section.push_str(&render_archived_task(t));
        } else {
            section.push_str(&format!("\n### {}\n", today_date));
            section.push_str(&render_archived_task(t));
        }
    }

    // ── Reassemble ───────────────────────────────────────────────────
    let mut out = "# Archived Tasks\n\n".to_string();

    if !completed.trim().is_empty() {
        out.push_str("## Completed\n");
        out.push_str(&completed);
        out.push('\n');
    }
    if !superseded.trim().is_empty() {
        out.push_str("## Superseded\n");
        out.push_str(&superseded);
        out.push('\n');
    }
    if !cancelled.trim().is_empty() {
        out.push_str("## Cancelled\n");
        out.push_str(&cancelled);
        out.push('\n');
    }

    out
}

// ── decisions.md ↔ Vec<ADR> ────────────────────────────────────────────────

/// Parse decisions.md content into a vector of ADRs.
///
/// Expected format:
/// ```markdown
/// ## ADR-{id}: {title}
///
/// **Status:** {active|superseded|deprecated|rejected}
///
/// ### Context
/// {context}
///
/// ### Decision
/// {decision}
///
/// **Tags:** {tag1}, {tag2}
/// ```
///
/// Malformed ADRs are skipped with a warning; empty file returns an empty Vec.
pub fn parse_decisions(md: &str) -> Result<Vec<ADR>> {
    let mut adrs = Vec::new();

    if md.trim().is_empty() {
        return Ok(adrs);
    }

    let mut in_adr = false;
    let mut section: &str = "none";
    let mut id = String::new();
    let mut title = String::new();
    let mut status = String::new();
    let mut context = String::new();
    let mut decision = String::new();
    let mut tags: Vec<String> = Vec::new();

    for line in md.lines() {
        let line = line.trim();

        // Detect new ADR header
        if line.starts_with("## ADR-") {
            // Save previous ADR
            if in_adr {
                adrs.push(ADR {
                    id: std::mem::take(&mut id),
                    title: std::mem::take(&mut title),
                    status: status.parse().unwrap_or(AdrStatus::Proposed),
                    context: std::mem::take(&mut context).trim().to_string(),
                    decision: std::mem::take(&mut decision).trim().to_string(),
                    tags: std::mem::take(&mut tags),
                });
            }

            in_adr = true;
            section = "none";
            context.clear();
            decision.clear();
            tags.clear();

            // Parse "ADR-{id}: {title}"
            let header = line.trim_start_matches("## ").trim();
            match header.find(':') {
                Some(colon_pos) => {
                    id = header[..colon_pos].trim().to_string();
                    title = header[colon_pos + 1..].trim().to_string();
                }
                None => {
                    in_adr = false;
                    eprintln!("[memguard] Warning: skipping malformed ADR header: {}", line);
                }
            }
            continue;
        }

        if !in_adr {
            continue;
        }

        // Parse status line
        if let Some(rest) = line.strip_prefix("**Status:**") {
            status = rest.trim().to_string();
            continue;
        }
        // Parse section markers
        if line == "### Context" {
            section = "context";
            continue;
        }
        if line == "### Decision" {
            section = "decision";
            continue;
        }

        // Parse tags line
        if let Some(rest) = line.strip_prefix("**Tags:**") {
            tags = rest
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            continue;
        }

        // Accumulate section content
        match section {
            "context" => {
                context.push_str(line);
                context.push('\n');
            }
            "decision" => {
                decision.push_str(line);
                decision.push('\n');
            }
            _ => {}
        }
    }

    // Save last ADR
    if in_adr {
        adrs.push(ADR {
            id,
            title,
            status: status.parse().unwrap_or(AdrStatus::Proposed),
            context: context.trim().to_string(),
            decision: decision.trim().to_string(),
            tags,
        });
    }

    Ok(adrs)
}

/// Render a single ADR to Markdown.
pub fn render_decision(adr: &ADR) -> String {
    let mut md = String::new();

    md.push_str(&format!("## {}: {}\n\n", adr.id, adr.title));
    md.push_str(&format!("**Status:** {}\n\n", adr.status));

    md.push_str("### Context\n");
    md.push_str(&adr.context);
    md.push_str("\n\n");

    md.push_str("### Decision\n");
    md.push_str(&adr.decision);
    md.push('\n');

    if !adr.tags.is_empty() {
        md.push_str(&format!("\n**Tags:** {}\n", adr.tags.join(", ")));
    }

    md
}

/// Render all ADRs, separated by a blank line.
pub fn render_decisions(adrs: &[ADR]) -> String {
    adrs.iter()
        .map(render_decision)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render active ADRs (Accepted, Proposed) to Markdown, with a header
/// linking to the archive when stale decisions exist.
#[allow(dead_code)]
pub fn render_active_decisions(adrs: &[ADR]) -> String {
    let active: Vec<_> = adrs
        .iter()
        .filter(|a| matches!(a.status, AdrStatus::Accepted | AdrStatus::Proposed))
        .cloned()
        .collect();
    if active.is_empty() {
        return String::new();
    }
    let stale_count = adrs
        .iter()
        .filter(|a| !matches!(a.status, AdrStatus::Accepted | AdrStatus::Proposed))
        .count();
    let mut md = String::new();
    if stale_count > 0 {
        md.push_str("> Historical decisions are in [decisions_archive.md](./decisions_archive.md)\n\n");
    }
    md.push_str(&render_decisions(&active));
    md
}

/// Render stale ADRs (Superseded, Rejected, Archived) to Markdown.
#[allow(dead_code)]
pub fn render_stale_decisions(adrs: &[ADR]) -> String {
    let stale: Vec<_> = adrs
        .iter()
        .filter(|a| !matches!(a.status, AdrStatus::Accepted | AdrStatus::Proposed))
        .cloned()
        .collect();
    render_decisions(&stale)
}

// ── traps.md ↔ Vec<Trap> ───────────────────────────────────────────────────

/// Parse traps.md content into a vector of Traps.
///
/// Expected format:
/// ```markdown
/// ## Trap: {error_signature}
///
/// ### Context
/// {context}
///
/// ### Solution
/// {solution}
/// ```
pub fn parse_traps(md: &str) -> Result<Vec<Trap>> {
    let mut traps = Vec::new();

    if md.trim().is_empty() {
        return Ok(traps);
    }

    let mut in_trap = false;
    let mut section: &str = "none";
    let mut error_signature = String::new();
    let mut context = String::new();
    let mut root_cause = String::new();
    let mut solution = String::new();
    let mut prevention = String::new();

    for line in md.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("## Trap:") {
            // Save previous trap
            if in_trap {
                traps.push(Trap {
                    error_signature: std::mem::take(&mut error_signature),
                    context: std::mem::take(&mut context).trim().to_string(),
                    root_cause: std::mem::take(&mut root_cause).trim().to_string(),
                    solution: std::mem::take(&mut solution).trim().to_string(),
                    prevention: std::mem::take(&mut prevention).trim().to_string(),
                });
            }

            in_trap = true;
            section = "none";
            context.clear();
            root_cause.clear();
            solution.clear();
            prevention.clear();
            error_signature = rest.trim().to_string();
            continue;
        }

        if !in_trap {
            continue;
        }

        if line == "### Context" {
            section = "context";
            continue;
        }
        if line == "### Root Cause" {
            section = "root_cause";
            continue;
        }
        if line == "### Solution" {
            section = "solution";
            continue;
        }
        if line == "### Prevention" {
            section = "prevention";
            continue;
        }

        match section {
            "context" => {
                context.push_str(line);
                context.push('\n');
            }
            "root_cause" => {
                root_cause.push_str(line);
                root_cause.push('\n');
            }
            "solution" => {
                solution.push_str(line);
                solution.push('\n');
            }
            "prevention" => {
                prevention.push_str(line);
                prevention.push('\n');
            }
            _ => {}
        }
    }

    if in_trap {
        traps.push(Trap {
            error_signature,
            context: context.trim().to_string(),
            root_cause: root_cause.trim().to_string(),
            solution: solution.trim().to_string(),
            prevention: prevention.trim().to_string(),
        });
    }

    Ok(traps)
}

/// Render a single Trap to Markdown.
pub fn render_trap(trap: &Trap) -> String {
    let mut md = String::new();

    md.push_str(&format!("## Trap: {}\n\n", trap.error_signature));
    md.push_str("### Context\n");
    md.push_str(&trap.context);
    md.push('\n');

    if !trap.root_cause.is_empty() {
        md.push_str("\n### Root Cause\n");
        md.push_str(&trap.root_cause);
        md.push('\n');
    }

    md.push_str("\n### Solution\n");
    md.push_str(&trap.solution);
    md.push('\n');

    if !trap.prevention.is_empty() {
        md.push_str("\n### Prevention\n");
        md.push_str(&trap.prevention);
        md.push('\n');
    }

    md
}

/// Render all Traps, separated by a blank line.
pub fn render_traps(traps: &[Trap]) -> String {
    traps
        .iter()
        .map(render_trap)
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Internal helpers ───────────────────────────────────────────────────────

/// Normalize a phase string to its canonical short identifier.
///
/// Maps known Chinese and verbose English variants to the SOP §5 canonical set:
/// `explore`, `plan`, `implement`, `verify`, `complete`.
///
/// Already-canonical strings are returned unchanged.  Unknown inputs pass
/// through with a warning logged to stderr, so the system degrades
/// gracefully rather than rejecting novel phases.
pub fn canonicalize_phase(raw: &str) -> String {
    let trimmed = raw.trim();

    // Already canonical — fast path, no allocation.
    match trimmed {
        "explore" | "plan" | "implement" | "verify" | "complete" => {
            return trimmed.to_string()
        }
        _ => {}
    }

    let lower = trimmed.to_lowercase();

    // ── Chinese → canonical ────────────────────────────────────
    #[allow(clippy::match_single_binding)]
    match lower.as_str() {
        "探索模式" | "探索" => return "explore".to_string(),
        "规划" | "计划" | "架构设计" => return "plan".to_string(),
        "执行模式" | "实施" | "实现" | "开发" | "开发中" | "开发阶段" => return "implement".to_string(),
        "验证" | "测试" | "校验" => return "verify".to_string(),
        "完成" | "交付" => return "complete".to_string(),
        _ => {}
    }

    // ── English verbose → canonical ───────────────────────────
    match lower.as_str() {
        "exploration" | "divergence" => return "explore".to_string(),
        "planning" | "architecture design" => return "plan".to_string(),
        "execution mode" | "execution" | "implementation" => {
            return "implement".to_string()
        }
        "testing" | "verification" | "validation" => return "verify".to_string(),
        "delivered" | "completion" | "done" => return "complete".to_string(),
        _ => {}
    }

    // ── Unknown: warn, but pass through (graceful degradation) ─
    eprintln!(
        "[memguard] WARNING: unknown phase '{}' — using as-is. \
         Canonical phases are: explore, plan, implement, verify, complete.",
        trimmed
    );
    trimmed.to_string()
}

/// Extract the body text after a section header (skip header line, trim).
fn extract_section_body(rest: &str) -> String {
    rest.lines()
        .skip(1)
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse task lines into a Vec<Task>.
///
/// Supports three formats:
/// - New:   `- [Todo] [TASK-XXX] description`
/// - Legacy checkbox: `- [ ] description`
/// - Legacy plain:    `- description`
///
/// Task IDs are extracted from the markdown if present, otherwise generated
/// sequentially.  Legacy formats without explicit status default to `Todo`.
fn parse_task_lines(rest: &str) -> Vec<Task> {
    static TASK_LINE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^-\s*\[(Todo|InProgress|Done|Blocked)\]\s*(?:\[(TASK-\d{3})\]\s*)?(.*)").unwrap()
    });

    static LEGACY_CB_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^-\s*\[\s*\]\s*(.*)").unwrap()
    });

    static LEGACY_PLAIN_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^-\s+(.*)").unwrap()
    });

    let mut tasks = Vec::new();

    for line in rest.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try new format first.
        if let Some(caps) = TASK_LINE_RE.captures(line) {
            let status_str = &caps[1];
            let id = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| format!("TASK-{:03}", tasks.len()));
            let desc = caps[3].trim().to_string();

            let status = match status_str {
                "Todo" => TaskStatus::Todo,
                "InProgress" => TaskStatus::InProgress,
                "Done" => TaskStatus::Done,
                "Blocked" => TaskStatus::Blocked,
                _ => continue,
            };
            tasks.push(Task {
                id,
                description: desc,
                status,
                superseded_by: None,
            });
            continue;
        }

        // Legacy checkbox: `- [ ] description`
        if let Some(caps) = LEGACY_CB_RE.captures(line) {
            let desc = caps[1].trim().to_string();
            if !desc.is_empty() {
                tasks.push(Task {
                    id: format!("TASK-{:03}", tasks.len()),
                    description: desc,
                    status: TaskStatus::Todo,
                    superseded_by: None,
                });
            }
            continue;
        }

        // Legacy plain bullet: `- description`
        // Guard against double-matching new format (already tried above).
        if let Some(caps) = LEGACY_PLAIN_RE.captures(line) {
            let desc = caps[1].trim().to_string();
            if !desc.is_empty() {
                tasks.push(Task {
                    id: format!("TASK-{:03}", tasks.len()),
                    description: desc,
                    status: TaskStatus::Todo,
                    superseded_by: None,
                });
            }
        }
    }

    tasks
}

/// Parse bullet-list lines (starting with `- `) into a Vec<String>.
fn parse_bullet_list(rest: &str) -> Vec<String> {
    rest.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('-').trim().to_string())
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── context.md round-trip ──────────────────────────────────────────

    #[test]
    fn test_parse_context_minimal() {
        let md = "# Current Phase\nplanning\n\n# Active Tasks\n\n# Constraints\n";
        let state = parse_context(md).unwrap();
        assert_eq!(state.current_phase, "plan");
        assert!(state.active_tasks.is_empty());
        assert!(state.constraints.is_empty());
    }

    #[test]
    fn test_parse_context_full() {
        let md = concat!(
            "# Current Phase\n",
            "implementation\n\n",
            "# Active Tasks\n",
            "- [Todo] Add login page\n",
            "- [InProgress] Refactor database\n",
            "- [Done] Setup CI\n\n",
            "# Constraints\n",
            "- Must use PostgreSQL\n",
            "- No ORM allowed\n"
        );
        let state = parse_context(md).unwrap();
        assert_eq!(state.current_phase, "implement");
        assert_eq!(state.active_tasks.len(), 3);
        assert_eq!(state.active_tasks[0].description, "Add login page");
        assert!(matches!(state.active_tasks[0].status, TaskStatus::Todo));
        assert!(matches!(state.active_tasks[1].status, TaskStatus::InProgress));
        assert!(matches!(state.active_tasks[2].status, TaskStatus::Done));
        assert_eq!(state.constraints.len(), 2);
        assert_eq!(state.constraints[0], "Must use PostgreSQL");
    }

    #[test]
    fn test_render_context_round_trip() {
        let md = "# Current Phase\nimplementation\n\n# Active Tasks\n- [Todo] Task A\n- [Done] Task B\n\n# Constraints\n- Limit memory\n\n";
        let state = parse_context(md).unwrap();
        let rendered = render_context(&state);
        let state2 = parse_context(&rendered).unwrap();
        assert_eq!(state.current_phase, state2.current_phase);
        // Done tasks are filtered from render output, so round-trip loses them.
        assert_eq!(state2.active_tasks.len(), 1);
        assert_eq!(state2.active_tasks[0].description, "Task A");
    }

    #[test]
    fn test_render_context_filters_done() {
        let state = RuntimeState {
            current_phase: "implement".into(),
            active_tasks: vec![
                Task {
            id: "TASK-000".into(),
            description: "Todo task".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        },
                Task {
            id: "TASK-001".into(),
            description: "InProgress task".into(),
            status: TaskStatus::InProgress,
            superseded_by: None,
        },
                Task {
            id: "TASK-002".into(),
            description: "Done task".into(),
            status: TaskStatus::Done,
            superseded_by: None,
        },
                Task {
            id: "TASK-003".into(),
            description: "Blocked task".into(),
            status: TaskStatus::Blocked,
            superseded_by: None,
        },
            ],
            done_tasks: vec![],
            constraints: vec![],
        };
        let rendered = render_context(&state);
        assert!(!rendered.contains("Done task"));
        assert!(rendered.contains("Todo task"));
        assert!(rendered.contains("InProgress task"));
        assert!(rendered.contains("Blocked task"));
    }

    #[test]
    fn test_parse_task_lines_blocked() {
        let md = "- [Blocked] [TASK-001] test";
        let tasks = parse_task_lines(md);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "TASK-001");
        assert_eq!(tasks[0].description, "test");
        assert!(matches!(tasks[0].status, TaskStatus::Blocked));
    }

    #[test]
    fn test_roundtrip_excludes_done() {
        let md = concat!(
            "# Current Phase\n",
            "implementation\n\n",
            "# Active Tasks\n",
            "- [Todo] [TASK-000] Active task\n",
            "- [Done] [TASK-001] Finished task\n\n",
            "# Constraints\n",
            "- Limit memory\n"
        );
        let state = parse_context(md).unwrap();
        assert_eq!(state.active_tasks.len(), 2);
        let rendered = render_context(&state);
        let state2 = parse_context(&rendered).unwrap();
        assert_eq!(state2.active_tasks.len(), 1);
        assert_eq!(state2.active_tasks[0].description, "Active task");
        assert!(matches!(state2.active_tasks[0].status, TaskStatus::Todo));
    }

    // ── Legacy Chinese H2 format (v2 skill) ────────────────────────────

    #[test]
    fn test_parse_context_legacy_chinese_h2() {
        let md = concat!(
            "# Current Context\n\n",
            "## 当前阶段\n",
            "执行模式\n\n",
            "## 关键任务\n",
            "- [ ] 任务一\n",
            "- 任务二\n\n",
            "## 当前约束\n",
            "- 必须使用 PostgreSQL\n",
            "- 不允许使用 ORM\n"
        );
        let state = parse_context(md).unwrap();
        assert_eq!(state.current_phase, "implement");
        assert_eq!(state.active_tasks.len(), 2);
        assert_eq!(state.active_tasks[0].description, "任务一");
        assert!(matches!(state.active_tasks[0].status, TaskStatus::Todo));
        assert_eq!(state.active_tasks[1].description, "任务二");
        assert!(matches!(state.active_tasks[1].status, TaskStatus::Todo));
        assert_eq!(state.constraints.len(), 2);
        assert_eq!(state.constraints[0], "必须使用 PostgreSQL");
    }

    #[test]
    fn test_parse_context_legacy_mixed() {
        // A file that has both H1 and H2 — the parser should pick the
        // dominant level and still produce correct results.
        let md = concat!(
            "# Current Context\n\n",
            "## 当前阶段\n",
            "planning\n\n",
            "## 关键任务\n",
            "- [Todo] [TASK-000] Modern task\n",
            "- [ ] Legacy task\n\n",
            "## 当前约束\n",
            "- Limit memory\n"
        );
        let state = parse_context(md).unwrap();
        assert_eq!(state.current_phase, "plan");
        assert_eq!(state.active_tasks.len(), 2);
        assert_eq!(state.active_tasks[0].description, "Modern task");
        assert_eq!(state.active_tasks[1].description, "Legacy task");
    }

    // ── decisions.md ───────────────────────────────────────────────────

    #[test]
    fn test_parse_decisions_empty() {
        let adrs = parse_decisions("").unwrap();
        assert!(adrs.is_empty());
    }

    #[test]
    fn test_parse_decisions_single() {
        let md = concat!(
            "## ADR-001: Use Rust for backend\n\n",
            "**Status:** Accepted\n\n",
            "### Context\n",
            "Need high performance.\n\n",
            "### Decision\n",
            "Use Rust with Tokio.\n\n",
            "**Tags:** rust, backend, performance\n"
        );
        let adrs = parse_decisions(md).unwrap();
        assert_eq!(adrs.len(), 1);
        assert_eq!(adrs[0].id, "ADR-001");
        assert_eq!(adrs[0].title, "Use Rust for backend");
        assert_eq!(adrs[0].status, AdrStatus::Accepted);
        assert_eq!(adrs[0].tags, vec!["rust", "backend", "performance"]);
    }

    #[test]
    fn test_render_decision_round_trip() {
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Test".into(),
            status: AdrStatus::Accepted,
            context: "Some context.\nMore context.".into(),
            decision: "The decision.".into(),
            tags: vec!["a".into(), "b".into()],
        };
        let rendered = render_decision(&adr);
        let parsed = parse_decisions(&rendered).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "ADR-001");
        assert_eq!(parsed[0].title, "Test");
    }

    // ── traps.md ───────────────────────────────────────────────────────

    #[test]
    fn test_parse_traps_empty() {
        let traps = parse_traps("").unwrap();
        assert!(traps.is_empty());
    }

    #[test]
    fn test_parse_traps_single() {
        let md = concat!(
            "## Trap: NPE in auth handler\n\n",
            "### Context\n",
            "Null pointer when token expired.\n\n",
            "### Solution\n",
            "Add null check before dereference.\n"
        );
        let traps = parse_traps(md).unwrap();
        assert_eq!(traps.len(), 1);
        assert_eq!(traps[0].error_signature, "NPE in auth handler");
        assert!(traps[0].context.contains("token expired"));
        assert!(traps[0].solution.contains("null check"));
    }

    #[test]
    fn test_render_trap_round_trip() {
        let trap = Trap {
            error_signature: "Timeout".into(),
            context: "Request took too long.".into(),
            solution: "Add timeout config.".into(),
            root_cause: String::new(),
            prevention: String::new(),
        };
        let rendered = render_trap(&trap);
        let parsed = parse_traps(&rendered).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].error_signature, "Timeout");
    }

    #[test]
    fn test_parse_traps_with_root_cause_prevention() {
        let md = concat!(
            "## Trap: Race in cache\n\n",
            "### Context\n",
            "Concurrent writes corrupt state.\n\n",
            "### Root Cause\n",
            "Missing lock around shared map.\n\n",
            "### Solution\n",
            "Use RwLock for the cache.\n\n",
            "### Prevention\n",
            "Audit all shared mutable state.\n"
        );
        let traps = parse_traps(md).unwrap();
        assert_eq!(traps.len(), 1);
        assert_eq!(traps[0].error_signature, "Race in cache");
        assert!(traps[0].context.contains("Concurrent writes"));
        assert_eq!(traps[0].root_cause, "Missing lock around shared map.");
        assert!(traps[0].solution.contains("RwLock"));
        assert_eq!(traps[0].prevention, "Audit all shared mutable state.");
    }

    #[test]
    fn test_render_trap_skips_empty_sections() {
        let trap = Trap {
            error_signature: "Leak".into(),
            context: "Memory grows unbounded.".into(),
            root_cause: String::new(),
            solution: "Drop resources after use.".into(),
            prevention: String::new(),
        };
        let rendered = render_trap(&trap);
        assert!(rendered.contains("### Context"));
        assert!(!rendered.contains("### Root Cause"));
        assert!(rendered.contains("### Solution"));
        assert!(!rendered.contains("### Prevention"));
    }

    #[test]
    fn test_trap_roundtrip_all_fields() {
        let original = Trap {
            error_signature: "Deadlock".into(),
            context: "Two threads wait forever.".into(),
            root_cause: "Circular dependency on locks.".into(),
            solution: "Enforce lock ordering.".into(),
            prevention: "Use try_lock with timeout.".into(),
        };
        let rendered = render_trap(&original);
        let parsed = parse_traps(&rendered).unwrap();
        assert_eq!(parsed.len(), 1);
        let round = &parsed[0];
        assert_eq!(round.error_signature, "Deadlock");
        assert_eq!(round.context, "Two threads wait forever.");
        assert_eq!(round.root_cause, "Circular dependency on locks.");
        assert_eq!(round.solution, "Enforce lock ordering.");
        assert_eq!(round.prevention, "Use try_lock with timeout.");
    }

    // ── ADR status partition ───────────────────────────────────────────

    #[test]
    fn test_adr_partition_by_status() {
        let adrs = vec![
            ADR {
                id: "ADR-001".into(),
                title: "Accepted".into(),
                status: AdrStatus::Accepted,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-002".into(),
                title: "Proposed".into(),
                status: AdrStatus::Proposed,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-003".into(),
                title: "Superseded".into(),
                status: AdrStatus::Superseded,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-004".into(),
                title: "Rejected".into(),
                status: AdrStatus::Rejected,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-005".into(),
                title: "Archived".into(),
                status: AdrStatus::Archived,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
        ];

        let active = render_active_decisions(&adrs);
        assert!(active.contains("## ADR-001: Accepted"));
        assert!(active.contains("## ADR-002: Proposed"));
        assert!(!active.contains("## ADR-003:"));
        assert!(!active.contains("## ADR-004:"));
        assert!(!active.contains("## ADR-005:"));
        assert!(
            active.contains("> Historical decisions are in [decisions_archive.md](./decisions_archive.md)")
        );

        let stale = render_stale_decisions(&adrs);
        assert!(stale.contains("## ADR-003: Superseded"));
        assert!(stale.contains("## ADR-004: Rejected"));
        assert!(stale.contains("## ADR-005: Archived"));
        assert!(!stale.contains("## ADR-001:"));
        assert!(!stale.contains("## ADR-002:"));
    }

    #[test]
    fn test_roundtrip_adr_status() {
        let adrs = vec![
            ADR {
                id: "ADR-001".into(),
                title: "A".into(),
                status: AdrStatus::Accepted,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-002".into(),
                title: "P".into(),
                status: AdrStatus::Proposed,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-003".into(),
                title: "S".into(),
                status: AdrStatus::Superseded,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-004".into(),
                title: "R".into(),
                status: AdrStatus::Rejected,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
            ADR {
                id: "ADR-005".into(),
                title: "Ar".into(),
                status: AdrStatus::Archived,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec![],
            },
        ];

        let rendered = render_decisions(&adrs);
        let parsed = parse_decisions(&rendered).unwrap();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[0].status, AdrStatus::Accepted);
        assert_eq!(parsed[1].status, AdrStatus::Proposed);
        assert_eq!(parsed[2].status, AdrStatus::Superseded);
        assert_eq!(parsed[3].status, AdrStatus::Rejected);
        assert_eq!(parsed[4].status, AdrStatus::Archived);
    }

    // ── tasks_archive global dedup ────────────────────────────────────

    #[test]
    fn test_append_tasks_archive_global_dedup() {
        let existing = concat!(
            "# Archived Tasks\n\n",
            "## 2026-06-01\n",
            "- [Done] [TASK-001] Old task\n\n",
        );
        let new_task = Task {
            id: "TASK-001".into(),
            description: "Same task".into(),
            status: TaskStatus::Done,
            superseded_by: None,
        };
        let result = append_tasks_archive(existing, &[new_task], "2026-06-02");
        // TASK-001 already exists globally — new section must NOT be created.
        assert!(!result.contains("## 2026-06-02"));
    }

    #[test]
    fn test_append_tasks_archive_keeps_earliest() {
        let existing = concat!(
            "# Archived Tasks\n\n",
            "## 2026-06-01\n",
            "- [Done] [TASK-001] First\n\n",
        );
        let new_task = Task {
            id: "TASK-001".into(),
            description: "Second".into(),
            status: TaskStatus::Done,
            superseded_by: None,
        };
        let result = append_tasks_archive(existing, &[new_task], "2026-06-02");
        // Should still only have TASK-001 in 2026-06-01.
        assert!(result.contains("## 2026-06-01"));
        assert!(result.contains("[TASK-001] First"));
        // Should NOT have it in a 2026-06-02 section.
        let parts: Vec<&str> = result.split("## 2026-06-02").collect();
        if parts.len() > 1 {
            assert!(!parts[1].contains("TASK-001"));
        }
    }

}
