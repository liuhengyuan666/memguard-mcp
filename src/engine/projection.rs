use crate::models::*;
use anyhow::{anyhow, Result};
use regex::Regex;

// ── context.md ↔ RuntimeState ──────────────────────────────────────────────

/// Parse context.md content into RuntimeState.
///
/// Expected format:
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
pub fn parse_context(md: &str) -> Result<RuntimeState> {
    let sections = md.split("\n# ");

    let mut current_phase: Option<String> = None;
    let mut tasks: Vec<Task> = Vec::new();
    let mut constraints: Vec<String> = Vec::new();

    for section in sections {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }

        // The first section keeps the "# " prefix from the split boundary.
        // Subsequent sections have it stripped.  Handle both cases.
        if let Some(rest) = section
            .strip_prefix("Current Phase")
            .or_else(|| section.strip_prefix("# Current Phase"))
        {
            current_phase = Some(extract_section_body(rest));
        } else if let Some(rest) = section
            .strip_prefix("Active Tasks")
            .or_else(|| section.strip_prefix("# Active Tasks"))
        {
            tasks = parse_task_lines(rest);
        } else if let Some(rest) = section
            .strip_prefix("Constraints")
            .or_else(|| section.strip_prefix("# Constraints"))
        {
            constraints = parse_bullet_list(rest);
        }
    }

    let current_phase =
        current_phase.ok_or_else(|| anyhow!("Missing '# Current Phase' section in context.md"))?;

    Ok(RuntimeState {
        current_phase,
        active_tasks: tasks,
        constraints,
    })
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
            let status = match task.status {
                TaskStatus::Todo => "Todo",
                TaskStatus::InProgress => "InProgress",
                TaskStatus::Done => "Done",
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
                    status: std::mem::take(&mut status),
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
            status,
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
    let mut solution = String::new();

    for line in md.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("## Trap:") {
            // Save previous trap
            if in_trap {
                traps.push(Trap {
                    error_signature: std::mem::take(&mut error_signature),
                    context: std::mem::take(&mut context).trim().to_string(),
                    solution: std::mem::take(&mut solution).trim().to_string(),
                });
            }

            in_trap = true;
            section = "none";
            context.clear();
            solution.clear();
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
        if line == "### Solution" {
            section = "solution";
            continue;
        }

        match section {
            "context" => {
                context.push_str(line);
                context.push('\n');
            }
            "solution" => {
                solution.push_str(line);
                solution.push('\n');
            }
            _ => {}
        }
    }

    if in_trap {
        traps.push(Trap {
            error_signature,
            context: context.trim().to_string(),
            solution: solution.trim().to_string(),
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
    md.push_str("\n\n### Solution\n");
    md.push_str(&trap.solution);
    md.push('\n');

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

/// Extract the body text after a section header (skip header line, trim).
fn extract_section_body(rest: &str) -> String {
    rest.lines()
        .skip(1)
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse lines like `- [Todo] [TASK-XXX] description` or `- [Todo] description` into a Vec<Task>.
/// Task IDs are extracted from the markdown if present, otherwise generated sequentially.
fn parse_task_lines(rest: &str) -> Vec<Task> {
    static TASK_LINE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^-\s*\[(Todo|InProgress|Done)\]\s*(?:\[(TASK-\d{3})\]\s*)?(.*)").unwrap()
    });

    let mut tasks = Vec::new();

    for line in rest.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
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
                _ => continue,
            };
            tasks.push(Task {
                id,
                description: desc,
                status,
            });
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
        assert_eq!(state.current_phase, "planning");
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
        assert_eq!(state.current_phase, "implementation");
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
        assert_eq!(state.active_tasks.len(), state2.active_tasks.len());
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
        assert_eq!(adrs[0].status, "Accepted");
        assert_eq!(adrs[0].tags, vec!["rust", "backend", "performance"]);
    }

    #[test]
    fn test_render_decision_round_trip() {
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Test".into(),
            status: "Accepted".into(),
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
        };
        let rendered = render_trap(&trap);
        let parsed = parse_traps(&rendered).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].error_signature, "Timeout");
    }
}
