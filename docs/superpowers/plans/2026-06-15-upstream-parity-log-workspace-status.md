# Upstream Parity Log And Workspace Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the two current unresolved PR review issues by honoring the configured log directory and restoring upstream `query_workspaces` `type` and `status` behavior.

**Architecture:** Keep the Rust port aligned with the upstream baseline documented in `README.md`: `perforce/p4mcp-server` `v2026.2.2955897` at commit `a64efb07511b2a62db41aeed110ab96744c4076a`. Logging remains initialized from the CLI config, but when `log_dir` is configured it also writes to `<log_dir>/p4mcp.log` while preserving stderr output. Workspace `type` and `status` are read-only server-level workflows because upstream computes them from workspace spec data and multiple P4 queries rather than from a single action-only command mapping.

**Tech Stack:** Rust 2024, rmcp, tracing/tracing-subscriber, tracing-appender, direct `p4` CLI invocations, Cargo tests.

---

## Context

Review threads being addressed:

- `PRRT_kwDOS5FH5M6JcFUJ`: `--log-dir` / `P4MCP_LOG_DIR` is parsed into `AppConfig.log_dir` but `run_from_cli` calls `init_logging()` without using it.
- `PRRT_kwDOS5FH5M6JcFUK`: upstream `query_workspaces` exposes `list`, `get`, `type`, and `status`, but the Rust port currently exposes only `list` and `get`.

Upstream facts to preserve:

- `p4mcp/main.py` resolves log directory as CLI `--log-dir`, then `P4MCP_LOG_DIR`, then default, and passes it to `setup_logging("INFO", log_dir=log_dir)`.
- `p4mcp/logging/global_logging.py` writes to `p4mcp.log` under the configured log directory and also adds stderr logging.
- `p4mcp/tools/workspace_tools.py` declares `query_workspaces` actions `list`, `get`, `type`, and `status`.
- `p4mcp/handlers/workspace_handlers.py` requires `workspace_name` for `get`, `type`, and `status`.
- `get_workspace_type` returns `stream` when a workspace spec has `Stream`, `standard` when its view contains `//depot/`, and `custom` otherwise.
- `get_workspace_status` validates the workspace, then reads current-client opened files, sync preview, pending resolves, and the latest `#have` changelist.

Scope boundaries:

- Do not reintroduce `query_workspaces.opened`, `query_workspaces.changes`, or `query_workspaces.where`; those remain deferred functional extensions outside this upstream-parity PR.
- Do not add log rotation. The review issue is that the configured directory is a no-op. A stable `<log_dir>/p4mcp.log` file matches the upstream active log filename and avoids time-dependent test assertions.
- Do not change the approved write approval gate behavior. These two issues are read-only/runtime plumbing parity fixes.

## File Structure

- Modify `Cargo.toml`
  - Add `tracing-appender = "0.2"` to runtime dependencies.

- Modify `Cargo.lock`
  - Let `cargo test` update the lockfile for `tracing-appender`.

- Modify `src/server.rs`
  - Thread `config.log_dir.as_deref()` into logging initialization.
  - Add a private `LoggingGuard` that keeps the file logging worker alive for the process lifetime.
  - Add `configured_log_file(log_dir)` and update `init_logging(log_dir)` to install stderr plus optional file layers.
  - Add `query_workspace_type` and `query_workspace_status` helper methods.
  - Update `query_workspaces` description and dispatch to cover `type` and `status`.
  - Add focused unit tests for configured file logging helper.

- Modify `src/tools/workspaces.rs`
  - Add `type` to the single-command workspace query builder by mapping it to `p4 client -o <workspace>`.
  - Keep `list` and `get` unchanged.
  - Keep `status` out of this builder because it is a multi-command workflow in the server.

- Modify `tests/tool_mapping_tests.rs`
  - Add direct builder coverage for `query_workspaces.type`.
  - Keep existing rejection tests for `where`, `opened`, and `changes`.

