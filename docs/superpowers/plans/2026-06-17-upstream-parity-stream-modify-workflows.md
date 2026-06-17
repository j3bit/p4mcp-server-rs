# Upstream Parity Stream Modify Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore upstream-shaped stream modify execution so `create`, `update`, and `switch` preserve the original Python service workflow, naturally fixing the two open PR #2 review issues.

**Architecture:** Keep the Rust public `ModifyStreamsParams` schema and the documented write approval gate. Preserve simple `P4Invocation` builders for atomic one-command actions, but stop flattening upstream multi-step service actions into a single command too early. Route `create`, `update`, and `switch` through explicit server-side workflows that run after approval and perform the same read validations before any write command.

**Tech Stack:** Rust 2024, `rmcp`, direct `p4` CLI through `P4Executor`, `serde_json::Value`, existing fake/queued executor tests, `rtk cargo test`, `rtk cargo clippy`.

---

## Scope And Assumptions

- This is upstream-parity work for `perforce/p4mcp-server` `v2026.2.2955897` at `a64efb07511b2a62db41aeed110ab96744c4076a`.
- Keep the Rust safety extension from `README.md`: every `modify_*` call must pass the write approval gate before any `p4` command runs.
- Therefore, upstream validations that require `p4` reads happen after approval in this Rust port, but before the first mutating `p4` command.
- Do not add new stream actions or Rust-only ergonomics.
- The two open PR review issues are symptoms:
  - `switch` preview is not read-only.
  - `create` and `update` are collapsed into one workflow and miss existence semantics.

## File Structure

- Modify: `src/tools/streams.rs`
  - Responsibility: map public stream modify params into either atomic invocations or explicit workflow commands.
  - Change: split `CreateOrUpdate` into `Create`, `Update`, and `Switch` workflow variants.
- Modify: `src/server.rs`
  - Responsibility: approval gating and multi-step workflows that need executor access.
  - Change: add stream existence helpers and explicit `create`, `update`, and `switch` execution helpers.
- Modify: `tests/tool_mapping_tests.rs`
  - Responsibility: direct builder/command-shape tests.
  - Change: prove create/update/switch are no longer flattened into the wrong command.
- Modify: `src/server.rs` internal tests
  - Responsibility: approval, validation, and executor sequencing for write workflows.
  - Change: cover create duplicate rejection, update missing-stream rejection, switch preview read-only behavior, and approved switch execution.

---

### Task 1: Split Stream Modify Workflow Commands

**Files:**
- Modify: `src/tools/streams.rs`
- Test: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add failing builder tests**

Add these tests in `tests/tool_mapping_tests.rs` immediately after `modify_streams_edit_spec_uses_stream_spec_edit_command`:

```rust
#[test]
fn modify_streams_create_and_update_use_distinct_workflow_commands() {
    let mut create_params = modify_streams_params(StreamModifyAction::Create);
    create_params.stream_name = Some("//streams/new".to_string());
    create_params.stream_type = Some("mainline".to_string());

    let create_command = build_stream_modify_command(&create_params).unwrap();
    match create_command {
        StreamModifyCommand::Create { stream_name } => {
            assert_eq!(stream_name, "//streams/new");
        }
        other => panic!("create should use Create workflow, got {other:?}"),
    }

    let mut update_params = modify_streams_params(StreamModifyAction::Update);
    update_params.stream_name = Some("//streams/dev".to_string());

    let update_command = build_stream_modify_command(&update_params).unwrap();
    match update_command {
        StreamModifyCommand::Update { stream_name } => {
            assert_eq!(stream_name, "//streams/dev");
        }
        other => panic!("update should use Update workflow, got {other:?}"),
    }
}

#[test]
fn modify_streams_create_requires_stream_type_before_approval() {
    let mut params = modify_streams_params(StreamModifyAction::Create);
    params.stream_name = Some("//streams/new".to_string());

    let error = build_stream_modify_command(&params)
        .unwrap_err()
        .to_string();

    assert!(error.contains("stream_type is required for create"));
}

#[test]
fn modify_streams_switch_uses_workflow_command_with_preview_flag() {
    let mut params = modify_streams_params(StreamModifyAction::Switch);
    params.stream_name = Some("//streams/dev".to_string());
    params.workspace = Some("ws-main".to_string());
    params.preview = true;

    let command = build_stream_modify_command(&params).unwrap();

    match command {
        StreamModifyCommand::Switch {
            stream_name,
            workspace,
            preview,
        } => {
            assert_eq!(stream_name, "//streams/dev");
            assert_eq!(workspace.as_deref(), Some("ws-main"));
            assert!(preview);
        }
        other => panic!("switch should use Switch workflow, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run builder tests and confirm failure**

Run:

```bash
rtk cargo test modify_streams_create_and_update_use_distinct_workflow_commands --test tool_mapping_tests
rtk cargo test modify_streams_create_requires_stream_type_before_approval --test tool_mapping_tests
rtk cargo test modify_streams_switch_uses_workflow_command_with_preview_flag --test tool_mapping_tests
```

Expected:

```text
FAIL
```

The first and third tests should fail because `StreamModifyCommand::Create`, `Update`, and `Switch` do not exist yet. The second should fail because create currently only requires `stream_name`.

- [ ] **Step 3: Split the command enum and builder**

In `src/tools/streams.rs`, replace the current `StreamModifyCommand` enum and its `into_single_invocation` match with:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamModifyCommand {
    Single(P4Invocation),
    Create {
        stream_name: String,
    },
    Update {
        stream_name: String,
    },
    Switch {
        stream_name: String,
        workspace: Option<String>,
        preview: bool,
    },
    CreateWorkspace {
        stream_name: String,
        workspace_name: String,
        root: String,
    },
}

impl StreamModifyCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            Self::Create { .. }
            | Self::Update { .. }
            | Self::Switch { .. }
            | Self::CreateWorkspace { .. } => None,
        }
    }
}
```

