# MCP（Model Context Protocol）开发与调试白皮书

## —— OpenCode / oh-my-openagent / Cursor / Claude Desktop / Rust / Windows 实战避坑指南

------

# 一、前言

在为：

- OpenCode
- oh-my-openagent
- Cursor
- Claude Desktop
- VSCode Agent Framework

等现代 AI IDE / AI Agent Runtime 开发 MCP（Model Context Protocol）Server 时：

很多人会误以为：

```text
协议逻辑最难
```

实际上：

# 真正最容易崩的是：

- stdio
- child_process
- Windows shell
- argv
- cwd
- transport
- host runtime

这些工程层细节。

------

# 二、MCP 的本质

很多人把 MCP Server 理解成：

```text
一个 CLI 工具
```

这是错误的。

------

# MCP 真正的本质：

```text
一个基于 stdin/stdout 的长生命周期 daemon
```

即：

```text
Host
  ↕ stdin/stdout
MCP Server
```

MCP Server：

- 不应该像普通 CLI 那样输出日志
- 不应该随便退出
- 不应该污染 stdout
- 不应该依赖 shell wrapper

------

# 三、MCP Host 架构理解

以 OpenCode + oh-my-openagent 为例：

真正的 MCP 加载链路：

```text
oh-my-openagent
    ↓
Provider Registry
    ↓
Builtin MCPs
    ↓
User MCPs
    ↓
Plugin MCPs
```

源码核心逻辑：

```js
const merged = {
  ...createBuiltinMcps(),
  ...mcpResult.servers,
  ...userMcp,
  ...pluginComponents.mcpServers
}
```

------

# 结论

## 1. 用户 MCP 是支持的

你完全可以：

- 注册 github MCP
- 注册自定义 MCP
- 注册本地 Rust MCP

------

## 2. userMcp 会覆盖 builtin MCP

例如：

```json
"context7": {}
```

会覆盖：

```text
oh-my-openagent 内置 context7
```

------

## 3. MCP 名称冲突会导致 registry 混乱

不要覆盖：

- context7
- websearch
- lsp

除非你明确知道自己在做什么。

------

# 四、MCP 配置 Schema（最常见坑）

------

# ❌ 错误写法

## 1. command 写成字符串

```json
"command": "npx.cmd -y xxx"
```

------

## 2. 使用 args 字段

```json
"args": []
```

很多 MCP Host 根本不支持。

------

## 3. 使用 bat/cmd wrapper

```json
"command": ["start-mcp.bat"]
```

------

# ✅ 正确写法

```json
"command": [
  "npx.cmd",
  "-y",
  "@modelcontextprotocol/server-github"
]
```

------

# Rust MCP 推荐写法

```json
"command": [
  "T:\\work\\test-memguard-mcp\\target\\release\\memguard-mcp.exe",
  "T:\\work\\test-memguard-mcp"
]
```

------

# 五、理解 command 数组的真正含义

很多人误以为：

```json
"command": ["exe"]
```

只是程序路径。

实际上：

# command 数组 = 完整 argv

例如：

```json
"command": [
  "memguard-mcp.exe",
  "T:\\project"
]
```

等价于：

```bash
memguard-mcp.exe T:\project
```

------

# 六、Windows 最大天坑：不要用 .bat

------

# 现象

```text
The system cannot find the path specified
```

------

# 根因

Windows 下：

```text
.bat
.cmd
```

不是可执行文件。

Node/Bun 的：

```text
child_process.spawn()
```

无法直接运行它们。

------

# 你必须：

```text
cmd.exe /c xxx.bat
```

才能间接执行。

------

# 但问题远不止于此

bat/cmd wrapper 会：

- 吞 stdout
- 缓冲 stdio
- 重定向 stderr
- 提前退出 shell
- 破坏 pipe 生命周期

而：

# MCP 极度依赖纯净 stdio。

------

# ❌ 错误方案

```json
"command": [
  "start-mcp.bat"
]
```

------

# ✅ 正确方案

永远直接调用 exe：

```json
"command": [
  "T:\\work\\test-memguard-mcp\\target\\release\\memguard-mcp.exe",
  "T:\\work\\test-memguard-mcp"
]
```

------

# 七、Stdio 洁癖（MCP 最致命红线）

这是：

# MCP 开发第一原则。

------

# stdout

只能输出：

```json
{"jsonrpc":"2.0"...}
```

------

# stderr

才允许：

```text
[memguard] starting...
```

------

# 八、Rust 最容易踩的坑

------

# ❌ 错误

```rust
println!("server started");
dbg!(x);
```

------

# 为什么危险

因为：

```text
stdout 被污染
```

Host 会立刻：

- JSON parse failed
- Connection closed
- Error -32000

------

# ✅ 正确做法

## 使用 eprintln!

```rust
eprintln!("server started");
```

------

## tracing 指向 stderr

```rust
tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
```

------

# 九、Terminal Illusion（终端错觉）

这是非常经典的误判来源。

------

# 人眼看到：

```text
[memguard] starting...
{"jsonrpc":"2.0"...}
```

会误以为：

```text
协议正常
```

------

# 实际上

terminal 会混合：

- stdout
- stderr

一起显示。

------

# 所以：

你必须确认：

```rust
日志到底写到了哪里
```

而不是：

```text
终端看起来正常
```

------

