# Upstream Parity File Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align Rust `query_files` and `modify_files` behavior with upstream `perforce/p4mcp-server` file workflow semantics.

**Architecture:** Keep the existing Rust CLI backend and typed params. Add narrowly scoped multi-invocation helpers only where upstream service workflows execute more than one P4 command: recursive file search and batch move. Preserve the write approval gate by approving once before any write side effect, and extend approval preview so multi-command writes show every planned P4 command.

**Tech Stack:** Rust 2024, `rmcp`, `serde_json`, `schemars`, Tokio async tests, direct `p4` CLI invocation.

---

## Context And Scope

Upstream baseline:

- Repository: `perforce/p4mcp-server`
- Version pinned in this Rust port README: `v2026.2.2955897`
- Commit: `a64efb07511b2a62db41aeed110ab96744c4076a`
- Relevant upstream files:
  - `p4mcp/tools/file_tools.py`
  - `p4mcp/handlers/file_handlers.py`
  - `p4mcp/models/file_models.py`
  - `p4mcp/services/file_services.py`

Open PR review threads this plan addresses:

- `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325640`
  - `modify_files.move` accepts arrays publicly but currently rejects more than one source/target pair.
- `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325643`
  - `query_files.search` under `//depot/proj/...` currently searches only `//depot/proj/.../<pattern>` and misses root-level files.

Additional same-root parity gap included in this plan:

- `query_files.diff` currently treats `diff2=false` as a one-path workspace diff and rejects `file2`.
- Upstream `QueryFilesParams` requires `file2` for every `diff` action and dispatches `p4 diff file1 file2` when `diff2=false`.
- This existing Rust test is therefore wrong and must be replaced: `query_file_workspace_diff_uses_single_path`.

Out of scope:

- Do not change non-file tools.
- Do not add new file tool actions.
- Do not change the approved write approval gate behavior beyond adding multi-command preview data.
- Do not change the upstream-pinned README baseline.

## File Structure

- Modify `src/tools/files.rs`
  - Keep single-command builders for existing file actions.
  - Add `build_file_search_invocations()` for upstream recursive search semantics.
  - Add `build_file_move_invocations()` for upstream batch move semantics.
  - Align `query_files.diff` command construction with upstream.

- Modify `src/server.rs`
  - Route `query_files.search` through the new multi-invocation search helper.
  - Ignore per-search `no such file(s)` failures, matching upstream.
  - Route `modify_files.move` through one approval request and post-approval sequential P4 execution.
  - Add a small sequence execution helper for write actions that intentionally execute multiple P4 invocations.
  - Add server-level tests for search aggregation and batch move approval/execution.

- Modify `src/approval.rs`
  - Add `commands: Option<Vec<Vec<String>>>` to `ApprovalPreview`.
  - Keep existing `command: Option<Vec<String>>` for all single-command previews.
  - Render multi-command previews in elicitation messages without changing single-command behavior.

- Modify `tests/tool_mapping_tests.rs`
  - Replace Rust-only diff expectations with upstream diff expectations.
  - Add direct mapping tests for recursive search filespecs.
  - Add direct mapping tests for multi-pair file move commands.

- Modify `tests/schema_contract_tests.rs`
  - Add file tool schema contract coverage so file params get the same upstream-parity guard already added for changelists, jobs, streams, and reviews.

---

### Task 1: Align File Schema And Diff Semantics

**Files:**

- Modify: `tests/schema_contract_tests.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`
- Modify: `src/tools/files.rs`

- [ ] **Step 1: Add file schema contract coverage**

In `tests/schema_contract_tests.rs`, extend the existing `params` import so it includes the file params and actions:

```rust
use p4mcp_server_rs::tools::{
    params::{
        ChangelistModifyAction, FileModifyAction, FileQueryAction, JobModifyAction,
        ModifyChangelistsParams, ModifyFilesParams, ModifyJobsParams, ModifyShelvesParams,
        ModifyStreamsParams, ModifyWorkspacesParams, QueryFilesParams, ShelfModifyAction,
        StreamModifyAction, WorkspaceModifyAction,
    },
    reviews::{ModifyReviewsParams, QueryReviewsParams, ReviewModifyAction, ReviewQueryAction},
    server::QueryServerParams,
};
```

Add this test after `query_server_schema_matches_upstream_params_object`:

