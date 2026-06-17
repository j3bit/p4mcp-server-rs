# Upstream Parity Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the two unresolved PR review issues by restoring upstream-compatible argument propagation for shelf file deletion and changelist user filtering.

**Architecture:** Keep the existing Rust port shape: MCP handlers in `src/server.rs` validate policy and construct `P4Invocation`, while command-specific query builders stay in `src/tools/*.rs`. Do not add new shelf actions or new query fields; only thread existing `CommonModifyParams.files` and `CommonQueryParams.user` into the already-supported P4 CLI invocations.

**Tech Stack:** Rust 2024, `rmcp`, async trait based fake executors, `cargo test`, `cargo clippy`, local `p4` CLI invocation model.

---

## Source Anchors

- Upstream reference commit: `perforce/p4mcp-server@a64efb07511b2a62db41aeed110ab96744c4076a`.
- Upstream shelf design: `p4mcp/tools/shelve_tools.py` accepts `file_paths` for `modify_shelves`; `p4mcp/services/shelve_services.py` appends file paths to shelf delete command arguments when present.
- Upstream changelist design: `p4mcp/tools/changelist_tools.py` accepts `user` for `query_changelists`; `p4mcp/services/changelist_services.py` adds `-u <user>` to `p4 changes` after status and workspace filters.
- P4 CLI mapping to preserve:
  - Shelf partial delete: `p4 shelve -d -c <change> <file>...`
  - Changelist list by user: `p4 changes -m <n> -s <status> -c <workspace> -u <user>`

## File Structure

- Modify `src/server.rs`
  - `modify_shelves_inner`: append `params.files` to the generated shelf command before approval preview and execution.
  - Existing test module: update shelf delete preview expectations and add an approved-write regression test.
- Modify `src/tools/changelists.rs`
  - `build_changelist_query_invocation`: accept `user: Option<&str>` and append `-u <user>` for `list`.
- Modify `tests/mcp_smoke_tests.rs`
  - `query_changelists_calls_injected_executor`: assert that a `user` query parameter reaches the executor as `-u alice`.
- Modify `tests/tool_mapping_tests.rs`
  - Existing direct `build_changelist_query_invocation` call sites: pass `None` for the new `user` argument.

## Scope Boundaries

- Do not add upstream shelf actions that this Rust port does not currently expose, such as `update` or `unshelve_to_changelist`.
- Do not add changelist depot-path filtering in this fix; this plan targets the unresolved `user` filter review item only.
- Do not change approval-gate behavior. The shelf file list must appear in both the approval preview and the final executor invocation because both use the same `P4Invocation`.

### Task 1: Preserve Shelf File Arguments

**Files:**
- Modify: `src/server.rs:252-265`
- Test: `src/server.rs` test module near `modify_shelves_without_approval_does_not_call_executor`

- [ ] **Step 1: Write the failing shelf delete tests**

In `src/server.rs`, update the existing `modify_shelves_without_approval_does_not_call_executor` preview assertion so the approval preview reflects the selected file path:

```rust
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/file.txt".to_string(),
            ])
        );
```

Then add this test immediately after `modify_shelves_without_approval_does_not_call_executor`:

```rust
    #[tokio::test]
    async fn modify_shelves_delete_after_approval_passes_file_paths_to_p4() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"change": "123"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("delete");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/file.txt".to_string(),
            "//depot/main/other.txt".to_string(),
        ];

        let response = server
            .modify_shelves_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved shelf delete should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "delete");

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].args,
            [
                "shelve",
                "-d",
                "-c",
                "123",
                "//depot/main/file.txt",
                "//depot/main/other.txt",
            ]
        );

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/file.txt".to_string(),
                "//depot/main/other.txt".to_string(),
            ])
        );
    }
```

- [ ] **Step 2: Run the shelf tests to verify they fail**

Run:

```bash
rtk cargo test modify_shelves --lib
```

Expected: FAIL. The failure should show `preview.command` or executor args missing `//depot/main/file.txt` because `modify_shelves_inner` currently builds only `p4 shelve -d -c 123`.

- [ ] **Step 3: Pass `params.files` into shelf invocations**

In `src/server.rs`, replace the `let args = match ...` block inside `modify_shelves_inner` with this block:

```rust
        let mut args = match params.action.as_str() {
            "shelve" => vec!["shelve".to_string(), "-c".to_string(), change.clone()],
            "unshelve" => vec!["unshelve".to_string(), "-s".to_string(), change.clone()],
            "delete" => {
                vec![
                    "shelve".to_string(),
                    "-d".to_string(),
                    "-c".to_string(),
                    change.clone(),
                ]
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        args.extend(params.files.iter().cloned());
```

Leave the next line as:

```rust
        let invocation = json_invocation(args, None);
```

This mirrors upstream's file-path contract for the shelf operations currently supported by the Rust port. Empty `params.files` still preserves the existing whole-shelf command.

- [ ] **Step 4: Run the shelf tests to verify they pass**

Run:

```bash
rtk cargo test modify_shelves --lib
```

Expected: PASS. The output should include the updated preview-only test and the approved execution regression test.

- [ ] **Step 5: Commit the shelf fix**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: preserve shelf file arguments"
```

Expected: commit succeeds with only `src/server.rs` staged for this task.

### Task 2: Honor Changelist User Filters

**Files:**
- Modify: `src/tools/changelists.rs:6-31`
- Modify: `src/server.rs:590-596`
- Test: `tests/mcp_smoke_tests.rs:144-176`
- Test: `tests/tool_mapping_tests.rs` direct changelist query builder call sites

- [ ] **Step 1: Write the failing changelist user-filter test**

In `tests/mcp_smoke_tests.rs`, update `query_changelists_calls_injected_executor` so it sends `user: Some("alice".to_string())`:

```rust
            user: Some("alice".to_string()),
```

Replace the expected executor args assertion with:

```rust
    assert_eq!(
        executor.invocations()[0].args,
        [
            "changes", "-m", "7", "-s", "pending", "-c", "ws-main", "-u", "alice"
        ]
    );
```

- [ ] **Step 2: Run the changelist smoke test to verify it fails**

Run:

```bash
rtk cargo test query_changelists_calls_injected_executor --test mcp_smoke_tests
```

Expected: FAIL. The assertion should show the actual args missing `-u alice`.

- [ ] **Step 3: Thread the `user` parameter into the changelist query builder**

In `src/tools/changelists.rs`, change the builder signature to include `user` before `max_results`:

```rust
pub fn build_changelist_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    status: Option<&str>,
    workspace_name: Option<&str>,
    user: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
```

In the `"list"` branch, add the upstream-compatible user filter after the workspace filter:

```rust
            if let Some(user) = user {
                args.extend(["-u".into(), user.into()]);
            }
```

The full `"list"` branch should read:

```rust
        "list" => {
            let mut args = vec!["changes".into(), "-m".into(), max_results.to_string()];
            if let Some(status) = status {
                args.extend(["-s".into(), status.into()]);
            }
            if let Some(workspace) = workspace_name {
                args.extend(["-c".into(), workspace.into()]);
            }
            if let Some(user) = user {
                args.extend(["-u".into(), user.into()]);
            }
            args
        }
```

In `src/server.rs`, update the `query_changelists` call site to pass `params.user.as_deref()` before `params.max_results`:

```rust
        let invocation = build_changelist_query_invocation(
            &params.action,
            params.changelist_id.as_deref(),
            params.status.as_deref(),
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
```

In `tests/tool_mapping_tests.rs`, update any direct `build_changelist_query_invocation` calls to pass `None` for the new `user` argument unless the test is explicitly covering user filtering.

- [ ] **Step 4: Run the changelist smoke test to verify it passes**

Run:

```bash
rtk cargo test query_changelists_calls_injected_executor --test mcp_smoke_tests
```

Expected: PASS. The fake executor should receive `["changes", "-m", "7", "-s", "pending", "-c", "ws-main", "-u", "alice"]`.

- [ ] **Step 5: Commit the changelist filter fix**

Run:

```bash
rtk git add src/tools/changelists.rs src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: honor changelist user filter"
```

Expected: commit succeeds with only the changelist query files staged for this task.

### Task 3: Full Verification

**Files:**
- Inspect: `src/server.rs`
- Inspect: `src/tools/changelists.rs`
- Inspect: `tests/mcp_smoke_tests.rs`
- Inspect: `docs/superpowers/plans/2026-06-14-upstream-parity-review-fixes.md`

- [ ] **Step 1: Run formatting check**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS with no formatting diff.

- [ ] **Step 2: Run the focused regression tests**

Run:

```bash
rtk cargo test modify_shelves --lib
rtk cargo test query_changelists_calls_injected_executor --test mcp_smoke_tests
```

Expected: both commands PASS.

- [ ] **Step 3: Run the full test suite**

Run:

```bash
rtk cargo test
```

Expected: PASS for all Rust tests.

- [ ] **Step 4: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 5: Inspect the final diff**

Run:

```bash
rtk git status --short --branch
rtk git log --oneline --decorate -5
```

Expected:

```text
* codex/add-gitignore...origin/codex/add-gitignore [ahead 2]
```

The recent commits should include:

```text
fix: honor changelist user filter
fix: preserve shelf file arguments
```

If this plan file is committed as a separate docs artifact, the branch may be ahead by 3 instead.

## Plan Self-Review

- Spec coverage: Task 1 covers the shelf delete selected-file issue while preserving upstream's file-path command contract. Task 2 covers the changelist `user` filter issue with upstream argument order. Task 3 covers verification.
- Placeholder scan: no open-ended implementation steps remain; every code step includes exact snippets and commands.
- Type consistency: `CommonModifyParams.files` is already `Vec<String>`, `CommonQueryParams.user` is already `Option<String>`, and the updated builder accepts `Option<&str>` to match existing `as_deref()` call-site style.
