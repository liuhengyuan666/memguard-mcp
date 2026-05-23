# MemGuard MCP — Git-Native Memory Runtime

> **Version**: 0.1.0 (core) / 3.0.0 (spec)  
> **Runtime**: Rust MCP Server · RwLock thread-safe · Source-of-Truth in Markdown  
> **Companion**: [memguard Skill](https://github.com/liuhengyuan666/memguard) — Agent SOP (行为契约)

---

MemGuard MCP 是 memguard v3 双层架构的**能力层**——一个 Git-native 的 Rust MCP Server，提供 3 个原子工具供 AI Agent 调用。

> **⚠️ 重要**：本仓库只包含 MCP 运行时。Agent 行为契约（何时调用工具、遵循什么规则）在 [memguard 仓](https://github.com/liuhengyuan666/memguard) 的 `SKILL.md` 中。**两者必须同时安装才能正常工作。**

---

## 安装

### 从 npm（推荐）

```bash
npm install -g @henry_lhy/memguard-mcp
```

### 从源码编译

```bash
cargo build --release
# 二进制位于 target/release/memguard-mcp.exe (Windows)
# 或 target/release/memguard-mcp (macOS/Linux)
```

---

## 配置

在你的项目 `opencode.json` 中注册 MCP server：

```jsonc
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

然后安装配套 Skill（**必须**）：

```bash
mkdir -p .opencode/skills/memguard
curl -o .opencode/skills/memguard/SKILL.md \
  https://raw.githubusercontent.com/liuhengyuan666/memguard/main/memguard/SKILL.md
```

完整安装指南见 [memguard README](https://github.com/liuhengyuan666/memguard#readme)。

---

## MCP 工具

| 工具 | 功能 | 关键参数 |
|------|------|----------|
| `runtime_bootstrap` | 读取 memory/*.md，重建缓存，返回运行时摘要 | `project_root` (可选) |
| `runtime_commit_event` | 统一状态变更：TaskUpdated / AdrCommitted / TrapRecorded / PhaseChanged | `event_type` + `payload` |
| `runtime_query_memory` | 关键词搜索 ADR 和 Traps | `query_intent` (必填) |

> Agent **不应该**直接调用这些工具——应该由 Skill（SKILL.md）告诉 Agent **何时**调用。详见配套 Skill 的 SOP。

---

## Memory 目录

```
[项目根]/
├── memory/                  # Source of Truth（人类可读，随 Git 提交）
│   ├── context.md           # 当前阶段、活跃任务、约束
│   ├── decisions.md         # ADR 格式架构决策
│   └── traps.md             # 踩坑记录
│
└── .memguard/               # Runtime Cache（机器可读，应 .gitignore）
    ├── runtime_state.json   # 状态快照
    └── search_index.json    # 关键词索引
```

---

## 架构参考

- [architecture.md](architecture.md) — 完整架构设计文档
- [blueprint.md](blueprint.md) — 原始设计蓝图
- [MCP 开发与调试白皮书](MCP（Model%20Context%20Protocol）开发与调试白皮书.md)

---

## 许可

The source code is licensed under MIT.

However, the project name, logo, and branding are not permitted
to be used for commercial distribution without explicit permission.