In `build_stream_modify_command`, replace the `Create | Update` arm and the `Switch` arm with:

```rust
        StreamModifyAction::Create => {
            let stream_name =
                required_for_action(params.stream_name.as_deref(), "stream_name", &params.action)?;
            required_for_action(params.stream_type.as_deref(), "stream_type", &params.action)?;
            return Ok(StreamModifyCommand::Create { stream_name });
        }
        StreamModifyAction::Update => {
            return Ok(StreamModifyCommand::Update {
                stream_name: required_for_action(
                    params.stream_name.as_deref(),
                    "stream_name",
                    &params.action,
                )?,
            });
        }
```

And:

```rust
        StreamModifyAction::Switch => {
            return Ok(StreamModifyCommand::Switch {
                stream_name: required(params.stream_name.as_deref(), "stream_name")?,
                workspace: non_blank(params.workspace.as_deref()).map(str::to_string),
                preview: params.preview,
            });
        }
```

- [ ] **Step 4: Update server match arms to compile**

In `src/server.rs`, temporarily preserve old behavior for the new variants so the project compiles before deeper workflow changes. In the `preview_invocation` match inside `modify_streams_inner`, replace the `CreateOrUpdate` arm with:

```rust
            StreamModifyCommand::Create { stream_name } | StreamModifyCommand::Update { stream_name } => {
                json_invocation(
                    vec!["stream".to_string(), "-i".to_string()],
                    Some(format!(
                        "Stream: {stream_name}\n\n<patched after approval>\n"
                    )),
                )
            }
            StreamModifyCommand::Switch {
                stream_name,
                workspace,
                preview,
            } => {
                if *preview {
                    json_invocation(
                        vec!["stream".to_string(), "-o".to_string(), stream_name.clone()],
                        None,
                    )
                } else {
                    let mut args = vec![
                        "client".to_string(),
                        "-s".to_string(),
                        "-S".to_string(),
                        stream_name.clone(),
                    ];
                    if let Some(workspace) = workspace {
                        args.push(workspace.clone());
                    }
                    json_invocation(args, None)
                }
            }
```

In the execution `match command`, temporarily map the new `Create` and `Update` variants to the existing fetch-patch-save block by replacing:

```rust
            StreamModifyCommand::CreateOrUpdate { stream_name } => {
```

with:

```rust
            StreamModifyCommand::Create { stream_name } | StreamModifyCommand::Update { stream_name } => {
```

And add a temporary `Switch` arm before `CreateWorkspace`:

```rust
            StreamModifyCommand::Switch {
                stream_name,
                workspace,
                preview: _,
            } => {
                let mut args = vec![
                    "client".to_string(),
                    "-s".to_string(),
                    "-S".to_string(),
                    stream_name,
                ];
                if let Some(workspace) = workspace {
                    args.push(workspace);
                }
                self.call_p4_tool(&action, json_invocation(args, None)).await
            }
```

This step is an intermediate compile step only. Later tasks replace the temporary behavior with upstream workflows.

- [ ] **Step 5: Run builder tests and compile-focused stream tests**

Run:

```bash
rtk cargo test modify_streams_create_and_update_use_distinct_workflow_commands --test tool_mapping_tests
rtk cargo test modify_streams_create_requires_stream_type_before_approval --test tool_mapping_tests
rtk cargo test modify_streams_switch_uses_workflow_command_with_preview_flag --test tool_mapping_tests
rtk cargo test modify_streams_create_and_update_require_stream_name_before_approval
```

Expected:

```text
PASS
```

- [ ] **Step 6: Commit**

```bash
rtk git add src/tools/streams.rs src/server.rs tests/tool_mapping_tests.rs
rtk git commit -m "refactor: split stream modify workflows"
```

---

### Task 2: Implement Upstream Create Stream Workflow

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Add failing server tests for create semantics**

Add these tests in the `#[cfg(test)] mod tests` section of `src/server.rs` immediately after `modify_streams_create_and_update_require_stream_name_before_approval`:

