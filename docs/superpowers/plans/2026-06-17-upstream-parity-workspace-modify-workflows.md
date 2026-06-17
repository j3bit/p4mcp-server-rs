# Workspace Modify Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Align `modify_workspaces` with upstream workspace semantics by moving `switch` to Rust CLI `P4CLIENT` context and splitting create/update/delete/switch into action-specific workflows.

**Architecture:** `modify_workspaces` should stop flattening all actions into one `P4Invocation`. Add a workspace modify command enum in `src/tools/workspaces.rs`, add active-client context to `P4McpServer`, and route each action through a dedicated execution helper. `switch` changes the server's active Perforce client context for later `p4` calls; it must not run `p4 client -s`.

**Tech Stack:** Rust 2024, `rmcp`, `serde_json`, direct `p4` CLI through `P4Executor`, `P4Env`, existing approval gate tests, upstream baseline `perforce/p4mcp-server` `v2026.2.2955897` at `a64efb07511b2a62db41aeed110ab96744c4076a`.

---

## Source Facts

- Upstream `modify_workspaces.switch` means `p4.client = workspace_name` in the P4Python connection. It does not edit a client spec.
- The Rust CLI equivalent is to set `P4CLIENT=<workspace_name>` in the runner environment for later `p4` commands.
- Upstream create/update require workspace specification input. In this Rust flat schema, that means at least one of `workspace_root`, `workspace_description`, or `workspace_view` must be supplied, matching the upstream model check. `workspace_options` and `workspace_line_end` defaults alone are not enough.
- Do not add a narrow `if action == ...` patch inside the current flat branch. Add action-specific command/workflow structure like the existing `StreamModifyCommand` pattern.

## File Structure

- Modify `src/tools/workspaces.rs`
  - Add `WorkspaceModifyCommand`.
  - Add `build_workspace_modify_command`.
  - Keep low-level invocation builders here.
- Modify `src/server.rs`
  - Add active P4 client state to `P4McpServer`.
  - Add `p4_env`, `set_active_client`, `run_p4_with_env`, and `call_p4_tool_with_env`.
  - Rewrite `modify_workspaces_inner` to dispatch by `WorkspaceModifyCommand`.
  - Add action-specific helpers for workspace create, update, delete, and switch.
- Inspect `src/tools/params.rs`
  - Keep the public schema unchanged.
  - Do not remove `switch`.
- Modify `tests/tool_mapping_tests.rs`
  - Add builder tests for action-specific command routing and validation.
- Modify `src/server.rs` test module
  - Add server behavior tests for active-client context and create/update validation.
- Modify `docs/superpowers/plans/2026-06-17-upstream-parity-workspace-modify-workflows.md`
  - Mark completed tasks and append verification notes during execution.

---

### Task 1: Add Workspace Modify Command Builder

**Files:**
- Modify: `src/tools/workspaces.rs`
- Test: `tests/tool_mapping_tests.rs`

- [x] **Step 1: Write failing command-builder tests**

Add these imports near the existing workspace imports in `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::workspaces::{
    WorkspaceModifyCommand, build_workspace_modify_command,
};
```

If `tests/tool_mapping_tests.rs` already imports from `tools::workspaces`, merge these names into the existing grouped import instead of adding a duplicate import.

Add these tests after `modify_workspaces_blank_name_delete_errors`:

