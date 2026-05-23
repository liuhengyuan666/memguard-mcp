# MemGuard v3 — Architecture & Design Reference

> **Runtime**: Rust MCP Server · Git-native · RwLock thread-safe · Source-of-Truth in Markdown
> **Version**: 0.1.0 (core) / 3.0.0 (spec)
> **Compatibility**: OpenCode MCP client

---

## 1. 架构哲学 (Architecture Philosophy)

| 原则 | 含义 |
|---|---|
| **Git-Native** | 所有持久化状态以 Markdown 落地，天然兼容 Git diff / merge / blame |
| **Source of Truth** | `memory/*.md` 是唯一真相源，人类可读；`.memguard/*.json` 是纯派生缓存 |
| **Graceful Degradation** | 删除 `.memguard/` 后，`runtime_bootstrap` 必须能从 `memory/` 完美重建 |
| **Concurrency Safety** | 所有跨 Agent 的状态写入通过 `Arc<RwLock<T>>` 排队序列化，杜绝竞态 |
| **Multi-Project Isolation** | 一个 MCP 服务器实例可以服务多个项目目录，记忆严格隔离 |
| **Project-Aware** | 启动时自动探测项目根；MCP 握手时根据客户端声明自动修正 |

---

## 2. 目录体系 (Directory Structure)

MemGuard 在每个宿主项目根目录下接管以下文件结构：

```
[Host Project Root]/           # ← 动态确定的项目根目录
├── memory/                    # Source of Truth（人类可读，随 Git 提交）
│   ├── context.md             # 当前阶段、活跃任务、约束条件
│   ├── decisions.md           # ADR 格式的架构决策记录（追加式）
│   └── traps.md               # 踩坑记录：错误签名 + 上下文 + 解决方案
│
└── .memguard/                 # Runtime Cache（机器可读，应被 .gitignore）
    ├── runtime_state.json     # 完整内存状态快照
    └── search_index.json      # ADR / Trap 的轻量关键词索引
```

**隔离子目录**（不随 Git 提交）：

```
target/                        # Rust 编译产物
src/                           # 源代码
  ├── main.rs                  # 入口点 + 路径解析
  ├── models.rs                # 领域数据模型
  ├── engine/
  │   ├── mod.rs
  │   ├── state_manager.rs     # 状态机 + 防抖写入 + 项目切换
  │   └── projection.rs        # Markdown ↔ Rust 双向转换
  └── mcp/
      ├── mod.rs
      └── server.rs            # MCP JSON-RPC 2.0 协议层
```

---

## 3. 核心数据模型 (`src/models.rs`)

所有结构体通过 `serde` 序列化。`RuntimeEvent` 是统一事件总线入口。

```
RuntimeState
├── current_phase: String          # e.g. "planning", "implementation"
├── active_tasks: Vec<Task>        # 当前活跃任务列表
└── constraints: Vec<String>       # 架构约束条件

Task
├── id: String                     # e.g. "TASK-000"
├── description: String
└── status: TaskStatus             # Todo | InProgress | Done

ADR (Architecture Decision Record)
├── id, title, status              # status: active | superseded
├── context, decision              # 自由文本 Markdown
└── tags: Vec<String>              # e.g. ["rust", "backend"]

Trap
├── error_signature: String        # e.g. "NPE in auth handler"
├── context: String                # 触发语境
└── solution: String               # 修复方案

RuntimeEvent (enum, serde tagged)
├── TaskUpdated { task_id, new_status }
├── AdrCommitted(ADR)
├── TrapRecorded(Trap)
└── PhaseChanged(String)
```

---

## 4. 模块划分与职责 (Module Breakdown)

### 4.1 `main.rs` — 入口点 & 项目路径解析

**职责**：
- 启动 Tokio 多线程运行时
- 调用 `resolve_project_root()` 确定活跃项目目录
- 构造 `StateManager` + `McpServer`，启动 MCP 监听循环

**路径解析策略（三级回退）**：

