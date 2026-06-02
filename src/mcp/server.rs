use crate::engine::projection::canonicalize_phase;
use crate::engine::state_manager::{AdrError, StateManager};
use crate::models::{AdrStatus, RuntimeEvent, TaskStatus};
use crate::search::index::{SearchIndex, SearchItem};
use crate::search::scorer::truncate;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ── JSON-RPC 2.0 Types ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// ── MCP Server ────────────────────────────────────────────────────────────

/// MCP-compliant JSON-RPC 2.0 server that reads from stdin and writes
/// responses to stdout.  All business logic is delegated to `StateManager`.
pub struct McpServer {
    state_manager: Arc<StateManager>,
}

impl McpServer {
    pub fn new(state_manager: Arc<StateManager>) -> Self {
        Self { state_manager }
    }

    /// Run the server loop: read JSON-RPC lines from stdin, dispatch, write
    /// responses to stdout.  Runs until stdin is closed.
    pub async fn run(&self, project_root: &str) -> Result<()> {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        let mut stdout = tokio::io::stdout();
        eprintln!(
            "[memguard] MCP server starting for project: {}",
            project_root
        );

        // Bootstrap is deferred to `handle_initialize()` —the MCP client
        // sends its workspace root in the initialize request, which is the
        // authoritative project path.  Starting with CWD avoids heuristic
        // guesswork that could find the wrong project.
        eprintln!("[memguard] Waiting for MCP initialize handshake...");

        // TODO(Robustness): Current stdio transport assumes strict JSON-Lines (newline delimited).
        // If the MCP client sends LSP-style headers (e.g., Content-Length: \r\n\r\n),
        // this lines() iterator will fail. Upgrade to a length-prefixed buffer reader if needed.
        while let Some(line) = lines
            .next_line()
            .await
            .context("stdin read error")?
        {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    // Filter out non-JSON transport headers (LSP-style, HTTP-style).
                    if line.starts_with("Content-Length:")
                        || line.starts_with("Content-Type:")
                        || line.trim().is_empty()
                    {
                        continue; // silently skip transport headers
                    }
                    let err_resp = JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: None,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                            data: None,
                        }),
                    };
                    let json = serde_json::to_string(&err_resp)?;
                    stdout.write_all(json.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                    continue;
                }
            };

            // Handle notifications (no id field).
            if req.id.is_none() {
                if req.method == "notifications/initialized" {
                    eprintln!("[memguard] MCP handshake complete.");
                }
                // Notifications get no response.
                continue;
            }

            let response = self.dispatch(&req.method, req.params).await;

            let resp = match response {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: req.id,
                    result: Some(result),
                    error: None,
                },
                Err(e) => {
                    eprintln!(
                        "[memguard] Error handling '{}': {}",
                        req.method, e
                    );
                    JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(match e {
                            McpError::MethodNotFound(m) => JsonRpcError {
                                code: -32601,
                                message: m,
                                data: None,
                            },
                            McpError::InvalidParams(m) => JsonRpcError {
                                code: -32602,
                                message: m,
                                data: None,
                            },
                            McpError::Internal(m) => JsonRpcError {
                                code: -32603,
                                message: m,
                                data: None,
                            },
                        }),
                    }
                }
            };

            let json = serde_json::to_string(&resp)?;
            stdout.write_all(json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        eprintln!("[memguard] stdin closed, flushing state before shutdown...");
        if let Err(e) = self.state_manager.flush_now().await {
            eprintln!("[memguard] ERROR during shutdown flush: {}", e);
        }
        eprintln!("[memguard] shutting down.");
        Ok(())
    }

    /// Route a method name to the correct handler.
    async fn dispatch(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        match method {
            "initialize" => self.handle_initialize(params).await,
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => self.handle_tools_call(params).await,
            _ => Err(McpError::MethodNotFound(format!(
                "Unknown method: {}",
                method
            ))),
        }
    }

    // ── MCP Protocol Handlers ─────────────────────────────────────────

    /// MCP initialize handshake.
    ///
    /// Uses `workspaceFolders` or `rootUri` from the client to determine
    /// the authoritative project root.  Bootstraps runtime state from that
    /// directory's `memory/*.md` files.  This is the SINGLE bootstrap point —
    /// `server.run()` does NOT bootstrap, so the server always starts from
    /// the correct workspace.
    async fn handle_initialize(
        &self,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        let mut workspace_root: Option<PathBuf> = None;

        if let Some(p) = params {
            let maybe_uri = p
                .get("workspaceFolders")
                .and_then(|wf| wf.as_array())
                .and_then(|arr| arr.first())
                .and_then(|folder| folder.get("uri"))
                .and_then(|uri| uri.as_str())
                .or_else(|| p.get("rootUri").and_then(|uri| uri.as_str()));

            if let Some(uri) = maybe_uri {
                let inferred = uri
                    .strip_prefix("file:///")
                    .or_else(|| uri.strip_prefix("file://"))
                    .unwrap_or(uri);
                let inferred_path = PathBuf::from(inferred);
                let inferred_path = inferred_path.canonicalize().unwrap_or_else(|e| {
                    eprintln!(
                        "[memguard] WARNING: cannot canonicalize workspace path '{}': {} —using raw path",
                        inferred, e
                    );
                    PathBuf::from(inferred)
                });
                workspace_root = Some(inferred_path);
            }
        }

        // Bootstrap from the authoritative workspace root.
        if let Some(root) = workspace_root {
            let current = self.state_manager.project_root.read().await.clone();
            if root != current {
                eprintln!(
                    "[memguard] MCP workspace differs from startup root. \
                     Switching: {} -> {}",
                    current.display(),
                    root.display()
                );
                if let Err(e) = self.state_manager.switch_project(root).await {
                    eprintln!(
                        "[memguard] ERROR bootstrapping from workspace: {}",
                        e
                    );
                }
            } else {
                eprintln!(
                    "[memguard] MCP workspace aligns with startup root: {}",
                    root.display()
                );
                if let Err(e) = self.state_manager.bootstrap().await {
                    eprintln!(
                        "[memguard] ERROR bootstrapping from aligned root: {}",
                        e
                    );
                }
            }
        } else {
            eprintln!(
                "[memguard] WARNING: MCP client did not provide workspaceFolders or rootUri."
            );
            eprintln!(
                "[memguard] Bootstrapping from startup project root (CWD): \
                 ensure OpenCode spawns this server with the project directory as CWD."
            );
            if let Err(e) = self.state_manager.bootstrap().await {
                eprintln!(
                    "[memguard] ERROR bootstrapping from startup root: {}",
                    e
                );
            }
        }

        eprintln!("[memguard] Runtime state bootstrapped.");

        Ok(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "memguard-mcp",
                "version": "0.3.3"
            }
        }))
    }

    /// Return the tool list per MCP specification.
    async fn handle_tools_list(&self) -> Result<Value, McpError> {
        Ok(serde_json::json!({
            "tools": [
                {
                    "name": "runtime_bootstrap",
                    "description": "Start session or recover from context loss. Reads memory/*.md, rebuilds cache, returns compressed runtime summary. Optionally accepts project_root to switch to a different project directory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_root": {
                                "type": "string",
                                "description": "Optional absolute path to the project root directory. If provided, memguard will switch to this project's memory context."
                            }
                        }
                    }
                },
                {
                    "name": "runtime_commit_event",
                    "description": "Unified state change entrypoint. Commit a task update, ADR, trap, or phase change to runtime state.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "event_type": {
                                "type": "string",
                                "enum": ["TaskCreated", "TaskUpdated", "AdrCommitted", "TrapRecorded", "PhaseChanged"],
                                "description": "Type of runtime event"
                            },
                            "payload": {
                                "type": "object",
                                "description": "Event payload matching the event_type schema"
                            }
                        },
                        "required": ["event_type", "payload"]
                    }
                },
                {
                    "name": "runtime_query_memory",
                    "description": "Semantic search over decisions and traps. Call before writing core code to check history.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query_intent": {
                                "type": "string",
                                "description": "Natural language description of what you're looking for"
                            },
                            "limit": {
                                "type": "integer",
                                "default": 3,
                                "description": "Max number of results"
                            },
                            "include_stale": {
                                "type": "boolean",
                                "default": false,
                                "description": "Include superseded/deprecated/rejected ADRs in search results"
                            }
                        },
                        "required": ["query_intent"]
                    }
                }
            ]
        }))
    }

    /// Route tools/call to the appropriate tool implementation.
    async fn handle_tools_call(
        &self,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".into()))?;

        let tool_name = params["name"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("Missing tool name".into()))?;

        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

        match tool_name {
            "runtime_bootstrap" => self.tool_runtime_bootstrap(arguments).await,
            "runtime_commit_event" => {
                self.tool_runtime_commit_event(arguments).await
            }
            "runtime_query_memory" => {
                self.tool_runtime_query_memory(arguments).await
            }
            other => Err(McpError::MethodNotFound(format!(
                "Unknown tool: {}",
                other
            ))),
        }
    }

    // ── Tool Implementations ──────────────────────────────────────────

    /// runtime_bootstrap: return compressed runtime summary.
    ///
    /// If `project_root` is provided in `args`, switches the active project
    /// context before returning the summary.  This allows a single memguard
    /// process to serve multiple projects.
    async fn tool_runtime_bootstrap(
        &self,
        args: Value,
    ) -> Result<Value, McpError> {
        // Optional project switch.
        if let Some(root_str) = args.get("project_root").and_then(|v| v.as_str()) {
            let new_root = PathBuf::from(root_str)
                .canonicalize()
                .map_err(|e| {
                    McpError::InvalidParams(format!(
                        "Invalid project_root path '{}': {}",
                        root_str, e
                    ))
                })?;
            eprintln!(
                "[memguard] runtime_bootstrap switching to project: {}",
                new_root.display()
            );
            self.state_manager
                .switch_project(new_root)
                .await
                .map_err(|e| {
                    McpError::Internal(format!(
                        "Project switch failed: {}",
                        e
                    ))
                })?;
        }

        let state = self.state_manager.state.read().await;
        let decisions = self.state_manager.decisions.read().await;
        let traps = self.state_manager.traps.read().await;

        let adr_count = decisions.len();
        let trap_count = traps.len();

        let latest_adr = decisions
            .iter()
            .rev()
            .find(|a| matches!(a.status, AdrStatus::Accepted | AdrStatus::Proposed))
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "title": a.title,
                    "status": a.status,
                })
            });

        let tasks: Vec<Value> = state
            .active_tasks
            .iter()
            .filter(|t| !matches!(t.status, TaskStatus::Done))
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "description": t.description,
                    "status": format!("{:?}", t.status),
                })
            })
            .collect();

        // Order: decisions & constraints first, tasks last.
        // This prevents the bootstrap output from biasing toward
        // task-management over architectural continuity.
        Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&serde_json::json!({
                    "current_phase": state.current_phase,
                    "constraints": state.constraints,
                    "latest_adr": latest_adr,
                    "adr_count": adr_count,
                    "trap_count": trap_count,
                    "active_tasks": tasks,
                })).unwrap_or_default()
            }]
        }))
    }

    /// runtime_commit_event: deserialize and apply a RuntimeEvent.
    async fn tool_runtime_commit_event(
        &self,
        args: Value,
    ) -> Result<Value, McpError> {
        let event_type = args["event_type"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("Missing event_type".into()))?;

        let payload = args
            .get("payload")
            .ok_or_else(|| McpError::InvalidParams("Missing payload".into()))?;

        let event = parse_event(event_type, payload)?;

        match self.state_manager.apply_event(event).await {
            Ok(()) => Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": "Event successfully committed to runtime state."
                }]
            })),
            Err(ref e) if e.to_string().starts_with("[CONFLICT]") => {
                let conflict_json = if let Some(adr_err) = e.downcast_ref::<AdrError>() {
                    match adr_err {
                        AdrError::InvalidTransition { id, current_status, valid_next } => {
                            let valid_strs: Vec<String> = valid_next
                                .iter()
                                .map(|s| format!("{}", s))
                                .collect();
                            serde_json::json!({
                                "error": "invalid_transition",
                                "adr_id": id,
                                "reason": "terminal_state",
                                "current_status": format!("{}", current_status),
                                "valid_next": valid_strs,
                                "suggestion": format!("ADR {} is in terminal state {} and cannot transition further.", id, current_status)
                            })
                        }
                    }
                } else {
                    serde_json::json!({
                        "error": "conflict",
                        "message": e.to_string()
                    })
                };
                Ok(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string(&conflict_json).unwrap_or_default()
                    }],
                    "isError": true
                }))
            }
            Err(e) => Err(McpError::Internal(format!("State update failed: {}", e))),
        }
    }

    /// runtime_query_memory: keyword search over ADRs and Traps.
    async fn tool_runtime_query_memory(
        &self,
        args: Value,
    ) -> Result<Value, McpError> {
        let query = args["query_intent"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("Missing query_intent".into()))?
            .to_lowercase();

        let limit = args["limit"]
            .as_u64()
            .unwrap_or(3)
            .clamp(1, 20) as usize;

        let include_stale = args["include_stale"].as_bool().unwrap_or(false);

        let decisions = self.state_manager.decisions.read().await;
        let traps = self.state_manager.traps.read().await;

        let index = SearchIndex::build(&decisions, &traps);
        let search_results = index.search(&query, limit, include_stale);

        let items: Vec<Value> = search_results
            .into_iter()
            .map(|result| match result.item {
                SearchItem::Adr(adr) => serde_json::json!({
                    "type": "ADR",
                    "id": adr.id,
                    "title": adr.title,
                    "status": adr.status,
                    "summary": truncate(&adr.decision, 200),
                    "tags": adr.tags,
                }),
                SearchItem::Trap(trap) => serde_json::json!({
                    "type": "Trap",
                    "signature": trap.error_signature,
                    "solution": trap.solution,
                    "root_cause": trap.root_cause,
                    "prevention": trap.prevention,
                }),
            })
            .collect();

        Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&serde_json::json!({
                    "query": query,
                    "results": items,
                    "total": items.len(),
                })).unwrap_or_default()
            }]
        }))
    }
}

