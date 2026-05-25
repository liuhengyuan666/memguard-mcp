# MemGuard MCP — Quick Tool Reference

> ⛔ **THIS IS NOT THE SKILL FILE.**
>
> This is a human-readable reference card for the MCP server. It does **NOT**
> provide Agent behavior rules or Standard Operating Procedures.
>
> **The Agent SOP (behavioral contract) lives at:**
> https://github.com/liuhengyuan666/memguard/blob/main/memguard/SKILL.md
>
> To install the Skill, add this alongside the `mcp` entry in your
> `opencode.json`:
>
> ```json
> {
>   "skills": {
>     "urls": [
>       "https://raw.githubusercontent.com/liuhengyuan666/memguard/main/"
>     ]
>   }
> }
> ```

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