```rust
#[test]
fn modify_workspaces_switch_builds_context_command() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Switch,
        workspace_name: "ws-main".to_string(),
        workspace_root: None,
        workspace_description: None,
        workspace_options: None,
        workspace_line_end: None,
        workspace_view: None,
        approval_token: None,
    };

    let command = build_workspace_modify_command(&params).unwrap();

    assert_eq!(
        command,
        WorkspaceModifyCommand::Switch {
            workspace_name: "ws-main".to_string(),
        }
    );
}

#[test]
fn modify_workspaces_create_requires_workspace_spec_fields() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Create,
        workspace_name: "ws-main".to_string(),
        workspace_root: None,
        workspace_description: None,
        workspace_options: None,
        workspace_line_end: None,
        workspace_view: None,
        approval_token: None,
    };

    let error = build_workspace_modify_command(&params)
        .unwrap_err()
        .to_string();

    assert!(error.contains("workspace specification fields are required for create"));
}

#[test]
fn modify_workspaces_update_requires_workspace_spec_fields() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Update,
        workspace_name: "ws-main".to_string(),
        workspace_root: None,
        workspace_description: None,
        workspace_options: None,
        workspace_line_end: None,
        workspace_view: None,
        approval_token: None,
    };

    let error = build_workspace_modify_command(&params)
        .unwrap_err()
        .to_string();

    assert!(error.contains("workspace specification fields are required for update"));
}

#[test]
fn modify_workspaces_create_builds_create_command_when_spec_fields_exist() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Create,
        workspace_name: "ws-main".to_string(),
        workspace_root: Some("/workspace/root".to_string()),
        workspace_description: None,
        workspace_options: None,
        workspace_line_end: None,
        workspace_view: None,
        approval_token: None,
    };

    let command = build_workspace_modify_command(&params).unwrap();

    assert_eq!(
        command,
        WorkspaceModifyCommand::Create {
            workspace_name: "ws-main".to_string(),
        }
    );
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test modify_workspaces_switch_builds_context_command --test tool_mapping_tests
rtk cargo test modify_workspaces_create_requires_workspace_spec_fields --test tool_mapping_tests
rtk cargo test modify_workspaces_update_requires_workspace_spec_fields --test tool_mapping_tests
rtk cargo test modify_workspaces_create_builds_create_command_when_spec_fields_exist --test tool_mapping_tests
```

Expected: FAIL because `WorkspaceModifyCommand` and `build_workspace_modify_command` do not exist.

- [x] **Step 3: Add the command builder**

In `src/tools/workspaces.rs`, replace the top import block with this block so `WorkspaceModifyAction` is available:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{ModifyWorkspacesParams, WorkspaceModifyAction},
};
```

Add this enum and builder above `build_workspace_delete_invocation`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceModifyCommand {
    Create { workspace_name: String },
    Update { workspace_name: String },
    Delete(P4Invocation),
    Switch { workspace_name: String },
}

impl WorkspaceModifyCommand {
    pub fn preview_invocation(&self) -> Option<&P4Invocation> {
        match self {
            Self::Delete(invocation) => Some(invocation),
            Self::Create { .. } | Self::Update { .. } | Self::Switch { .. } => None,
        }
    }
}

pub fn build_workspace_modify_command(
    params: &ModifyWorkspacesParams,
) -> Result<WorkspaceModifyCommand> {
    let workspace_name = required_workspace_name(&params.workspace_name, params.action.as_str())?;
    match params.action {
        WorkspaceModifyAction::Create => {
            require_workspace_spec_fields(params, params.action.as_str())?;
            Ok(WorkspaceModifyCommand::Create { workspace_name })
        }
        WorkspaceModifyAction::Update => {
            require_workspace_spec_fields(params, params.action.as_str())?;
            Ok(WorkspaceModifyCommand::Update { workspace_name })
        }
        WorkspaceModifyAction::Delete => {
            Ok(WorkspaceModifyCommand::Delete(build_workspace_delete_invocation(
                params,
            )?))
        }
        WorkspaceModifyAction::Switch => Ok(WorkspaceModifyCommand::Switch { workspace_name }),
    }
}

fn require_workspace_spec_fields(params: &ModifyWorkspacesParams, action: &str) -> Result<()> {
    let has_spec_fields = params.workspace_root.as_ref().is_some_and(|value| !value.trim().is_empty())
        || params
            .workspace_description
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        || params
            .workspace_view
            .as_ref()
            .is_some_and(|view| !view.is_empty());
    if !has_spec_fields {
        return Err(P4McpError::InvalidInput {
            message: format!("workspace specification fields are required for {action}"),
        });
    }
    Ok(())
}
```

- [x] **Step 4: Run tests to verify they pass**

Run:

```bash
rtk cargo test modify_workspaces_switch_builds_context_command --test tool_mapping_tests
rtk cargo test modify_workspaces_create_requires_workspace_spec_fields --test tool_mapping_tests
rtk cargo test modify_workspaces_update_requires_workspace_spec_fields --test tool_mapping_tests
rtk cargo test modify_workspaces_create_builds_create_command_when_spec_fields_exist --test tool_mapping_tests
```

Expected: PASS.

- [x] **Step 5: Commit**

Run:

```bash
rtk git add src/tools/workspaces.rs tests/tool_mapping_tests.rs
rtk git commit -m "refactor: split workspace modify commands"
```

---

### Task 2: Add Active P4 Client Context To The Runner Path

**Files:**
- Modify: `src/server.rs`

- [x] **Step 1: Add failing server context test**