// ── Event Parsing ─────────────────────────────────────────────────────────

/// Parse an event_type + payload JSON into a RuntimeEvent.
fn parse_event(event_type: &str, payload: &Value) -> Result<RuntimeEvent, McpError> {
    match event_type {
        "TaskUpdated" => {
            let task_id = payload["task_id"]
                .as_str()
                .ok_or_else(|| McpError::InvalidParams("Missing task_id".into()))?
                .to_string();
            let status_str = payload["new_status"]
                .as_str()
                .ok_or_else(|| {
                    McpError::InvalidParams("Missing new_status".into())
                })?;
            let new_status = match status_str {
                "Todo" => TaskStatus::Todo,
                "InProgress" => TaskStatus::InProgress,
                "Done" => TaskStatus::Done,
                other => {
                    return Err(McpError::InvalidParams(format!(
                        "Invalid task status: {} (expected Todo|InProgress|Done)",
                        other
                    )));
                }
            };
            Ok(RuntimeEvent::TaskUpdated {
                task_id,
                new_status,
            })
        }
        "AdrCommitted" => {
            let adr: crate::models::ADR =
                serde_json::from_value(payload.clone()).map_err(|e| {
                    McpError::InvalidParams(format!(
                        "Invalid ADR payload: {}",
                        e
                    ))
                })?;
            Ok(RuntimeEvent::AdrCommitted(adr))
        }
        "TrapRecorded" => {
            let trap: crate::models::Trap =
                serde_json::from_value(payload.clone()).map_err(|e| {
                    McpError::InvalidParams(format!(
                        "Invalid Trap payload: {}",
                        e
                    ))
                })?;
            Ok(RuntimeEvent::TrapRecorded(trap))
        }
        "PhaseChanged" => {
            let phase = payload["new_phase"]
                .as_str()
                .or_else(|| payload.as_str())
                .ok_or_else(|| {
                    McpError::InvalidParams("Missing new_phase".into())
                })?;
            Ok(RuntimeEvent::PhaseChanged(canonicalize_phase(phase)))
        }
        "TaskCreated" => {
            let payload: crate::models::TaskCreatePayload =
                serde_json::from_value(payload.clone()).map_err(|e| {
                    McpError::InvalidParams(format!(
                        "Invalid Task payload: {}",
                        e
                    ))
                })?;
            Ok(RuntimeEvent::TaskCreated(crate::models::Task {
                id: payload.id,
                description: payload.description,
                status: crate::models::TaskStatus::Todo,
            }))
        }
        other => Err(McpError::InvalidParams(format!(
            "Unknown event_type: {}",
            other
        ))),
    }
}