```
Tier 1: MCP initialize workspaceFolders / rootUri
        → 客户端显式声明（最高权威，自动修正）
Tier 2: MEMGUARD_PROJECT_ROOT 环境变量
        → 用户显式覆盖（静态配置）
Tier 3: std::env::current_dir()
        → OpenCode 启动 MCP 时自动设为项目目录（全局安装默认）
Tier 4: CLI 参数 args[1]
        → 遗留单项目部署模式（DEPRECATED，使用时打印 WARNING）
```

> **设计 rationale**：全局安装场景下，exe 的物理位置与项目根无关。CLI args 极易被误配（常见错误：将 exe 目录作为 args 传入），因此 CWD 和 MCP handshake 才是可靠来源。`handle_initialize()` 在收到 `workspaceFolders` 时会调用 `switch_project()` 自动修正到正确目录。

### 4.2 `engine/state_manager.rs` — 状态机引擎

**职责**：
- 维护 `RuntimeState`、`Vec<ADR>`、`Vec<Trap>` 三棵内存状态树（各自由 `Arc<RwLock<T>>` 保护）
- 接收 `RuntimeEvent`，修改内存状态，触发防抖写入
- 管理 `project_root`（`Arc<RwLock<PathBuf>>`）支持运行时切换
- 实现 Bootstrap（从磁盘加载）和 Greenfield（初始化空项目）

**核心方法**：

| 方法 | 行为 |
|---|---|
| `bootstrap()` | 读取 `memory/*.md` → 解析 → 填充内存；若目录不存在则创建默认文件 |
| `apply_event(event)` | 加写锁 → 修改内存状态 → 释放锁 → 发送防抖信号 |
| `flush_now()` | 立即强制写入所有 Markdown + JSON（bypass 防抖） |
| `switch_project(new_root)` | **flush_now()** → bump generation → 更新 root → 清空状态 → **bootstrap()** |

**防抖写入（Debounced Flush）**：
- `apply_event` 不直接写磁盘，只向 `mpsc::unbounded_channel` 发送信号
- 后台 Tokio task 等待 500ms 静默窗口后再执行批量写入
- 连续事件在 500ms 内重置计时器，避免频繁 I/O

### 4.3 `engine/projection.rs` — Markdown 双向转换

**职责**：负责 Memory 层和 Rust 结构体之间的双向序列化/反序列化。

| 方向 | 函数 | Markdown 文件 |
|---|---|---|
| Markdown → Struct | `parse_context()` | `context.md` |
| Markdown → Struct | `parse_decisions()` | `decisions.md` |
| Markdown → Struct | `parse_traps()` | `traps.md` |
| Struct → Markdown | `render_context()` | `context.md` |
| Struct → Markdown | `render_decisions()` | `decisions.md` |
| Struct → Markdown | `render_traps()` | `traps.md` |

解析层使用 `regex` 做结构化行匹配；格式错误的条目会被跳过并输出 warning。

### 4.4 `mcp/server.rs` — MCP 协议层

**职责**：实现 JSON-RPC 2.0 over stdio 的 MCP Server。

| MCP 方法 | 实现 |
|---|---|
| `initialize` | 解析 `workspaceFolders` / `rootUri`；若不匹配则**自动调用 `switch_project` 修正** |
| `tools/list` | 返回 3 个工具的 JSON Schema |
| `tools/call` | 路由到对应的 tool handler |

**MCP 工具**：

| Tool | 参数 | 行为 |
|---|---|---|
| `runtime_bootstrap` | `project_root?` (string, 可选) | 切换项目 → bootstrap → 返回压缩摘要 |
| `runtime_commit_event` | `event_type` + `payload` | 反序列化 `RuntimeEvent` → `apply_event` |
| `runtime_query_memory` | `query_intent` + `limit?` | 对 ADR / Trap 做关键词匹配，按分数排序 |

**`initialize` 握手自动修正**：
当客户端在 `initialize` 请求中声明 `workspaceFolders` 或 `rootUri` 与当前 `project_root` 不一致时，服务器不再只输出警告，而是直接调用 `StateManager::switch_project()` 修正到正确目录。这解决了 "exe 放在目录 A，项目在目录 B" 的经典部署场景。