# 十、OpenCode 不会复用你手动启动的 MCP

很多人：

```bash
memguard-mcp.exe
```

然后：

```bash
opencode
```

期待 OpenCode 自动连接。

------

# 这是错误理解。

------

# MCP stdio transport 的本质：

```text
Host spawn child process
```

Host：

- 自己启动进程
- 自己接管 stdin
- 自己接管 stdout
- 自己接管 stderr

不会连接你手动开的进程。

------

# 正确测试方法

------

# 第一阶段：协议测试

手动运行：

```bash
memguard-mcp.exe project_root
```

然后：

手动发送 JSON-RPC。

验证：

- initialize
- tools/list
- tools/call

------

# 第二阶段：集成测试

不要手动启动 MCP。

只：

```bash
opencode
```

让 Host 自己 spawn。

------

# 十一、argv 与 Host 隐藏参数问题

很多 MCP Host 会偷偷注入：

```text
--stdio
```

或者：

```text
--transport stdio
```

------

# ❌ 错误写法

```rust
let root = std::env::args().nth(1).unwrap();
```

------

# 风险

实际：

```bash
memguard-mcp.exe --stdio
```

结果：

```rust
root == "--stdio"
```

然后：

- path panic
- mkdir fail
- connection closed

------

# ✅ 正确做法

------

# 方案1：使用 clap

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long)]
    stdio: bool,

    project_root: Option<String>,
}
```

------

# 方案2：过滤参数

```rust
let args: Vec<String> = std::env::args()
    .skip(1)
    .filter(|a| !a.starts_with("--"))
    .collect();
```

------

# 方案3：提供 graceful fallback

```rust
let project_root = std::env::args()
    .nth(1)
    .unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap()
            .display()
            .to_string()
    });
```

------

# 十二、最关键调试方法论

------

# ❌ 错误思路

不停：

- 手动测试 JSON
- tools/list
- initialize

但：

集成依旧失败。

------

# 原因

手动启动：

- argv 不同
- cwd 不同
- env 不同
- stdio 不同

------

# 手动测试只能验证：

```text
协议逻辑
```

------

# 不能验证：

```text
系统集成
```

------

# ✅ 正确调试方式

在 main() 第一行：

```rust
eprintln!("ARGS = {:?}", std::env::args().collect::<Vec<_>>());

eprintln!("CWD = {:?}", std::env::current_dir());
```

------

# 然后：

直接：

```bash
opencode
```

------

# 去查看：

Host 的 error log。

------

# 真正重要的信息：

- argv
- cwd
- stderr
- panic
- spawn error

------

# 十三、如何快速判断问题属于哪一层

------

# 1. OpenCode 启动即报：

```text
The system cannot find the path specified
```

说明：

- 路径错误
- bat/cmd 问题
- shell wrapper 问题
- spawn 问题

不是协议问题。

------

# 2. Connection closed

说明：

- 子进程退出
- panic
- stdout 污染
- initialize 崩溃

------

# 3. JSON parse failed

说明：

# stdout 被污染。

------

# 4. tools/list 正常但 tool 调用失败

说明：

业务逻辑问题。

------

# 十四、memguard 实战案例（真实踩坑）

------

# 初始错误配置

```json
"command": [
  "T:\\work\\test-memguard-mcp\\start-mcp.bat"
]
```

------

# 导致：

```text
The system cannot find the path specified
```

以及：

```text
Connection closed
```

------

# 根因

bat wrapper：

- 不能直接 spawn
- 破坏 stdio
- child_process 生命周期异常

------

# 最终修复

```json
"memguard": {
  "type": "local",
  "enabled": true,
  "command": [
    "T:\\work\\test-memguard-mcp\\target\\release\\memguard-mcp.exe",
    "T:\\work\\test-memguard-mcp"
  ]
}
```

------

# 最终状态

```text
ast_grep Connected
context7 Connected
github Connected
grep_app Connected
lsp Connected
memguard Connected
websearch Connected
```

------

# 十五、MCP Server 最推荐工程实践

------

# 1. 使用官方 SDK

推荐：

- [Model Context Protocol 官方组织](https://github.com/modelcontextprotocol?utm_source=chatgpt.com)
- [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk?utm_source=chatgpt.com)

------

# 2. 永远 direct exe

不要 shell wrapper。

------

# 3. 所有日志统一 stderr

建议封装：

```rust
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!($($arg)*)
    };
}
```

------

# 4. 不要依赖 argv 固定位置

Host 行为并不统一。

------

# 5. 不要覆盖 builtin MCP 名称

例如：

- context7
- websearch
- lsp

------

# 6. 给所有 tool 做 graceful error

返回：

```json
{
  "code": -32603,
  "message": "xxx"
}
```

而不是 panic。

------

# 7. bootstrap 不要只依赖内存

缓存重建逻辑：

最好支持：

- runtime bootstrap
- process restart
- cache missing rebuild

------

# 十六、最终结论

MCP 开发最难的：

不是：

- Prompt
- LLM
- JSON-RPC

而是：

# stdio + process + transport 工程细节。

尤其：

- Windows
- Bun
- Node child_process
- stdio transport

组合。

------

# 最终一句话总结

真正稳定的 MCP Server：

```text
不是一个 CLI 工具
```

而是：

# 一个对 stdin/stdout 极度洁癖的长生命周期 daemon。