Add this test in the `#[cfg(test)]` module of `src/server.rs`, after `modify_workspaces_update_preview_uses_workspace_name`:

```rust
#[tokio::test]
async fn active_client_context_applies_to_later_p4_calls() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: vec![json!({"client": "ws-main"})],
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(false), executor.clone());

    server.set_active_client("ws-main".to_string()).await;
    let response = server
        .call_p4_tool(
            "list",
            json_invocation(
                vec!["clients".to_string(), "-m".to_string(), "100".to_string()],
                None,
            ),
        )
        .await
        .expect("query should succeed with active client context");

    assert_eq!(response.0.status, "success");
    let envs = executor.envs();
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].get("P4CLIENT").map(String::as_str), Some("ws-main"));
}
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
rtk cargo test active_client_context_applies_to_later_p4_calls
```

Expected: FAIL because `set_active_client` does not exist and `call_p4_tool` always uses an empty `P4Env`.

- [x] **Step 3: Add active client state and env helpers**

In `src/server.rs`, change the import at the top from:

```rust
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
```

to:

```rust
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;
```

Change `P4McpServer` from:

```rust
pub struct P4McpServer {
    config: Arc<AppConfig>,
    executor: Arc<dyn P4Executor>,
    approval_gate: Arc<dyn WriteApprovalGate>,
}
```

to:

```rust
pub struct P4McpServer {
    config: Arc<AppConfig>,
    executor: Arc<dyn P4Executor>,
    approval_gate: Arc<dyn WriteApprovalGate>,
    active_client: Arc<RwLock<Option<String>>>,
}
```

Update `with_executor_and_approval` so the returned struct includes the new field:

```rust
        Self {
            config: Arc::new(config),
            executor,
            approval_gate,
            active_client: Arc::new(RwLock::new(None)),
        }
```

Replace `run_p4` and `call_p4_tool` with these methods:

```rust
    async fn p4_env(&self) -> P4Env {
        let mut env = P4Env::new();
        if let Some(client) = self.active_client.read().await.clone() {
            env.insert("P4CLIENT".to_string(), client);
        }
        env
    }

    async fn set_active_client(&self, workspace_name: String) {
        *self.active_client.write().await = Some(workspace_name);
    }

    async fn run_p4(&self, invocation: P4Invocation) -> McpResult<P4CommandOutput> {
        let env = self.p4_env().await;
        self.executor.run(invocation, env).await.map_err(to_mcp_error)
    }

    async fn run_p4_with_env(
        &self,
        invocation: P4Invocation,
        env: P4Env,
    ) -> McpResult<P4CommandOutput> {
        self.executor.run(invocation, env).await.map_err(to_mcp_error)
    }

    async fn call_p4_tool(
        &self,
        action: &str,
        invocation: P4Invocation,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }

    async fn call_p4_tool_with_env(
        &self,
        action: &str,
        invocation: P4Invocation,
        env: P4Env,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self.run_p4_with_env(invocation, env).await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }
```

Then update `call_p4_tool_with_benign_success` so it uses the active client env:

```rust
        let env = self.p4_env().await;
        match self.executor.run(invocation, env).await {
            Ok(output) => Ok(Json(ToolResponse::success(action, output_message(output)))),
            Err(error) if error.to_string().contains(benign_message) => {
                Ok(Json(ToolResponse::success(action, success_message)))
            }
            Err(error) => Err(to_mcp_error(error)),
        }
```

Update `call_p4_sequence_tool` so each invocation uses the active client env:

```rust
        let mut messages = Vec::new();
        for invocation in invocations {
            let env = self.p4_env().await;
            let output = self
                .executor
                .run(invocation, env)
                .await
                .map_err(to_mcp_error)?;
            messages.push(output_message(output));
        }
```

Do not change the existing explicit `P4CLIENT` env used by stream switch sync. That explicit env must continue to override the ambient active client for that one follow-up command.

- [x] **Step 4: Run test to verify it passes**

Run:

```bash
rtk cargo test active_client_context_applies_to_later_p4_calls
```

Expected: PASS.

- [x] **Step 5: Run focused regression tests for env behavior**

Run:

```bash
rtk cargo test modify_streams_switch_real_runs_client_switch_and_sync_k
rtk cargo test modify_files_sync_treats_up_to_date_as_success
```

Expected: PASS. These prove the new active client env did not break existing explicit-env and benign-error paths.

