# Workspace Query Upstream Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address the two unresolved PR review threads by honoring upstream workspace user filtering and removing Rust-only workspace query extensions from this PR.

**Architecture:** Keep `query_workspaces` as a single read-only MCP tool backed by `src/tools/workspaces.rs`. Thread the existing `CommonQueryParams.user` into the workspace list command. Remove the Rust-only `opened`, `changes`, and `where` workspace query actions from this PR, then document how to reintroduce them later as deliberate functional extensions rather than accidental porting drift.

**Tech Stack:** Rust, rmcp, serde/schemars parameter structs, `p4` CLI invocations, Cargo tests, GitHub PR review threads.

---

## Context

Upstream reference: `perforce/p4mcp-server` `main` at `a64efb07511b2a62db41aeed110ab96744c4076a`.

Review threads being addressed:

- `PRRT_kwDOS5FH5M6JZnls`: `query_workspaces(action="list", user=...)` drops the user filter.
- `PRRT_kwDOS5FH5M6JZnlu`: `query_workspaces(action="opened" | "changes", workspace_name=...)` drops the workspace filter.

Decision from the human reviewer:

- The `user` filter issue is a true upstream-parity issue. Fix it in this PR.
- `write approval gate` is an approved Rust-port extension because it is a safety measure, not a functional feature.
- `query_workspaces.opened`, `query_workspaces.changes`, and `query_workspaces.where` are functional extensions. Remove them from this PR to respect upstream scope.
- Preserve the future extension path in `docs/todos/`, with explicit behavior:
  - `opened`: if `workspace_name` is provided, run `p4 opened -C <workspace>`.
  - `changes`: prefer steering callers to `query_changelists(action="list", workspace_name=...)`; if retained as a workspace query, run `p4 changes -c <workspace>`.
  - `where`: current-client mapping only, `file_path` required, `workspace_name` unsupported.

Important upstream facts:

- Upstream `query_workspaces` tool declares actions `list`, `get`, `type`, and `status`.
- Upstream `list_workspaces(user, limit)` runs `p4.run("clients", "-u", user, f"-m{limit}")` when `user` is provided.
- Upstream does not expose `opened`, `changes`, or `where` as `query_workspaces` actions.
- This plan is intentionally limited to the two current unresolved review items. It does not implement missing upstream `type` or `status`; that should be handled as a separate parity item if required.

## File Structure

- Modify `src/tools/workspaces.rs`
  - Add `user: Option<&str>` to `build_workspace_query_invocation`.
  - Append `-u <user>` for `action == "list"` when `user` is present and non-blank.
  - Remove `where`, `opened`, and `changes` match arms.

- Modify `src/server.rs`
  - Pass `params.user.as_deref()` to the workspace query builder.
  - Stop passing `params.file_path` to the workspace query builder.
  - Narrow the tool description from "List, get, map, or inspect workspaces" to "List or get workspaces".

- Modify `tests/tool_mapping_tests.rs`
  - Add direct builder coverage for `query_workspaces list` with `user`.
  - Replace the existing `where` mapping test with rejection tests for `where`, `opened`, and `changes`.
  - Update direct builder call sites for the new signature.

- Modify `tests/mcp_smoke_tests.rs`
  - Add server-to-executor coverage showing `CommonQueryParams.user` reaches `p4 clients`.

- Create `docs/todos/workspace-query-extensions.md`
  - Document the deferred extension design for `opened`, `changes`, and `where`.

- Modify `docs/superpowers/plans/2026-06-14-workspace-query-upstream-parity.md`
  - Include this plan file in the final documentation commit if it is not already committed.

---

### Task 1: Fix Workspace List User Filtering