```rust
    #[tokio::test]
    async fn modify_streams_create_rejects_existing_stream_after_approval() {
        let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        }]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Create);
        params.stream_name = Some("//streams/dev".to_string());
        params.stream_type = Some("mainline".to_string());

        let err = match server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("duplicate stream create should be rejected"),
            Err(err) => err,
        };

        assert!(err.message.contains("Stream '//streams/dev' already exists"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/dev"]);
        assert_eq!(invocations[0].mode, OutputMode::JsonLines);
    }

    #[tokio::test]
    async fn modify_streams_create_rejects_missing_parent_for_child_stream_after_approval() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Create);
        params.stream_name = Some("//streams/dev".to_string());
        params.stream_type = Some("development".to_string());

        let err = match server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("child stream create without parent should be rejected"),
            Err(err) => err,
        };

        assert!(err.message.contains("Parent stream is required"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/dev"]);
        assert_eq!(
            invocations[1].args,
            ["streams", "-a", "-F", "Stream=//streams/dev"]
        );
    }

    #[tokio::test]
    async fn modify_streams_create_saves_new_mainline_stream_after_approval() {
        let template_form = "\
Stream: //streams/dev
Type: development
Parent: //streams/main
Name: old

Description:
\told description

Paths:
\tshare ...
";
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({"stdout": template_form, "stderr": ""}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/main"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Create);
        params.stream_name = Some("//streams/main".to_string());
        params.stream_type = Some("mainline".to_string());
        params.name = Some("Main".to_string());
        params.description = Some("mainline stream".to_string());

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved stream create should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "create");
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 4);
        assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/main"]);
        assert_eq!(
            invocations[1].args,
            ["streams", "-a", "-F", "Stream=//streams/main"]
        );
        assert_eq!(invocations[2].args, ["stream", "-o", "//streams/main"]);
        assert_eq!(invocations[2].mode, OutputMode::Text);
        assert_eq!(invocations[3].args, ["stream", "-i"]);
        let saved_form = invocations[3]
            .stdin
            .as_deref()
            .expect("stream -i should receive patched form");
        assert!(saved_form.contains("Stream: //streams/main"));
        assert!(saved_form.contains("Type: mainline"));
        assert!(saved_form.contains("Parent: none"));
        assert!(saved_form.contains("Name: Main"));
        assert!(saved_form.contains("Description:\n\tmainline stream"));
    }
```

- [ ] **Step 2: Run create tests and confirm failure**

Run:

```bash
rtk cargo test modify_streams_create_rejects_existing_stream_after_approval
rtk cargo test modify_streams_create_rejects_missing_parent_for_child_stream_after_approval
rtk cargo test modify_streams_create_saves_new_mainline_stream_after_approval
```

Expected:

```text
FAIL
```

Current behavior reaches `p4 stream -o` for create and does not check duplicate streams or parent requirements.

- [ ] **Step 3: Add stream existence and create helpers**

In `src/server.rs`, add this enum near `struct P4ApprovalContext<'a>`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamExistence {
    Active,
    Deleted,
    Missing,
}
```

Inside `impl P4McpServer`, add these helpers after `require_existing_workspace`:

```rust
    async fn stream_existence(&self, stream_name: &str) -> McpResult<StreamExistence> {
        let active = self
            .run_p4(json_invocation(
                vec![
                    "streams".to_string(),
                    "-F".to_string(),
                    format!("Stream={stream_name}"),
                ],
                None,
            ))
            .await?;
        if !active.records.is_empty() {
            return Ok(StreamExistence::Active);
        }

        let deleted_or_inactive = self
            .run_p4(json_invocation(
                vec![
                    "streams".to_string(),
                    "-a".to_string(),
                    "-F".to_string(),
                    format!("Stream={stream_name}"),
                ],
                None,
            ))
            .await?;
        if deleted_or_inactive.records.is_empty() {
            Ok(StreamExistence::Missing)
        } else {
            Ok(StreamExistence::Deleted)
        }
    }

    async fn require_active_stream(&self, stream_name: &str, label: &str) -> McpResult<()> {
        match self.stream_existence(stream_name).await? {
            StreamExistence::Active => Ok(()),
            StreamExistence::Deleted => Err(to_mcp_error(invalid_input(format!(
                "{label} stream '{stream_name}' has been deleted"
            )))),
            StreamExistence::Missing => Err(to_mcp_error(invalid_input(format!(
                "{label} stream '{stream_name}' does not exist"
            )))),
        }
    }

    async fn fetch_stream_form(&self, stream_name: &str) -> McpResult<String> {
        let output = self
            .run_p4(text_invocation(vec![
                "stream".to_string(),
                "-o".to_string(),
                stream_name.to_string(),
            ]))
            .await?;
        output
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| to_mcp_error(invalid_input("p4 stream -o did not return stdout")))
    }

    async fn save_stream_form(
        &self,
        action: &str,
        patched: String,
    ) -> McpResult<Json<ToolResponse>> {
        self.call_p4_tool(
            action,
            json_invocation(vec!["stream".to_string(), "-i".to_string()], Some(patched)),
        )
        .await
    }