```rust
#[test]
fn file_tool_schemas_match_upstream_fields() {
    for field in [
        "action",
        "file_path",
        "file2",
        "diff2",
        "max_results",
        "pattern",
        "case_insensitive",
    ] {
        assert_has::<QueryFilesParams>(field);
    }

    let query: QueryFilesParams = serde_json::from_value(serde_json::json!({
        "action": "diff",
        "file_path": "//depot/main/file.txt",
        "file2": "//depot/dev/file.txt",
        "diff2": false
    }))
    .unwrap();
    assert_eq!(query.action, FileQueryAction::Diff);
    assert_eq!(query.file2.as_deref(), Some("//depot/dev/file.txt"));
    assert!(!query.diff2);

    for field in [
        "action",
        "file_paths",
        "changelist",
        "source_paths",
        "target_paths",
        "mode",
        "force",
        "approval_token",
    ] {
        assert_has::<ModifyFilesParams>(field);
    }

    for field in ["files", "form", "confirmation"] {
        assert_omits::<ModifyFilesParams>(field);
    }

    let modify: ModifyFilesParams = serde_json::from_value(serde_json::json!({
        "action": "move",
        "source_paths": ["//depot/main/a.txt", "//depot/main/b.txt"],
        "target_paths": ["//depot/dev/a.txt", "//depot/dev/b.txt"]
    }))
    .unwrap();
    assert_eq!(modify.action, FileModifyAction::Move);
    assert_eq!(modify.source_paths.as_ref().unwrap().len(), 2);
    assert_eq!(modify.target_paths.as_ref().unwrap().len(), 2);
}
```

- [ ] **Step 2: Run the schema contract test**

Run:

```bash
rtk cargo test file_tool_schemas_match_upstream_fields --test schema_contract_tests
```

Expected: PASS. This test documents the public schema contract before changing behavior.

- [ ] **Step 3: Replace the Rust-only workspace diff tests with upstream diff tests**

In `tests/tool_mapping_tests.rs`, replace the current `query_file_workspace_diff_uses_single_path` test with:

```rust
#[test]
fn query_file_workspace_diff_requires_second_path_like_upstream() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: false,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };

    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("file2 is required for diff action"));
}
```

Replace the current `query_file_workspace_diff_rejects_second_path` test with:

```rust
#[test]
fn query_file_workspace_diff_uses_second_path_like_upstream() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: Some("//depot/dev/file.txt".to_string()),
        diff2: false,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };

    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation,
        P4Invocation {
            args: vec![
                "diff".into(),
                "//depot/main/file.txt".into(),
                "//depot/dev/file.txt".into()
            ],
            stdin: None,
            mode: OutputMode::Text,
        }
    );
}
```

Replace `query_file_diff2_requires_second_path` with:

```rust
#[test]
fn query_file_diff2_requires_second_path() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };

    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("file2 is required for diff action"));
}
```

- [ ] **Step 4: Update the smoke test expectation for invalid diff params**

In `tests/mcp_smoke_tests.rs`, inside `invalid_tool_params_return_invalid_params_error`, replace:

```rust
assert!(err.message.contains("file2 is required for diff2"));
```

with:

```rust
assert!(err.message.contains("file2 is required for diff action"));
```

- [ ] **Step 5: Run the diff tests and verify they fail**

Run:

```bash
rtk cargo test query_file_workspace_diff --test tool_mapping_tests
```

Expected: FAIL before implementation because the Rust port currently accepts `diff2=false` without `file2` and rejects `diff2=false` with `file2`.

Run:

```bash
rtk cargo test query_file_diff2_requires_second_path --test tool_mapping_tests
```

Expected: FAIL before implementation because the error message still says `diff2`.

- [ ] **Step 6: Align `query_files.diff` in the CLI builder**

In `src/tools/files.rs`, replace the `FileQueryAction::Diff` match arm with:

```rust
        FileQueryAction::Diff => {
            let file2 = params
                .file2
                .clone()
                .ok_or_else(|| P4McpError::InvalidInput {
                    message: "file2 is required for diff action".to_string(),
                })?;
            let command = if params.diff2 { "diff2" } else { "diff" };
            P4Invocation {
                args: vec![command.into(), params.file_path.clone(), file2],
                stdin: None,
                mode: OutputMode::Text,
            }
        }
```

- [ ] **Step 7: Run the focused diff tests**

Run:

```bash
rtk cargo test query_file_workspace_diff --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_file_diff2_requires_second_path --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test invalid_tool_params_return_invalid_params_error --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 8: Commit the diff parity fix**

Run:

```bash
rtk git add src/tools/files.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs tests/schema_contract_tests.rs
rtk git commit -m "fix: align file diff contract with upstream"
```

Expected: one commit containing only file schema contract coverage and diff behavior alignment.

---

### Task 2: Implement Upstream Recursive File Search Workflow

**Files:**

- Modify: `src/tools/files.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add direct search invocation tests**

In `tests/tool_mapping_tests.rs`, update the files import to include the new helper:

```rust
use p4mcp_server_rs::tools::files::{
    build_file_invocation, build_file_modify_invocation, build_file_search_invocations,
};
```

Add these tests after `query_file_grep_maps_pattern`:

```rust
#[test]
fn query_file_search_recursive_base_builds_root_and_recursive_specs() {
    let params = QueryFilesParams {
        action: FileQueryAction::Search,
        file_path: "//depot/proj/...".to_string(),
        file2: None,
        diff2: true,
        max_results: 5,
        pattern: Some("*.rs".to_string()),
        case_insensitive: false,
    };

    let invocations = build_file_search_invocations(&params).unwrap();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0],
        P4Invocation {
            args: vec![
                "files".into(),
                "-m".into(),
                "5".into(),
                "//depot/proj/*.rs".into()
            ],
            stdin: None,
            mode: OutputMode::JsonLines,
        }
    );
    assert_eq!(
        invocations[1],
        P4Invocation {
            args: vec![
                "files".into(),
                "-m".into(),
                "5".into(),
                "//depot/proj/.../*.rs".into()
            ],
            stdin: None,
            mode: OutputMode::JsonLines,
        }
    );
}

#[test]
fn query_file_search_non_recursive_base_builds_one_spec() {
    let params = QueryFilesParams {
        action: FileQueryAction::Search,
        file_path: "//depot/proj".to_string(),
        file2: None,
        diff2: true,
        max_results: 10,
        pattern: Some("*.rs".to_string()),
        case_insensitive: false,
    };

    let invocations = build_file_search_invocations(&params).unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0].args,
        vec!["files", "-m", "10", "//depot/proj/*.rs"]
    );
}
```

- [ ] **Step 2: Add server-level search aggregation tests**

In `src/server.rs`, inside `#[cfg(test)] mod tests`, add these tests after `modify_files_edit_preview_includes_p4_edit`:

```rust
    #[tokio::test]
    async fn query_files_search_recursive_base_aggregates_and_caps_results() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({"depotFile": "//depot/proj/root.rs"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![
                    json!({"depotFile": "//depot/proj/src/lib.rs"}),
                    json!({"depotFile": "//depot/proj/src/main.rs"}),
                ],
                text: json!({}),
            },
        ]));
        let server = P4McpServer::with_executor(test_config(false), executor.clone());

        let response = server
            .query_files(Parameters(QueryFilesParams {
                action: FileQueryAction::Search,
                file_path: "//depot/proj/...".to_string(),
                file2: None,
                diff2: true,
                max_results: 2,
                pattern: Some("*.rs".to_string()),
                case_insensitive: false,
            }))
            .await
            .expect("recursive search should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "search");
        assert_eq!(
            response.0.message,
            json!([
                {"depotFile": "//depot/proj/root.rs"},
                {"depotFile": "//depot/proj/src/lib.rs"},
            ])
        );

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].args, ["files", "-m", "2", "//depot/proj/*.rs"]);
        assert_eq!(
            invocations[1].args,
            ["files", "-m", "2", "//depot/proj/.../*.rs"]
        );
    }

    #[tokio::test]
    async fn query_files_search_ignores_no_such_file_for_one_pattern() {
        let executor = Arc::new(QueuedExecutor::results(vec![
            Err(P4McpError::P4Command {
                message: "p4 exited with failure; stderr: no such file(s)".to_string(),
            }),
            Ok(P4CommandOutput {
                records: vec![json!({"depotFile": "//depot/proj/src/lib.rs"})],
                text: json!({}),
            }),
        ]));
        let server = P4McpServer::with_executor(test_config(false), executor.clone());

        let response = server
            .query_files(Parameters(QueryFilesParams {
                action: FileQueryAction::Search,
                file_path: "//depot/proj/...".to_string(),
                file2: None,
                diff2: true,
                max_results: 10,
                pattern: Some("*.rs".to_string()),
                case_insensitive: false,
            }))
            .await
            .expect("one no-such pattern should be ignored");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "search");
        assert_eq!(
            response.0.message,
            json!([{"depotFile": "//depot/proj/src/lib.rs"}])
        );

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].args, ["files", "-m", "10", "//depot/proj/*.rs"]);
        assert_eq!(
            invocations[1].args,
            ["files", "-m", "10", "//depot/proj/.../*.rs"]
        );
    }
```

- [ ] **Step 3: Run the search tests and verify they fail**

Run:

```bash
rtk cargo test query_file_search_ --test tool_mapping_tests
```

Expected: FAIL because `build_file_search_invocations` does not exist yet.

Run:

```bash
rtk cargo test query_files_search_
```

Expected: FAIL because `query_files.search` still executes a single P4 invocation.

- [ ] **Step 4: Add search invocation helpers**

In `src/tools/files.rs`, add this helper above `build_file_invocation`:

```rust
pub fn build_file_search_invocations(params: &QueryFilesParams) -> Result<Vec<P4Invocation>> {
    let pattern = params
        .pattern
        .clone()
        .ok_or_else(|| P4McpError::InvalidInput {
            message: "pattern is required for search".to_string(),
        })?;

    Ok(search_filespecs(&params.file_path, &pattern)
        .into_iter()
        .map(|filespec| P4Invocation {
            args: vec![
                "files".into(),
                "-m".into(),
                params.max_results.to_string(),
                filespec,
            ],
            stdin: None,
            mode: OutputMode::JsonLines,
        })
        .collect())
}

fn search_filespecs(depot_path: &str, pattern: &str) -> Vec<String> {
    if let Some(base) = depot_path.strip_suffix("...") {
        vec![format!("{base}{pattern}"), format!("{base}.../{pattern}")]
    } else if depot_path.ends_with('/') {
        vec![format!("{depot_path}{pattern}")]
    } else {
        vec![format!("{depot_path}/{pattern}")]
    }
}
```

Then replace the current `FileQueryAction::Search` arm in `build_file_invocation` with:

```rust
        FileQueryAction::Search => {
            let mut invocations = build_file_search_invocations(params)?;
            if invocations.len() == 1 {
                invocations.remove(0)
            } else {
                return Err(P4McpError::InvalidInput {
                    message: "recursive search requires build_file_search_invocations"
                        .to_string(),
                });
            }
        }
```

This keeps the existing single-invocation builder useful for non-recursive direct tests while forcing server code to use the sequence helper for recursive search.

- [ ] **Step 5: Route `query_files.search` through the sequence helper**

In `src/server.rs`, change the file import:

```rust
        files::{
            build_file_invocation, build_file_modify_invocation, build_file_search_invocations,
        },
```

Add this method inside `impl P4McpServer`, immediately before `pub async fn query_files`:

```rust
    async fn query_files_search(
        &self,
        params: QueryFilesParams,
    ) -> McpResult<Json<ToolResponse>> {
        let mut records = Vec::new();
        let invocations = build_file_search_invocations(&params).map_err(to_mcp_error)?;

        for invocation in invocations {
            match self.executor.run(invocation, P4Env::new()).await {
                Ok(output) => records.extend(output.records),
                Err(error) if is_no_such_file_error(&error) => {}
                Err(error) => return Err(to_mcp_error(error)),
            }
        }

        records.truncate(params.max_results as usize);
        Ok(Json(ToolResponse::success("search", Value::Array(records))))
    }
```

Add this helper near `output_message_with_record_limit`:

```rust
fn is_no_such_file_error(error: &P4McpError) -> bool {
    error.to_string().to_ascii_lowercase().contains("no such file(s)")
}
```

Then update `query_files` so search exits early before the single-invocation builder:

```rust
    pub async fn query_files(
        &self,
        Parameters(params): Parameters<QueryFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Files, "query_files")
            .map_err(to_mcp_error)?;
        if params.action == FileQueryAction::Search {
            return self.query_files_search(params).await;
        }

        let action = params.action.as_str();
        let grep_max_results =
            (params.action == FileQueryAction::Grep).then_some(params.max_results as usize);
        let invocation = build_file_invocation(&params).map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        let message = if let Some(max_results) = grep_max_results {
            output_message_with_record_limit(output, max_results)
        } else {
            output_message(output)
        };
        Ok(Json(ToolResponse::success(action, message)))
    }
```

- [ ] **Step 6: Run the focused search tests**

Run:

```bash
rtk cargo test query_file_search_ --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_files_search_
```

Expected: PASS.

- [ ] **Step 7: Run existing grep coverage**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern --test tool_mapping_tests
```

Expected: PASS. The grep command must remain `grep -n [-i] -e <pattern> <path>` with client-side result capping in `src/server.rs`.

Run:

```bash
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 8: Commit the search parity fix**

Run:

```bash
rtk git add src/tools/files.rs src/server.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: align file search workflow with upstream"
```

Expected: one commit containing only search workflow changes and tests.

---

### Task 3: Add Multi-Command Approval Preview Support

**Files:**

- Modify: `src/approval.rs`
- Modify: `src/server.rs`

- [ ] **Step 1: Add a failing approval preview test**

In `src/approval.rs`, inside `#[cfg(test)] mod tests`, add this test after `sample_request()`:

```rust
    #[test]
    fn format_elicitation_message_lists_multiple_commands() {
        let mut preview = sample_request().preview;
        preview.command = None;
        preview.commands = Some(vec![
            vec![
                "p4".to_string(),
                "move".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/a.txt".to_string(),
                "//depot/dev/a.txt".to_string(),
            ],
            vec![
                "p4".to_string(),
                "move".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/b.txt".to_string(),
                "//depot/dev/b.txt".to_string(),
            ],
        ]);

        let message = format_elicitation_message(&preview);

        assert!(message.contains(
            "Command 1: p4 move -c 123 //depot/main/a.txt //depot/dev/a.txt"
        ));
        assert!(message.contains(
            "Command 2: p4 move -c 123 //depot/main/b.txt //depot/dev/b.txt"
        ));
    }
```

