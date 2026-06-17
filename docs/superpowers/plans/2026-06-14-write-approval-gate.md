# Write Approval Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Require explicit approval before every `modify_*` tool executes a P4 or review HTTP write, while preserving default writable startup and `--readonly` as the deployment-level kill switch.

**Architecture:** Add one approval module that owns previews, canonical request digests, MCP elicitation, and one-time fallback tokens. Update each write handler so it checks `SafetyPolicy`, builds a side-effect-free preview, runs the approval gate, and only then calls the existing P4 executor or review client path. Remove all model-controlled `confirmation` inputs and use `approval_token` only for the fallback retry path.

**Tech Stack:** Rust 2024, `rmcp` 1.7 with the `elicitation` feature, `schemars`, `serde`, `tokio`, existing `P4Executor`, existing review request builders, `sha2` for request digests, and `getrandom` for fallback token generation.

---

## Assumptions

- Building a `P4Invocation`, Perforce form string, or review request object is side-effect-free and can happen before approval to produce the user preview.
- Any call to `P4Executor::run` or a review HTTP write client is a write boundary and must happen only after approval.
- A client without form elicitation support receives the soft fallback response; a client with form elicitation support receives `elicitation/create`.
- The fallback token is process-local, single-use, and valid for 300 seconds.
- `query_*` tools and readonly/toolset checks keep their current behavior.

## Target Files

- `Cargo.toml`
- `src/lib.rs`
- `src/error.rs`
- `src/approval.rs`
- `src/server.rs`
- `src/tools/params.rs`
- `src/tools/files.rs`
- `src/tools/reviews.rs`
- `src/tools/response.rs`
- `tests/tool_mapping_tests.rs`
- `tests/review_client_tests.rs`
- `tests/mcp_smoke_tests.rs`

## Design Details

### Approval Channel

Use a small enum so production tool handlers can pass an MCP peer and server unit tests can exercise fallback behavior without constructing a peer:

```rust
pub enum ApprovalChannel {
    Elicitation(Peer<RoleServer>),
    FallbackOnly,
}
```

`P4McpServer::modify_*` tool handlers take `peer: Peer<RoleServer>` as an additional context argument and call private `*_inner` methods with `ApprovalChannel::Elicitation(peer)`. Tests inside `src/server.rs` call the private inner methods with `ApprovalChannel::FallbackOnly` or an injected fake gate.

### Approval Gate Contract

Add an async trait to keep server tests deterministic:

```rust
#[async_trait]
pub trait WriteApprovalGate: Send + Sync {
    async fn approve(
        &self,
        channel: ApprovalChannel,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> Result<ApprovalDecision>;
}
```

The production implementation is `DefaultWriteApprovalGate`. Tests use a fake implementation returning `ApprovalDecision::Approved` or `ApprovalDecision::Response(ToolResponse)`.

### Approval Decision

```rust
pub enum ApprovalDecision {
    Approved,
    Response(ToolResponse),
}
```

`Response` covers `approval_required`, user-declined, user-cancelled, timeout, and invalid elicitation content. Those outcomes are successful MCP tool responses with no write execution. Invalid, expired, reused, or mismatched fallback tokens are parameter errors.

### Preview Shape

Use one serializable structure for P4 and review writes:

```rust
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
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
    pub request: Option<HttpPreview>,
}
```

`command` is the compact `p4` argument preview such as `["p4", "edit", "-c", "123", "//depot/a.rs"]`. `request` contains only HTTP method and path, never credentials, authorization headers, or full URLs.

### Fallback Response

Add:

```rust
impl ToolResponse {
    pub fn approval_required(action: impl Into<String>, message: Value) -> Self {
        Self {
            status: "approval_required".to_string(),
            action: action.into(),
            message,
        }
    }

    pub fn cancelled(action: impl Into<String>, message: Value) -> Self {
        Self {
            status: "cancelled".to_string(),
            action: action.into(),
            message,
        }
    }
}
```

The fallback message includes:

```json
{
  "preview": { },
  "digest": "sha256:<hex>",
  "approval_token": "<opaque-token>",
  "ttl_seconds": 300,
  "instruction": "Ask the user to approve this write, then retry the same tool call with approval_token before it expires."
}
```

### Digest Rules

Compute the digest from canonical JSON:

```json
{
  "tool": "modify_files",
  "action": "edit",
  "params": { "approval_token": null },
  "preview": { }
}
```

Before hashing, remove approval-only fields and credentials:

- `approval_token`
- `confirmation`
- `password`
- `ticket`
- `authorization`
- `Authorization`
- `P4PASSWD`
- `P4TICKETS`

Reject a fallback token when its stored digest differs from the digest of the retried request.

### Elicitation Prompt

