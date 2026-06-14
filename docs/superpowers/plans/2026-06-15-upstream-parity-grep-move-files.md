# Upstream Parity Grep And Move Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the two unresolved PR review issues by matching upstream `perforce/p4mcp-server` behavior for `query_files.grep` result limiting and `modify_changelists.move_files`.

**Architecture:** Keep the Rust port's existing CLI builder and server routing structure. Apply the grep cap after `p4 grep` returns tagged records, because upstream caps client-side and `p4 grep` has no `-m` result-limit flag in the current mapped invocation. Add `move_files` to the changelist modify path as the upstream changelist-reorganization action, implemented as `p4 reopen -c <change> <file>...`, while preserving the Rust write approval gate before execution.

**Tech Stack:** Rust, `rmcp`, `serde_json`, async tests with `tokio`, local `p4` CLI invocation builders, existing fake executor and fake approval gate test helpers.

---

## Source Of Truth

Use the upstream baseline already fixed in `README.md`: `perforce/p4mcp-server` `v2026.2.2955897` at commit `a64efb07511b2a62db41aeed110ab96744c4076a`.

Relevant upstream behavior to mirror:

- `p4mcp/services/file_services.py::grep_files` builds `p4 grep -n [-i] -e <pattern> <depot_path>`, then filters dict records and truncates `matches` to `max_results`.
- `p4mcp/tools/changelist_tools.py::modify_changelists` exposes action `move_files`.
- `p4mcp/handlers/changelist_handlers.py::_handle_modify_changelists` requires `file_paths` for `move_files`.
- `p4mcp/services/changelist_services.py::move_files_to_changelist` requires a valid changelist and runs `p4 reopen -c <changelist_id> <file>` for each file.

Rust-port adaptation:

- Keep the local `CommonModifyParams.files` field instead of adding an upstream-shaped `file_paths` alias, because the Rust port already uses `files` consistently for common modify tools.
- Preserve the approved Rust-only write approval gate. `move_files` must be approved before the executor receives any `p4 reopen` invocation.
- Do not add workspace query extensions or unrelated tool surface changes.

## File Structure

- Modify `src/server.rs`
  - Cap `query_files` grep records before building `ToolResponse`.
  - Allow `modify_changelists` action `move_files`.
  - Include `move_files` file paths in the approval preview command and targets.
  - Add focused approval gate tests for the new write action.
- Modify `src/tools/changelists.rs`
  - Extend `build_changelist_modify_invocation` to accept files.
  - Map `move_files` to `p4 reopen -c <changelist_id> <file>...`.
  - Validate changelist id and files before returning an invocation.
- Modify `tests/tool_mapping_tests.rs`
  - Update existing changelist modify builder calls to pass `&[]`.
  - Add direct builder tests for `move_files`.
- Modify `tests/mcp_smoke_tests.rs`
  - Add a server-to-executor test proving `query_files.grep` returns at most `max_results` records and does not add a fake CLI limit flag.

## Task 1: Cap Grep Records In `query_files`

**Files:**
- Modify: `tests/mcp_smoke_tests.rs`
- Modify: `src/server.rs`

- [ ] **Step 1: Write the failing smoke test**

Add this test in `tests/mcp_smoke_tests.rs` after `query_workspaces_list_by_user_calls_injected_executor`:

```rust
#[tokio::test]
async fn query_files_grep_caps_records_by_max_results() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![
            json!({"depotFile": "//depot/main/a.rs", "line": "1"}),
            json!({"depotFile": "//depot/main/b.rs", "line": "2"}),
            json!({"depotFile": "//depot/main/c.rs", "line": "3"}),
        ],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_files(Parameters(QueryFilesParams {
            action: FileQueryAction::Grep,
            file_path: "//depot/main/...".to_string(),
            file2: None,
            diff2: true,
            max_results: 2,
            pattern: Some("needle".to_string()),
            case_insensitive: true,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "grep");
    assert_eq!(
        response.0.message,
        json!([
            {"depotFile": "//depot/main/a.rs", "line": "1"},
            {"depotFile": "//depot/main/b.rs", "line": "2"},
        ])
    );
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(
        executor.invocations()[0].args,
        ["grep", "-n", "-i", "-e", "needle", "//depot/main/..."]
    );
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: the test fails because `response.0.message` still contains all three fake grep records.

- [ ] **Step 3: Implement the server-side grep cap**

In `src/server.rs`, replace the current `query_files` method body with:

```rust
    pub async fn query_files(
        &self,
        Parameters(params): Parameters<QueryFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Files, "query_files")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let is_grep = params.action == FileQueryAction::Grep;
        let max_results = params.max_results as usize;
        let invocation = build_file_invocation(&params).map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        let message = if is_grep {
            output_message_with_record_limit(output, max_results)
        } else {
            output_message(output)
        };
        Ok(Json(ToolResponse::success(action, message)))
    }