- [ ] **Step 2: Run the approval preview test and verify it fails**

Run:

```bash
rtk cargo test format_elicitation_message_lists_multiple_commands
```

Expected: FAIL because `ApprovalPreview` does not yet have a `commands` field.

- [ ] **Step 3: Extend `ApprovalPreview`**

In `src/approval.rs`, update `ApprovalPreview`:

```rust
pub struct ApprovalPreview {
    pub summary: String,
    pub tool: String,
    pub action: String,
    pub targets: Vec<String>,
    pub changelist: Option<String>,
    pub workspace: Option<String>,
    pub stream: Option<String>,
    pub review: Option<String>,
    pub command: Option<Vec<String>>,
    pub commands: Option<Vec<Vec<String>>>,
    pub request: Option<HttpPreview>,
}
```

In `sample_request()`, add `commands: None,` immediately after the existing `command: Some(...)` block:

```rust
                commands: None,
                request: Some(HttpPreview {
                    method: "POST".to_string(),
                    path: "/mcp/tools/modify_files".to_string(),
                }),
```

- [ ] **Step 4: Render multiple commands in elicitation text**

In `src/approval.rs`, update `format_elicitation_message()` by inserting this block immediately after the existing `if let Some(command) = &preview.command` block:

```rust
    if let Some(commands) = &preview.commands {
        for (index, command) in commands.iter().enumerate() {
            lines.push(format!("Command {}: {}", index + 1, command.join(" ")));
        }
    }
```

- [ ] **Step 5: Update server-side preview initializers**

In `src/server.rs`, update every `ApprovalPreview { ... }` literal to include `commands: None` unless it is the new multi-command file move preview from Task 4.

For `p4_approval_preview()`, the final fields should be:

```rust
            review: None,
            command: Some(command_preview(&self.config.p4_bin, context.invocation)),
            commands: None,
            request: None,
```

For review API approval previews, the final fields should be:

```rust
            review,
            command: None,
            commands: None,
            request: Some(HttpPreview {
                method: request.method.clone(),
                path: request.path.clone(),
            }),
```

- [ ] **Step 6: Run approval tests**

Run:

```bash
rtk cargo test format_elicitation_message_lists_multiple_commands
```

Expected: PASS.

Run:

```bash
rtk cargo test fallback_without_token_returns_approval_required
```

Expected: PASS. This verifies serialized approval preview responses still include the preview data expected by fallback clients.

- [ ] **Step 7: Commit the approval preview support**

Run:

```bash
rtk git add src/approval.rs src/server.rs
rtk git commit -m "fix: preview multi-command file writes"
```

Expected: one commit containing only preview data support and tests.

---

### Task 4: Implement Upstream Batch File Move Workflow

**Files:**

- Modify: `src/tools/files.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add direct batch move builder tests**

In `tests/tool_mapping_tests.rs`, update the files import to include the new helper:

```rust
use p4mcp_server_rs::tools::files::{
    build_file_invocation, build_file_modify_invocation, build_file_move_invocations,
    build_file_search_invocations,
};
```

Add these tests after `modify_file_move_source_target_mismatch_errors`:

```rust
#[test]
fn modify_file_move_multiple_pairs_builds_one_invocation_per_pair() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Move,
        file_paths: None,
        changelist: "123".to_string(),
        source_paths: Some(vec![
            "//depot/main/a.txt".to_string(),
            "//depot/main/b.txt".to_string(),
        ]),
        target_paths: Some(vec![
            "//depot/dev/a.txt".to_string(),
            "//depot/dev/b.txt".to_string(),
        ]),
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let invocations = build_file_move_invocations(&params).unwrap();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0].args,
        ["move", "-c", "123", "//depot/main/a.txt", "//depot/dev/a.txt"]
    );
    assert_eq!(
        invocations[1].args,
        ["move", "-c", "123", "//depot/main/b.txt", "//depot/dev/b.txt"]
    );
}

#[test]
fn modify_file_move_requires_source_and_target_paths() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Move,
        file_paths: None,
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let error = build_file_move_invocations(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("source_paths and target_paths required for move action"));
}
```

- [ ] **Step 2: Add server approval and execution tests for batch move**

In `src/server.rs`, inside `#[cfg(test)] mod tests`, add these tests after `modify_files_edit_preview_includes_p4_edit`:

```rust
    #[tokio::test]
    async fn modify_files_move_without_approval_previews_all_move_commands() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = modify_files_params(None);
        params.action = FileModifyAction::Move;
        params.force = false;
        params.file_paths = None;
        params.changelist = "123".to_string();
        params.source_paths = Some(vec![
            "//depot/main/a.txt".to_string(),
            "//depot/main/b.txt".to_string(),
        ]);
        params.target_paths = Some(vec![
            "//depot/dev/a.txt".to_string(),
            "//depot/dev/b.txt".to_string(),
        ]);

        let response = server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].request.tool, "modify_files");
        assert_eq!(calls[0].request.action, "move");
        assert_eq!(calls[0].request.preview.command, None);
        assert_eq!(
            calls[0].request.preview.commands,
            Some(vec![
                vec![
                    "p4".to_string(),
                    "move".to_string(),
                    "-c".to_string(),
                    "123".to_string(),
                    "//depot/main/a.txt".to_string(),
                    "//depot/dev/a.txt".to_string(),
                ],
                vec![
                    "p4".to_string(),
                    "move".to_string(),
                    "-c".to_string(),
                    "123".to_string(),
                    "//depot/main/b.txt".to_string(),
                    "//depot/dev/b.txt".to_string(),
                ],
            ])
        );
        assert_eq!(
            calls[0].request.preview.targets,
            [
                "//depot/main/a.txt",
                "//depot/main/b.txt",
                "//depot/dev/a.txt",
                "//depot/dev/b.txt",
            ]
        );
    }

    #[tokio::test]
    async fn modify_files_move_after_approval_executes_all_pairs() {
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({"depotFile": "//depot/dev/a.txt"})],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({"depotFile": "//depot/dev/b.txt"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let mut params = modify_files_params(Some("approved-token"));
        params.action = FileModifyAction::Move;
        params.force = false;
        params.file_paths = None;
        params.changelist = "123".to_string();
        params.source_paths = Some(vec![
            "//depot/main/a.txt".to_string(),
            "//depot/main/b.txt".to_string(),
        ]);
        params.target_paths = Some(vec![
            "//depot/dev/a.txt".to_string(),
            "//depot/dev/b.txt".to_string(),
        ]);

        let response = server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved batch move should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "move");
        assert_eq!(
            response.0.message,
            json!([
                [{"depotFile": "//depot/dev/a.txt"}],
                [{"depotFile": "//depot/dev/b.txt"}],
            ])
        );

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(
            invocations[0].args,
            ["move", "-c", "123", "//depot/main/a.txt", "//depot/dev/a.txt"]
        );
        assert_eq!(
            invocations[1].args,
            ["move", "-c", "123", "//depot/main/b.txt", "//depot/dev/b.txt"]
        );
    }
```

- [ ] **Step 3: Run the batch move tests and verify they fail**

Run:

```bash
rtk cargo test modify_file_move_ --test tool_mapping_tests
```

Expected: FAIL because `build_file_move_invocations` does not exist and the current move builder rejects multi-pair input.

Run:

```bash
rtk cargo test modify_files_move_
```

Expected: FAIL because `ApprovalPreview.commands` is not wired for file moves and `modify_files_inner` still executes a single invocation.

- [ ] **Step 4: Add `build_file_move_invocations()`**

In `src/tools/files.rs`, add this helper immediately before `build_file_modify_invocation`:

```rust
pub fn build_file_move_invocations(params: &ModifyFilesParams) -> Result<Vec<P4Invocation>> {
    let sources = params.source_paths.clone().unwrap_or_default();
    let targets = params.target_paths.clone().unwrap_or_default();

    if sources.is_empty() || targets.is_empty() {
        return Err(P4McpError::InvalidInput {
            message: "source_paths and target_paths required for move action".to_string(),
        });
    }
    if sources.len() != targets.len() {
        return Err(P4McpError::InvalidInput {
            message: "source_paths and target_paths must have the same length".to_string(),
        });
    }

    Ok(sources
        .into_iter()
        .zip(targets)
        .map(|(source, target)| P4Invocation {
            args: vec![
                "move".into(),
                "-c".into(),
                params.changelist.clone(),
                source,
                target,
            ],
            stdin: None,
            mode: OutputMode::JsonLines,
        })
        .collect())
}
```

Then replace the existing `FileModifyAction::Move` arm in `build_file_modify_invocation` with:

```rust
        FileModifyAction::Move => {
            let mut invocations = build_file_move_invocations(params)?;
            if invocations.len() == 1 {
                invocations.remove(0)
            } else {
                return Err(P4McpError::InvalidInput {
                    message: "multi-pair move requires build_file_move_invocations".to_string(),
                });
            }
        }
```

- [ ] **Step 5: Add sequence execution and multi-command file preview**

In `src/server.rs`, change the file import:

```rust
        files::{
            build_file_invocation, build_file_modify_invocation, build_file_move_invocations,
            build_file_search_invocations,
        },
```

Add this method inside `impl P4McpServer`, near `call_p4_tool_with_benign_success`:

```rust
    async fn call_p4_sequence_tool(
        &self,
        action: &str,
        invocations: Vec<P4Invocation>,
    ) -> McpResult<Json<ToolResponse>> {
        let mut messages = Vec::new();
        for invocation in invocations {
            let output = self
                .executor
                .run(invocation, P4Env::new())
                .await
                .map_err(to_mcp_error)?;
            messages.push(output_message(output));
        }

        Ok(Json(ToolResponse::success(action, Value::Array(messages))))
    }
```

Add this helper near `p4_approval_preview()`:

```rust
    fn p4_file_approval_preview(
        &self,
        action: &str,
        targets: Vec<String>,
        changelist: Option<String>,
        invocations: &[P4Invocation],
    ) -> ApprovalPreview {
        let commands: Vec<Vec<String>> = invocations
            .iter()
            .map(|invocation| command_preview(&self.config.p4_bin, invocation))
            .collect();
        let command = if commands.len() == 1 {
            commands.first().cloned()
        } else {
            None
        };
        let commands = if commands.len() > 1 {
            Some(commands)
        } else {
            None
        };

        ApprovalPreview {
            summary: approval_summary(action, &targets),
            tool: "modify_files".to_string(),
            action: action.to_string(),
            targets,
            changelist,
            workspace: None,
            stream: None,
            review: None,
            command,
            commands,
            request: None,
        }
    }
```

Update `modify_files_approval_request()` to accept a slice:

```rust
    fn modify_files_approval_request(
        &self,
        params: &ModifyFilesParams,
        invocations: &[P4Invocation],
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;
        let targets = modify_files_targets(params);
        let action = params.action.as_str().to_string();

        ApprovalRequest {
            tool: "modify_files".to_string(),
            action: action.clone(),
            params: serde_json::to_value(approval_params)
                .expect("modify files params serialize to JSON"),
            preview: self.p4_file_approval_preview(
                &action,
                targets,
                Some(params.changelist.clone()),
                invocations,
            ),
        }
    }
```

Update the first half of `modify_files_inner()`:

```rust
        let action = params.action.as_str();
        let invocations = if params.action == FileModifyAction::Move {
            build_file_move_invocations(&params).map_err(to_mcp_error)?
        } else {
            vec![build_file_modify_invocation(&params).map_err(to_mcp_error)?]
        };
        let request = self.modify_files_approval_request(&params, &invocations);
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        if params.action == FileModifyAction::Sync {
            let invocation = invocations
                .into_iter()
                .next()
                .expect("sync builds exactly one invocation");
            return self
                .call_p4_tool_with_benign_success(
                    action,
                    invocation,
                    "File(s) up-to-date",
                    json!("Workspace is already up-to-date"),
                )
                .await;
        }
        if params.action == FileModifyAction::Move {
            return self.call_p4_sequence_tool(action, invocations).await;
        }
        let invocation = invocations
            .into_iter()
            .next()
            .expect("non-move file action builds exactly one invocation");
        self.call_p4_tool(action, invocation).await
```

- [ ] **Step 6: Run the focused batch move tests**

Run:

```bash
rtk cargo test modify_file_move_ --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test modify_files_move_
```

Expected: PASS.

- [ ] **Step 7: Run existing file write approval tests**

Run:

```bash
rtk cargo test modify_files_without_approval_does_not_call_executor
```

Expected: PASS. Existing single-command file write previews must still use `preview.command`.

Run:

```bash
rtk cargo test modify_files_after_approval_calls_executor_once
```

Expected: PASS. Existing single-command file writes must still execute exactly once.

- [ ] **Step 8: Commit the batch move parity fix**

Run:

```bash
rtk git add src/tools/files.rs src/server.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: support batch file moves"
```

Expected: one commit containing only batch file move behavior and related approval preview wiring.

---

### Task 5: Full Verification And PR Review Follow-Up

**Files:**

- Verify: all modified files
- Optional GitHub write: PR review threads after user approval in the implementation session

- [ ] **Step 1: Run formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS.

If it fails, run:

```bash
rtk cargo fmt
```

Then rerun:

```bash
rtk cargo fmt --check
```

Expected: PASS.

- [ ] **Step 2: Run focused file workflow tests**

Run:

```bash
rtk cargo test file_tool_schemas_match_upstream_fields --test schema_contract_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_file_workspace_diff --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_file_search_ --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_files_search_
```

Expected: PASS.

Run:

```bash
rtk cargo test modify_file_move_ --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test modify_files_move_
```

Expected: PASS.

- [ ] **Step 3: Run regression tests for adjacent file behavior**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern --test tool_mapping_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: PASS.

Run:

```bash
rtk cargo test modify_files_sync_up_to_date_after_approval_returns_success
```

Expected: PASS.