- Modify `tests/mcp_smoke_tests.rs`
  - Add server-to-executor coverage for `query_workspaces.type`.
  - Add server-to-executor coverage for `query_workspaces.status`, including exact command order and response shape.
  - Add a queued fake executor helper for multi-command server workflows.

---

### Task 1: Honor Configured Log Directory

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/server.rs`

- [ ] **Step 1: Write the failing file-log helper test**

In `src/server.rs`, inside the existing `#[cfg(test)] mod tests`, add `io::Write` to the `std` imports:

```rust
    use std::{
        io::Write,
        net::{IpAddr, Ipv4Addr},
        sync::{Arc, Mutex},
    };
```

Add this test near the top of the test module, immediately after the existing imports:

```rust
    #[test]
    fn configured_log_file_writes_to_p4mcp_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = configured_log_file(dir.path()).unwrap();

        writeln!(writer, "log-dir smoke").unwrap();
        drop(writer);

        let contents = std::fs::read_to_string(dir.path().join("p4mcp.log")).unwrap();
        assert!(contents.contains("log-dir smoke"));
    }
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
rtk cargo test configured_log_file_writes_to_p4mcp_log
```

Expected: FAIL to compile because `configured_log_file` does not exist.

- [ ] **Step 3: Add `tracing-appender`**

In `Cargo.toml`, add this dependency immediately after `tracing = "0.1"`:

```toml
tracing-appender = "0.2"
```

- [ ] **Step 4: Implement configured file logging**

At the top of `src/server.rs`, replace:

```rust
use std::sync::Arc;
```

with:

```rust
use std::{path::Path, sync::Arc};
```

Replace:

```rust
use serde_json::Value;
```

with:

```rust
use serde_json::{Value, json};
```

Replace:

```rust
use tracing_subscriber::EnvFilter;
```

with:

```rust
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
```

In `run_from_cli`, replace:

```rust
    let config = Cli::parse().into_config()?;
    init_logging();
```

with:

```rust
    let config = Cli::parse().into_config()?;
    let _logging_guard = init_logging(config.log_dir.as_deref())?;
```

Replace the existing `init_logging()` function with:

```rust
#[must_use]
struct LoggingGuard {
    _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

fn init_logging(log_dir: Option<&Path>) -> Result<LoggingGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    if let Some(log_dir) = log_dir {
        let file_appender = configured_log_file(log_dir)?;
        let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file_writer);

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init();

        Ok(LoggingGuard {
            _file_guard: Some(file_guard),
        })
    } else {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .try_init();

        Ok(LoggingGuard { _file_guard: None })
    }
}

fn configured_log_file(log_dir: &Path) -> Result<tracing_appender::rolling::RollingFileAppender> {
    std::fs::create_dir_all(log_dir)?;
    Ok(tracing_appender::rolling::never(log_dir, "p4mcp.log"))
}
```

- [ ] **Step 5: Run the focused test and verify it passes**

Run:

```bash
rtk cargo test configured_log_file_writes_to_p4mcp_log
```

Expected: PASS. `Cargo.lock` may change because `tracing-appender` was added.

- [ ] **Step 6: Run logging-related config tests**

Run:

```bash
rtk cargo test --test config_tests
```

Expected: PASS. Existing CLI/env config behavior remains unchanged.

- [ ] **Step 7: Commit Task 1**

Run:

```bash
rtk git status --short
rtk git add Cargo.toml Cargo.lock src/server.rs
rtk git commit -m "fix: honor configured log directory"
```

Expected: commit succeeds and includes only logging-related changes.

---

### Task 2: Restore Workspace Type Query