- [x] **Step 6: Commit**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "feat: track active p4 client context"
```

---

### Task 3: Rebuild Workspace Switch As Context Switch

**Files:**
- Modify: `src/server.rs`

- [x] **Step 1: Add failing switch tests**

Add these tests after `active_client_context_applies_to_later_p4_calls`:

```rust
#[tokio::test]
async fn modify_workspaces_switch_preview_does_not_emit_client_s() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"Client": "ws-main", "Owner": "alice"})],
        text: json!({}),
    }));
    let approval_gate = Arc::new(FakeApprovalGate::approval_required());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate.clone(),
    );
    let params = modify_workspaces_params(WorkspaceModifyAction::Switch);

    let response = server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
        .expect("approval response should be returned");

    assert_eq!(response.0.status, "approval_required");
    assert!(executor.invocations().is_empty());
    let calls = approval_gate.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].request.tool, "modify_workspaces");
    assert_eq!(calls[0].request.action, "switch");
    assert_eq!(calls[0].request.preview.targets, ["ws-main"]);
    assert_eq!(
        calls[0].request.preview.workspace.as_deref(),
        Some("ws-main")
    );
    assert_eq!(calls[0].request.preview.command, None);
    assert_eq!(calls[0].request.preview.commands, None);
}

#[tokio::test]
async fn modify_workspaces_switch_sets_active_client_after_approval() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"userName": "alice"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "Client: ws-main\nOwner: alice\nRoot: /workspace/root\n"
            }),
        },
        P4CommandOutput {
            records: vec![json!({"client": "ws-main"})],
            text: json!({}),
        },
    ]));
    let approval_gate = Arc::new(FakeApprovalGate::approved());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate,
    );
    let params = modify_workspaces_params(WorkspaceModifyAction::Switch);

    let response = server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
        .expect("switch should succeed");

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "switch");
    assert_eq!(
        response.0.message,
        json!("Switched to workspace 'ws-main'")
    );

    let query_response = server
        .call_p4_tool(
            "list",
            json_invocation(
                vec!["clients".to_string(), "-m".to_string(), "100".to_string()],
                None,
            ),
        )
        .await
        .expect("later p4 call should use switched active client");
    assert_eq!(query_response.0.status, "success");

    let invocations = executor.invocations();
    assert_eq!(
        invocations.iter().map(|invocation| invocation.args.clone()).collect::<Vec<_>>(),
        vec![
            vec!["info"],
            vec!["client", "-o", "ws-main"],
            vec!["clients", "-m", "100"],
        ]
    );
    let envs = executor.envs();
    assert_eq!(envs[0].get("P4CLIENT"), None);
    assert_eq!(envs[1].get("P4CLIENT"), None);
    assert_eq!(envs[2].get("P4CLIENT").map(String::as_str), Some("ws-main"));
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test modify_workspaces_switch_preview_does_not_emit_client_s
rtk cargo test modify_workspaces_switch_sets_active_client_after_approval
```

Expected: FAIL because current switch preview still emits `p4 client -s ws-main`, and execution still calls that invalid command instead of changing active context.

- [x] **Step 3: Import the new command builder**

In `src/server.rs`, update the workspace import from:

```rust
        workspaces::{
            build_workspace_delete_invocation, build_workspace_exists_invocation,
            build_workspace_query_invocation, required_workspace_name,
        },
```

to:

```rust
        workspaces::{
            WorkspaceModifyCommand, build_workspace_delete_invocation,
            build_workspace_exists_invocation, build_workspace_modify_command,
            build_workspace_query_invocation, required_workspace_name,
        },
```

- [x] **Step 4: Add workspace-specific approval preview**

Add this helper near `p4_approval_preview`:

```rust
    fn workspace_context_approval_preview(
        &self,
        action: &str,
        workspace_name: String,
    ) -> ApprovalPreview {
        ApprovalPreview {
            summary: approval_summary(action, std::slice::from_ref(&workspace_name)),
            tool: "modify_workspaces".to_string(),
            action: action.to_string(),
            targets: vec![workspace_name.clone()],
            changelist: None,
            workspace: Some(workspace_name),
            stream: None,
            review: None,
            command: None,
            commands: None,
            request: None,
        }
    }
```

Add this helper below `modify_workspaces_approval_request`:

```rust
    fn modify_workspaces_context_approval_request(
        &self,
        params: &ModifyWorkspacesParams,
        action: &str,
        workspace_name: String,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: "modify_workspaces".to_string(),
            action: action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("modify workspaces params serialize to JSON"),
            preview: self.workspace_context_approval_preview(action, workspace_name),
        }
    }