---

## 5. 并发模型与数据安全 (Concurrency Model)

### 5.1 锁层级（无死锁保障）

MemGuard 维护四把锁，所有代码路径严格遵循"获取 → 使用 → 释放"的顺序，**绝不跨 `.await` 持有锁**：

| 锁 | 类型 | 用途 |
|---|---|---|
| `state` | `Arc<RwLock<RuntimeState>>` | 阶段 / 任务 / 约束 |
| `decisions` | `Arc<RwLock<Vec<ADR>>>` | 架构决策历史 |
| `traps` | `Arc<RwLock<Vec<Trap>>>` | 踩坑记录 |
| `project_root` | `Arc<RwLock<PathBuf>>` | 当前活跃项目目录 |

**锁获取顺序审计**：

| 代码路径 | 锁序列 |
|---|---|
| `bootstrap()` | `project_root.read()` → `state.write()` → `decisions.write()` → `traps.write()` |
| `apply_event(event)` | 单把写锁（state OR decisions OR traps），关键区极短 |
| `query_memory` | `decisions.read()` + `traps.read()` 同时持有（均为读锁，安全） |
| `flush_inner()` | `state.read()` → `decisions.read()` → `traps.read()`（顺序获取，用完释放） |
| `switch_project()` | `project_root.write()` → `state.write()` → `decisions.write()` → `traps.write()` → `bootstrap()`（其中 bootstrap 会再次 `project_root.read()` — 但此时 write lock 已释放） |

**结论**：无循环等待 = 无死锁。

### 5.2 Generation Counter（防竞态）

**背景**：flush 后台任务分两个时刻读取 `project_root` 和 `state`，中间存在时间窗口。`switch_project` 可能在此间隙内切换项目和状态。

**机制**：`AtomicU64` 全局 generation counter。

```
switch_project():
  ① flush_now()           ← 旧项目数据安全落盘
  ② gen.fetch_add(1)      ← 通知所有在途 flush 任务："世界已变"
  ③ 更新 root → 清空状态 → bootstrap  ← 切换

flush 任务（每个周期）:
  ① 快照 gen_snapshot
  ② 读取 root
  ③ 二次校验 gen == gen_snapshot → 不匹配则 abort（跳过本轮写入）
  ④ 写入 memory/*.md + .memguard/*.json
  ⑤ 事后校验 gen == gen_snapshot → 不匹配则 WARNING（下次 flush 自动修正）
```

**效果**：flush 与 switch 之间不会发生 "新项目数据写入旧项目目录" 的跨项目数据泄漏。

---

## 6. 完整工作流 (End-to-End Lifecycle)

### 6.1 服务器启动 → 项目就绪

```
1. OpenCode 启动 MCP 子进程 (memguard-mcp.exe)
2. main.rs → resolve_project_root()
   ├─ 尝试 CLI args[1]
   ├─ 尝试 MEMGUARD_PROJECT_ROOT 环境变量
   ├─ 从 process CWD 向上查找 .git / .omo / Cargo.toml 等
   └─ 回退到 process CWD（带 WARNING）
3. 创建 StateManager(project_root) → spawn 防抖 flush task
4. 创建 McpServer(state_manager)
5. server.run() → bootstrap() → 读取/初始化 memory/ 文件
6. 进入 stdin → JSON-RPC → dispatch 循环
```

### 6.2 MCP 握手 → 自动修正

```
1. 客户端发送 initialize 请求
   { "workspaceFolders": [{ "uri": "file:///T:/work/my-project" }] }
2. handle_initialize 解析 workspaceFolders[0].uri → 去掉 "file:///" 前缀
3. 与当前 project_root 比较
   ├─ 一致 → 日志 "aligns"
   └─ 不一致 → 日志 WARNING → 调用 switch_project(my-project) → bootstrap
4. 返回 initialize response（protocolVersion, capabilities, serverInfo）
```

### 6.3 Agent 调用工具 → 状态持久化