Run:

```bash
rtk cargo test modify_files_without_approval_does_not_call_executor
```

Expected: PASS.

- [ ] **Step 4: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 5: Run the full test suite**

Run:

```bash
rtk cargo test
```

Expected: PASS. If a sandbox-local network bind failure appears from WireMock tests, rerun the same command with sandbox escalation in the controller session and record that the escalated full suite passed.

- [ ] **Step 6: Inspect the final diff**

Run:

```bash
rtk git status --short --branch
```

Expected: branch shows only committed changes or a clean working tree.

Run:

```bash
rtk git diff --stat origin/main...HEAD
```

Expected: modified files are limited to `src/tools/files.rs`, `src/server.rs`, `src/approval.rs`, `tests/tool_mapping_tests.rs`, `tests/schema_contract_tests.rs`, `tests/mcp_smoke_tests.rs`, plus this plan file if it is committed with the implementation.

- [ ] **Step 7: Push the branch**

Run:

```bash
rtk git push
```

Expected: branch pushes to `origin/codex/add-gitignore`.

- [ ] **Step 8: Reply to the batch move review thread**

Only execute this GitHub write step if the user has asked the implementation session to update PR review comments.

Run:

```bash
rtk gh api graphql \
  -f thread=PRRT_kwDOS5FH5M6J0-5e \
  -f body='Fixed in the latest push.

This was a true upstream-parity issue. Upstream `modify_files.move` accepts `source_paths` and `target_paths` arrays, validates that the pair counts match, then runs one `p4 move -c <changelist> <source> <target>` per pair. The Rust port now follows that workflow: it approves the write once, shows every planned move command in the approval preview, and executes all pairs only after approval.' \
  -f query='mutation($thread:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread,body:$body}){comment{id url}}}'
```

Expected: GitHub creates an inline reply in `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325640`.

- [ ] **Step 9: Resolve the batch move review thread**

Only execute this GitHub write step if Step 8 was executed.

Run:

```bash
rtk gh api graphql \
  -f thread=PRRT_kwDOS5FH5M6J0-5e \
  -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}'
```

Expected: `isResolved` is `true`.

- [ ] **Step 10: Reply to the recursive search review thread**

Only execute this GitHub write step if the user has asked the implementation session to update PR review comments.

Run:

```bash
rtk gh api graphql \
  -f thread=PRRT_kwDOS5FH5M6J0-5h \
  -f body='Fixed in the latest push.

This was a true upstream-parity issue. Upstream `search_files()` treats a recursive base ending in `...` as two searches: one root-level filespec and one recursive filespec, then aggregates and caps the result set. The Rust port now builds the same filespec sequence, ignores per-pattern `no such file(s)` failures like upstream, and truncates the aggregate results to `max_results`.' \
  -f query='mutation($thread:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread,body:$body}){comment{id url}}}'
```

Expected: GitHub creates an inline reply in `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325643`.

- [ ] **Step 11: Resolve the recursive search review thread**

Only execute this GitHub write step if Step 10 was executed.

Run:

```bash
rtk gh api graphql \
  -f thread=PRRT_kwDOS5FH5M6J0-5h \
  -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}'
```

Expected: `isResolved` is `true`.

- [ ] **Step 12: Confirm no unresolved review threads remain for these comments**

Run:

```bash
rtk gh api graphql \
  -F owner=j3bit \
  -F repo=p4mcp-server-rs \
  -F number=2 \
  -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewThreads(first:100){nodes{id isResolved path comments(first:1){nodes{url body}}}}}}}' \
  --jq '.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved==false) | {id,path,url:.comments.nodes[0].url,title:(.comments.nodes[0].body|split("\n\n")[0])}'
```

Expected: no unresolved entries for:

- `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325640`
- `https://github.com/j3bit/p4mcp-server-rs/pull/2#discussion_r3419325643`

---

## Self-Review

Spec coverage:

- Batch `modify_files.move` source/target pair support: Task 4.
- Recursive root-preserving `query_files.search`: Task 2.
- Same-root `query_files.diff` parity gap: Task 1.
- Approval preview safety for multi-command write: Task 3 and Task 4.
- Upstream public schema guard for file tools: Task 1.
- Final verification and PR follow-up: Task 5.

Placeholder scan:

- The plan contains no unresolved implementation slots and no generic edge-case instructions.
- Every code-changing step includes concrete code snippets.

Type consistency:

- New helpers are consistently named `build_file_search_invocations()` and `build_file_move_invocations()`.
- `ApprovalPreview.commands` is consistently `Option<Vec<Vec<String>>>`.
- Server tests use existing `FakeExecutor`, `QueuedExecutor`, `FakeApprovalGate`, `P4CommandOutput`, and `P4McpError` test utilities already present in `src/server.rs`.
