---
name: memguard
description: Structured memory management and behavioral runtime contract via MCP. Provides context continuity, ADR-based decision anchoring, and controlled Explore → Execution workflows backed by a Git-native Rust runtime.
version: 3.0.0
compatibility: opencode
mcp:
  server: memguard
---

# MemGuard v3 — Agent SOP Reference

> **This file is a brief reference.** The full Agent SOP (Standard Operating Procedure)
> is maintained in the [memguard Skill repository](https://github.com/liuhengyuan666/memguard)
> as `SKILL.md`. Always use the latest version from there.

---

## Quick Reference: MCP Tools

| Tool | When to Call | Key Parameter |
|------|-------------|---------------|
| `memguard_runtime_bootstrap` | **Session start** — before any code generation | `project_root` (optional) |
| `memguard_runtime_query_memory` | **Before decisions** — check history | `query_intent` (required) |
| `memguard_runtime_commit_event` | **After changes** — persist state | `event_type` + `payload` |

## Quick Reference: Event Types

| event_type | When | Key payload fields |
|-----------|------|-------------------|
| `AdrCommitted` | Architecture decisions | `id, title, status, context, decision, tags` |
| `TaskUpdated` | Task status transitions | `task_id, new_status` |
| `TrapRecorded` | Non-trivial bugs with solutions | `error_signature, context, solution` |
| `PhaseChanged` | Mode/phase transitions | `new_phase` |

## Full SOP

See: https://github.com/liuhengyuan666/memguard/blob/main/memguard/SKILL.md