Use `Peer<RoleServer>::elicit_with_timeout::<WriteApprovalChoice>(message, Some(Duration::from_secs(300)))`.

The choice type is:

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct WriteApprovalChoice {
    pub decision: WriteApprovalDecision,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WriteApprovalDecision {
    Proceed,
    Cancel,
}
```

Only `Accept` with `decision == Proceed` returns `Approved`. `Decline`, `Cancel`, timeout, missing content, parse errors, and `decision == Cancel` return a `cancelled` tool response. `CapabilityNotSupported` enters fallback token mode.

## Tasks

### 1. Add the Core Approval Module Tests

- [ ] Create `src/approval.rs` with type declarations behind failing tests first.
- [ ] Add unit tests in `src/approval.rs`:
  - [ ] `fallback_without_token_returns_approval_required`
  - [ ] `fallback_token_approves_same_digest_once`
  - [ ] `fallback_token_reuse_is_rejected`
  - [ ] `fallback_token_expires`
  - [ ] `fallback_token_rejects_changed_digest`
  - [ ] `digest_omits_approval_and_secret_fields`
- [ ] Add dependencies in `Cargo.toml`:

```toml
sha2 = "0.10"
getrandom = "0.3"
```

Run:

```bash
rtk cargo test approval::
```

Expected before implementation: tests fail to compile or fail at runtime because `DefaultWriteApprovalGate` and token behavior do not exist.

Implement:

- [ ] `ApprovalPreview`
- [ ] `HttpPreview`
- [ ] `ApprovalRequest`
- [ ] `ApprovalDecision`
- [ ] `ApprovalChannel`
- [ ] `WriteApprovalGate`
- [ ] `DefaultWriteApprovalGate`
- [ ] in-memory token store using `Mutex<HashMap<String, ApprovalTokenRecord>>`
- [ ] 300-second TTL
- [ ] one-time token consumption before returning `Approved`
- [ ] canonical JSON digest with recursive field redaction

Run:

```bash
rtk cargo test approval::
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add Cargo.toml Cargo.lock src/approval.rs src/lib.rs
rtk git commit -m "feat: add write approval gate core"
```

### 2. Replace `confirmation` Tool Inputs With `approval_token`

- [ ] In `tests/tool_mapping_tests.rs`, replace confirmation-specific tests with schema and builder tests:
  - [ ] `modify_file_params_schema_exposes_approval_token`
  - [ ] `modify_file_params_schema_omits_confirmation`
  - [ ] `modify_file_delete_builds_without_confirmation`
  - [ ] `modify_file_resolve_theirs_builds_without_confirmation`
  - [ ] `common_modify_params_schema_exposes_approval_token`
  - [ ] `common_modify_params_schema_omits_confirmation`
- [ ] In `tests/review_client_tests.rs`, replace `obliterate_review_requires_confirmation` with:
  - [ ] `review_request_schema_exposes_approval_token`
  - [ ] `review_request_schema_omits_confirmation`
  - [ ] `obliterate_review_builds_delete_without_body_confirmation`

Run:

```bash
rtk cargo test tool_mapping_tests review_client_tests
```

Expected before implementation: tests fail because schemas and structs still contain `confirmation`.

Implement:

- [ ] In `src/tools/params.rs`, remove `confirmation` from `ModifyFilesParams` and `CommonModifyParams`.
- [ ] Add `pub approval_token: Option<String>` to both structs with `#[serde(default)]`.
- [ ] Delete `ModifyFilesParams::requires_confirmation` and `ModifyFilesParams::confirmed`.
- [ ] In `src/tools/files.rs`, remove `params.confirmed()?` and `require_proceed_confirmation`.
- [ ] In `src/tools/reviews.rs`, add top-level `approval_token: Option<String>` to `ReviewRequest` with `#[serde(default)]`.
- [ ] In `src/tools/reviews.rs`, remove the `body.confirmation == "PROCEED"` obliterate check.
- [ ] In `src/error.rs`, remove `ConfirmationRequired`.
- [ ] Update all existing test builders to set `approval_token: None`.

Run:

```bash
rtk cargo test tool_mapping_tests review_client_tests
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add src/tools/params.rs src/tools/files.rs src/tools/reviews.rs src/error.rs tests/tool_mapping_tests.rs tests/review_client_tests.rs
rtk git commit -m "refactor: replace write confirmation fields with approval tokens"
```

### 3. Add Approval Responses and Server Injection

- [ ] Add tests in `src/server.rs` under `#[cfg(test)] mod tests` using a fake `WriteApprovalGate`:
  - [ ] `modify_files_without_approval_does_not_call_executor`
  - [ ] `modify_files_after_approval_calls_executor_once`
  - [ ] `readonly_blocks_before_approval_gate`
- [ ] Remove direct write method calls from `tests/mcp_smoke_tests.rs` and keep its query and parameter-validation coverage there.

Run:

```bash
rtk cargo test server::tests::modify_files_without_approval_does_not_call_executor server::tests::modify_files_after_approval_calls_executor_once server::tests::readonly_blocks_before_approval_gate
```

Expected before implementation: tests fail because `P4McpServer` has no approval gate injection and write handlers execute immediately.

Implement:

- [ ] Add `approval_gate: Arc<dyn WriteApprovalGate>` to `P4McpServer`.
- [ ] Update `P4McpServer::new` to use `DefaultWriteApprovalGate::new()`.
- [ ] Update `P4McpServer::with_executor` to use the default approval gate.
- [ ] Add `P4McpServer::with_executor_and_approval(config, executor, approval_gate)` for tests.
- [ ] Add `ToolResponse::approval_required`.
- [ ] Add `ToolResponse::cancelled`.
- [ ] Add a helper on `P4McpServer`:

```rust
async fn require_write_approval(
    &self,
    channel: ApprovalChannel,
    request: ApprovalRequest,
    approval_token: Option<&str>,
) -> McpResult<Option<Json<ToolResponse>>>
```

Return `Ok(Some(Json(response)))` when no write should run, and `Ok(None)` when the write is approved.

Run:

```bash
rtk cargo test server::tests::
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add src/server.rs src/tools/response.rs
rtk git commit -m "feat: inject write approval gate into server"
```

### 4. Gate Every P4 `modify_*` Handler

- [ ] Add server unit tests for every P4 write tool:
  - [ ] `modify_changelists_without_approval_does_not_call_executor`
  - [ ] `modify_shelves_without_approval_does_not_call_executor`
  - [ ] `modify_workspaces_without_approval_does_not_call_executor`
  - [ ] `modify_jobs_without_approval_does_not_call_executor`
  - [ ] `modify_streams_without_approval_does_not_call_executor`
- [ ] Add preview assertions for representative commands:
  - [ ] files edit preview includes `p4 edit`
  - [ ] changelist submit preview includes `p4 submit`
  - [ ] workspace delete preview includes `p4 client -d`
  - [ ] stream delete preview includes `p4 stream -d`

Run:

```bash
rtk cargo test server::tests::modify_
```

Expected before implementation: newly added tests fail for ungated handlers.

Implement:

- [ ] Change each P4 `modify_*` tool handler signature to accept `peer: Peer<RoleServer>`.
- [ ] Move handler bodies into private `*_inner(params, ApprovalChannel)` methods.
- [ ] Keep `SafetyPolicy` checks before approval.
- [ ] Build the side-effect-free invocation or form before approval.
- [ ] Create `ApprovalRequest` with tool name, action, sanitized params, and preview.
- [ ] Call `require_write_approval`.
- [ ] Execute the invocation only when approval returns `None`.
- [ ] Delete `require_confirmation` from `src/server.rs`.

Preview mapping:

- `modify_files`: use `P4Invocation.args` and `params.files`.
- `modify_changelists`: use changelist id, files, and `P4Invocation.args`.
- `modify_shelves`: use changelist id, files, and `P4Invocation.args`.
- `modify_workspaces`: use workspace name and `P4Invocation.args`.
- `modify_jobs`: use job id, files, and `P4Invocation.args`.
- `modify_streams`: use stream and `P4Invocation.args`.

Run:

```bash
rtk cargo test server::tests::modify_ tool_mapping_tests mcp_smoke_tests
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "feat: gate p4 modify tools behind approval"
```

### 5. Gate Review Writes

- [ ] Add review server tests:
  - [ ] `modify_reviews_without_approval_does_not_return_write_dry_run`
  - [ ] `modify_reviews_after_approval_returns_request_metadata`
  - [ ] `modify_reviews_approval_preview_uses_method_and_path`

Run:

```bash
rtk cargo test server::tests::modify_reviews review_client_tests
```

Expected before implementation: tests fail because review writes are not gated.

Implement:

- [ ] Change `modify_reviews` handler signature to accept `peer: Peer<RoleServer>`.
- [ ] Move review write logic into `modify_reviews_inner(params, ApprovalChannel)`.
- [ ] Keep `SafetyPolicy` before approval.
- [ ] Build `ReviewRequest::to_http`.
- [ ] Create `ApprovalRequest` with HTTP method and path preview.
- [ ] Call `require_write_approval`.
- [ ] Return existing dry-run request metadata only after approval.

Run:

```bash
rtk cargo test server::tests::modify_reviews review_client_tests
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add src/server.rs src/tools/reviews.rs tests/review_client_tests.rs
rtk git commit -m "feat: gate review modify tool behind approval"
```

### 6. Add Real MCP Elicitation

- [ ] Enable `rmcp` elicitation in `Cargo.toml`:

```toml
rmcp = { version = "1.7", features = ["server", "macros", "schemars", "elicitation", "transport-io", "transport-streamable-http-server"] }
```

- [ ] Add `WriteApprovalChoice` and `WriteApprovalDecision` in `src/approval.rs`.
- [ ] Add tests for elicitation result handling with a small pure function:
  - [ ] `accepted_proceed_approves`
  - [ ] `accepted_cancel_returns_cancelled`
  - [ ] `decline_returns_cancelled`
  - [ ] `cancel_returns_cancelled`
  - [ ] `timeout_returns_cancelled`
  - [ ] `capability_not_supported_returns_fallback_required`

Run:

```bash
rtk cargo test approval::tests::accepted_proceed_approves approval::tests::capability_not_supported_returns_fallback_required
```

Expected before implementation: tests fail because elicitation handling does not exist.

Implement:

- [ ] In `DefaultWriteApprovalGate::approve`, for `ApprovalChannel::Elicitation(peer)`, call `peer.elicit_with_timeout::<WriteApprovalChoice>`.
- [ ] Format the elicitation message with the same preview fields used in fallback.
- [ ] Treat only accepted `Proceed` as approved.
- [ ] Treat accepted `Cancel`, decline, cancel, timeout, no content, parse errors, and transport errors as non-executing `cancelled` responses.
- [ ] Treat only `CapabilityNotSupported` as the fallback token path.
- [ ] In `ApprovalChannel::FallbackOnly`, always use fallback token mode.

Run:

```bash
rtk cargo test approval::
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add Cargo.toml Cargo.lock src/approval.rs
rtk git commit -m "feat: request write approval through mcp elicitation"
```

### 7. Add Tool Annotations

- [ ] Add tests around the registered tool metadata:
  - [ ] every `query_*` tool has `read_only_hint = true`
  - [ ] every `modify_*` tool has `read_only_hint = false`
  - [ ] delete, revert, submit, shelve, stream, workspace, and review modification tools include destructive hints where the macro supports them at the tool level

Run:

```bash
rtk cargo test tool_metadata
```

Expected before implementation: tests fail because annotations are absent.

Implement:

- [ ] Update each `#[tool]` attribute in `src/server.rs` to include the relevant `annotations` block.
- [ ] Use rmcp macro syntax verified from `rmcp-macros`:

```rust
#[tool(description = "Query Perforce files", annotations(read_only_hint = true))]
#[tool(description = "Modify Perforce files", annotations(read_only_hint = false, destructive_hint = true))]
```

- [ ] Do not use annotations as a security check.

Run:

```bash
rtk cargo test tool_metadata
```

Expected after implementation: `test result: ok`.

Commit:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "docs: annotate mcp tool safety hints"
```

### 8. Full Regression and Local P4 Verification

- [ ] Run formatting:

```bash
rtk cargo fmt --check
```

Expected: no formatting diff.

- [ ] Run the full test suite:

```bash
rtk cargo test
```

Expected: `test result: ok`.

- [ ] Run linting:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] Run a smoke check against installed binaries:

```bash
rtk p4 -V
rtk p4d -V
```

Expected: both commands print version information.

- [ ] If the repository already has a local p4d integration fixture, add one write-approval integration test that starts from a non-executed fallback response, retries with the returned token, and verifies the P4 write happened exactly once. If no fixture exists, do not add a new p4d harness in this change.

Commit:

```bash
rtk git status --short
rtk git log --oneline -8
```

Expected: clean worktree after the final commit set.

### 9. PR Review Response

- [ ] After implementation and verification, reply to the PR review thread with:

```text
Resolved by keeping default writable startup for agentic workflows while moving write safety to per-call approval. The direct model-controlled confirmation field has been removed. Every modify_* path now checks SafetyPolicy first, then requires MCP elicitation approval or a same-request one-time fallback token before any P4 executor or review write path runs.
```

- [ ] Resolve the thread after the pushed branch includes the commits and CI is green.

## Final Verification Checklist

- [ ] `confirmation` does not appear in `src/` except in historical docs or PR discussion text.
- [ ] `approval_token` appears in every modify params schema.
- [ ] `--readonly` blocks before approval.
- [ ] Every `modify_*` path has a no-execution test for missing approval.
- [ ] Fallback tokens are one-time and digest-bound.
- [ ] Elicitation unsupported clients receive `approval_required`.
- [ ] Elicitation decline, cancel, timeout, parse error, and transport failure do not execute writes.
- [ ] Query tools remain unchanged.
- [ ] `rtk cargo fmt --check` passes.
- [ ] `rtk cargo test` passes.
- [ ] `rtk cargo clippy --all-targets -- -D warnings` passes.