```

Then add this helper immediately after the existing `output_message` function in `src/server.rs`:

```rust
fn output_message_with_record_limit(mut output: P4CommandOutput, max_records: usize) -> Value {
    if output.records.is_empty() {
        output.text
    } else {
        output.records.truncate(max_records);
        Value::Array(output.records)
    }
}
```

Do not add `-m` to `p4 grep`. The upstream behavior is client-side truncation after grep records are parsed.

- [ ] **Step 4: Run the focused test and verify it passes**

Run:

```bash
rtk cargo test query_files_grep_caps_records_by_max_results --test mcp_smoke_tests
```

Expected: one test passes and the executor invocation remains `["grep", "-n", "-i", "-e", "needle", "//depot/main/..."]`.

- [ ] **Step 5: Run the existing grep mapping test**

Run:

```bash
rtk cargo test query_file_grep_maps_pattern --test tool_mapping_tests
```

Expected: one test passes. This confirms the CLI builder still avoids an unsupported grep result-limit flag.

- [ ] **Step 6: Commit the grep fix**

Run:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: cap grep results"
```

Expected: commit succeeds with only the grep cap changes staged.

## Task 2: Add Changelist `move_files` CLI Mapping

**Files:**
- Modify: `src/tools/changelists.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Write the failing builder tests**

In `tests/tool_mapping_tests.rs`, update every existing call to `build_changelist_modify_invocation` to pass a fourth argument `&[]`. For example:

```rust
let invocation = build_changelist_modify_invocation("submit", "123", None, &[]).unwrap();
```

Then add these tests after `changelist_submit_uses_numbered_change`:

```rust
#[test]
fn changelist_move_files_uses_reopen() {
    let files = vec![
        "//depot/main/a.rs".to_string(),
        "//depot/main/b.rs".to_string(),
    ];

    let invocation =
        build_changelist_modify_invocation("move_files", "123", None, &files).unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "reopen",
            "-c",
            "123",
            "//depot/main/a.rs",
            "//depot/main/b.rs",
        ]
    );
    assert_eq!(invocation.stdin, None);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn changelist_move_files_requires_files() {
    let error = build_changelist_modify_invocation("move_files", "123", None, &[])
        .unwrap_err()
        .to_string();

    assert!(error.contains("files is required for move_files"));
}

#[test]
fn changelist_move_files_empty_id_errors() {
    let files = vec!["//depot/main/a.rs".to_string()];
    let error = build_changelist_modify_invocation("move_files", " ", None, &files)
        .unwrap_err()
        .to_string();

    assert!(error.contains("changelist_id is required for move_files"));
}
```

- [ ] **Step 2: Run the focused builder tests and verify they fail**

Run:

```bash
rtk cargo test changelist_move_files --test tool_mapping_tests
```

Expected: the tests fail because `build_changelist_modify_invocation` does not accept files and does not recognize `move_files`.

- [ ] **Step 3: Extend the changelist modify builder**

In `src/tools/changelists.rs`, replace the builder signature and body with:

```rust
pub fn build_changelist_modify_invocation(
    action: &str,
    changelist_id: &str,
    stdin: Option<String>,
    files: &[String],
) -> Result<P4Invocation> {
    let (args, stdin) = match action {
        "create" => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "create")?),
        ),
        "update" => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "update")?),
        ),
        "submit" => (
            vec![
                "submit".into(),
                "-c".into(),
                required_value(changelist_id, "changelist_id", "submit")?,
            ],
            None,
        ),
        "delete" => (
            vec![
                "change".into(),
                "-d".into(),
                required_value(changelist_id, "changelist_id", "delete")?,
            ],
            None,
        ),
        "move_files" => {
            let change = required_value(changelist_id, "changelist_id", "move_files")?;
            required_files(files, "move_files")?;
            let mut args = vec!["reopen".into(), "-c".into(), change];
            args.extend(files.iter().cloned());
            (args, None)
        }
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin,
        mode: OutputMode::JsonLines,
    })
}
```

Add this helper after `required_value`:

```rust
fn required_files(files: &[String], action: &str) -> Result<()> {
    if files.is_empty() {
        Err(P4McpError::InvalidInput {
            message: format!("files is required for {action}"),
        })
    } else {
        Ok(())
    }
}
```

- [ ] **Step 4: Update all Rust call sites for the new signature**

In `src/server.rs`, change the current call from:

```rust
let invocation = build_changelist_modify_invocation(&params.action, &changelist_id, stdin)
    .map_err(to_mcp_error)?;