**Files:**
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`
- Modify: `src/tools/workspaces.rs`
- Modify: `src/server.rs`

- [ ] **Step 1: Write the failing direct builder test**

In `tests/tool_mapping_tests.rs`, add this test immediately before `workspace_get_blank_name_errors`:

```rust
#[test]
fn workspace_list_by_user_uses_user_filter() {
    let invocation = build_workspace_query_invocation("list", None, Some("alice"), 7).unwrap();
    assert_eq!(
        invocation.args,
        vec!["clients", "-m", "7", "-u", "alice"]
    );
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

- [ ] **Step 2: Write the failing MCP smoke test**

In `tests/mcp_smoke_tests.rs`, add this test immediately after `query_changelists_calls_injected_executor`:

```rust
#[tokio::test]
async fn query_workspaces_list_by_user_calls_injected_executor() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"client": "ws-main", "Owner": "alice"})],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(CommonQueryParams {
            action: "list".to_string(),
            changelist_id: None,
            workspace_name: None,
            file_path: None,
            user: Some("alice".to_string()),
            status: None,
            job_id: None,
            stream: None,
            owner: None,
            max_results: 7,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
    assert_eq!(
        response.0.message,
        json!([{"client": "ws-main", "Owner": "alice"}])
    );
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(
        executor.invocations()[0].args,
        ["clients", "-m", "7", "-u", "alice"]
    );
}
```

- [ ] **Step 3: Run the new tests to verify they fail**

Run:

```bash
rtk cargo test workspace_list_by_user_uses_user_filter --test tool_mapping_tests
rtk cargo test query_workspaces_list_by_user_calls_injected_executor --test mcp_smoke_tests
```

Expected:

- `workspace_list_by_user_uses_user_filter` fails to compile because `build_workspace_query_invocation` still accepts four arguments.
- `query_workspaces_list_by_user_calls_injected_executor` runs through the current server path and fails because the executor receives `["clients", "-m", "7"]` without `-u alice`.

- [ ] **Step 4: Update the workspace query builder**

Replace the top-level function in `src/tools/workspaces.rs` with:

```rust
pub fn build_workspace_query_invocation(
    action: &str,
    workspace_name: Option<&str>,
    user: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list" => {
            let mut args = vec!["clients".into(), "-m".into(), max_results.to_string()];
            if let Some(user) = non_blank(user) {
                args.extend(["-u".into(), user.into()]);
            }
            args
        }
        "get" => vec![
            "client".into(),
            "-o".into(),
            required(workspace_name, "workspace_name")?,
        ],
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

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}
```

Keep the existing `required` function below this new `non_blank` helper.

- [ ] **Step 5: Update the server call site**

In `src/server.rs`, change the `query_workspaces` tool description and builder call to:

```rust
    #[tool(
        description = "List or get workspaces",
        annotations(read_only_hint = true)
    )]
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
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

- [ ] **Step 6: Run the focused tests to verify the user filter passes**

Run:

```bash
rtk cargo test workspace_list_by_user_uses_user_filter --test tool_mapping_tests
rtk cargo test query_workspaces_list_by_user_calls_injected_executor --test mcp_smoke_tests
```

Expected:

- Both tests pass.

- [ ] **Step 7: Commit the user filter fix**

Run:

```bash
rtk git add src/tools/workspaces.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: honor workspace user filter"
```

Expected:

- A commit is created with only workspace user-filter code and tests.

---

### Task 2: Remove Rust-Only Workspace Query Actions

**Files:**
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `src/tools/workspaces.rs`

- [ ] **Step 1: Replace the `where` mapping test with extension rejection tests**

In `tests/tool_mapping_tests.rs`, remove the current `workspace_where_maps_file_argument` test:

```rust
#[test]
fn workspace_where_maps_file_argument() {
    let invocation =
        build_workspace_query_invocation("where", None, Some("//depot/main/file.rs"), 10).unwrap();
    assert_eq!(invocation.args, vec!["where", "//depot/main/file.rs"]);
}
```

Add these tests in its place:

```rust
#[test]
fn workspace_where_is_rejected_as_deferred_extension() {
    let error = build_workspace_query_invocation("where", None, None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown action: where"));
}

#[test]
fn workspace_opened_is_rejected_as_deferred_extension() {
    let error = build_workspace_query_invocation("opened", Some("ws-main"), None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown action: opened"));
}

#[test]
fn workspace_changes_is_rejected_as_deferred_extension() {
    let error = build_workspace_query_invocation("changes", Some("ws-main"), None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown action: changes"));
}
```

- [ ] **Step 2: Update the blank workspace test call site**

In `tests/tool_mapping_tests.rs`, update `workspace_get_blank_name_errors` to use the new builder signature:

```rust
#[test]
fn workspace_get_blank_name_errors() {
    let error = build_workspace_query_invocation("get", Some(" "), None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("workspace_name is required"));
}
```

If the previous task has already changed the signature to `(action, workspace_name, user, max_results)`, this exact code is already correct. Do not add a `file_path` argument back.

- [ ] **Step 3: Run the rejection tests**

Run:

```bash
rtk cargo test workspace_where_is_rejected_as_deferred_extension --test tool_mapping_tests
rtk cargo test workspace_opened_is_rejected_as_deferred_extension --test tool_mapping_tests
rtk cargo test workspace_changes_is_rejected_as_deferred_extension --test tool_mapping_tests
```

Expected:

- All three tests pass because `src/tools/workspaces.rs` no longer has `where`, `opened`, or `changes` match arms after Task 1.

- [ ] **Step 4: Search for stale workspace extension action references**

Run:

```bash
rtk rg -n '"where"|"opened"|"changes"|workspace_where|workspace_opened|workspace_changes' src tests README.md docs/superpowers/plans
```

Expected:

- No active `src/` implementation references for `query_workspaces` `where`, `opened`, or `changes`.
- The existing historical implementation plan may still mention those strings. Do not edit historical plans unless they describe current behavior.
- The new rejection test names may appear in `tests/tool_mapping_tests.rs`.

- [ ] **Step 5: Commit the workspace extension removal**

Run:

```bash
rtk git add src/tools/workspaces.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: remove workspace query extensions"
```

Expected:

- A commit is created only if Task 1 and Task 2 were not already committed together.
- If `rtk git status --short` shows no staged changes because Task 1 already committed all code edits, do not create an empty commit.

---

### Task 3: Document Deferred Workspace Query Extensions

**Files:**
- Create: `docs/todos/workspace-query-extensions.md`
- Modify: `docs/superpowers/plans/2026-06-14-workspace-query-upstream-parity.md`

- [ ] **Step 1: Create the TODO directory if needed**

Run:

```bash
rtk mkdir -p docs/todos
```

Expected:

- `docs/todos/` exists.

- [ ] **Step 2: Add the deferred extension plan**

Create `docs/todos/workspace-query-extensions.md` with exactly this content:

```markdown
# Workspace Query Extensions

Status: deferred from PR #2 to keep the Rust port aligned with upstream `perforce/p4mcp-server`.

## Reason

The write approval gate is an approved Rust-port extension because it is a safety measure. The previous `query_workspaces` actions `opened`, `changes`, and `where` were functional extensions. They should not be shipped accidentally inside the upstream-parity porting PR.

## Future Design

### `opened`

- Keep the action read-only.
- If `workspace_name` is present, run `p4 opened -C <workspace>`.
- If `workspace_name` is absent, run `p4 opened` for the current client.
- Tests must cover both the current-client and explicit-workspace forms.

### `changes`

- Prefer guiding callers to `query_changelists` with `action: "list"` and `workspace_name` set. That tool already owns `p4 changes` query behavior.
- If `changes` remains in `query_workspaces`, require `workspace_name` for workspace-scoped behavior and run `p4 changes -c <workspace>`.
- Preserve `max_results` with `-m <n>` if this action is reintroduced.
- Tests must prove that `workspace_name` is never silently ignored.

### `where`

- Treat this as a current-client mapping query only.
- Require `file_path`.
- Do not support `workspace_name`; `p4 where` resolves through the active client context.
- If a future implementation receives both `where` and `workspace_name`, return an invalid-params error rather than pretending the named workspace is used.

## Reintroduction Checklist

- Add explicit action documentation before exposing the action.
- Add direct builder tests for every generated `p4` argument list.
- Add server-to-executor tests for every parameter that changes command scope.
- Verify the action does not duplicate an existing tool unless the user-facing workflow is clearer in `query_workspaces`.
- Reply to the relevant PR or issue explaining that this is a deliberate functional extension, not upstream-parity work.
```

- [ ] **Step 3: Verify the TODO content**

Run:

```bash
rtk sed -n '1,220p' docs/todos/workspace-query-extensions.md
```

Expected:

- The file contains the `opened`, `changes`, and `where` behavior listed in the human review decision.

- [ ] **Step 4: Commit the TODO and implementation plan**

Run:

```bash
rtk git add docs/todos/workspace-query-extensions.md docs/superpowers/plans/2026-06-14-workspace-query-upstream-parity.md
rtk git commit -m "docs: plan workspace query extensions"
```

Expected:

- A docs commit is created.

---

### Task 4: Verify, Push, and Reply to Review Threads

**Files:**
- No source file edits.
- GitHub PR review threads:
  - `PRRT_kwDOS5FH5M6JZnls`
  - `PRRT_kwDOS5FH5M6JZnlu`

- [ ] **Step 1: Run formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected:

- Passes with no output.

- [ ] **Step 2: Run focused tests**

Run:

```bash
rtk cargo test workspace_list_by_user_uses_user_filter --test tool_mapping_tests
rtk cargo test query_workspaces_list_by_user_calls_injected_executor --test mcp_smoke_tests
rtk cargo test workspace_where_is_rejected_as_deferred_extension --test tool_mapping_tests
rtk cargo test workspace_opened_is_rejected_as_deferred_extension --test tool_mapping_tests
rtk cargo test workspace_changes_is_rejected_as_deferred_extension --test tool_mapping_tests
```

Expected:

- All focused tests pass.

- [ ] **Step 3: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected:

- Passes with no warnings.

- [ ] **Step 4: Run the full test suite**

Run:

```bash
rtk cargo test
```

Expected:

- Passes.
- If WireMock or local port binding fails with a sandbox permission error, rerun this exact command with escalation because the suite needs local port binding.

- [ ] **Step 5: Inspect final diff and commit graph**

Run:

```bash
rtk git status --short --branch
rtk git log --oneline -5
```

Expected:

- Working tree is clean.
- Recent commits include:
  - `fix: honor workspace user filter`
  - `fix: remove workspace query extensions` if it was not folded into the first fix commit
  - `docs: plan workspace query extensions`

- [ ] **Step 6: Push the branch**

Run:

```bash
rtk git push
```

Expected:

- `codex/add-gitignore` pushes to `origin/codex/add-gitignore`.

- [ ] **Step 7: Fetch fresh unresolved review thread state**

Run:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected:

- `PRRT_kwDOS5FH5M6JZnls` and `PRRT_kwDOS5FH5M6JZnlu` are still present unless GitHub marked them outdated.
- The next step fetches the REST `databaseId` values needed for inline review replies.

- [ ] **Step 8: Fetch REST comment IDs for inline replies**

Run:

```bash
USER_FILTER_COMMENT_ID=$(rtk gh api graphql \
  -f owner='j3bit' \
  -f repo='p4mcp-server-rs' \
  -F number=2 \
  -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewThreads(first:100){nodes{id comments(first:1){nodes{databaseId}}}}}}}' \
  --jq '.data.repository.pullRequest.reviewThreads.nodes[] | select(.id == "PRRT_kwDOS5FH5M6JZnls") | .comments.nodes[0].databaseId')

EXTENSION_COMMENT_ID=$(rtk gh api graphql \
  -f owner='j3bit' \
  -f repo='p4mcp-server-rs' \
  -F number=2 \
  -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewThreads(first:100){nodes{id comments(first:1){nodes{databaseId}}}}}}}' \
  --jq '.data.repository.pullRequest.reviewThreads.nodes[] | select(.id == "PRRT_kwDOS5FH5M6JZnlu") | .comments.nodes[0].databaseId')