```

- [x] **Step 5: Add switch execution helper**

Add this method inside `impl P4McpServer`, near the workspace query helpers:

```rust
    async fn execute_workspace_switch(
        &self,
        workspace_name: String,
    ) -> McpResult<Json<ToolResponse>> {
        let _info = self
            .run_p4(json_invocation(vec!["info".to_string()], None))
            .await?;
        let output = self
            .run_p4(text_invocation(vec![
                "client".to_string(),
                "-o".to_string(),
                workspace_name.clone(),
            ]))
            .await?;
        let existing = output
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| to_mcp_error(invalid_input("p4 client -o did not return stdout")))?;
        if !form_has_non_empty_field(existing, "Update") && !form_has_non_empty_field(existing, "Access") {
            return Err(to_mcp_error(invalid_input(format!(
                "Workspace '{workspace_name}' does not exist"
            ))));
        }

        self.set_active_client(workspace_name.clone()).await;
        Ok(Json(ToolResponse::success(
            "switch",
            json!(format!("Switched to workspace '{workspace_name}'")),
        )))
    }
```

This helper intentionally does not compare `Owner` against the current user. Upstream only logs a warning when the owner differs and still switches.

- [x] **Step 6: Rewrite `modify_workspaces_inner` around command dispatch**

Replace the full body of `modify_workspaces_inner` with this implementation:

```rust
    async fn modify_workspaces_inner(
        &self,
        params: ModifyWorkspacesParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Workspaces, "modify_workspaces")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str().to_string();
        let command = build_workspace_modify_command(&params).map_err(to_mcp_error)?;
        let request = match &command {
            WorkspaceModifyCommand::Create { workspace_name }
            | WorkspaceModifyCommand::Update { workspace_name } => {
                self.modify_workspaces_context_approval_request(
                    &params,
                    &action,
                    workspace_name.clone(),
                )
            }
            WorkspaceModifyCommand::Delete(invocation) => {
                let workspace_name =
                    required_workspace_name(&params.workspace_name, params.action.as_str())
                        .map_err(to_mcp_error)?;
                self.modify_workspaces_approval_request(
                    &params,
                    P4ApprovalContext {
                        tool: "modify_workspaces",
                        action: &action,
                        targets: vec![workspace_name.clone()],
                        changelist: None,
                        workspace: Some(workspace_name),
                        stream: None,
                        invocation,
                    },
                )
            }
            WorkspaceModifyCommand::Switch { workspace_name } => {
                self.modify_workspaces_context_approval_request(
                    &params,
                    &action,
                    workspace_name.clone(),
                )
            }
        };
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }

        match command {
            WorkspaceModifyCommand::Create { workspace_name } => {
                self.execute_workspace_create(&params, workspace_name).await
            }
            WorkspaceModifyCommand::Update { workspace_name } => {
                self.execute_workspace_update(&params, workspace_name).await
            }
            WorkspaceModifyCommand::Delete(invocation) => {
                self.call_p4_tool(&action, invocation).await
            }
            WorkspaceModifyCommand::Switch { workspace_name } => {
                self.execute_workspace_switch(workspace_name).await
            }
        }
    }