```

to:

```rust
let invocation =
    build_changelist_modify_invocation(&params.action, &changelist_id, stdin, &params.files)
        .map_err(to_mcp_error)?;
```

In `tests/tool_mapping_tests.rs`, make sure every non-`move_files` builder test passes `&[]` as the fourth argument.

- [ ] **Step 5: Run the focused builder tests and verify they pass**

Run:

```bash
rtk cargo test changelist_move_files --test tool_mapping_tests
```

Expected: all tests whose names contain `changelist_move_files` pass.

- [ ] **Step 6: Run the existing changelist modify mapping tests**

Run:

```bash
rtk cargo test changelist_ --test tool_mapping_tests
```

Expected: existing changelist query and modify tests pass with the new builder signature.

- [ ] **Step 7: Commit the builder mapping**

Run:

```bash
rtk git add src/tools/changelists.rs tests/tool_mapping_tests.rs src/server.rs
rtk git commit -m "fix: map changelist move files"
```

Expected: commit succeeds with the `move_files` CLI mapping and signature updates staged.

## Task 3: Wire `move_files` Through Server Approval And Execution

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Write the failing approval-required test**

In the `#[cfg(test)] mod tests` section of `src/server.rs`, add this test after `modify_changelists_without_approval_does_not_call_executor`:

```rust
    #[tokio::test]
    async fn modify_changelists_move_files_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/a.rs"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("move_files");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ];

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_changelists");
        assert_eq!(calls[0].request.action, "move_files");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(
            calls[0].request.preview.targets,
            [
                "changelist:123",
                "//depot/main/a.rs",
                "//depot/main/b.rs",
            ]
        );
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "reopen".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/a.rs".to_string(),
                "//depot/main/b.rs".to_string(),
            ])
        );
    }
```

- [ ] **Step 2: Write the failing approved execution test**

Add this test immediately after the approval-required test:

```rust
    #[tokio::test]
    async fn modify_changelists_move_files_after_approval_reopens_files() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/a.rs"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("move_files");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ];
        params.approval_token = Some("approved-token".to_string());

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved write should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "move_files");
        assert_eq!(response.0.message, json!([{"depotFile": "//depot/main/a.rs"}]));

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].args,
            [
                "reopen",
                "-c",
                "123",
                "//depot/main/a.rs",
                "//depot/main/b.rs",
            ]
        );

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].approval_token.as_deref(), Some("approved-token"));
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
    }
```

- [ ] **Step 3: Run the focused server tests and verify they fail**

Run:

```bash
rtk cargo test modify_changelists_move_files
```

Expected: the tests fail before server routing accepts `move_files`.

- [ ] **Step 4: Allow `move_files` in `modify_changelists_inner`**

In `src/server.rs`, change the `changelist_id` match from:

```rust
        let changelist_id = match params.action.as_str() {
            "create" => "new".to_string(),
            "update" | "submit" | "delete" => required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &params.action,
            )?,
            other => return Err(to_mcp_error(unknown_action(other))),
        };
```

to:

```rust
        let changelist_id = match params.action.as_str() {
            "create" => "new".to_string(),
            "update" | "submit" | "delete" | "move_files" => required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &params.action,
            )?,
            other => return Err(to_mcp_error(unknown_action(other))),
        };
```

- [ ] **Step 5: Include moved files in approval preview targets**

In `src/server.rs`, change the changelist approval context target line from:

```rust
                targets: changelist_targets(&changelist_id),
```

to:

```rust
                targets: changelist_modify_targets(&params.action, &changelist_id, &params.files),
```

Add this helper immediately after `changelist_targets`:

```rust
fn changelist_modify_targets(action: &str, changelist_id: &str, files: &[String]) -> Vec<String> {
    let mut targets = changelist_targets(changelist_id);
    if action == "move_files" {
        targets.extend(files.iter().cloned());
    }
    targets
}
```

- [ ] **Step 6: Run the focused server tests and verify they pass**

Run:

```bash
rtk cargo test modify_changelists_move_files
```

Expected: both `modify_changelists_move_files_*` tests pass. The executor receives no invocation before approval and receives exactly one `reopen` invocation after approval.

- [ ] **Step 7: Commit the server routing**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: support changelist move files"
```

Expected: commit succeeds with the server approval and execution changes staged.

## Task 4: Final Verification

**Files:**
- Verify: repository root

- [ ] **Step 1: Check formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: no formatting diff.

- [ ] **Step 2: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 3: Run the full test suite**

Run:

```bash
rtk cargo test
```

Expected: all tests pass. If sandboxed `cargo test` fails only because WireMock cannot bind a local OS port, rerun the same command with escalated execution and record that the first failure was port-binding only.

- [ ] **Step 4: Check the working tree**

Run:

```bash
rtk git status --short --branch
```

Expected: branch is ahead of origin by the new commits and the working tree is clean.

- [ ] **Step 5: Push the branch**

Run:

```bash
rtk git push
```

Expected: the PR branch updates successfully.

## Task 5: Reply To The Two GitHub Review Threads

**Files:**
- Verify: GitHub PR #2 review threads

- [ ] **Step 1: Fetch thread state**

Run:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected: the two unresolved threads are still the grep cap thread and the changelist `move_files` thread.

- [ ] **Step 2: Reply to the grep thread**

Run:

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3409762713/replies -f body=$'Fixed in the latest push.\n\n`query_files.grep` now mirrors upstream `perforce/p4mcp-server` behavior: the `p4 grep` invocation remains `grep -n [-i] -e <pattern> <path>`, and the server truncates parsed records to `max_results` before returning the tool response. I added a server-to-executor smoke test that proves the returned records are capped while the CLI invocation does not invent an unsupported grep limit flag.'
```

Expected: GitHub creates an inline reply in the existing grep review thread.

- [ ] **Step 3: Reply to the move_files thread**

Run:

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3409762715/replies -f body=$'Fixed in the latest push.\n\n`modify_changelists` now supports the upstream `move_files` action as changelist reorganization, distinct from `modify_files.move` rename operations. The Rust port maps it to `p4 reopen -c <change> <file>...`, requires `changelist_id` and `files`, and still runs through the write approval gate before the executor can call `p4`. I added direct builder coverage plus approval-required and approved-execution tests.'
```

Expected: GitHub creates an inline reply in the existing `move_files` review thread.

- [ ] **Step 4: Resolve both review threads**

Run:

```bash
rtk gh api graphql -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' -f thread=PRRT_kwDOS5FH5M6JaLIY
rtk gh api graphql -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' -f thread=PRRT_kwDOS5FH5M6JaLIZ
```

Expected: a fresh `fetch_comments.py` run shows both target threads with `isResolved: true`.

## Self-Review Results

- Spec coverage: Task 1 covers grep `max_results`; Tasks 2 and 3 cover upstream `modify_changelists.move_files`; Task 4 covers verification; Task 5 covers PR review follow-up.
- Red-flag scan: the plan contains exact files, exact code snippets, exact commands, and expected outcomes.
- Type consistency: `CommonModifyParams.files`, `FileQueryAction::Grep`, `build_changelist_modify_invocation(..., files: &[String])`, `P4CommandOutput`, `ToolResponse`, and `P4Invocation` match the current Rust code structure.
