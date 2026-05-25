# MemGuard MCP — High-Performance Agent Memory Runtime

[![npm](https://img.shields.io/npm/v/@henry_lhy/memguard-mcp?color=orange)](https://www.npmjs.com/package/@henry_lhy/memguard-mcp)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/language-Rust-black.svg?logo=rust)](https://www.rust-lang.org/)

> **The Muscle for MemGuard v3.**
> A Git-Native, thread-safe Model Context Protocol (MCP) server written in Rust.

`MemGuard MCP` is the capability engine behind the MemGuard v3 architecture. It manages the physical reading, concurrent writing, validation guarding, and semantic indexing of your agent's memory trees.

⚠️ **Crucial Requirement:** This is the execution runtime. To govern *when* and *how* the agent calls these tools, you **must** install the companion behavioral contract: [memguard Core Specification](https://github.com/liuhengyuan666/memguard).

---

## 🚀 Core Capabilities

- **Thread-Safe Concurrency (`RwLock`)**: Prevents data race conditions or state file corruption when multi-agent swarms or parallel reasoning paths access the project simultaneously.
- **500ms Write Debouncing**: Groups aggressive, high-frequency agent thought logs into atomic file writes, mitigating disk I/O chokepoints.
- **Phase Canonicalization**: Normalizes Chinese (`执行模式`), verbose English (`planning`), and legacy phase strings to SOP-canonical short identifiers (`explore`, `plan`, `implement`, `verify`, `complete`), ensuring agent mode-switching logic is never broken by non-standard phase names.
- **ADR-Driven Continuity**: Bootstrap output surfaces `adr_count` and `trap_count` signals, ordering architectural decisions and constraints before task lists so agents prioritize project continuity over task management.

---

## 📦 Installation & Setup

### From npm (Recommended)

```bash
npm install -g @henry_lhy/memguard-mcp
```

### Build From Source

Ensure you have Rust and Cargo installed:

```bash
git clone https://github.com/liuhengyuan666/memguard-mcp.git
cd memguard-mcp
cargo build --release
```

The optimized binary will be at `target/release/memguard-mcp` (Linux/macOS) or `target/release/memguard-mcp.exe` (Windows).

---

## 🔌 Protocol Tool Specifications

Once mounted via JSON-RPC over Stdio, `memguard-mcp` exposes 3 atomic capabilities to your LLM/Agent environment:

| Tool | Function | Key Parameters |
|---|---|---|
| `runtime_bootstrap` | Reads `memory/*.md`, rebuilds cache, returns summary with phase, constraints, `adr_count`/`trap_count`, latest ADR, active tasks (in priority order) | `project_root` (optional) |
| `runtime_commit_event` | Unified state change entrypoint: TaskUpdated / AdrCommitted / TrapRecorded / PhaseChanged (phase names are auto-canonicalized) | `event_type` + `payload` |
| `runtime_query_memory` | Keyword search over decisions and traps | `query_intent` (required), `limit` (optional, default 3) |

> Agent **should not** call these tools directly — the Skill layer (SKILL.md) tells the Agent *when* to invoke them. See the companion [memguard Skill](https://github.com/liuhengyuan666/memguard) for the SOP.

---

## ⚙️ Mounting into MCP Hosts

### OpenCode Configuration (`opencode.json`)

```json
{
  "mcp": {
    "memguard": {
      "type": "local",
      "command": ["npx", "-y", "@henry_lhy/memguard-mcp"],
      "enabled": true
    }
  }
}
```

### Claude Desktop Configuration

```json
{
  "mcpServers": {
    "memguard": {
      "command": "npx",
      "args": ["-y", "@henry_lhy/memguard-mcp"]
    }
  }
}
```

---

## ❓ Troubleshooting

### Agent says `Skill "memguard" not found`

You installed the MCP runtime (`memguard-mcp`) but **not** the Skill (the Agent
SOP). The Skill is a separate behavioral contract that tells the Agent WHEN and
HOW to call the MCP tools.

Add this to your project's `opencode.json` alongside the `mcp` entry:

```json
{
  "skills": {
    "urls": [
      "https://raw.githubusercontent.com/liuhengyuan666/memguard/main/"
    ]
  }
}
```

Then restart OpenCode. See the [memguard Skill
repository](https://github.com/liuhengyuan666/memguard) for complete installation
instructions.

### MCP returns `MCP error -32602: Missing new_status`

The Agent is calling `memguard_runtime_commit_event` without the Skill's SOP
guidance. Without the Skill, the Agent guesses payload field names and often
gets them wrong.

**Fix**: Install the Skill (above), then restart the session. With the Skill
installed, the Agent follows the SOP and uses correct payload fields:
- `TaskUpdated`: use `task_id` + `new_status` (values: `Todo` | `InProgress` | `Done`)
- `AdrCommitted`: include all 6 fields (`id`, `title`, `status`, `context`, `decision`, `tags`)

### MCP returns `MCP error -32602: Invalid ADR payload`

Same root cause: Agent without Skill guidance. The `AdrCommitted` event requires
a complete ADR object with all fields: `{ id, title, status, context, decision, tags }`.

Install the Skill to provide the Agent with correct payload schemas.

### Quick Verification Checklist

- [ ] `opencode.json` has `mcp.memguard` entry (MCP runtime)
- [ ] `opencode.json` has `skills.urls` pointing to memguard repo (Agent SOP)
- [ ] Restarted OpenCode after configuration changes
- [ ] Agent called `memguard_runtime_bootstrap` successfully at session start

---

## 📐 Memory Layout

```text
[Project Root]
├── memory/                  # Source of Truth (Human Readable, Git Committed)
│   ├── context.md           # Active phase, goals, current tasks, and constraints
│   ├── decisions.md         # Architecture Decision Records (ADR, Append-Only)
│   └── traps.md             # Error signatures, context, and solutions
│
└── .memguard/               # Runtime Cache (Machine Readable, add to .gitignore)
    ├── runtime_state.json   # Serialized state graph for concurrent validation
    └── search_index.json    # Lightweight keyword index for instant retrieval
```

---

## 📐 Internal State Flow

```text
  [Agent Input] ──► [SOP Verification] ──► [MCP Tool Call]
                                                 │
                                                 ▼
  [Git MD Docs] ◄── [500ms Debounce] ◄── [Rust RwLock Engine] ──► [.memguard/ Cache]
```

---

## 📚 Architecture Reference

- [architecture.md](architecture.md) — Full architecture design document
- [blueprint.md](blueprint.md) — Original design blueprint
- [MCP Development & Debugging Whitepaper](MCP（Model%20Context%20Protocol）开发与调试白皮书.md)

---

## ⚖️ License

Licensed under the MIT License. Brand identity and commercial distribution controls apply — see the [memguard specification](https://github.com/liuhengyuan666/memguard) for full terms.