test -n "$USER_FILTER_COMMENT_ID"
test -n "$EXTENSION_COMMENT_ID"
```

Expected:

- Both `test -n` commands pass.

- [ ] **Step 9: Reply to the workspace user-filter thread**

Run:

```text
Fixed in the latest push.

`query_workspaces` now threads `params.user` into the workspace list builder, and `list` appends `-u <user>` to the `p4 clients` invocation. This matches upstream `perforce/p4mcp-server`, where `list_workspaces(user, limit)` calls `p4 clients -u <user> -m<limit>`. I added both direct builder coverage and a server-to-executor smoke test.
```

```bash
rtk gh api "repos/j3bit/p4mcp-server-rs/pulls/2/comments/${USER_FILTER_COMMENT_ID}/replies" -f body='Fixed in the latest push.

`query_workspaces` now threads `params.user` into the workspace list builder, and `list` appends `-u <user>` to the `p4 clients` invocation. This matches upstream `perforce/p4mcp-server`, where `list_workspaces(user, limit)` calls `p4 clients -u <user> -m<limit>`. I added both direct builder coverage and a server-to-executor smoke test.'
```

- [ ] **Step 10: Reply to the workspace extension thread**

Run:

```text
Handled in the latest push by removing the Rust-only workspace query extensions from this PR.