**Files:**
- Modify: `src/tools/workspaces.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add the failing direct builder test**

In `tests/tool_mapping_tests.rs`, add this test immediately before `workspace_get_blank_name_errors`:

```rust
#[test]
fn workspace_type_uses_client_spec() {
    let invocation = build_workspace_query_invocation("type", Some("ws-stream"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["client", "-o", "ws-stream"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

- [ ] **Step 2: Add the failing server smoke test**

In `tests/mcp_smoke_tests.rs`, add this test immediately after `query_workspaces_list_by_user_calls_injected_executor`:

```rust
#[tokio::test]
async fn query_workspaces_type_classifies_stream_workspace() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({
            "Client": "ws-stream",
            "Stream": "//streams/main"
        })],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(CommonQueryParams {
            action: "type".to_string(),
            changelist_id: None,
            workspace_name: Some("ws-stream".to_string()),
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
            max_results: 10,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "type");
    assert_eq!(response.0.message, json!({"workspace_type": "stream"}));
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(executor.invocations()[0].args, ["client", "-o", "ws-stream"]);
}
```

- [ ] **Step 3: Run focused tests and verify they fail**

Run:

```bash
rtk cargo test workspace_type_uses_client_spec --test tool_mapping_tests
rtk cargo test query_workspaces_type_classifies_stream_workspace --test mcp_smoke_tests
```

Expected:

- `workspace_type_uses_client_spec` fails because `type` is currently an unknown action.
- `query_workspaces_type_classifies_stream_workspace` fails because the server still routes through the unknown action path.

- [ ] **Step 4: Map `type` to workspace spec retrieval**

In `src/tools/workspaces.rs`, replace the current `"get"` arm:

```rust
        "get" => vec![
            "client".into(),
            "-o".into(),
            required(workspace_name, "workspace_name")?,
        ],
```

with:

```rust
        "get" | "type" => vec![
            "client".into(),
            "-o".into(),
            required(workspace_name, "workspace_name")?,
        ],
```

- [ ] **Step 5: Add workspace type response helpers**

In `src/server.rs`, inside `impl P4McpServer`, add this helper immediately after `call_p4_tool`:

```rust
    async fn query_workspace_type(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let invocation = build_workspace_query_invocation("type", workspace_name, None, 100)
            .map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(
            "type",
            json!({"workspace_type": workspace_type_from_records(&output.records)}),
        )))
    }
```

Add these private helpers near `output_message`:

```rust
fn workspace_type_from_records(records: &[Value]) -> &'static str {
    if records
        .iter()
        .any(|record| non_empty_string_field(record, "Stream").is_some())
    {
        return "stream";
    }

    if records.iter().any(record_has_depot_view) {
        return "standard";
    }

    "custom"
}

fn record_has_depot_view(record: &Value) -> bool {
    let Some(object) = record.as_object() else {
        return false;
    };

    object.iter().any(|(key, value)| {
        (key == "View" || key.starts_with("View"))
            && value.as_str().is_some_and(|view| view.contains("//depot/"))
    })
}

fn non_empty_string_field(record: &Value, field: &str) -> Option<String> {
    record
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
```

- [ ] **Step 6: Route `query_workspaces.type` through the helper**

In `src/server.rs`, replace the `query_workspaces` tool attribute:

```rust
    #[tool(
        description = "List or get workspaces",
        annotations(read_only_hint = true)
    )]
```

with:

```rust
    #[tool(
        description = "List, get, classify, or inspect workspace status",
        annotations(read_only_hint = true)
    )]
```

Inside `query_workspaces`, insert the `type` dispatch after the policy check and before the existing builder call:

```rust
        if params.action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
```

The resulting method body should start like this:

```rust
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        if params.action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
        let invocation = build_workspace_query_invocation(
            &params.action,
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }
```

- [ ] **Step 7: Run focused tests and verify they pass**

Run:

```bash
rtk cargo test workspace_type_uses_client_spec --test tool_mapping_tests
rtk cargo test query_workspaces_type_classifies_stream_workspace --test mcp_smoke_tests
```

Expected: both PASS.

- [ ] **Step 8: Commit Task 2**

Run:

```bash
rtk git status --short
rtk git add src/tools/workspaces.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: support workspace type query"
```

Expected: commit succeeds and contains only workspace `type` support.

---

### Task 3: Restore Workspace Status Query

**Files:**
- Modify: `src/server.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add a queued executor for multi-command smoke tests**

In `tests/mcp_smoke_tests.rs`, add these imports at the top:

```rust
use std::collections::VecDeque;
```

Then add this helper after the existing `FakeExecutor` implementation:

```rust
struct QueuedExecutor {
    outputs: Mutex<VecDeque<Result<P4CommandOutput, P4McpError>>>,
    invocations: Mutex<Vec<P4Invocation>>,
}

impl QueuedExecutor {
    fn success(outputs: Vec<P4CommandOutput>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn invocations(&self) -> Vec<P4Invocation> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl P4Executor for QueuedExecutor {
    async fn run(&self, invocation: P4Invocation, _env: P4Env) -> Result<P4CommandOutput, P4McpError> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(invocation);

        self.outputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                Err(P4McpError::P4Command {
                    message: "queued executor exhausted".to_string(),
                })
            })
    }
}
```

- [ ] **Step 2: Add the failing workspace status smoke test**

In `tests/mcp_smoke_tests.rs`, add this test immediately after `query_workspaces_type_classifies_stream_workspace`:

```rust
#[tokio::test]
async fn query_workspaces_status_runs_upstream_status_commands() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Client": "ws-main", "View0": "//depot/... //ws-main/..."})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/open.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/out-of-sync.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"fromFile": "//depot/main/base.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"change": "42"})],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(CommonQueryParams {
            action: "status".to_string(),
            changelist_id: None,
            workspace_name: Some("ws-main".to_string()),
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
            max_results: 10,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "status");
    assert_eq!(
        response.0.message,
        json!({
            "opened_files": ["//depot/main/open.rs"],
            "out_of_sync_files": ["//depot/main/out-of-sync.rs"],
            "sync_warnings": [],
            "pending_resolves": ["//depot/main/base.rs"],
            "last_synced_cl": "42"
        })
    );

    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 5);
    assert_eq!(invocations[0].args, ["client", "-o", "ws-main"]);
    assert_eq!(invocations[1].args, ["opened"]);
    assert_eq!(invocations[2].args, ["sync", "-n"]);
    assert_eq!(invocations[3].args, ["resolve", "-n"]);
    assert_eq!(invocations[4].args, ["changes", "-m1", "#have"]);
}
```

- [ ] **Step 3: Add the failing missing-workspace-name test**

In `tests/mcp_smoke_tests.rs`, add this test immediately after `query_workspaces_status_runs_upstream_status_commands`:

```rust
#[tokio::test]
async fn query_workspaces_status_requires_workspace_name() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_workspaces(Parameters(CommonQueryParams {
            action: "status".to_string(),
            changelist_id: None,
            workspace_name: None,
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
            max_results: 10,
        }))
        .await
    {
        Ok(_) => panic!("query_workspaces status should require workspace_name"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("workspace_name is required"));
    assert!(executor.invocations().is_empty());
}
```

- [ ] **Step 4: Run focused tests and verify they fail**

Run:

```bash
rtk cargo test query_workspaces_status --test mcp_smoke_tests
```

Expected: FAIL because `status` is still an unknown workspace query action.

- [ ] **Step 5: Add status command helpers**

In `src/server.rs`, inside `impl P4McpServer`, add these helpers immediately after `query_workspace_type`:

```rust
    async fn query_workspace_status(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_name = require_non_blank(workspace_name, "workspace_name")
            .map_err(to_mcp_error)?;

        let workspace_spec = self
            .run_workspace_status_command(
                json_invocation(vec!["client".into(), "-o".into(), workspace_name], None),
                None,
            )
            .await?;
        let opened = self
            .run_workspace_status_command(json_invocation(vec!["opened".into()], None), None)
            .await?;
        let out_of_sync = self
            .run_workspace_status_command(
                json_invocation(vec!["sync".into(), "-n".into()], None),
                Some("File(s) up-to-date"),
            )
            .await?;
        let pending_resolves = self
            .run_workspace_status_command(
                json_invocation(vec!["resolve".into(), "-n".into()], None),
                Some("No file(s) to resolve"),
            )
            .await?;
        let synced_changes = self
            .run_workspace_status_command(
                json_invocation(vec!["changes".into(), "-m1".into(), "#have".into()], None),
                None,
            )
            .await?;

        Ok(Json(ToolResponse::success(
            "status",
            workspace_status_message(
                &workspace_spec.records,
                &opened.records,
                &out_of_sync.records,
                &pending_resolves.records,
                &synced_changes.records,
            ),
        )))
    }

    async fn run_workspace_status_command(
        &self,
        invocation: P4Invocation,
        benign_empty_message: Option<&str>,
    ) -> McpResult<P4CommandOutput> {
        match self.executor.run(invocation, P4Env::new()).await {
            Ok(output) => Ok(output),
            Err(error) => {
                if benign_empty_message.is_some_and(|message| error.to_string().contains(message)) {
                    Ok(empty_p4_output())
                } else {
                    Err(to_mcp_error(error))
                }
            }
        }
    }