```

The methods `execute_workspace_create` and `execute_workspace_update` are added in Task 4, so the project will not compile until Task 4 is complete. If executing task-by-task with commits, do not commit Task 3 until Task 4 is also implemented or temporarily keep the old create/update match arms. The recommended route is to complete Task 3 and Task 4 in one worker pass, then commit.

- [x] **Step 7: Run switch tests to verify they pass after Task 4**

Run after Task 4 implementation exists:

```bash
rtk cargo test modify_workspaces_switch_preview_does_not_emit_client_s
rtk cargo test modify_workspaces_switch_sets_active_client_after_approval
```

Expected: PASS.

---

### Task 4: Split Workspace Create And Update Execution

**Files:**
- Modify: `src/server.rs`

- [x] **Step 1: Add failing create/update validation tests**

Add these tests after the switch tests from Task 3:

```rust
#[tokio::test]
async fn modify_workspaces_create_without_spec_rejects_before_approval() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"client": "ws-main"})],
        text: json!({}),
    }));
    let approval_gate = Arc::new(FakeApprovalGate::approval_required());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate.clone(),
    );
    let params = modify_workspaces_params(WorkspaceModifyAction::Create);

    let err = match server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
    {
        Ok(_) => panic!("create without spec fields should be rejected"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(
        err.message
            .contains("workspace specification fields are required for create")
    );
    assert!(executor.invocations().is_empty());
    assert!(approval_gate.calls().is_empty());
}

#[tokio::test]
async fn modify_workspaces_update_without_spec_rejects_before_approval() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"client": "ws-main"})],
        text: json!({}),
    }));
    let approval_gate = Arc::new(FakeApprovalGate::approval_required());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate.clone(),
    );
    let params = modify_workspaces_params(WorkspaceModifyAction::Update);

    let err = match server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
    {
        Ok(_) => panic!("update without spec fields should be rejected"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(
        err.message
            .contains("workspace specification fields are required for update")
    );
    assert!(executor.invocations().is_empty());
    assert!(approval_gate.calls().is_empty());
}

#[tokio::test]
async fn modify_workspaces_create_fetches_and_saves_after_approval() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "Client: ws-main\nRoot: /old/root\nOptions: noallwrite noclobber nocompress unlocked nomodtime normdir\nLineEnd: local\nView:\n\t//depot/... //ws-main/...\n"
            }),
        },
        P4CommandOutput {
            records: vec![json!({"client": "ws-main"})],
            text: json!({}),
        },
    ]));
    let approval_gate = Arc::new(FakeApprovalGate::approved());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate,
    );
    let mut params = modify_workspaces_params(WorkspaceModifyAction::Create);
    params.workspace_root = Some("/workspace/root".to_string());
    params.workspace_view = Some(vec!["//depot/main/... //ws-main/main/...".to_string()]);

    let response = server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
        .expect("create should save patched client form");

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "create");
    let invocations = executor.invocations();
    assert_eq!(
        invocations.iter().map(|invocation| invocation.args.clone()).collect::<Vec<_>>(),
        vec![vec!["client", "-o", "ws-main"], vec!["client", "-i"]]
    );
    assert!(
        invocations[1]
            .stdin
            .as_deref()
            .expect("client -i should receive patched form")
            .contains("Root: /workspace/root")
    );
    assert!(
        invocations[1]
            .stdin
            .as_deref()
            .expect("client -i should receive patched form")
            .contains("//depot/main/... //ws-main/main/...")
    );
}

#[tokio::test]
async fn modify_workspaces_update_fetches_info_then_saves_after_approval() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"userName": "alice"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "Client: ws-main\nOwner: bob\nRoot: /old/root\nOptions: noallwrite noclobber nocompress unlocked nomodtime normdir\nLineEnd: local\nView:\n\t//depot/... //ws-main/...\n"
            }),
        },
        P4CommandOutput {
            records: vec![json!({"client": "ws-main"})],
            text: json!({}),
        },
    ]));
    let approval_gate = Arc::new(FakeApprovalGate::approved());
    let server = P4McpServer::with_executor_and_approval(
        test_config(false),
        executor.clone(),
        approval_gate,
    );
    let mut params = modify_workspaces_params(WorkspaceModifyAction::Update);
    params.workspace_description = Some("Updated workspace".to_string());

    let response = server
        .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
        .await
        .expect("update should save patched client form");

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "update");
    let invocations = executor.invocations();
    assert_eq!(
        invocations.iter().map(|invocation| invocation.args.clone()).collect::<Vec<_>>(),
        vec![vec!["info"], vec!["client", "-o", "ws-main"], vec!["client", "-i"]]
    );
    assert!(
        invocations[2]
            .stdin
            .as_deref()
            .expect("client -i should receive patched form")
            .contains("Description:\n\tUpdated workspace")
    );
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test modify_workspaces_create_without_spec_rejects_before_approval
rtk cargo test modify_workspaces_update_without_spec_rejects_before_approval
rtk cargo test modify_workspaces_create_fetches_and_saves_after_approval
rtk cargo test modify_workspaces_update_fetches_info_then_saves_after_approval
```

Expected: FAIL until the command builder is wired into the server and create/update helpers exist.

- [x] **Step 3: Add create/update execution helpers**

Add these helpers inside `impl P4McpServer`, near `execute_workspace_switch`:

```rust
    async fn execute_workspace_create(
        &self,
        params: &ModifyWorkspacesParams,
        workspace_name: String,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self
            .run_p4(text_invocation(vec![
                "client".to_string(),
                "-o".to_string(),
                workspace_name,
            ]))
            .await?;
        let existing = output
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| to_mcp_error(invalid_input("p4 client -o did not return stdout")))?;
        let patched = patch_workspace_form(existing, &workspace_form_patch(params))
            .map_err(to_mcp_error)?;
        self.call_p4_tool(
            "create",
            json_invocation(vec!["client".to_string(), "-i".to_string()], Some(patched)),
        )
        .await
    }

    async fn execute_workspace_update(
        &self,
        params: &ModifyWorkspacesParams,
        workspace_name: String,
    ) -> McpResult<Json<ToolResponse>> {
        let _info = self
            .run_p4(json_invocation(vec!["info".to_string()], None))
            .await?;
        let output = self
            .run_p4(text_invocation(vec![
                "client".to_string(),
                "-o".to_string(),
                workspace_name,
            ]))
            .await?;
        let existing = output
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| to_mcp_error(invalid_input("p4 client -o did not return stdout")))?;
        let patched = patch_workspace_form(existing, &workspace_form_patch(params))
            .map_err(to_mcp_error)?;
        self.call_p4_tool(
            "update",
            json_invocation(vec!["client".to_string(), "-i".to_string()], Some(patched)),
        )
        .await
    }