```
1. Agent 调用 runtime_bootstrap
   ├─ (可选) 传入 project_root → switch_project → bootstrap
   └─ 返回当前 phase / tasks / latest ADR / constraints
2. Agent 调用 runtime_query_memory
   └─ 关键词匹配 → 按分数排序 → 返回截断摘要
3. Agent 写代码 / 做决策
4. Agent 调用 runtime_commit_event { AdrCommitted / TaskUpdated / ... }
   ├─ StateManager.apply_event() → 加写锁 → 修改内存
   ├─ 释放写锁 → 发送 flush 信号到 mpsc channel
   └─ 500ms 静默后 → flush task 写入 Markdown + JSON
```

### 6.4 跨项目切换

```
1. Agent 在新项目调用 runtime_bootstrap({ "project_root": "/new/project" })
2. tool_runtime_bootstrap → state_manager.switch_project("/new/project")
   ├─ flush_now() ← 旧项目数据安全落盘
   ├─ gen.fetch_add(1) ← 终止在途 flush
   ├─ project_root.write() = "/new/project"
   ├─ 清空 state / decisions / traps
   └─ bootstrap() ← 从 /new/project/memory/ 加载
3. 后续所有操作（query_memory, commit_event）自动操作新项目的数据
```

---

## 7. 构建与部署 (Build & Deploy)

### 7.1 构建

```powershell
.\build.ps1              # 编译 release + 运行测试 + 输出配置示例
.\build.ps1 -Install     # 同上 + 显示针对当前目录的 opencode.json 配置
```

### 7.2 OpenCode 配置

**方式 A — 环境变量（推荐，灵活）**：

```json
{
  "mcpServers": {
    "memguard": {
      "command": "path/to/memguard-mcp.exe",
      "env": { "MEMGUARD_PROJECT_ROOT": "T:/work/my-project" }
    }
  }
}
```

**方式 B — CLI 参数（固定路径）**：

```json
{
  "mcpServers": {
    "memguard": {
      "command": "path/to/memguard-mcp.exe",
      "args": ["T:/work/my-project"]
    }
  }
}
```

**方式 C — 自动探测（无需额外配置）**：
如果 OpenCode 启动 MCP 服务器时的 CWD 就是项目目录，MemGuard 会自动向上查找 `.git` / `package.json` 等标记定位项目根。如果不匹配，MCP 的 `initialize` 握手阶段也会自动修正。

---

## 8. 关键设计决策记录 (Key Design Decisions)

| 决策 | 理由 |
|---|---|
| **Rust 而非 TypeScript** | MCP Server 作为独立二进制发行，零依赖安装；RwLock 天然并发安全 |
| **Markdown 作为持久化格式** | Git diff 友好，人类可读，无需数据库 |
| **防抖写入（500ms）** | 避免 Agent 频繁调用 `commit_event` 时产生过多 I/O |
| **Generation Counter 代替互斥锁** | 避免 flush task 和 switch 之间的大范围阻塞；允许 flush 无损终止 |
| **`project_root` 动态切换** | 支持全局配置 + 多项目隔离，避免 "exe 目录污染" 问题 |
| **`initialize` 握手自动修正** | 即使启动时路径解析错误，客户端声明的工作区路径也能在握手阶段修正 |

---

## 9. 文件索引 (File Index)

| 文件 | 行数 | 职责 |
|---|---|---|
| `src/main.rs` | ~109 | 入口点 + `resolve_project_root()` + `find_project_root()` |
| `src/models.rs` | ~51 | 5 个领域结构体 + `RuntimeEvent` 枚举 |
| `src/engine/state_manager.rs` | ~690 | 状态机、防抖 flush、generation counter、项目切换 |
| `src/engine/projection.rs` | ~561 | Markdown ↔ Rust 双向转换 + 18 个单元测试 |
| `src/mcp/server.rs` | ~700 | MCP JSON-RPC 2.0、工具路由、initialize 自动修正 |
| `build.ps1` | ~90 | 一键构建 + 安装配置指南 |
| `memguard.md` | ~109 | Agent Skill 规范（面向 LLM 的运行时契约） |
| `blueprint.md` | ~154 | 原始架构蓝图（设计阶段产物） |