```

Add these private helpers near `workspace_type_from_records`:

```rust
fn workspace_status_message(
    _workspace_spec: &[Value],
    opened: &[Value],
    out_of_sync: &[Value],
    pending_resolves: &[Value],
    synced_changes: &[Value],
) -> Value {
    json!({
        "opened_files": collect_string_field(opened, "depotFile"),
        "out_of_sync_files": collect_string_field(out_of_sync, "depotFile"),
        "sync_warnings": collect_string_values(out_of_sync),
        "pending_resolves": collect_string_field(pending_resolves, "fromFile"),
        "last_synced_cl": synced_changes
            .first()
            .and_then(|record| non_empty_string_field(record, "change")),
    })
}

fn collect_string_field(records: &[Value], field: &str) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| non_empty_string_field(record, field))
        .collect()
}

fn collect_string_values(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn empty_p4_output() -> P4CommandOutput {
    P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }
}

fn require_non_blank(value: Option<&str>, name: &str) -> std::result::Result<String, P4McpError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(invalid_input(format!("{name} is required"))),
    }
}
```

The `_workspace_spec` parameter is intentionally retained in `workspace_status_message` because the upstream status flow validates the workspace before collecting current-client status. Do not remove the initial `client -o <workspace>` invocation.

- [ ] **Step 6: Route `query_workspaces.status` through the helper**

In `src/server.rs`, inside `query_workspaces`, insert this dispatch immediately after the `type` branch:

```rust
        if params.action == "status" {
            return self
                .query_workspace_status(params.workspace_name.as_deref())
                .await;
        }