The review is correct that the previous `opened` and `changes` arms silently ignored `workspace_name`. Rather than patch those as functional extensions inside the upstream-parity port, this PR now removes `query_workspaces.opened`, `query_workspaces.changes`, and `query_workspaces.where`. That keeps the current PR closer to upstream `perforce/p4mcp-server`, where `query_workspaces` does not expose those actions.

I documented the intended future extension path in `docs/todos/workspace-query-extensions.md`: `opened` should use `p4 opened -C <workspace>` when a workspace is supplied, `changes` should preferably route callers to `query_changelists.list` or use `p4 changes -c <workspace>` if retained here, and `where` should remain a current-client mapping query with required `file_path` and no `workspace_name` support.
```

```bash
rtk gh api "repos/j3bit/p4mcp-server-rs/pulls/2/comments/${EXTENSION_COMMENT_ID}/replies" -f body='Handled in the latest push by removing the Rust-only workspace query extensions from this PR.

The review is correct that the previous `opened` and `changes` arms silently ignored `workspace_name`. Rather than patch those as functional extensions inside the upstream-parity port, this PR now removes `query_workspaces.opened`, `query_workspaces.changes`, and `query_workspaces.where`. That keeps the current PR closer to upstream `perforce/p4mcp-server`, where `query_workspaces` does not expose those actions.

