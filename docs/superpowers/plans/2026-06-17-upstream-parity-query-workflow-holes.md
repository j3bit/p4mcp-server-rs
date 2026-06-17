# Upstream Parity Query Workflow Holes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the root query workflow parity holes behind the current PR review comments for file grep, job listing limits, and workspace existence checks.

**Architecture:** Keep the upstream-shaped public tool schemas unchanged and fix the execution layer where Rust collapsed upstream service workflows into shallow single-command paths. Use Perforce CLI-native behavior where it is the direct porting equivalent of upstream P4Python behavior: `p4 grep -s` for benign long-line skips, `p4 fixes -m<limit>` for bounded job listings, and `p4 clients -e <workspace>` before reading named client forms. Do not add Rust-only query actions or broaden the PR beyond upstream parity plus parity bug fixes.

**Tech Stack:** Rust 2024, `rmcp`, `serde_json`, direct `p4` CLI invocations through `P4Executor`, upstream baseline `perforce/p4mcp-server` `v2026.2.2955897` at commit `a64efb07511b2a62db41aeed110ab96744c4076a`.

---

## Context

Current unresolved review threads:

- `PRRT_kwDOS5FH5M6J77bI`: `query_files.grep` fails broad searches when `p4 grep` encounters a text file with a line longer than 4096 characters.
- `PRRT_kwDOS5FH5M6J77bZ`: `query_jobs.list_jobs` ignores `max_results`.
- `PRRT_kwDOS5FH5M6J77bg`: `query_workspaces.get` and `query_workspaces.type` can return a generated `p4 client -o` template for a missing workspace.

Verified upstream and CLI facts:

- Upstream `p4mcp/services/file_services.py::grep_files()` runs `p4 grep -n [-i] -e <pattern> <depot_path>` with P4Python exceptions disabled, filters `"maximum line length"` errors as benign, truncates matches to `max_results`, and returns remaining matches successfully.
- `p4 help grep` documents `-s` as suppressing errors from abandoned files whose single-line length exceeds 4096 characters. In the CLI port, `-s` is the simplest faithful equivalent of upstream's benign-error filtering because the generic executor otherwise treats the nonzero exit as a command failure before parsed matches can be returned.
- Upstream `p4mcp/services/job_services.py::list_jobs_from_changelist()` runs `p4 fixes -m{limit} -c <changelist>`.
- `p4 help fixes` documents `-m max` as limiting job fixes.
- Upstream `p4mcp/services/workspace_services.py::get_workspace()` checks `p4 clients -e <workspace>` before running `p4 client -o <workspace>`.
- `p4 client -o <missing>` can return a new/default client form template, so a named workspace query must prove existence before reading the form.

Non-goals:

- Do not add new workspace query actions such as `opened`, `changes`, or `where`.
- Do not change public param field names.
- Do not change write approval gate behavior.
- Do not add a general nonzero-exit recovery mode to `P4Executor` for this PR. The current issue has a stable CLI-native `p4 grep -s` equivalent.

## File Structure

- Modify `src/tools/files.rs`
  - Add `-s` to the `query_files.grep` command builder so long-line errors are suppressed by the CLI.
- Modify `src/tools/jobs.rs`
  - Thread `max_results` into `list_jobs` as `-m<max_results>`.
- Modify `src/tools/workspaces.rs`
  - Add `build_workspace_exists_invocation()` for `p4 clients -e <workspace>`.
  - Keep `build_workspace_query_invocation()` for the actual `client -o` read and existing direct builder tests.
- Modify `src/server.rs`
  - Add a `require_existing_workspace()` workflow helper.
  - Route `query_workspaces.get`, `type`, and `status` through existence validation before any `client -o`.
  - Keep `list` as the existing single-command workflow.
- Modify `tests/tool_mapping_tests.rs`
  - Update grep and jobs command-builder contract tests.
  - Add direct coverage for the workspace existence helper.
- Modify `tests/mcp_smoke_tests.rs`
  - Update server smoke tests for grep, jobs, and workspace get/type/status workflows.
  - Add missing-workspace tests proving `client -o` is not invoked for phantom workspaces.

---

### Task 1: Port Upstream Grep Long-Line Semantics

**Files:**
- Modify: `src/tools/files.rs:95-111`
- Test: `tests/tool_mapping_tests.rs:284-301`
- Test: `tests/mcp_smoke_tests.rs:996-1034`

- [ ] **Step 1: Update the failing grep builder test**