```

Add these free functions near `form_has_non_empty_field`:

```rust
fn is_valid_stream_type(stream_type: &str) -> bool {
    matches!(
        stream_type,
        "mainline" | "development" | "sparsedev" | "release" | "sparserel" | "task" | "virtual"
    )
}

fn is_mainline_stream_type(stream_type: &str) -> bool {
    stream_type == "mainline"
}
```

- [ ] **Step 4: Implement create workflow after approval**

Inside `impl P4McpServer`, add this helper after `fetch_stream_form`:

```rust
    async fn execute_stream_create(
        &self,
        params: &ModifyStreamsParams,
        stream_name: String,
    ) -> McpResult<Json<ToolResponse>> {
        let stream_type = required_option(params.stream_type.as_deref(), "stream_type", "create")?;
        if !is_valid_stream_type(&stream_type) {
            return Err(to_mcp_error(invalid_input(format!(
                "Invalid stream type '{stream_type}'. Must be one of: development, mainline, release, sparsedev, sparserel, task, virtual"
            ))));
        }

        match self.stream_existence(&stream_name).await? {
            StreamExistence::Active => {
                return Err(to_mcp_error(invalid_input(format!(
                    "Stream '{stream_name}' already exists"
                ))));
            }
            StreamExistence::Deleted => {
                return Err(to_mcp_error(invalid_input(format!(
                    "Stream '{stream_name}' has been deleted"
                ))));
            }
            StreamExistence::Missing => {}
        }

        let parent = if is_mainline_stream_type(&stream_type) {
            Some("none".to_string())
        } else {
            let parent = required_option(params.parent.as_deref(), "parent", "create")?;
            self.require_active_stream(&parent, "Parent").await?;
            Some(parent)
        };

        let current_form = self.fetch_stream_form(&stream_name).await?;
        let patch = StreamFormPatch {
            stream: Some(stream_name),
            stream_type: Some(stream_type),
            parent,
            name: params.name.clone(),
            description: params.description.clone(),
            options: params.options.clone(),
            parent_view: params.parent_view.clone(),
            paths: params.paths.clone(),
            remapped: params.remapped.clone(),
            ignored: params.ignored.clone(),
        };
        let patched = patch_stream_form(&current_form, &patch).map_err(to_mcp_error)?;
        self.save_stream_form("create", patched).await
    }
```

In the `match command` inside `modify_streams_inner`, replace the temporary `Create | Update` arm with a separate create arm:

```rust
            StreamModifyCommand::Create { stream_name } => {
                self.execute_stream_create(&params, stream_name).await
            }
```

Leave `Update` mapped to the old fetch-patch-save block until Task 3.

- [ ] **Step 5: Run create workflow tests**

Run:

```bash
rtk cargo test modify_streams_create_rejects_existing_stream_after_approval
rtk cargo test modify_streams_create_rejects_missing_parent_for_child_stream_after_approval
rtk cargo test modify_streams_create_saves_new_mainline_stream_after_approval
```

Expected:

```text
PASS
```

- [ ] **Step 6: Commit**

```bash
rtk git add src/server.rs
rtk git commit -m "fix: restore stream create workflow"
```

---

### Task 3: Implement Upstream Update Stream Workflow

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Add failing server tests for update semantics**

Add these tests in `src/server.rs` immediately after `modify_streams_update_fetches_and_saves_stream_form_after_approval`:

```rust
    #[tokio::test]
    async fn modify_streams_update_rejects_missing_stream_after_approval() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Update);
        params.stream_name = Some("//streams/missing".to_string());
        params.description = Some("new description".to_string());

        let err = match server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("missing stream update should be rejected"),
            Err(err) => err,
        };

        assert!(err.message.contains("Stream '//streams/missing' does not exist"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/missing"]);
        assert_eq!(
            invocations[1].args,
            ["streams", "-a", "-F", "Stream=//streams/missing"]
        );
    }

    #[tokio::test]
    async fn modify_streams_update_rejects_view_change_when_bound_workspace_has_open_files() {
        let existing_form = "\
Stream: //streams/dev
Options: allsubmit unlocked toparent fromparent

Paths:
\tshare ...
";
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({"stdout": existing_form, "stderr": ""}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"client": "ws-dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"depotFile": "//streams/dev/file.txt"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Update);
        params.stream_name = Some("//streams/dev".to_string());
        params.paths = Some(vec!["share ...".to_string(), "isolate generated/...".to_string()]);

        let err = match server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("view-affecting update with open files should be rejected"),
            Err(err) => err,
        };

        assert!(err.message.contains("Cannot modify stream view"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 5);
        assert_eq!(invocations[3].args, ["clients", "-S", "//streams/dev"]);
        assert_eq!(invocations[4].args, ["opened", "-C", "ws-dev"]);
    }

    #[tokio::test]
    async fn modify_streams_update_changes_parent_view_with_parentview_command() {
        let existing_form = "\
Stream: //streams/dev
Options: allsubmit unlocked toparent fromparent
ParentView: inherit

Paths:
\tshare ...
";
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({"stdout": existing_form, "stderr": ""}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Update);
        params.stream_name = Some("//streams/dev".to_string());
        params.description = Some("new description".to_string());
        params.parent_view = Some("noinherit".to_string());

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved parent_view update should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "update");
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 6);
        assert_eq!(invocations[3].args, ["clients", "-S", "//streams/dev"]);
        assert_eq!(invocations[4].args, ["stream", "-i"]);
        let saved_form = invocations[4]
            .stdin
            .as_deref()
            .expect("stream -i should receive patched form");
        assert!(!saved_form.contains("ParentView: noinherit"));
        assert_eq!(
            invocations[5].args,
            ["stream", "parentview", "--noinherit", "//streams/dev"]
        );
    }