I documented the intended future extension path in `docs/todos/workspace-query-extensions.md`: `opened` should use `p4 opened -C <workspace>` when a workspace is supplied, `changes` should preferably route callers to `query_changelists.list` or use `p4 changes -c <workspace>` if retained here, and `where` should remain a current-client mapping query with required `file_path` and no `workspace_name` support.'
```

- [ ] **Step 11: Resolve both review threads**

Run these GraphQL mutations, substituting the thread IDs exactly as shown:

```bash
rtk gh api graphql -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' -f thread='PRRT_kwDOS5FH5M6JZnls'
rtk gh api graphql -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}' -f thread='PRRT_kwDOS5FH5M6JZnlu'
```

Expected:

- Both responses include `"isResolved": true`.

- [ ] **Step 12: Confirm no unresolved review threads remain**

Run:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected:

- No actionable unresolved review threads remain.

---

## Self-Review

Spec coverage:

- Reviewer/user decision for issue 1 is covered by Task 1.
- Reviewer/user decision for issue 2 is covered by Task 2.
- Future extension documentation under `docs/todos/` is covered by Task 3.
- Verification, push, reply, and resolve workflow is covered by Task 4.

Placeholder scan:

- The GitHub reply steps use shell variables populated by explicit GraphQL commands. There are no unstated reply bodies, thread IDs, test names, or code snippets.

Type consistency:

- The final `build_workspace_query_invocation` signature is `(action: &str, workspace_name: Option<&str>, user: Option<&str>, max_results: u16)`.
- `src/server.rs` passes `params.user.as_deref()` as the third argument.
- All direct tests use the same signature.