```

The start of `query_workspaces` should now be:

```rust
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        if params.action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
        if params.action == "status" {
            return self
                .query_workspace_status(params.workspace_name.as_deref())
                .await;
        }
        let invocation = build_workspace_query_invocation(
            &params.action,
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }
```

- [ ] **Step 7: Run focused tests and verify they pass**

Run:

```bash
rtk cargo test query_workspaces_status --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 8: Run workspace mapping tests**

Run:

```bash
rtk cargo test workspace_ --test tool_mapping_tests
```

Expected: PASS. Existing rejection tests for `where`, `opened`, and `changes` remain passing.

- [ ] **Step 9: Commit Task 3**

Run:

```bash
rtk git status --short
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: support workspace status query"
```

Expected: commit succeeds and contains only workspace `status` support.

---

### Task 4: Verification And PR Review Follow-Up

**Files:**
- Inspect: all modified files
- Use: GitHub PR #2 review threads `PRRT_kwDOS5FH5M6JcFUJ` and `PRRT_kwDOS5FH5M6JcFUK`

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS. If it fails, run `rtk cargo fmt`, inspect the diff, and include the formatting in the relevant previous commit with `rtk git commit --amend --no-edit`.

- [ ] **Step 2: Lint**

Run:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 3: Run the full test suite**

Run:

```bash
rtk cargo test
```

Expected: PASS. If the sandbox blocks local port binding for WireMock, rerun the same command with approved elevated execution and record that the sandboxed failure was a bind-permission issue.

- [ ] **Step 4: Inspect final diff and commit graph**

Run:

```bash
rtk git status --short --branch
rtk git log --oneline --decorate -5
rtk git diff --stat origin/main...HEAD
```

Expected:

- Working tree is clean.
- Recent commits include:
  - `fix: honor configured log directory`
  - `fix: support workspace type query`
  - `fix: support workspace status query`
- Diff touches only logging parity, workspace `type/status` parity, tests, and dependency metadata.

- [ ] **Step 5: Push the branch**

Run:

```bash
rtk git push
```

Expected: push succeeds to the PR branch.

- [ ] **Step 6: Reply to the log-dir review thread**

Use a thread-level GraphQL reply once the push is complete:

```bash
rtk gh api graphql \
  -f query='mutation($thread:ID!, $body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread, body:$body}){comment{id}}}' \
  -f thread='PRRT_kwDOS5FH5M6JcFUJ' \
  -f body='Fixed in the latest push.

`--log-dir` and `P4MCP_LOG_DIR` now feed into logging initialization instead of stopping at `AppConfig.log_dir`. When configured, the Rust port creates `<log_dir>/p4mcp.log` and installs it alongside stderr logging, which matches the upstream baseline behavior where `setup_logging("INFO", log_dir=...)` writes to the configured log directory. I added a focused test proving the configured log writer writes to `p4mcp.log`.'
```

Expected: GitHub returns a new review comment id under the existing thread. Do not post a top-level PR comment.

- [ ] **Step 7: Reply to the workspace type/status review thread**

Use a thread-level GraphQL reply once the push is complete:

```bash
rtk gh api graphql \
  -f query='mutation($thread:ID!, $body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread, body:$body}){comment{id}}}' \
  -f thread='PRRT_kwDOS5FH5M6JcFUK' \
  -f body='Fixed in the latest push.

`query_workspaces` now supports the upstream baseline actions `type` and `status` while keeping the previously removed Rust-only `opened`, `changes`, and `where` actions out of this PR. `type` reads the workspace spec with `p4 client -o <workspace>` and classifies stream/standard/custom workspaces. `status` follows the upstream workflow by validating the workspace and then collecting opened files, sync preview, pending resolves, and the latest `#have` changelist. I added direct mapping coverage plus server-to-executor smoke tests for both actions.'
```

Expected: GitHub returns a new review comment id under the existing thread. Do not post a top-level PR comment.

- [ ] **Step 8: Resolve both review threads**

Resolve only after the pushed branch includes the fixes and local verification is passing:

```bash
rtk gh api graphql \
  -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' \
  -f thread='PRRT_kwDOS5FH5M6JcFUJ'

rtk gh api graphql \
  -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' \
  -f thread='PRRT_kwDOS5FH5M6JcFUK'
```

Expected: both GraphQL responses include `"isResolved": true`.

- [ ] **Step 9: Confirm GitHub thread state**

Run:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected:

- `PRRT_kwDOS5FH5M6JcFUJ` has `isResolved: true`.
- `PRRT_kwDOS5FH5M6JcFUK` has `isResolved: true`.
- Both thread replies are present.

---

## Self-Review

- Spec coverage: Task 1 fixes configured log directory usage. Task 2 fixes upstream workspace `type`. Task 3 fixes upstream workspace `status`. Task 4 covers verification, push, inline replies, and thread resolution.
- Placeholder scan: no placeholder implementation steps remain; every code change step includes exact code or exact replacement text.
- Type consistency: `build_workspace_query_invocation` keeps the existing signature `(action, workspace_name, user, max_results)`. `query_workspace_type` and `query_workspace_status` both return `McpResult<Json<ToolResponse>>`. `require_non_blank` returns `P4McpError` so it can be mapped through `to_mcp_error`.