// ── Error Type ────────────────────────────────────────────────────────────

#[derive(Debug)]
enum McpError {
    MethodNotFound(String),
    InvalidParams(String),
    Internal(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::MethodNotFound(m) => write!(f, "Method not found: {}", m),
            McpError::InvalidParams(m) => write!(f, "Invalid params: {}", m),
            McpError::Internal(m) => write!(f, "Internal error: {}", m),
        }
    }
}

impl std::error::Error for McpError {}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_event_task_updated() {
        let payload = serde_json::json!({
            "task_id": "TASK-000",
            "new_status": "Done"
        });
        let event = parse_event("TaskUpdated", &payload).unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::TaskUpdated { .. }
        ));
    }

    #[test]
    fn test_parse_event_invalid_status() {
        let payload = serde_json::json!({
            "task_id": "TASK-000",
            "new_status": "InvalidStatus"
        });
        let result = parse_event("TaskUpdated", &payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_event_unknown_type() {
        let payload = serde_json::json!({});
        let result = parse_event("UnknownEvent", &payload);
        assert!(result.is_err());
    }

    // ── P1-D: stale ADR filtering tests ─────────────────────────────────────

    use crate::models::{ADR, Task, TaskStatus, Trap};

    /// Helper: extract the "results" array from tool_runtime_query_memory output.
    fn extract_results(resp: &serde_json::Value) -> Vec<serde_json::Value> {
        let text = resp["content"][0]["text"].as_str().unwrap_or("[]");
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        parsed["results"].as_array().cloned().unwrap_or_default()
    }

    #[tokio::test]
    async fn test_query_memory_filters_stale_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        // Inject a superseded ADR that would match "auth" and an unrelated active ADR.
        {
            let mut decisions = server.state_manager.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "auth module".into(),
                status: AdrStatus::Superseded,
                context: "legacy".into(),
                decision: "old".into(),
                tags: vec!["security".into()],
            });
            decisions.push(ADR {
                id: "ADR-002".into(),
                title: "database migration".into(),
                status: AdrStatus::Accepted,
                context: "sql".into(),
                decision: "use postgres".into(),
                tags: vec!["db".into()],
            });
        }

        let args = serde_json::json!({"query_intent": "auth"});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 0, "superseded ADR should be filtered by default");
    }

    #[tokio::test]
    async fn test_query_memory_include_stale() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        {
            let mut decisions = server.state_manager.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "auth module".into(),
                status: AdrStatus::Accepted,
                context: "legacy".into(),
                decision: "old".into(),
                tags: vec!["security".into()],
            });
            decisions.push(ADR {
                id: "ADR-002".into(),
                title: "auth module".into(),
                status: AdrStatus::Superseded,
                context: "legacy".into(),
                decision: "old".into(),
                tags: vec!["security".into()],
            });
        }

        let args = serde_json::json!({"query_intent": "auth", "include_stale": true});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 2, "both ADRs should be returned");
        assert_eq!(results[0]["id"], "ADR-001", "active ADR should rank higher");
        assert_eq!(results[1]["id"], "ADR-002", "stale ADR should be penalized");
    }

    #[tokio::test]
    async fn test_query_memory_traps_unaffected() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        {
            let mut decisions = server.state_manager.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "auth module".into(),
                status: AdrStatus::Superseded,
                context: "legacy".into(),
                decision: "old".into(),
                tags: vec!["security".into()],
            });
        }
        {
            let mut traps = server.state_manager.traps.write().await;
            traps.push(Trap {
                error_signature: "auth failure".into(),
                context: "legacy".into(),
                solution: "retry".into(),
                root_cause: String::new(),
                prevention: String::new(),
            });
        }

        let args = serde_json::json!({"query_intent": "auth"});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 1, "only trap should be returned");
        assert_eq!(results[0]["type"], "Trap", "traps must not be affected by ADR stale filter");
    }

    #[tokio::test]
    async fn test_query_memory_filters_non_accepted_adrs() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        {
            let mut decisions = server.state_manager.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "auth module".into(),
                status: AdrStatus::Superseded,
                context: "legacy".into(),
                decision: "old".into(),
                tags: vec!["security".into()],
            });
        }

        let args = serde_json::json!({"query_intent": "auth"});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 0, "superseded ADR should be filtered by default");
    }

    #[tokio::test]
    async fn test_query_memory_trap_boost() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        {
            let mut decisions = server.state_manager.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "database".into(),
                status: AdrStatus::Accepted,
                context: "sql".into(),
                decision: "postgres".into(),
                tags: vec!["db".into()],
            });
        }
        {
            let mut traps = server.state_manager.traps.write().await;
            traps.push(Trap {
                error_signature: "database".into(),
                context: "legacy".into(),
                solution: "retry".into(),
                root_cause: "network".into(),
                prevention: "timeout".into(),
            });
        }

        let args = serde_json::json!({"query_intent": "database"});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 2, "both ADR and Trap should be returned");
        assert_eq!(results[0]["type"], "Trap", "Trap should rank higher due to boosted weights");
        assert_eq!(results[1]["type"], "ADR", "ADR should rank lower");
    }

    #[tokio::test]
    async fn test_query_memory_trap_enriched() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());
        let server = McpServer::new(Arc::new(sm));

        {
            let mut traps = server.state_manager.traps.write().await;
            traps.push(Trap {
                error_signature: "auth failure".into(),
                context: "legacy".into(),
                solution: "retry".into(),
                root_cause: "token expired".into(),
                prevention: "refresh token".into(),
            });
        }

        let args = serde_json::json!({"query_intent": "auth"});
        let resp = server.tool_runtime_query_memory(args).await.unwrap();
        let results = extract_results(&resp);

        assert_eq!(results.len(), 1, "trap should be returned");
        assert_eq!(results[0]["type"], "Trap");
        assert_eq!(results[0]["root_cause"], "token expired", "Trap result should include root_cause");
        assert_eq!(results[0]["prevention"], "refresh token", "Trap result should include prevention");
    }

    #[tokio::test]
    async fn test_latest_adr_prefers_active() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());

        // Inject ADR-001 active, then ADR-002 superseded.
        // If latest_adr used decisions.last(), it would return ADR-002.
        {
            let mut decisions = sm.decisions.write().await;
            decisions.push(ADR {
                id: "ADR-001".into(),
                title: "Active Choice".into(),
                status: AdrStatus::Accepted,
                context: "ctx".into(),
                decision: "dec".into(),
                tags: vec!["a".into()],
            });
            decisions.push(ADR {
                id: "ADR-002".into(),
                title: "Superseded Choice".into(),
                status: AdrStatus::Superseded,
                context: "old".into(),
                decision: "old dec".into(),
                tags: vec![],
            });
        }

        let server = McpServer::new(Arc::new(sm));
        let args = serde_json::json!({});
        let resp = server.tool_runtime_bootstrap(args).await.unwrap();
        let text = resp["content"][0]["text"].as_str().unwrap_or("{}");
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let latest = parsed["latest_adr"].as_object().expect("latest_adr should exist");
        assert_eq!(latest["id"], "ADR-001", "latest_adr should be active, not last in Vec");
        assert_eq!(latest["status"], "Accepted");
    }

    #[test]
    fn test_parse_event_task_created() {
        let payload = serde_json::json!({
            "id": "TASK-011",
            "description": "New task"
        });
        let event = parse_event("TaskCreated", &payload).unwrap();
        assert!(matches!(event, RuntimeEvent::TaskCreated(_)));
        if let RuntimeEvent::TaskCreated(task) = event {
            assert_eq!(task.id, "TASK-011");
            assert_eq!(task.description, "New task");
        }
    }

    #[test]
    fn test_parse_event_task_created_with_status_ignored() {
        let payload = serde_json::json!({
            "id": "TASK-012",
            "description": "Task with status",
            "status": "InProgress"
        });
        let event = parse_event("TaskCreated", &payload).unwrap();
        if let RuntimeEvent::TaskCreated(task) = event {
            assert_eq!(task.id, "TASK-012");
            assert!(
                matches!(task.status, crate::models::TaskStatus::Todo),
                "status should be forced to Todo, got {:?}",
                task.status
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_filters_done_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let sm = StateManager::new(dir.path().to_path_buf());

        {
            let mut state = sm.state.write().await;
            state.active_tasks = vec![
                Task {
                    id: "TASK-001".into(),
                    description: "Todo task".into(),
                    status: TaskStatus::Todo,
                },
                Task {
                    id: "TASK-002".into(),
                    description: "InProgress task".into(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    id: "TASK-003".into(),
                    description: "Done task".into(),
                    status: TaskStatus::Done,
                },
                Task {
                    id: "TASK-004".into(),
                    description: "Blocked task".into(),
                    status: TaskStatus::Blocked,
                },
            ];
        }

        let server = McpServer::new(Arc::new(sm));
        let args = serde_json::json!({});
        let resp = server.tool_runtime_bootstrap(args).await.unwrap();
        let text = resp["content"][0]["text"].as_str().unwrap_or("{}");
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let tasks = parsed["active_tasks"].as_array().expect("active_tasks should be an array");
        assert_eq!(tasks.len(), 3, "Done task should be filtered out");

        for task in tasks {
            let status = task["status"].as_str().unwrap_or("");
            assert_ne!(status, "Done", "Done tasks must not appear in bootstrap output");
        }
    }
}