```

Add this free function near the existing helper functions at the bottom of `src/server.rs`:

```rust
fn workspace_form_patch(params: &ModifyWorkspacesParams) -> WorkspaceFormPatch {
    WorkspaceFormPatch {
        root: params.workspace_root.clone(),
        description: params.workspace_description.clone(),
        options: params.workspace_options.clone(),
        line_end: params.workspace_line_end.clone(),
        view: params.workspace_view.clone(),
    }
}
```

This preserves the upstream distinction: create and update are separate workflows, but both still use Perforce's fetch-and-save client spec behavior. Do not add duplicate/missing existence policy checks in this task.

- [x] **Step 4: Run create/update tests to verify they pass**

Run:

```bash
rtk cargo test modify_workspaces_create_without_spec_rejects_before_approval
rtk cargo test modify_workspaces_update_without_spec_rejects_before_approval
rtk cargo test modify_workspaces_create_fetches_and_saves_after_approval
rtk cargo test modify_workspaces_update_fetches_info_then_saves_after_approval
```

Expected: PASS.

- [x] **Step 5: Run switch tests from Task 3**

Run:

```bash
rtk cargo test modify_workspaces_switch_preview_does_not_emit_client_s
rtk cargo test modify_workspaces_switch_sets_active_client_after_approval
```

Expected: PASS.

- [x] **Step 6: Commit Task 3 and Task 4 together**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: restore workspace modify workflows"
```

---

### Task 5: Update Schema Contract Expectations

**Files:**
- Modify: `tests/schema_contract_tests.rs`

- [x] **Step 1: Update the schema contract test**

In `tests/schema_contract_tests.rs`, replace this block inside `modify_workspaces_schema_matches_upstream_fields`:

```rust
    let params: ModifyWorkspacesParams = serde_json::from_value(serde_json::json!({
        "action": "update",
        "workspace_name": "ws-main"
    }))
    .unwrap();

    assert_eq!(params.action, WorkspaceModifyAction::Update);
    assert_eq!(params.workspace_options, None);
    assert_eq!(params.workspace_line_end, None);
```

with:

```rust
    let params: ModifyWorkspacesParams = serde_json::from_value(serde_json::json!({
        "action": "update",
        "workspace_name": "ws-main",
        "workspace_root": "/workspace/root"
    }))
    .unwrap();

    assert_eq!(params.action, WorkspaceModifyAction::Update);
    assert_eq!(params.workspace_root.as_deref(), Some("/workspace/root"));
    assert_eq!(params.workspace_options, None);
    assert_eq!(params.workspace_line_end, None);
```

This keeps JSON schema permissive at deserialization time but avoids teaching future tests that an update without spec fields is a valid workflow.

- [x] **Step 2: Run schema contract test**

Run:

```bash
rtk cargo test modify_workspaces_schema_matches_upstream_fields --test schema_contract_tests
```

Expected: PASS.

- [x] **Step 3: Commit**

Run:

```bash
rtk git add tests/schema_contract_tests.rs
rtk git commit -m "test: align workspace modify schema contract"
```

---

### Task 6: Verification And Review Thread Replies

**Files:**
- Modify: `docs/superpowers/plans/2026-06-17-upstream-parity-workspace-modify-workflows.md`
- GitHub: PR #2 review threads

- [x] **Step 1: Run focused workspace tests**