```

- [ ] **Step 2: Run update tests and confirm failure**

Run:

```bash
rtk cargo test modify_streams_update_rejects_missing_stream_after_approval
rtk cargo test modify_streams_update_rejects_view_change_when_bound_workspace_has_open_files
rtk cargo test modify_streams_update_changes_parent_view_with_parentview_command
```

Expected:

```text
FAIL
```

Current update behavior fetches and saves the stream form without checking existence first, bound workspace open files, or parent view command semantics.

- [ ] **Step 3: Add update helper functions**

Inside `impl P4McpServer`, add these helpers after `execute_stream_create`:

```rust
    async fn stream_bound_workspaces(&self, stream_name: &str) -> McpResult<Vec<String>> {
        let output = self
            .run_p4(json_invocation(
                vec![
                    "clients".to_string(),
                    "-S".to_string(),
                    stream_name.to_string(),
                ],
                None,
            ))
            .await?;
        Ok(output
            .records
            .iter()
            .filter_map(|record| record_string_field(record, "client").or_else(|| record_string_field(record, "Client")))
            .map(str::to_string)
            .collect())
    }

    async fn opened_files_for_workspace(&self, workspace: &str) -> McpResult<Vec<Value>> {
        let output = self
            .run_p4(json_invocation(
                vec![
                    "opened".to_string(),
                    "-C".to_string(),
                    workspace.to_string(),
                ],
                None,
            ))
            .await?;
        Ok(output.records)
    }

    async fn reject_view_update_when_workspaces_have_open_files(
        &self,
        stream_name: &str,
    ) -> McpResult<()> {
        let workspaces = self.stream_bound_workspaces(stream_name).await?;
        for workspace in workspaces {
            let opened = self.opened_files_for_workspace(&workspace).await?;
            if !opened.is_empty() {
                return Err(to_mcp_error(invalid_input(format!(
                    "Cannot modify stream view: workspace '{workspace}' has {} open file(s). Submit or revert changes first.",
                    opened.len()
                ))));
            }
        }
        Ok(())
    }
```

Add these free functions near `is_valid_stream_type`:

```rust
fn record_string_field<'a>(record: &'a Value, field: &str) -> Option<&'a str> {
    record
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn stream_update_is_view_affecting(params: &ModifyStreamsParams) -> bool {
    params.paths.is_some()
        || params.remapped.is_some()
        || params.ignored.is_some()
        || params.parent_view.is_some()
}

fn stream_options_are_locked(form: &str) -> bool {
    form.lines()
        .find_map(|line| line.strip_prefix("Options:"))
        .is_some_and(|options| {
            let lower = options.to_ascii_lowercase();
            lower.contains("locked") && !lower.contains("unlocked")
        })
}
```

- [ ] **Step 4: Implement update workflow**

Inside `impl P4McpServer`, add this helper after the Task 3 helpers:

```rust
    async fn execute_stream_update(
        &self,
        params: &ModifyStreamsParams,
        stream_name: String,
    ) -> McpResult<Json<ToolResponse>> {
        self.require_active_stream(&stream_name, "Stream").await?;

        let current_form = self.fetch_stream_form(&stream_name).await?;
        if stream_options_are_locked(&current_form) {
            return Err(to_mcp_error(invalid_input(format!(
                "Stream '{stream_name}' is locked. Force update is not supported."
            ))));
        }

        let resolve_preview = self
            .executor
            .run(
                json_invocation(
                    vec!["stream".to_string(), "resolve".to_string(), "-n".to_string()],
                    None,
                ),
                P4Env::new(),
            )
            .await;
        if let Ok(output) = resolve_preview
            && !output.records.is_empty()
        {
            return Err(to_mcp_error(invalid_input(format!(
                "Stream '{stream_name}' has pending spec conflicts that must be resolved before editing"
            ))));
        }

        if stream_update_is_view_affecting(params) {
            self.reject_view_update_when_workspaces_have_open_files(&stream_name)
                .await?;
        }

        let patch = StreamFormPatch {
            stream: Some(stream_name.clone()),
            stream_type: params.stream_type.clone(),
            parent: params.parent.clone(),
            name: params.name.clone(),
            description: params.description.clone(),
            options: params.options.clone(),
            parent_view: None,
            paths: params.paths.clone(),
            remapped: params.remapped.clone(),
            ignored: params.ignored.clone(),
        };
        let patched = patch_stream_form(&current_form, &patch).map_err(to_mcp_error)?;
        let save_response = self.save_stream_form("update", patched).await?;

        if let Some(parent_view) = params.parent_view.as_deref().filter(|value| !value.trim().is_empty()) {
            self.run_p4(json_invocation(
                vec![
                    "stream".to_string(),
                    "parentview".to_string(),
                    format!("--{parent_view}"),
                    stream_name,
                ],
                None,
            ))
            .await?;
        }

        Ok(save_response)
    }