In `tests/tool_mapping_tests.rs`, replace `query_file_grep_maps_pattern()` with:

```rust
#[test]
fn query_file_grep_maps_pattern_and_suppresses_long_line_errors() {
    let params = QueryFilesParams {
        action: FileQueryAction::Grep,
        file_path: "//depot/main/...".to_string(),
        file2: None,
        diff2: true,
        max_results: 50,
        pattern: Some("needle".to_string()),
        case_insensitive: true,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec![
            "grep",
            "-n",
            "-s",
            "-i",
            "-e",
            "needle",
            "//depot/main/..."
        ]
    );
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

- [ ] **Step 2: Run the grep builder test and verify it fails**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern_and_suppresses_long_line_errors --test tool_mapping_tests
```

Expected: FAIL because the current builder emits `["grep", "-n", "-i", "-e", ...]` without `-s`.

- [ ] **Step 3: Update the grep server smoke test expectation**

In `tests/mcp_smoke_tests.rs`, inside `query_files_grep_caps_records_by_max_results()`, replace the final invocation assertion with:

```rust
    assert_eq!(
        executor.invocations()[0].args,
        ["grep", "-n", "-s", "-i", "-e", "needle", "//depot/main/..."]
    );
```

- [ ] **Step 4: Run the grep smoke test and verify it fails**

Run:

```bash
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: FAIL because the server still calls the old grep invocation without `-s`.

- [ ] **Step 5: Add `-s` to the grep command builder**

In `src/tools/files.rs`, replace the grep arm with:

```rust
        FileQueryAction::Grep => {
            let pattern = params
                .pattern
                .clone()
                .ok_or_else(|| P4McpError::InvalidInput {
                    message: "pattern is required for grep".to_string(),
                })?;
            let mut args = vec!["grep".into(), "-n".into(), "-s".into()];
            if params.case_insensitive {
                args.push("-i".into());
            }
            args.extend(["-e".into(), pattern, params.file_path.clone()]);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
```

- [ ] **Step 6: Run grep-focused tests and verify they pass**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern_and_suppresses_long_line_errors --test tool_mapping_tests
rtk cargo test query_file_grep_requires_pattern --test tool_mapping_tests
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: all three commands PASS.

- [ ] **Step 7: Commit the grep workflow fix**

Run:

```bash
rtk git add src/tools/files.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align grep long-line workflow with upstream"
```

Expected: commit succeeds with only grep-related changes staged.

---

### Task 2: Thread `max_results` Into Job Fix Listings

**Files:**
- Modify: `src/tools/jobs.rs:7-31`
- Test: `tests/tool_mapping_tests.rs:1538-1542`
- Test: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Update the job builder test to require `-m<limit>`**

In `tests/tool_mapping_tests.rs`, replace `job_list_for_changelist_uses_fixes()` with:

```rust
#[test]
fn job_list_for_changelist_uses_fixes_with_max_results() {
    let invocation = build_job_query_invocation("list_jobs", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["fixes", "-m10", "-c", "123"]);
}
```

- [ ] **Step 2: Run the job builder test and verify it fails**

Run:

```bash
rtk cargo test job_list_for_changelist_uses_fixes_with_max_results --test tool_mapping_tests
```

Expected: FAIL because the current builder emits `["fixes", "-c", "123"]`.

- [ ] **Step 3: Add a server smoke test proving `max_results` reaches the executor**

In `tests/mcp_smoke_tests.rs`, update the import list at the top so `JobQueryAction` and `QueryJobsParams` are imported:

```rust
            ChangelistQueryAction, FileQueryAction, JobQueryAction, QueryChangelistsParams,
            QueryFilesParams, QueryJobsParams, QueryStreamsParams, QueryWorkspacesParams,
            StreamQueryAction, WorkspaceQueryAction,
```

Add this test after `query_changelists_calls_injected_executor()`:

```rust
#[tokio::test]
async fn query_jobs_list_threads_max_results_to_fixes() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"Job": "job000001", "Change": "123"})],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_jobs(Parameters(QueryJobsParams {
            action: JobQueryAction::ListJobs,
            changelist_id: Some("123".to_string()),
            job_id: None,
            max_results: 3,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list_jobs");
    assert_eq!(
        response.0.message,
        json!([{"Job": "job000001", "Change": "123"}])
    );
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(executor.invocations()[0].args, ["fixes", "-m3", "-c", "123"]);
}
```

- [ ] **Step 4: Run the job smoke test and verify it fails**

Run:

```bash
rtk cargo test query_jobs_list_threads_max_results_to_fixes --test mcp_smoke_tests
```

Expected: FAIL because the current server invocation omits `-m3`.

- [ ] **Step 5: Implement the jobs command-builder fix**

In `src/tools/jobs.rs`, change the function signature and `list_jobs` arm to use `max_results`:

```rust
pub fn build_job_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    job_id: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list_jobs" => vec![
            "fixes".into(),
            format!("-m{max_results}"),
            "-c".into(),
            required(changelist_id, "changelist_id")?,
        ],
        "get_job" => vec!["job".into(), "-o".into(), required(job_id, "job_id")?],
        other => {
            return Err(P4McpError::InvalidInput {
                message: format!("unknown action: {other}"),
            });
        }
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}
```

- [ ] **Step 6: Run jobs-focused tests and verify they pass**

Run:

```bash
rtk cargo test job_list_for_changelist_uses_fixes_with_max_results --test tool_mapping_tests
rtk cargo test job_get_blank_id_errors --test tool_mapping_tests
rtk cargo test query_jobs_list_threads_max_results_to_fixes --test mcp_smoke_tests
```

Expected: all three commands PASS.

- [ ] **Step 7: Confirm no query builder still discards `max_results`**

Run:

```bash
rtk rg "_max_results" src/tools
```

Expected: no matches. `rg` exits with status 1 when there are no matches; that is acceptable for this check.

- [ ] **Step 8: Commit the jobs workflow fix**

Run:

```bash
rtk git add src/tools/jobs.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: apply job listing limits"
```

Expected: commit succeeds with only jobs-related changes staged.

---

### Task 3: Add Workspace Existence Validation Workflows

**Files:**
- Modify: `src/tools/workspaces.rs:30-60`
- Modify: `src/server.rs:45-65`
- Modify: `src/server.rs:234-290`
- Modify: `src/server.rs:1456-1486`
- Test: `tests/tool_mapping_tests.rs:1234-1246`
- Test: `tests/mcp_smoke_tests.rs:315-390`

- [ ] **Step 1: Add a direct builder test for workspace existence lookup**

In `tests/tool_mapping_tests.rs`, replace the workspace import line:

```rust
        workspaces::{build_workspace_delete_invocation, build_workspace_query_invocation},