Run:

```bash
rtk cargo test modify_workspaces_ --test tool_mapping_tests
rtk cargo test modify_workspaces_
rtk cargo test active_client_context_applies_to_later_p4_calls
rtk cargo test modify_workspaces_schema_matches_upstream_fields --test schema_contract_tests
```

Expected: PASS.

- [x] **Step 2: Run broader verification**

Run:

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets --all-features -- -D warnings
rtk cargo test
```

Expected: PASS. If full `rtk cargo test` fails because local mock servers cannot bind ports in the sandbox, rerun that exact command with escalated sandbox permissions and record that the sandbox failure was bind-related.

- [x] **Step 3: Append verification notes to this plan**

Add a section at the bottom of this file:

```markdown
## Execution Notes

- `rtk cargo test modify_workspaces_ --test tool_mapping_tests`: PASS
- `rtk cargo test modify_workspaces_`: PASS
- `rtk cargo test active_client_context_applies_to_later_p4_calls`: PASS
- `rtk cargo test modify_workspaces_schema_matches_upstream_fields --test schema_contract_tests`: PASS
- `rtk cargo fmt --check`: PASS
- `rtk cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `rtk cargo test`: PASS
```

- [x] **Step 4: Commit verification notes**

Run:

```bash
rtk git add docs/superpowers/plans/2026-06-17-upstream-parity-workspace-modify-workflows.md
rtk git commit -m "docs: record workspace workflow verification"
```

- [x] **Step 5: Push the branch**

Run:

```bash
rtk git push
```

Expected: branch push succeeds.

- [x] **Step 6: Reply to the `Build a valid workspace switch command` thread**

Use a thread reply, not a top-level PR comment. Reply body:

```text
Fixed in the latest push.

This was a true Rust-port workflow mismatch. Upstream `modify_workspaces.switch` changes the P4Python connection's active client with `p4.client = workspace_name`; it does not edit the client spec and does not run `p4 client -s`.

The Rust port now maps that behavior to runner context: after approval, it validates the target workspace and stores it as the active `P4CLIENT` for later `p4` calls. The approval preview no longer shows `p4 client -s <workspace>`, because no Perforce client-spec switch command is executed.
```

- [x] **Step 7: Reply to the `Separate workspace create from update` thread**

Use a thread reply, not a top-level PR comment. Reply body:

```text
Fixed in the latest push.

The root issue was the flattened `modify_workspaces` workflow. The Rust port now uses action-specific workspace commands and execution helpers for create, update, delete, and switch.

For create/update, the port now matches upstream validation: workspace spec fields are required before approval/execution. After approval, create and update still follow the upstream Perforce workflow by fetching the client form, patching the requested spec fields, and saving it with `p4 client -i`. I did not add a separate duplicate/missing existence policy here because upstream itself relies on Perforce's fetch/save client behavior for these actions.
```

- [x] **Step 8: Resolve both review threads**

Resolve only after replies are posted and the branch is pushed.

Expected: both review threads are `is_resolved: true`.

---

## Self-Review

- Spec coverage:
  - `modify_workspaces.switch` is not removed or unsupported. It becomes active `P4CLIENT` context in Tasks 2 and 3.
  - `switch` no longer builds `p4 client -s <workspace>`. Task 3 tests assert the preview has no `client -s` command and execution only runs `info` and `client -o`.
  - `modify_workspaces` is rebuilt around action-specific commands in Tasks 1, 3, and 4.
  - create/update workspace spec validation follows upstream in Tasks 1 and 4.
- Placeholder scan:
  - No step uses placeholder language or asks for tests without concrete code.
- Type consistency:
  - `WorkspaceModifyCommand`, `build_workspace_modify_command`, `set_active_client`, `p4_env`, `execute_workspace_switch`, `execute_workspace_create`, and `execute_workspace_update` are defined before or in the same task where they are used.

## Execution Notes

- `rtk cargo test modify_workspaces_ --test tool_mapping_tests`: PASS
- `rtk cargo test modify_workspaces_`: PASS
- `rtk cargo test active_client_context_applies_to_later_p4_calls`: PASS
- `rtk cargo test modify_workspaces_schema_matches_upstream_fields --test schema_contract_tests`: PASS
- `rtk cargo fmt --check`: PASS
- `rtk cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `rtk cargo test`: sandbox run failed because `wiremock` could not bind a local OS port (`Operation not permitted`); rerun outside the sandbox: PASS, 267 tests passed.