```

In `modify_streams_inner`, replace the temporary `Update` block with:

```rust
            StreamModifyCommand::Update { stream_name } => {
                self.execute_stream_update(&params, stream_name).await
            }
```

- [ ] **Step 5: Update the existing update test for new read-validation sequence**

In `modify_streams_update_fetches_and_saves_stream_form_after_approval`, change the queued executor outputs from two outputs to five outputs:

```rust
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({"stdout": existing_form, "stderr": ""}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
        ]));
```

And change the invocation assertions to:

```rust
        assert_eq!(invocations.len(), 5);
        assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/dev"]);
        assert_eq!(invocations[1].args, ["stream", "-o", "//streams/dev"]);
        assert_eq!(invocations[1].mode, OutputMode::Text);
        assert_eq!(invocations[2].args, ["stream", "resolve", "-n"]);
        assert_eq!(invocations[3].args, ["clients", "-S", "//streams/dev"]);
        assert_eq!(invocations[4].args, ["stream", "-i"]);
        let saved_form = invocations[4]
            .stdin
            .as_deref()
            .expect("stream -i should receive patched form");
```

- [ ] **Step 6: Run update tests**

Run:

```bash
rtk cargo test modify_streams_update_fetches_and_saves_stream_form_after_approval
rtk cargo test modify_streams_update_rejects_missing_stream_after_approval
rtk cargo test modify_streams_update_rejects_view_change_when_bound_workspace_has_open_files
rtk cargo test modify_streams_update_changes_parent_view_with_parentview_command
```

Expected:

```text
PASS
```

- [ ] **Step 7: Commit**

```bash
rtk git add src/server.rs
rtk git commit -m "fix: restore stream update workflow"
```

---

### Task 4: Implement Upstream Switch Stream Workflow And Preview

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Add failing tests for switch preview and execution**

Add these tests in `src/server.rs` immediately before `modify_streams_create_workspace_previews_client_i_without_fetching_form`:

```rust
    #[tokio::test]
    async fn modify_streams_switch_preview_is_read_only_after_approval() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({
                    "Client": "ws-main",
                    "Update": "2026/06/17 10:00:00",
                    "Stream": "//streams/main"
                })],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({
                    "Stream": "//streams/dev",
                    "Type": "development",
                    "Parent": "//streams/main"
                })],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = modify_streams_params(StreamModifyAction::Switch);
        params.stream_name = Some("//streams/dev".to_string());
        params.workspace = Some("ws-main".to_string());
        params.preview = true;

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved switch preview should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "switch");
        assert_eq!(response.0.message["preview"], json!(true));
        assert_eq!(response.0.message["current_stream"], json!("//streams/main"));
        assert_eq!(response.0.message["target_stream"], json!("//streams/dev"));
        assert_eq!(response.0.message["workspace"], json!("ws-main"));

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 4);
        assert_eq!(invocations[0].args, ["client", "-o", "ws-main"]);
        assert_eq!(invocations[1].args, ["streams", "-F", "Stream=//streams/dev"]);
        assert_eq!(invocations[2].args, ["opened", "-C", "ws-main"]);
        assert_eq!(invocations[3].args, ["stream", "-o", "//streams/dev"]);
        assert!(
            invocations
                .iter()
                .all(|invocation| invocation.args != ["client", "-s", "-S", "//streams/dev", "ws-main"])
        );

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "stream".to_string(),
                "-o".to_string(),
                "//streams/dev".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_streams_switch_rejects_open_files_before_switching() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({
                    "Client": "ws-main",
                    "Update": "2026/06/17 10:00:00",
                    "Stream": "//streams/main"
                })],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"depotFile": "//streams/main/file.txt"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Switch);
        params.stream_name = Some("//streams/dev".to_string());
        params.workspace = Some("ws-main".to_string());

        let err = match server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("switch with open files should be rejected"),
            Err(err) => err,
        };

        assert!(err.message.contains("Cannot switch stream"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 3);
        assert_eq!(invocations[2].args, ["opened", "-C", "ws-main"]);
    }

    #[tokio::test]
    async fn modify_streams_switch_executes_client_switch_and_have_table_sync_after_approval() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({
                    "Client": "ws-main",
                    "Update": "2026/06/17 10:00:00",
                    "Stream": "//streams/main"
                })],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Stream": "//streams/dev"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"Client": "ws-main"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"synced": true})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_streams_params(StreamModifyAction::Switch);
        params.stream_name = Some("//streams/dev".to_string());
        params.workspace = Some("ws-main".to_string());

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved switch should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "switch");
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 5);
        assert_eq!(
            invocations[3].args,
            ["client", "-s", "-S", "//streams/dev", "ws-main"]
        );
        assert_eq!(invocations[4].args, ["sync", "-k", "//streams/dev/..."]);
    }