```

with:

```rust
        workspaces::{
            build_workspace_delete_invocation, build_workspace_exists_invocation,
            build_workspace_query_invocation,
        },
```

Replace the workspace tests around `workspace_list_by_user_uses_user_filter()` and `workspace_type_uses_client_spec()` with this block:

```rust
#[test]
fn workspace_list_by_user_uses_user_filter() {
    let invocation = build_workspace_query_invocation("list", None, Some("alice"), 7).unwrap();
    assert_eq!(invocation.args, vec!["clients", "-m", "7", "-u", "alice"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn workspace_exists_uses_clients_exact_filter() {
    let invocation = build_workspace_exists_invocation("ws-stream").unwrap();
    assert_eq!(invocation.args, vec!["clients", "-e", "ws-stream"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn workspace_type_uses_client_spec_after_existence_check() {
    let invocation = build_workspace_query_invocation("type", Some("ws-stream"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["client", "-o", "ws-stream"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

- [ ] **Step 2: Run the workspace builder test and verify it fails**

Run:

```bash
rtk cargo test workspace_exists_uses_clients_exact_filter --test tool_mapping_tests
```

Expected: FAIL because `build_workspace_exists_invocation` is not defined.

- [ ] **Step 3: Update workspace smoke tests for get, type, and status workflows**

In `tests/mcp_smoke_tests.rs`, replace `query_workspaces_type_classifies_stream_workspace()` with:

```rust
#[tokio::test]
async fn query_workspaces_type_checks_existence_then_classifies_stream_workspace() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"client": "ws-stream"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "Client": "ws-stream",
                "Update": "2026/06/17",
                "Stream": "//streams/main"
            })],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Type,
            workspace_name: Some("ws-stream".to_string()),
            user: None,
            max_results: 10,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "type");
    assert_eq!(response.0.message, json!({"workspace_type": "stream"}));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].args, ["clients", "-e", "ws-stream"]);
    assert_eq!(invocations[1].args, ["client", "-o", "ws-stream"]);
}
```

Add this test immediately after it:

```rust
#[tokio::test]
async fn query_workspaces_get_rejects_missing_workspace_before_client_form_read() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Get,
            workspace_name: Some("ghost-ws".to_string()),
            user: None,
            max_results: 10,
        }))
        .await
    {
        Ok(_) => panic!("query_workspaces get should reject missing workspace"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("Workspace 'ghost-ws' does not exist"));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["clients", "-e", "ghost-ws"]);
}
```

Add this test immediately after the missing get test:

```rust
#[tokio::test]
async fn query_workspaces_status_rejects_missing_workspace_before_status_commands() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Status,
            workspace_name: Some("ghost-ws".to_string()),
            user: None,
            max_results: 10,
        }))
        .await
    {
        Ok(_) => panic!("query_workspaces status should reject missing workspace"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("Workspace 'ghost-ws' does not exist"));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["clients", "-e", "ghost-ws"]);
}
```

In `query_workspaces_status_runs_upstream_status_commands()`, add an existence output before the client spec output:

```rust
        P4CommandOutput {
            records: vec![json!({"client": "ws-main"})],
            text: json!({}),
        },
```

Then replace the invocation assertions in that test with:

```rust
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 6);
    assert_eq!(invocations[0].args, ["clients", "-e", "ws-main"]);
    assert_eq!(invocations[1].args, ["client", "-o", "ws-main"]);
    assert_eq!(invocations[2].args, ["opened"]);
    assert_eq!(invocations[3].args, ["sync", "-n"]);
    assert_eq!(invocations[4].args, ["resolve", "-n"]);
    assert_eq!(invocations[5].args, ["changes", "-m1", "#have"]);
```

- [ ] **Step 4: Run workspace smoke tests and verify they fail**

Run:

```bash
rtk cargo test query_workspaces_type_checks_existence_then_classifies_stream_workspace --test mcp_smoke_tests
rtk cargo test query_workspaces_get_rejects_missing_workspace_before_client_form_read --test mcp_smoke_tests
rtk cargo test query_workspaces_status_rejects_missing_workspace_before_status_commands --test mcp_smoke_tests
rtk cargo test query_workspaces_status_runs_upstream_status_commands --test mcp_smoke_tests
```

Expected: the new type/get/status tests fail because the server does not yet run `clients -e` before `client -o`.

- [ ] **Step 5: Add the workspace existence invocation helper**

In `src/tools/workspaces.rs`, add this function immediately after `required_workspace_name()`:

```rust
pub fn build_workspace_exists_invocation(workspace_name: &str) -> Result<P4Invocation> {
    Ok(P4Invocation {
        args: vec![
            "clients".into(),
            "-e".into(),
            required(Some(workspace_name), "workspace_name")?,
        ],
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}
```

- [ ] **Step 6: Import the workspace existence helper in the server**

In `src/server.rs`, update the workspace tools import to include `build_workspace_exists_invocation`.

The import should read:

```rust
        workspaces::{
            build_workspace_delete_invocation, build_workspace_exists_invocation,
            build_workspace_query_invocation, patch_workspace_form, required_workspace_name,
        },
```

- [ ] **Step 7: Add server workspace workflow helpers**

In `src/server.rs`, replace the current `query_workspace_type()` helper with these helpers:

```rust
    async fn require_existing_workspace(&self, workspace_name: &str) -> McpResult<()> {
        let output = self
            .run_p4(
                build_workspace_exists_invocation(workspace_name).map_err(to_mcp_error)?,
            )
            .await?;
        if output.records.is_empty() {
            return Err(to_mcp_error(invalid_input(format!(
                "Workspace '{workspace_name}' does not exist"
            ))));
        }
        Ok(())
    }

    async fn query_workspace_get(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_name =
            require_non_blank(workspace_name, "workspace_name").map_err(to_mcp_error)?;
        self.require_existing_workspace(&workspace_name).await?;
        let invocation =
            build_workspace_query_invocation("get", Some(&workspace_name), None, 100)
                .map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success("get", output_message(output))))
    }

    async fn query_workspace_type(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_name =
            require_non_blank(workspace_name, "workspace_name").map_err(to_mcp_error)?;
        self.require_existing_workspace(&workspace_name).await?;
        let invocation =
            build_workspace_query_invocation("type", Some(&workspace_name), None, 100)
                .map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(
            "type",
            json!({"workspace_type": workspace_type_from_records(&output.records)}),
        )))
    }
```

- [ ] **Step 8: Route workspace `status` through the existence workflow**

In `src/server.rs`, in `query_workspace_status()`, insert the existence check immediately after `workspace_name` is required:

```rust
        self.require_existing_workspace(&workspace_name).await?;
```

The beginning of the function should read:

```rust
    async fn query_workspace_status(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_name =
            require_non_blank(workspace_name, "workspace_name").map_err(to_mcp_error)?;
        self.require_existing_workspace(&workspace_name).await?;

        let workspace_spec = self
            .run_workspace_status_command(
                json_invocation(vec!["client".into(), "-o".into(), workspace_name], None),
                None,
            )
            .await?;
```

- [ ] **Step 9: Route workspace `get` through the workflow**

In `src/server.rs`, in `query_workspaces()`, add a `get` branch before the `type` branch:

```rust
        if action == "get" {
            return self
                .query_workspace_get(params.workspace_name.as_deref())
                .await;
        }
        if action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
```

Keep the existing `status` branch after `type`, and keep `list` on the direct builder path.

- [ ] **Step 10: Run workspace-focused tests and verify they pass**

Run:

```bash
rtk cargo test workspace_exists_uses_clients_exact_filter --test tool_mapping_tests
rtk cargo test workspace_type_uses_client_spec_after_existence_check --test tool_mapping_tests
rtk cargo test query_workspaces_type_checks_existence_then_classifies_stream_workspace --test mcp_smoke_tests
rtk cargo test query_workspaces_get_rejects_missing_workspace_before_client_form_read --test mcp_smoke_tests
rtk cargo test query_workspaces_status_rejects_missing_workspace_before_status_commands --test mcp_smoke_tests
rtk cargo test query_workspaces_status_runs_upstream_status_commands --test mcp_smoke_tests
```

Expected: all six commands PASS.

- [ ] **Step 11: Commit the workspace workflow fix**

Run:

```bash
rtk git add src/tools/workspaces.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: validate workspace existence before form reads"
```

Expected: commit succeeds with only workspace workflow changes staged.

---

### Task 4: Full Verification

**Files:**
- Verify: entire repository

- [ ] **Step 1: Format the code**

Run:

```bash
rtk cargo fmt
```

Expected: command exits successfully.

- [ ] **Step 2: Check formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS.

- [ ] **Step 3: Run focused parity tests**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern_and_suppresses_long_line_errors --test tool_mapping_tests
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
rtk cargo test job_list_for_changelist_uses_fixes_with_max_results --test tool_mapping_tests
rtk cargo test query_jobs_list_threads_max_results_to_fixes --test mcp_smoke_tests
rtk cargo test workspace_exists_uses_clients_exact_filter --test tool_mapping_tests
rtk cargo test query_workspaces_type_checks_existence_then_classifies_stream_workspace --test mcp_smoke_tests
rtk cargo test query_workspaces_get_rejects_missing_workspace_before_client_form_read --test mcp_smoke_tests
rtk cargo test query_workspaces_status_rejects_missing_workspace_before_status_commands --test mcp_smoke_tests
rtk cargo test query_workspaces_status_runs_upstream_status_commands --test mcp_smoke_tests
```

Expected: all commands PASS.

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

Expected: PASS. If the sandbox blocks WireMock local port binding with `PermissionDenied`, rerun this same command outside the sandbox through the normal escalation flow and record that the escalated full suite passed.

- [ ] **Step 6: Inspect the final diff**

Run:

```bash
rtk git status --short
rtk git log --oneline -5
rtk git diff --stat origin/main...HEAD
```

Expected:

- Working tree is clean after commits.
- The latest commits are the three focused fixes from Tasks 1-3.
- Diff touches only `src/tools/files.rs`, `src/tools/jobs.rs`, `src/tools/workspaces.rs`, `src/server.rs`, `tests/tool_mapping_tests.rs`, and `tests/mcp_smoke_tests.rs`.

- [ ] **Step 7: Push the branch**

Run:

```bash
rtk git push
```

Expected: branch pushes to `origin/codex/add-gitignore`.

---

### Task 5: PR Review Thread Follow-Up

**Files:**
- GitHub PR: `https://github.com/j3bit/p4mcp-server-rs/pull/2`

- [ ] **Step 1: Confirm the three target threads are still unresolved**

Run:

```bash
rtk gh api graphql \
  -F owner=j3bit \
  -F repo=p4mcp-server-rs \
  -F number=2 \
  -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewThreads(first:100){nodes{id isResolved isOutdated path comments(first:10){nodes{body author{login}}}}}}}}'
```

Expected: the unresolved, non-outdated threads include:

- `PRRT_kwDOS5FH5M6J77bI`
- `PRRT_kwDOS5FH5M6J77bZ`
- `PRRT_kwDOS5FH5M6J77bg`

- [ ] **Step 2: Reply to the grep thread**

Run:

```bash
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bI \
  -f body='Fixed in the latest push.

This was a true upstream-parity workflow gap. Upstream grep keeps broad searches successful when p4 reports benign 4096-character long-line skips. In the Rust CLI port, the direct equivalent is to run p4 grep with -s so those long-line abandon errors are suppressed by the CLI while normal grep matches still flow through the existing parsed-record cap. The command now remains grep -n [-i] -e <pattern> <path> in behavior, with the additional CLI-native -s suppressor for this upstream benign-error case.' \
  -f query='mutation($threadId:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$threadId,body:$body}){comment{url}}}'
```

Expected: GitHub returns a comment URL for the inline reply.

- [ ] **Step 3: Reply to the jobs thread**

Run:

```bash
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bZ \
  -f body='Fixed in the latest push.

This was a true upstream-parity issue. Upstream list_jobs calls p4 fixes with the requested max_results as -m<limit>, and the Perforce CLI supports that limit for fixes. The Rust port now threads QueryJobsParams.max_results into the list_jobs invocation and has both direct builder coverage and a server-to-executor smoke test proving the public limit reaches p4 fixes.' \
  -f query='mutation($threadId:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$threadId,body:$body}){comment{url}}}'
```

Expected: GitHub returns a comment URL for the inline reply.

- [ ] **Step 4: Reply to the workspace thread**

Run:

```bash
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bg \
  -f body='Fixed in the latest push.

This was a true upstream-parity workflow issue. Upstream get_workspace checks p4 clients -e <workspace> before reading p4 client -o <workspace>, because client -o can return a template for a missing client. The Rust port now routes query_workspaces.get, type, and status through the same existence workflow before any client form read. Missing named workspaces return a clear invalid-params error and do not call client -o.' \
  -f query='mutation($threadId:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$threadId,body:$body}){comment{url}}}'
```

Expected: GitHub returns a comment URL for the inline reply.

- [ ] **Step 5: Resolve the three threads**

Run:

```bash
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bI \
  -f query='mutation($threadId:ID!){resolveReviewThread(input:{threadId:$threadId}){thread{id isResolved}}}'
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bZ \
  -f query='mutation($threadId:ID!){resolveReviewThread(input:{threadId:$threadId}){thread{id isResolved}}}'
rtk gh api graphql \
  -F threadId=PRRT_kwDOS5FH5M6J77bg \
  -f query='mutation($threadId:ID!){resolveReviewThread(input:{threadId:$threadId}){thread{id isResolved}}}'
```

Expected: each response reports `isResolved: true`.

- [ ] **Step 6: Confirm no target thread remains unresolved**

Run:

```bash
rtk gh api graphql \
  -F owner=j3bit \
  -F repo=p4mcp-server-rs \
  -F number=2 \
  -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewThreads(first:100){nodes{id isResolved isOutdated path}}}}}'
```

Expected: the three thread IDs from Step 1 are all `isResolved: true`.

---

## Self-Review

Spec coverage:

- Grep long-line root cause is covered by Task 1. The CLI port uses `-s`, which is the `p4` CLI equivalent of upstream's benign `"maximum line length"` filtering.
- Jobs limit propagation is covered by Task 2. The `max_results` field reaches `p4 fixes`.
- Workspace phantom form reads are covered by Task 3. Named `get`, `type`, and `status` all prove existence before `client -o`.
- Full verification and PR follow-up are covered by Tasks 4 and 5.

Placeholder scan:

- The plan contains no deferred implementation slots.
- Every code-changing step includes concrete code.
- Every verification step includes exact commands and expected results.

Type consistency:

- `build_workspace_exists_invocation()` returns `Result<P4Invocation>` and is imported alongside existing workspace helpers.
- `require_existing_workspace()` returns `McpResult<()>`, matching existing server helper style.
- `QueryJobsParams.max_results` remains `u16` and is formatted as `-m{max_results}` for upstream parity.
- `query_workspaces` keeps `list` on the direct builder path and routes only `get`, `type`, and `status` through server workflows.
