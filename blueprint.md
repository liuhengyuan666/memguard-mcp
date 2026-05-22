# MemGuard v3: Git-Native MCP Runtime Engine (架构与实现蓝图)

## 1. 架构哲学与核心约束 (Architecture Philosophy)

- **Git-Native (项目即状态)**：拒绝使用 SQLite 等重型数据库。所有最终状态必须以 Markdown 形式落地，确保与 Git 版本控制完美兼容。
- **Source of Truth (唯一真相源)**：memory/*.md 是唯一真相源，具备人类可读性。
- **Runtime Cache (运行时加速)**：.memguard/*.json 是纯粹的派生缓存层，专为大模型的高频读写和语义检索设计。如果该目录被删除，系统必须能从 memory/ 目录完美重建它（Graceful Degradation）。
- **并发安全 (Thread Safety)**：所有跨 Agent 的状态写入必须通过 Rust 进程内存中的 RwLock（读写锁）进行排队和状态合并，杜绝多进程并发读写同一个 Markdown 文件的竞态灾难。

## 2. 目录体系结构 (Directory Structure)

Memguard 初始化后，将在宿主项目根目录接管以下文件结构：

Plaintext

```
[Host Project Root]/
├── memory/                  # (Source of Truth) 人类可读，随 Git 提交
│   ├── context.md           # 宏观任务、约束与当前阶段
│   ├── decisions.md         # 架构决策记录 (ADR)
│   └── traps.md             # 踩坑记录与解决方案
│
├── .memguard/               # (Runtime Cache) 机器可读，高频读写，可被 .gitignore
│   ├── runtime_state.json   # 内存状态快照
│   └── search_index.json    # 简易语义/关键词倒排索引 (未来可升级向量)
```

## 3. Rust 核心数据模型 (Core Domain Models)

> **Agent 指导原则：** 使用 serde 和 serde_json 实现序列化。这些结构体是内存状态的映射。

Rust

```
// src/models.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub current_phase: String,
    pub active_tasks: Vec<Task>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub status: TaskStatus, // Todo, InProgress, Done
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ADR {
    pub id: String,         // e.g., "ADR-001"
    pub title: String,
    pub status: String,     // Proposed, Accepted, Superseded
    pub context: String,
    pub decision: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trap {
    pub error_signature: String,
    pub context: String,
    pub solution: String,
}

// 统一事件总线枚举
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RuntimeEvent {
    TaskUpdated { task_id: String, new_status: TaskStatus },
    AdrCommitted(ADR),
    TrapRecorded(Trap),
    PhaseChanged(String),
}
```

## 4. 核心模块划分 (Module Breakdown)

要求本地 Agent 严格按照以下模块进行物理文件拆分：

- **main.rs**: 程序的入口点，初始化异步运行时 (Tokio) 和日志，启动标准输入输出的 MCP 监听循环。
- **engine/state_manager.rs**: 维护一个 Arc<RwLock>。负责接收 RuntimeEvent，修改内存状态，并触发对应的防抖（Debounce）落地写入。
- **engine/projection.rs**: 包含两个方向的转换逻辑：
  - *Parser*: Markdown -> Rust Structs (用于 Bootstrap 自举)。
  - *Renderer*: Rust Structs -> Markdown (将最新状态格式化写入 memory/*.md)。
- **mcp/server.rs**: 基于标准 JSON-RPC 2.0 协议的 MCP Server 实现。负责处理来自 Opencode 的 initialize、tools/list 和 tools/call 请求。

## 5. MCP 接口契约 (Tool API Specifications)

> **Agent 指导原则：** 不要对外暴露“文件操作”。只向 LLM 暴露“领域意图”。所有的文件 I/O 必须对客户端屏蔽。

### Tool 1: runtime_bootstrap

- **描述**：启动会话或发生上下文丢失时调用。
- **行为**：Rust 读取 memory/*.md，重建 .memguard/ 缓存，返回高度压缩的当前项目运行时摘要。
- **参数**：无
- **返回 (JSON)**：当前 Phase, 活跃 Tasks, 最新的一条 ADR。

### Tool 2: runtime_commit_event

- **描述**：统一的状态变更收口。当 Agent 遇到重大报错、完成任务或做出架构变更时调用。

- **行为**：更新内存状态树，触发 Projection 层异步重写对应的 Markdown。

- **参数 (JSON)**：

  JSON

  ```
  {
    "event_type": "AdrCommitted", // 或 TaskUpdated, TrapRecorded
    "payload": { ... } // 对应 Event 结构体
  }
  ```

```
*   **返回 (String)**："Event successfully committed to runtime state."

### Tool 3: runtime_query_memory
*   **描述**：语义检索。在 Agent 开始编写核心代码前强制调用，查询历史决策和避坑指南。
*   **行为**：在 .memguard/search_index.json 或内存中进行关键词查找，返回精炼的摘要。
*   **参数 (JSON)**：
    ```json
    {
      "query_intent": "authentication token validation",
      "limit": 3
    }
    
```

- **返回 (JSON)**：相关的 ADR 列表与核心结论，剔除冗长的背景说明。

## 6. Sisyphus / Hephaestus 开发执行计划 (Action Plan)

> **@Sisyphus (编排者) 请注意：** 请严格按照以下 4 个 Sprint 分配任务并推进：

- **Sprint 1: 骨架构建**
  1. 运行 cargo new memguard-mcp --bin。
  2. 在 Cargo.toml 中引入 tokio (features = ["full"]), serde, serde_json, anyhow, regex (用于 Markdown 解析)。
  3. 建立 models.rs 引入本文档第 3 节的数据结构。
- **Sprint 2: 投影与状态引擎 (Projection & State)**
  1. 实现 projection.rs：手写正则或状态机，能将标准的 ADR Markdown 格式解析为 Vec，反之亦然。
  2. 实现 state_manager.rs：使用 RwLock 保证并发安全，编写 apply_event 方法。
- **Sprint 3: MCP 协议实现 (Protocol Layer)**
  1. 实现监听 stdin 的 buf_reader。
  2. 按照 MCP JSON-RPC 规范响应 initialize 握手。
  3. 声明本规范第 5 节的 3 个 Tools。
  4. 实现 tools/call 路由，将参数反序列化后交给 state_manager 处理。
- **Sprint 4: 打包与集成 (Integration)**
  1. 提供一键编译构建脚本。
  2. 输出最终极简版的 memguard.md Skill 提示词，以便挂载至系统。