```

- [ ] **Step 2: Run switch tests and confirm failure**

Run:

```bash
rtk cargo test modify_streams_switch_preview_is_read_only_after_approval
rtk cargo test modify_streams_switch_rejects_open_files_before_switching
rtk cargo test modify_streams_switch_executes_client_switch_and_have_table_sync_after_approval
```

Expected:

```text
FAIL
```

The preview test should currently fail because switch executes `p4 client -s -S`.

- [ ] **Step 3: Add switch helper functions**

Inside `impl P4McpServer`, add these helpers after `execute_stream_update`:

```rust
    async fn current_workspace_record(&self, workspace: Option<&str>) -> McpResult<Value> {
        let mut args = vec!["client".to_string(), "-o".to_string()];
        if let Some(workspace) = workspace.filter(|value| !value.trim().is_empty()) {
            args.push(workspace.to_string());
        }
        let output = self.run_p4(json_invocation(args, None)).await?;
        output
            .records
            .into_iter()
            .next()
            .ok_or_else(|| to_mcp_error(invalid_input("Workspace not found")))
    }

    async fn execute_stream_switch(
        &self,
        stream_name: String,
        workspace: Option<String>,
        preview: bool,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_record = self.current_workspace_record(workspace.as_deref()).await?;
        let workspace_name = workspace
            .clone()
            .or_else(|| {
                record_string_field(&workspace_record, "Client")
                    .or_else(|| record_string_field(&workspace_record, "client"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "current".to_string());

        let exists = record_string_field(&workspace_record, "Update").is_some()
            || record_string_field(&workspace_record, "Access").is_some();
        if !exists {
            return Err(to_mcp_error(invalid_input(format!(
                "Workspace '{workspace_name}' does not exist"
            ))));
        }

        let current_stream = record_string_field(&workspace_record, "Stream")
            .ok_or_else(|| {
                to_mcp_error(invalid_input(format!(
                    "Workspace '{workspace_name}' is not stream-based. Cannot switch streams on a classic workspace."
                )))
            })?
            .to_string();

        match self.stream_existence(&stream_name).await? {
            StreamExistence::Active => {}
            StreamExistence::Deleted => {
                return Err(to_mcp_error(invalid_input(format!(
                    "Target stream '{stream_name}' has been deleted. Cannot switch to a deleted stream."
                ))));
            }
            StreamExistence::Missing => {
                return Err(to_mcp_error(invalid_input(format!(
                    "Target stream '{stream_name}' does not exist"
                ))));
            }
        }

        let opened = self.opened_files_for_workspace(&workspace_name).await?;
        if !opened.is_empty() {
            return Err(to_mcp_error(invalid_input(format!(
                "Cannot switch stream: workspace '{workspace_name}' has {} open file(s). Revert or submit changes before switching.",
                opened.len()
            ))));
        }

        if preview {
            let target = self
                .run_p4(json_invocation(
                    vec![
                        "stream".to_string(),
                        "-o".to_string(),
                        stream_name.clone(),
                    ],
                    None,
                ))
                .await?;
            let first = target.records.first();
            return Ok(Json(ToolResponse::success(
                "switch",
                json!({
                    "preview": true,
                    "current_stream": current_stream,
                    "target_stream": stream_name,
                    "workspace": workspace_name,
                    "target_stream_type": first.and_then(|record| record_string_field(record, "Type")),
                    "target_stream_parent": first.and_then(|record| record_string_field(record, "Parent")),
                }),
            )));
        }

        let mut args = vec![
            "client".to_string(),
            "-s".to_string(),
            "-S".to_string(),
            stream_name.clone(),
        ];
        if let Some(workspace) = workspace {
            args.push(workspace);
        }
        let result = self.run_p4(json_invocation(args, None)).await?;
        let _sync_result = self
            .executor
            .run(
                json_invocation(
                    vec!["sync".to_string(), "-k".to_string(), format!("{stream_name}/...")],
                    None,
                ),
                P4Env::new(),
            )
            .await;

        Ok(Json(ToolResponse::success(
            "switch",
            json!({
                "message": format!("Workspace '{workspace_name}' switched from '{current_stream}' to stream '{stream_name}'"),
                "result": output_message(result),
            }),
        )))
    }
```

- [ ] **Step 4: Route switch to the new helper**

In `modify_streams_inner`, replace the temporary `StreamModifyCommand::Switch` arm with:

```rust
            StreamModifyCommand::Switch {
                stream_name,
                workspace,
                preview,
            } => {
                self.execute_stream_switch(stream_name, workspace, preview).await
            }
```

Keep the `preview_invocation` branch from Task 1 because it already shows `p4 stream -o <target>` for preview switches and `p4 client -s -S` for real switches.

- [ ] **Step 5: Run switch tests**

Run:

```bash
rtk cargo test modify_streams_switch_preview_is_read_only_after_approval
rtk cargo test modify_streams_switch_rejects_open_files_before_switching
rtk cargo test modify_streams_switch_executes_client_switch_and_have_table_sync_after_approval
```

Expected:

```text
PASS
```

- [ ] **Step 6: Commit**

```bash
rtk git add src/server.rs
rtk git commit -m "fix: restore stream switch workflow"
```

---

### Task 5: Verification And PR Review Response Draft

**Files:**
- Modify: `docs/superpowers/plans/2026-06-17-upstream-parity-stream-modify-workflows.md`

- [ ] **Step 1: Run focused stream tests**

Run:

```bash
rtk cargo test modify_streams_ --test tool_mapping_tests
rtk cargo test modify_streams_
```

Expected:

```text
PASS
```

- [ ] **Step 2: Run full verification**

Run:

```bash
rtk cargo test
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
PASS
PASS
```

- [ ] **Step 3: Check diff scope**

Run:

```bash
rtk git status --short
rtk git diff --stat
rtk git diff -- src/tools/streams.rs src/server.rs tests/tool_mapping_tests.rs docs/superpowers/plans/2026-06-17-upstream-parity-stream-modify-workflows.md
```

Expected:

```text
Only src/tools/streams.rs, src/server.rs, tests/tool_mapping_tests.rs, and this plan file are changed.
```

- [ ] **Step 4: Append execution evidence to this plan**

Append this section to the bottom of `docs/superpowers/plans/2026-06-17-upstream-parity-stream-modify-workflows.md` after commands pass:

```markdown
---

## Execution Evidence

- `rtk cargo test modify_streams_ --test tool_mapping_tests`: PASS
- `rtk cargo test modify_streams_`: PASS
- `rtk cargo test`: PASS
- `rtk cargo clippy --all-targets --all-features -- -D warnings`: PASS
```

- [ ] **Step 5: Draft PR review thread replies without posting**

Use these drafts for the two open GitHub threads if the user asks to reply:

```text
Fixed by restoring the upstream switch workflow instead of only changing the command preview.

`modify_streams.switch preview=true` now runs the same validation path as upstream, then returns a read-only preview response. It does not execute `p4 client -s -S ...`. The real switch path still runs only after approval and performs the workspace stream switch plus the upstream have-table `sync -k` follow-up.
```

```text
Fixed by splitting the collapsed `CreateOrUpdate` path into upstream-shaped `create` and `update` workflows.

`create` now rejects existing streams and validates stream type and parent semantics before saving a new stream form. `update` now rejects missing streams and runs the upstream service-level safety checks before saving. This addresses the duplicate-create issue as part of the broader Rust port alignment rather than adding a one-off check.
```

- [ ] **Step 6: Commit verification notes**

```bash
rtk git add docs/superpowers/plans/2026-06-17-upstream-parity-stream-modify-workflows.md
rtk git commit -m "docs: record stream workflow verification"
```

---

## Self-Review Results

- Spec coverage: covered both open review issues and the root cause: `create`, `update`, and `switch` now get action-specific workflows instead of early command flattening.
- Placeholder scan: no incomplete placeholders are intentionally left in this plan.
- Type consistency: `StreamModifyCommand::{Create, Update, Switch}` is introduced in Task 1 and used consistently by later tasks.
- Scope control: no new public stream actions are added; the public schema remains upstream-shaped and existing approval behavior remains stricter than upstream as documented.

---

## Execution Evidence

- `rtk cargo test modify_streams_ --test tool_mapping_tests`: PASS, 13 passed.
- `rtk cargo test modify_streams_update_rejects_create_only_fields_before_approval`: PASS, 1 passed.
- `rtk cargo test modify_streams_create_virtual_defaults_parent_flow_options_after_approval`: PASS, 1 passed.
- `rtk cargo test modify_streams_update_treats_no_files_to_resolve_as_noop`: PASS, 1 passed.
- `rtk cargo test modify_streams_switch_executes_client_switch_and_have_table_sync_after_approval`: PASS, 1 passed.
- `rtk cargo test modify_streams_update`: PASS, 10 passed.
- `rtk cargo test modify_streams_create`: PASS, 10 passed.
- `rtk cargo test modify_streams_switch`: PASS, 5 passed.
- `rtk cargo test modify_streams_`: PASS, 39 passed.
- `rtk cargo fmt --check`: PASS.
- `rtk cargo clippy --all-targets --all-features -- -D warnings`: PASS.
- `rtk cargo test`: first sandbox run failed because `wiremock` could not bind a local mock-server port; rerun outside sandbox PASS, 256 passed.
- PR review reply drafts were prepared in this plan and were not posted.
