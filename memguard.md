---
name: memguard
description: Structured memory management and behavioral runtime contract via MCP. Provides context continuity, ADR-based decision anchoring, and controlled Explore → Execution workflows backed by a Git-native Rust runtime.
version: 3.0.0
compatibility: opencode
mcp:
  server: memguard
---

# MemGuard v3 — Agent Memory & Runtime Spec

> **Runtime**: Rust MCP Server (Git-native, RwLock thread-safe, Source-of-Truth in Markdown)
> **Trigger**: Activates automatically via MCP tools. No manual file I/O needed.

## Core Principles

1. **Memory is persistent operational context.** Reference prior decisions before proposing alternatives. Preserve architectural continuity. Explicit user instructions override memory — conflict surfaces, requests confirmation, records override.

2. **Decisions are append-only history.** Before proposing new technical/product decisions, query `runtime_query_memory`. Rejected approaches must not be re-proposed without explaining the meaningful difference.

3. **Assumptions must be explicit.** All unverified claims use `[ASSUMPTION: ...]`. Implicit assumptions are forbidden.

4. **Dual-Mode Operation.** Explore (divergence, uncertainty reduction, solution analysis) ↔ Execution (deterministic implementation and delivery). Switch from Explore to Execution ONLY when: solution converges to 1-2 viable paths AND major uncertainty is validated AND MVP scope is sufficiently defined.

## MCP Tools

The Rust runtime exposes exactly 3 tools. Call them via MCP — never read/write memory files directly.

### Tool 1: `runtime_bootstrap`

**When**: Session start, context loss recovery, first interaction with a project.

**What it does**: Reads `memory/*.md`, rebuilds `.memguard/` cache, returns compressed runtime summary (current phase, active tasks, latest ADR, constraints).

**Parameters**: None.

### Tool 2: `runtime_commit_event`

**When**: Task status change, architecture decision made, error/trap recorded, phase transition.

**What it does**: Updates in-memory state (under RwLock), triggers debounced async write of all Markdown files. Projection layer auto-formats Memory files.

**Parameters**:
```json
{
  "event_type": "TaskUpdated | AdrCommitted | TrapRecorded | PhaseChanged",
  "payload": { ... }
}
```

**Event schemas**:

- `TaskUpdated`: `{ "task_id": "TASK-000", "new_status": "Todo|InProgress|Done" }`
- `AdrCommitted`: `{ "id": "ADR-001", "title": "...", "status": "active", "context": "...", "decision": "...", "tags": ["..."] }`
- `TrapRecorded`: `{ "error_signature": "...", "context": "...", "solution": "..." }`
- `PhaseChanged`: `{ "new_phase": "..." }`

### Tool 3: `runtime_query_memory`

**When**: Before writing core code, before proposing architecture, when checking for known traps.

**What it does**: Keyword search across all ADRs and Traps. Returns scored, truncated results.

**Parameters**:
```json
{
  "query_intent": "authentication token validation",
  "limit": 3
}
```

## Memory Directory Structure

The runtime manages these files (do NOT edit them directly):

```
memory/                  # Source of Truth — human-readable, Git-tracked
├── context.md           # Current phase, active tasks, constraints
├── decisions.md         # ADR format architecture decisions
└── traps.md             # Error signatures with context and solutions

.memguard/               # Runtime cache — machine-readable, .gitignore'd
├── runtime_state.json   # Serialized state snapshot
└── search_index.json    # Keyword index for query_memory
```

If `.memguard/` is deleted, `runtime_bootstrap` will rebuild it from `memory/`.

## Recommended Execution Order

```
1. Call runtime_bootstrap  → load current state
2. Call runtime_query_memory → check for relevant decisions/traps
3. Determine mode (Explore vs Execution)
4. Write code
5. Call runtime_commit_event → persist decisions, task updates, traps, phase changes
```

## Session Self-Check

Before ending a session, verify:
- Were new decisions made? → `runtime_commit_event { AdrCommitted }`
- Did goals/phase change? → `runtime_commit_event { PhaseChanged }`
- Were errors encountered? → `runtime_commit_event { TrapRecorded }`
- Did tasks change status? → `runtime_commit_event { TaskUpdated }`

## Design Philosophy

continuity > statelessness · decisions > conversation history · active context > historical detail · structure > improvisation · controlled autonomy > unrestricted generation
