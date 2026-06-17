# Upstream Parity Modify Review Contracts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the remaining common write/review request surfaces with upstream-shaped tool contracts and fix CLI form update semantics so the Rust port follows `perforce/p4mcp-server` public behavior.

**Architecture:** The Rust port keeps direct `p4` CLI execution and the approved write approval gate, but public MCP schemas must be tool-specific and upstream-shaped. Create explicit params for every modify tool, split review query/modify requests, and route update operations through approval first, then fetch-patch-save form execution. Keep small command builders for atomic P4 commands and use server-level helpers for multi-step fetch/patch/write workflows.

**Tech Stack:** Rust 2024, `rmcp`, `serde`, `schemars`, `tokio`, direct `p4` CLI, existing `P4Executor` abstraction, existing WireMock review tests.

---

## Scope And Baseline

Baseline upstream: `perforce/p4mcp-server` `v2026.2.2955897`, commit `a64efb07511b2a62db41aeed110ab96744c4076a`. The repository README already pins this baseline, and current upstream `main` resolves to the same SHA.

Approved Rust deltas that must remain:

- The write approval gate is a safety extension. Keep `approval_token` on modify params and do not reintroduce `confirmation`.
- `query_server` remains always registered, matching previous upstream-parity decision.
- `modify_files.sync` must keep requiring `file_paths`; upstream handler also requires `file_paths` for sync even though the upstream Pydantic model text is looser.

Root fix criteria:

```bash
rtk rg "Parameters<CommonModifyParams>|Parameters<ReviewRequest>" src/server.rs
```

Expected final output: no matches.

## File Structure

- `src/tools/params.rs`
  - Keep query params.
  - Add upstream-shaped `ModifyChangelistsParams`, `ModifyShelvesParams`, `ModifyWorkspacesParams`, `ModifyJobsParams`, and `ModifyStreamsParams`.
  - Remove `CommonModifyParams` after all call sites are migrated.

- `src/tools/server.rs`
  - Add `QueryServerParams { action: ServerQueryAction }` so `query_server` receives an object-shaped params schema like upstream.

- `src/tools/reviews.rs`
  - Replace `ReviewRequest` with `QueryReviewsParams` and `ModifyReviewsParams`.
  - Add `ReviewQueryAction`, `ReviewModifyAction`, and small payload helpers.
  - Keep `ReviewApiConfig`, `BuiltReviewRequest`, and `ReviewHttpClient`, but make client execution generic over built requests instead of a mixed request type.

- `src/p4/forms.rs`
  - Add deterministic text-form patch helpers for changelists, clients, and streams.
  - Preserve existing `change_form()`/`change_form_for()` for create and tests that still need synthetic create forms.

- `src/tools/changelists.rs`, `src/tools/shelves.rs`, `src/tools/jobs.rs`, `src/tools/streams.rs`
  - Update atomic command builders to accept tool-specific params and upstream action names.
  - Keep multi-step fetch-patch-save execution in `src/server.rs` because it needs the executor and approval gate.

- `src/server.rs`
  - Public routes switch away from `CommonModifyParams` and `ReviewRequest`.
  - Approval preview remains side-effect-free.
  - Fetch/patch/write starts only after approval succeeds.

- `tests/schema_contract_tests.rs`
  - New integration tests for public schema contracts.

- `tests/tool_mapping_tests.rs`, `tests/review_client_tests.rs`, `tests/mcp_smoke_tests.rs`
  - Update imports and focused mapping/execution tests.

## Reference Upstream Files

- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/changelist_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/shelve_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/workspace_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/job_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/stream_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/tools/review_tools.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/changelist_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/workspace_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/job_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/stream_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/review_services.py`

If `/private/tmp/p4mcp-upstream-a64` does not exist in the execution session, run:

```bash
rtk git clone https://github.com/perforce/p4mcp-server.git /private/tmp/p4mcp-upstream-a64
rtk git -C /private/tmp/p4mcp-upstream-a64 checkout a64efb07511b2a62db41aeed110ab96744c4076a
```

Expected: checkout at `a64efb07511b2a62db41aeed110ab96744c4076a`.

---

### Task 1: Server Params Object Shape

**Files:**
- Modify: `src/tools/server.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Test: `tests/schema_contract_tests.rs`

- [ ] **Step 1: Create a focused schema contract test file**

Create `tests/schema_contract_tests.rs` with this content:

```rust
use std::collections::BTreeSet;

use p4mcp_server_rs::tools::server::QueryServerParams;
use schemars::JsonSchema;

fn schema_properties<T: JsonSchema>() -> BTreeSet<String> {
    let schema = schemars::schema_for!(T);
    let schema = serde_json::to_value(schema).unwrap();
    schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn query_server_schema_matches_upstream_params_object() {
    let props = schema_properties::<QueryServerParams>();
    assert!(props.contains("action"));
    assert_eq!(props.len(), 1);
}
```

- [ ] **Step 2: Run the failing server schema test**

Run:

```bash
rtk cargo test query_server_schema_matches_upstream_params_object --test schema_contract_tests
```

Expected: FAIL because `QueryServerParams` does not exist.

- [ ] **Step 3: Add `QueryServerParams` and update the builder**

Modify `src/tools/server.rs` to this complete content:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::p4::runner::{OutputMode, P4Invocation};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerQueryAction {
    ServerInfo,
    CurrentUser,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryServerParams {
    pub action: ServerQueryAction,
}

pub fn build_server_invocation(params: &QueryServerParams) -> P4Invocation {
    let args = match params.action {
        ServerQueryAction::ServerInfo => vec!["info".to_string()],
        ServerQueryAction::CurrentUser => vec!["user".to_string(), "-o".to_string()],
    };
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}
```

- [ ] **Step 4: Update server route imports and handler**

In `src/server.rs`, change the server tool import:

```rust
server::{QueryServerParams, build_server_invocation},
```

Then replace `query_server` with:

```rust
#[tool(description = "Query Perforce server metadata")]
pub async fn query_server(
    &self,
    Parameters(params): Parameters<QueryServerParams>,
) -> McpResult<Json<ToolResponse>> {
    let invocation = build_server_invocation(&params);
    let output = self.run_p4(invocation).await?;
    Ok(Json(ToolResponse::success(
        "query_server",
        Value::Array(output.records),
    )))
}
```

- [ ] **Step 5: Update existing server mapping tests**

In `tests/tool_mapping_tests.rs`, change the server import to:

```rust
server::{QueryServerParams, ServerQueryAction, build_server_invocation},
```

Replace the two server tests with:

```rust
#[test]
fn query_server_info_maps_to_info() {
    let invocation = build_server_invocation(&QueryServerParams {
        action: ServerQueryAction::ServerInfo,
    });
    assert_eq!(invocation.args, vec!["info"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn query_current_user_maps_to_user_output() {
    let invocation = build_server_invocation(&QueryServerParams {
        action: ServerQueryAction::CurrentUser,
    });
    assert_eq!(invocation.args, vec!["user", "-o"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

In `tests/mcp_smoke_tests.rs`, update direct calls from:

```rust
.query_server(Parameters(ServerQueryAction::ServerInfo))
```

to:

```rust
.query_server(Parameters(QueryServerParams {
    action: ServerQueryAction::ServerInfo,
}))
```

and update the import to include `QueryServerParams`.

- [ ] **Step 6: Run focused tests**

Run:

```bash
rtk cargo test query_server_schema_matches_upstream_params_object --test schema_contract_tests
rtk cargo test query_server_info_maps_to_info --test tool_mapping_tests
rtk cargo test query_server_calls_injected_executor --test mcp_smoke_tests
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
rtk git add src/tools/server.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs tests/schema_contract_tests.rs
rtk git commit -m "fix: align server query params"
```

---

### Task 2: Tool-Specific Modify Params

**Files:**
- Modify: `src/tools/params.rs`
- Modify: `tests/schema_contract_tests.rs`

- [ ] **Step 1: Add upstream modify schema tests**

Append this code to `tests/schema_contract_tests.rs`:

```rust
use p4mcp_server_rs::tools::params::{
    ChangelistModifyAction, JobModifyAction, ModifyChangelistsParams, ModifyJobsParams,
    ModifyShelvesParams, ModifyStreamsParams, ModifyWorkspacesParams, ShelfModifyAction,
    StreamModifyAction, WorkspaceModifyAction,
};

fn assert_has<T: JsonSchema>(field: &str) {
    let props = schema_properties::<T>();
    assert!(props.contains(field), "missing schema field {field}");
}

fn assert_omits<T: JsonSchema>(field: &str) {
    let props = schema_properties::<T>();
    assert!(!props.contains(field), "unexpected schema field {field}");
}

#[test]
fn modify_changelists_schema_matches_upstream_fields() {
    for field in ["action", "changelist_id", "description", "file_paths", "approval_token"] {
        assert_has::<ModifyChangelistsParams>(field);
    }
    assert_omits::<ModifyChangelistsParams>("files");
    assert_omits::<ModifyChangelistsParams>("form");
    assert_omits::<ModifyChangelistsParams>("confirmation");

    let params: ModifyChangelistsParams = serde_json::from_value(serde_json::json!({
        "action": "move_files",
        "changelist_id": "12345",
        "file_paths": ["//depot/main/file.txt"]
    }))
    .unwrap();
    assert_eq!(params.action, ChangelistModifyAction::MoveFiles);
}

#[test]
fn modify_shelves_schema_matches_upstream_fields() {
    for field in [
        "action",
        "changelist_id",
        "file_paths",
        "target_changelist",
        "force",
        "approval_token",
    ] {
        assert_has::<ModifyShelvesParams>(field);
    }
    assert_omits::<ModifyShelvesParams>("files");
    assert_omits::<ModifyShelvesParams>("form");
    assert_omits::<ModifyShelvesParams>("confirmation");

    let params: ModifyShelvesParams = serde_json::from_value(serde_json::json!({
        "action": "unshelve_to_changelist",
        "changelist_id": "12345",
        "target_changelist": "54321",
        "force": true
    }))
    .unwrap();
    assert_eq!(params.action, ShelfModifyAction::UnshelveToChangelist);
    assert_eq!(params.target_changelist, "54321");
}

#[test]
fn modify_workspaces_schema_matches_upstream_fields() {
    for field in [
        "action",
        "workspace_name",
        "workspace_root",
        "workspace_description",
        "workspace_options",
        "workspace_line_end",
        "workspace_view",
        "approval_token",
    ] {
        assert_has::<ModifyWorkspacesParams>(field);
    }
    assert_omits::<ModifyWorkspacesParams>("form");
    assert_omits::<ModifyWorkspacesParams>("confirmation");

    let params: ModifyWorkspacesParams = serde_json::from_value(serde_json::json!({
        "action": "switch",
        "workspace_name": "ws-main"
    }))
    .unwrap();
    assert_eq!(params.action, WorkspaceModifyAction::Switch);
}

#[test]
fn modify_jobs_schema_matches_upstream_fields() {
    for field in ["action", "changelist_id", "job_id", "approval_token"] {
        assert_has::<ModifyJobsParams>(field);
    }
    assert_omits::<ModifyJobsParams>("files");
    assert_omits::<ModifyJobsParams>("form");
    assert_omits::<ModifyJobsParams>("confirmation");

    let params: ModifyJobsParams = serde_json::from_value(serde_json::json!({
        "action": "link_job",
        "changelist_id": "12345",
        "job_id": "job000123"
    }))
    .unwrap();
    assert_eq!(params.action, JobModifyAction::LinkJob);
}

#[test]
fn modify_streams_schema_matches_upstream_fields() {
    for field in [
        "action",
        "stream_name",
        "stream_type",
        "parent",
        "name",
        "description",
        "options",
        "parent_view",
        "paths",
        "remapped",
        "ignored",
        "changelist",
        "resolve_mode",
        "target_changelist",
        "parent_stream",
        "branch",
        "file_paths",
        "preview",
        "force",
        "reverse",
        "quiet",
        "max_files",
        "output_base",
        "virtual",
        "schedule_branch_resolve",
        "integrate_around_deleted",
        "skip_cherry_picked",
        "source_path",
        "target_path",
        "workspace",
        "workspace_name",
        "root",
        "host",
        "alt_roots",
        "approval_token",
    ] {
        assert_has::<ModifyStreamsParams>(field);
    }
    assert_omits::<ModifyStreamsParams>("stream");
    assert_omits::<ModifyStreamsParams>("form");
    assert_omits::<ModifyStreamsParams>("confirmation");

    let params: ModifyStreamsParams = serde_json::from_value(serde_json::json!({
        "action": "create_workspace",
        "stream_name": "//depot/main",
        "workspace_name": "ws-stream",
        "root": "/tmp/ws-stream"
    }))
    .unwrap();
    assert_eq!(params.action, StreamModifyAction::CreateWorkspace);
}
```

- [ ] **Step 2: Run the failing schema tests**

Run:

```bash
rtk cargo test modify_changelists_schema_matches_upstream_fields --test schema_contract_tests
```

Expected: FAIL because the modify params do not exist yet.

- [ ] **Step 3: Add modify enums and params**

In `src/tools/params.rs`, insert this block immediately before `fn default_true()` and keep existing query structs unchanged:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangelistModifyAction {
    Create,
    Update,
    Submit,
    Delete,
    MoveFiles,
}

impl ChangelistModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Submit => "submit",
            Self::Delete => "delete",
            Self::MoveFiles => "move_files",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyChangelistsParams {
    pub action: ChangelistModifyAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShelfModifyAction {
    Shelve,
    Unshelve,
    Update,
    Delete,
    UnshelveToChangelist,
}

impl ShelfModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shelve => "shelve",
            Self::Unshelve => "unshelve",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::UnshelveToChangelist => "unshelve_to_changelist",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyShelvesParams {
    pub action: ShelfModifyAction,
    pub changelist_id: String,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default = "default_changelist")]
    pub target_changelist: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceModifyAction {
    Create,
    Delete,
    Update,
    Switch,
}

impl WorkspaceModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Update => "update",
            Self::Switch => "switch",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyWorkspacesParams {
    pub action: WorkspaceModifyAction,
    pub workspace_name: String,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub workspace_description: Option<String>,
    #[serde(default = "default_workspace_options")]
    pub workspace_options: Option<String>,
    #[serde(default = "default_workspace_line_end")]
    pub workspace_line_end: Option<String>,
    #[serde(default)]
    pub workspace_view: Option<Vec<String>>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobModifyAction {
    LinkJob,
    UnlinkJob,
}

impl JobModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LinkJob => "link_job",
            Self::UnlinkJob => "unlink_job",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyJobsParams {
    pub action: JobModifyAction,
    pub changelist_id: String,
    pub job_id: String,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamModifyAction {
    Create,
    Update,
    Delete,
    EditSpec,
    ResolveSpec,
    RevertSpec,
    ShelveSpec,
    UnshelveSpec,
    Copy,
    Merge,
    Integrate,
    Populate,
    Switch,
    CreateWorkspace,
}

impl StreamModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::EditSpec => "edit_spec",
            Self::ResolveSpec => "resolve_spec",
            Self::RevertSpec => "revert_spec",
            Self::ShelveSpec => "shelve_spec",
            Self::UnshelveSpec => "unshelve_spec",
            Self::Copy => "copy",
            Self::Merge => "merge",
            Self::Integrate => "integrate",
            Self::Populate => "populate",
            Self::Switch => "switch",
            Self::CreateWorkspace => "create_workspace",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyStreamsParams {
    pub action: StreamModifyAction,
    #[serde(default)]
    pub stream_name: Option<String>,
    #[serde(default)]
    pub stream_type: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub options: Option<String>,
    #[serde(default)]
    pub parent_view: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub remapped: Option<Vec<String>>,
    #[serde(default)]
    pub ignored: Option<Vec<String>>,
    #[serde(default)]
    pub changelist: Option<String>,
    #[serde(default)]
    pub resolve_mode: Option<String>,
    #[serde(default)]
    pub target_changelist: Option<String>,
    #[serde(default)]
    pub parent_stream: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub max_files: Option<u16>,
    #[serde(default)]
    pub output_base: bool,
    #[serde(default, rename = "virtual")]
    pub virtual_stream: bool,
    #[serde(default)]
    pub schedule_branch_resolve: bool,
    #[serde(default)]
    pub integrate_around_deleted: bool,
    #[serde(default)]
    pub skip_cherry_picked: bool,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub alt_roots: Option<Vec<String>>,
    #[serde(default)]
    pub approval_token: Option<String>,
}
```

Add these default helpers near the existing defaults:

```rust
fn default_workspace_options() -> Option<String> {
    Some("noallwrite noclobber nocompress unlocked nomodtime normdir".to_string())
}

fn default_workspace_line_end() -> Option<String> {
    Some("local".to_string())
}
```

- [ ] **Step 4: Remove only the old schema tests for `CommonModifyParams`**

In `tests/tool_mapping_tests.rs`, delete:

```rust
#[test]
fn common_modify_params_schema_exposes_approval_token() {
    assert!(schema_has_property::<CommonModifyParams>("approval_token"));
}

#[test]
fn common_modify_params_schema_omits_confirmation() {
    assert!(!schema_has_property::<CommonModifyParams>("confirmation"));
}
```

Also remove `CommonModifyParams` from the import list in that file.

- [ ] **Step 5: Run schema tests**

Run:

```bash
rtk cargo test --test schema_contract_tests
rtk cargo test --test tool_mapping_tests
```

Expected: schema contract tests PASS. Existing mapping tests may fail only where they still import or construct `CommonModifyParams`; fix those compile errors in later route tasks if they are not touched here.

- [ ] **Step 6: Commit**

```bash
rtk git add src/tools/params.rs tests/schema_contract_tests.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: add upstream modify params"
```

---

### Task 3: Changelists And Jobs Write Routes

**Files:**
- Modify: `src/tools/changelists.rs`
- Modify: `src/tools/jobs.rs`
- Modify: `src/p4/forms.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add mapping tests for changelist and job modify behavior**

Append these tests to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::{
    changelists::build_changelist_modify_invocation,
    jobs::build_job_modify_invocation,
    params::{
        ChangelistModifyAction, JobModifyAction, ModifyChangelistsParams, ModifyJobsParams,
    },
};

#[test]
fn modify_changelists_move_files_uses_file_paths() {
    let params = ModifyChangelistsParams {
        action: ChangelistModifyAction::MoveFiles,
        changelist_id: Some("12345".to_string()),
        description: None,
        file_paths: Some(vec!["//depot/main/a.txt".to_string()]),
        approval_token: None,
    };

    let invocation = build_changelist_modify_invocation(&params, None).unwrap();

    assert_eq!(
        invocation.args,
        vec!["reopen", "-c", "12345", "//depot/main/a.txt"]
    );
    assert_eq!(invocation.stdin, None);
}

#[test]
fn modify_jobs_link_job_uses_upstream_action_and_job_id() {
    let params = ModifyJobsParams {
        action: JobModifyAction::LinkJob,
        changelist_id: "12345".to_string(),
        job_id: "job000123".to_string(),
        approval_token: None,
    };

    let invocation = build_job_modify_invocation(&params).unwrap();

    assert_eq!(invocation.args, vec!["fix", "-c", "12345", "job000123"]);
}

#[test]
fn modify_jobs_unlink_job_uses_upstream_action_and_job_id() {
    let params = ModifyJobsParams {
        action: JobModifyAction::UnlinkJob,
        changelist_id: "12345".to_string(),
        job_id: "job000123".to_string(),
        approval_token: None,
    };

    let invocation = build_job_modify_invocation(&params).unwrap();

    assert_eq!(
        invocation.args,
        vec!["fix", "-d", "-c", "12345", "job000123"]
    );
}
```

- [ ] **Step 2: Add a form patch test**

Append this test to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::p4::forms::patch_change_description_form;

#[test]
fn patch_change_description_preserves_existing_files() {
    let existing = "Change: 12345\n\nDescription:\n\told text\n\nFiles:\n\t//depot/main/a.txt#1 edit\n\t//depot/main/b.txt#1 edit\n";

    let patched = patch_change_description_form(existing, "new text").unwrap();

    assert!(patched.contains("Description:\n\tnew text\n\nFiles:"));
    assert!(patched.contains("\t//depot/main/a.txt#1 edit"));
    assert!(patched.contains("\t//depot/main/b.txt#1 edit"));
}
```

- [ ] **Step 3: Run failing tests**

Run:

```bash
rtk cargo test modify_changelists_move_files_uses_file_paths --test tool_mapping_tests
rtk cargo test patch_change_description_preserves_existing_files --test tool_mapping_tests
rtk cargo test modify_jobs_link_job_uses_upstream_action_and_job_id --test tool_mapping_tests
```

Expected: FAIL because builders and patch helper are not implemented.

- [ ] **Step 4: Add form patch helper**

In `src/p4/forms.rs`, append:

```rust
use crate::error::{P4McpError, Result};

pub fn patch_change_description_form(existing: &str, description: &str) -> Result<String> {
    replace_indented_block(existing, "Description", &indent_lines(description))
}

fn indent_lines(value: &str) -> String {
    let mut output = String::new();
    for line in value.lines() {
        output.push('\t');
        output.push_str(line);
        output.push('\n');
    }
    if output.is_empty() {
        output.push('\t');
        output.push('\n');
    }
    output
}

fn replace_indented_block(existing: &str, field: &str, replacement: &str) -> Result<String> {
    let marker = format!("{field}:\n");
    let Some(start) = existing.find(&marker) else {
        return Err(P4McpError::InvalidInput {
            message: format!("{field} field not found in p4 form"),
        });
    };
    let value_start = start + marker.len();
    let remainder = &existing[value_start..];
    let next_field = remainder
        .find("\n\n")
        .map(|offset| value_start + offset)
        .unwrap_or(existing.len());

    let mut patched = String::new();
    patched.push_str(&existing[..value_start]);
    patched.push_str(replacement);
    patched.push_str(&existing[next_field..]);
    Ok(patched)
}
```

- [ ] **Step 5: Replace changelist modify builder**

Replace `build_changelist_modify_invocation` in `src/tools/changelists.rs` with:

```rust
pub fn build_changelist_modify_invocation(
    params: &ModifyChangelistsParams,
    stdin: Option<String>,
) -> Result<P4Invocation> {
    let (args, stdin) = match params.action {
        ChangelistModifyAction::Create => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "create")?),
        ),
        ChangelistModifyAction::Update => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "update")?),
        ),
        ChangelistModifyAction::Submit => (
            vec![
                "submit".into(),
                "-c".into(),
                required(params.changelist_id.as_deref(), "changelist_id")?,
            ],
            None,
        ),
        ChangelistModifyAction::Delete => (
            vec![
                "change".into(),
                "-d".into(),
                required(params.changelist_id.as_deref(), "changelist_id")?,
            ],
            None,
        ),
        ChangelistModifyAction::MoveFiles => {
            let change = required(params.changelist_id.as_deref(), "changelist_id")?;
            let files = required_files(params.file_paths.as_deref(), "move_files")?;
            let mut args = vec!["reopen".into(), "-c".into(), change];
            args.extend(files);
            (args, None)
        }
    };
    Ok(P4Invocation {
        args,
        stdin,
        mode: OutputMode::JsonLines,
    })
}

fn required_files(files: Option<&[String]>, action: &str) -> Result<Vec<String>> {
    match files {
        Some(files) if !files.is_empty() => Ok(files.to_vec()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("file_paths is required for {action}"),
        }),
    }
}
```

Update the top import in `src/tools/changelists.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{ChangelistModifyAction, ModifyChangelistsParams},
};
```

- [ ] **Step 6: Add job modify builder**

In `src/tools/jobs.rs`, add this import:

```rust
use crate::tools::params::{JobModifyAction, ModifyJobsParams};
```

Append:

```rust
pub fn build_job_modify_invocation(params: &ModifyJobsParams) -> Result<P4Invocation> {
    let args = match params.action {
        JobModifyAction::LinkJob => vec![
            "fix".to_string(),
            "-c".to_string(),
            params.changelist_id.clone(),
            params.job_id.clone(),
        ],
        JobModifyAction::UnlinkJob => vec![
            "fix".to_string(),
            "-d".to_string(),
            "-c".to_string(),
            params.changelist_id.clone(),
            params.job_id.clone(),
        ],
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}
```

- [ ] **Step 7: Migrate changelist and job server internals**

In `src/server.rs`, update imports to include:

```rust
forms::{change_form, patch_change_description_form},
```

and params:

```rust
ModifyChangelistsParams, ModifyJobsParams,
```

Replace `modify_changelists_inner` with:

```rust
async fn modify_changelists_inner(
    &self,
    params: ModifyChangelistsParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Changelists, "modify_changelists")
        .map_err(to_mcp_error)?;
    let action = params.action.as_str();
    let changelist_id = match params.action {
        ChangelistModifyAction::Create => "new".to_string(),
        ChangelistModifyAction::Update
        | ChangelistModifyAction::Submit
        | ChangelistModifyAction::Delete
        | ChangelistModifyAction::MoveFiles => require_non_blank(
            params.changelist_id.as_deref(),
            "changelist_id",
        )
        .map_err(to_mcp_error)?,
    };
    let preview_invocation = match params.action {
        ChangelistModifyAction::Create => build_changelist_modify_invocation(
            &params,
            Some(change_form(
                params.description.as_deref().unwrap_or_default(),
                &[],
            )),
        ),
        ChangelistModifyAction::Update => Ok(json_invocation(
            vec!["change".to_string(), "-i".to_string()],
            Some(format!("Change: {changelist_id}\n\nDescription:\n\t<patched after approval>\n")),
        )),
        _ => build_changelist_modify_invocation(&params, None),
    }
    .map_err(to_mcp_error)?;
    let request = self.modify_changelists_approval_request(&params, &changelist_id, &preview_invocation);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }

    let invocation = match params.action {
        ChangelistModifyAction::Create => build_changelist_modify_invocation(
            &params,
            Some(change_form(
                params.description.as_deref().unwrap_or_default(),
                &[],
            )),
        ),
        ChangelistModifyAction::Update => {
            let current = self
                .run_p4(text_invocation(vec!["change".to_string(), "-o".to_string(), changelist_id.clone()]))
                .await?;
            let current_form = current
                .text
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let patched = patch_change_description_form(
                current_form,
                params.description.as_deref().unwrap_or_default(),
            )
            .map_err(to_mcp_error)?;
            build_changelist_modify_invocation(&params, Some(patched))
        }
        _ => build_changelist_modify_invocation(&params, None),
    }
    .map_err(to_mcp_error)?;
    self.call_p4_tool(action, invocation).await
}
```

Add this helper near `modify_files_approval_request`:

```rust
fn modify_changelists_approval_request(
    &self,
    params: &ModifyChangelistsParams,
    changelist_id: &str,
    invocation: &P4Invocation,
) -> ApprovalRequest {
    let mut approval_params = params.clone();
    approval_params.approval_token = None;
    let files = params.file_paths.clone().unwrap_or_default();
    ApprovalRequest {
        tool: "modify_changelists".to_string(),
        action: params.action.as_str().to_string(),
        params: serde_json::to_value(approval_params)
            .expect("modify changelists params serialize to JSON"),
        preview: self.p4_approval_preview(P4ApprovalContext {
            tool: "modify_changelists",
            action: params.action.as_str(),
            targets: changelist_modify_targets(params.action.as_str(), changelist_id, &files),
            changelist: Some(changelist_id.to_string()),
            workspace: None,
            stream: None,
            invocation,
        }),
    }
}
```

Replace `modify_jobs_inner` with:

```rust
async fn modify_jobs_inner(
    &self,
    params: ModifyJobsParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Jobs, "modify_jobs")
        .map_err(to_mcp_error)?;
    let invocation = build_job_modify_invocation(&params).map_err(to_mcp_error)?;
    let request = self.modify_jobs_approval_request(&params, &invocation);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }
    self.call_p4_tool(params.action.as_str(), invocation).await
}
```

Add:

```rust
fn modify_jobs_approval_request(
    &self,
    params: &ModifyJobsParams,
    invocation: &P4Invocation,
) -> ApprovalRequest {
    let mut approval_params = params.clone();
    approval_params.approval_token = None;
    ApprovalRequest {
        tool: "modify_jobs".to_string(),
        action: params.action.as_str().to_string(),
        params: serde_json::to_value(approval_params).expect("modify jobs params serialize to JSON"),
        preview: self.p4_approval_preview(P4ApprovalContext {
            tool: "modify_jobs",
            action: params.action.as_str(),
            targets: vec![format!("job:{}", params.job_id)],
            changelist: Some(params.changelist_id.clone()),
            workspace: None,
            stream: None,
            invocation,
        }),
    }
}
```

Update public routes:

```rust
Parameters(params): Parameters<ModifyChangelistsParams>,
```

and:

```rust
Parameters(params): Parameters<ModifyJobsParams>,
```

- [ ] **Step 8: Run focused tests**

Run:

```bash
rtk cargo test modify_changelists_move_files_uses_file_paths --test tool_mapping_tests
rtk cargo test patch_change_description_preserves_existing_files --test tool_mapping_tests
rtk cargo test modify_jobs_link_job_uses_upstream_action_and_job_id --test tool_mapping_tests
rtk cargo test modify_jobs_unlink_job_uses_upstream_action_and_job_id --test tool_mapping_tests
rtk cargo test modify_changelists_without_approval_does_not_call_executor
rtk cargo test modify_jobs_without_approval_does_not_call_executor
```

Expected: all PASS. Update existing tests to construct `ModifyChangelistsParams` or `ModifyJobsParams` instead of `CommonModifyParams`.

- [ ] **Step 9: Commit**

```bash
rtk git add src/tools/changelists.rs src/tools/jobs.rs src/p4/forms.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align changelist and job write contracts"
```

---

### Task 4: Shelves And Workspaces Write Routes

**Files:**
- Modify: `src/tools/shelves.rs`
- Modify: `src/tools/workspaces.rs`
- Modify: `src/p4/forms.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add shelf and workspace mapping tests**

Append to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::{
    shelves::build_shelf_modify_invocation,
    workspaces::build_workspace_delete_invocation,
};

#[test]
fn modify_shelves_unshelve_to_changelist_uses_target_changelist() {
    let params = ModifyShelvesParams {
        action: ShelfModifyAction::UnshelveToChangelist,
        changelist_id: "12345".to_string(),
        file_paths: None,
        target_changelist: "54321".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_shelf_modify_invocation(&params).unwrap();

    assert_eq!(invocation.args, vec!["unshelve", "-s", "12345", "-c", "54321"]);
}

#[test]
fn modify_shelves_force_shelve_uses_file_paths() {
    let params = ModifyShelvesParams {
        action: ShelfModifyAction::Shelve,
        changelist_id: "12345".to_string(),
        file_paths: Some(vec!["//depot/main/a.txt".to_string()]),
        target_changelist: "default".to_string(),
        force: true,
        approval_token: None,
    };

    let invocation = build_shelf_modify_invocation(&params).unwrap();

    assert_eq!(
        invocation.args,
        vec!["shelve", "-f", "-c", "12345", "//depot/main/a.txt"]
    );
}

#[test]
fn modify_workspaces_delete_maps_to_client_delete() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Delete,
        workspace_name: "ws-main".to_string(),
        workspace_root: None,
        workspace_description: None,
        workspace_options: Some("noallwrite noclobber nocompress unlocked nomodtime normdir".to_string()),
        workspace_line_end: Some("local".to_string()),
        workspace_view: None,
        approval_token: None,
    };

    let invocation = build_workspace_delete_invocation(&params).unwrap();

    assert_eq!(invocation.args, vec!["client", "-d", "ws-main"]);
}
```

- [ ] **Step 2: Add client form patch test**

Append:

```rust
use p4mcp_server_rs::p4::forms::{WorkspaceFormPatch, patch_workspace_form};

#[test]
fn patch_workspace_form_preserves_view_when_view_is_not_supplied() {
    let existing = "Client: ws-main\n\nRoot: /old/root\n\nOptions: noallwrite noclobber nocompress unlocked nomodtime normdir\n\nLineEnd: local\n\nView:\n\t//depot/... //ws-main/...\n";
    let patch = WorkspaceFormPatch {
        root: Some("/new/root".to_string()),
        description: Some("New workspace".to_string()),
        options: None,
        line_end: None,
        view: None,
    };

    let patched = patch_workspace_form(existing, &patch).unwrap();

    assert!(patched.contains("Root: /new/root"));
    assert!(patched.contains("Description:\n\tNew workspace"));
    assert!(patched.contains("\t//depot/... //ws-main/..."));
}
```

- [ ] **Step 3: Run failing tests**

Run:

```bash
rtk cargo test modify_shelves_unshelve_to_changelist_uses_target_changelist --test tool_mapping_tests
rtk cargo test patch_workspace_form_preserves_view_when_view_is_not_supplied --test tool_mapping_tests
```

Expected: FAIL because shelf builder and workspace form patch helper are not implemented.

- [ ] **Step 4: Add shelf modify builder**

In `src/tools/shelves.rs`, add imports:

```rust
use crate::tools::params::{ModifyShelvesParams, ShelfModifyAction};
```

Append:

```rust
pub fn build_shelf_modify_invocation(params: &ModifyShelvesParams) -> Result<P4Invocation> {
    let files = params.file_paths.clone().unwrap_or_default();
    let mut args = match params.action {
        ShelfModifyAction::Shelve => {
            required_files(&files, "shelve")?;
            let mut args = vec!["shelve".to_string()];
            if params.force {
                args.push("-f".to_string());
            }
            args.extend(["-c".to_string(), params.changelist_id.clone()]);
            args
        }
        ShelfModifyAction::Unshelve => {
            let mut args = vec!["unshelve".to_string()];
            if params.force {
                args.push("-f".to_string());
            }
            args.extend(["-s".to_string(), params.changelist_id.clone()]);
            args
        }
        ShelfModifyAction::Update => {
            required_files(&files, "update")?;
            let mut args = vec!["shelve".to_string()];
            if params.force {
                args.push("-f".to_string());
            }
            args.extend(["-c".to_string(), params.changelist_id.clone()]);
            args
        }
        ShelfModifyAction::Delete => {
            let mut args = vec![
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                params.changelist_id.clone(),
            ];
            args.extend(files.iter().cloned());
            return Ok(P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            });
        }
        ShelfModifyAction::UnshelveToChangelist => {
            vec![
                "unshelve".to_string(),
                "-s".to_string(),
                params.changelist_id.clone(),
                "-c".to_string(),
                params.target_changelist.clone(),
            ]
        }
    };
    args.extend(files);
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

fn required_files(files: &[String], action: &str) -> Result<()> {
    if files.is_empty() {
        Err(P4McpError::InvalidInput {
            message: format!("file_paths is required for {action}"),
        })
    } else {
        Ok(())
    }
}
```

- [ ] **Step 5: Add workspace form patch helper**

Append to `src/p4/forms.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFormPatch {
    pub root: Option<String>,
    pub description: Option<String>,
    pub options: Option<String>,
    pub line_end: Option<String>,
    pub view: Option<Vec<String>>,
}

pub fn patch_workspace_form(existing: &str, patch: &WorkspaceFormPatch) -> Result<String> {
    let mut form = existing.to_string();
    if let Some(root) = &patch.root {
        form = replace_single_line_field(&form, "Root", root)?;
    }
    if let Some(description) = &patch.description {
        form = upsert_indented_block(&form, "Description", &indent_lines(description));
    }
    if let Some(options) = &patch.options {
        form = replace_single_line_field(&form, "Options", options)?;
    }
    if let Some(line_end) = &patch.line_end {
        form = replace_single_line_field(&form, "LineEnd", line_end)?;
    }
    if let Some(view) = &patch.view {
        let replacement = view
            .iter()
            .map(|line| format!("\t{line}\n"))
            .collect::<String>();
        form = replace_indented_block(&form, "View", &replacement)?;
    }
    Ok(form)
}

fn replace_single_line_field(existing: &str, field: &str, value: &str) -> Result<String> {
    let marker = format!("{field}:");
    let Some(start) = existing.find(&marker) else {
        return Err(P4McpError::InvalidInput {
            message: format!("{field} field not found in p4 form"),
        });
    };
    let line_end = existing[start..]
        .find('\n')
        .map(|offset| start + offset)
        .unwrap_or(existing.len());
    let mut patched = String::new();
    patched.push_str(&existing[..start]);
    patched.push_str(&format!("{field}: {value}"));
    patched.push_str(&existing[line_end..]);
    Ok(patched)
}

fn upsert_indented_block(existing: &str, field: &str, replacement: &str) -> String {
    if existing.contains(&format!("{field}:\n")) {
        return replace_indented_block(existing, field, replacement)
            .expect("existing field can be replaced");
    }
    let mut form = existing.to_string();
    if !form.ends_with('\n') {
        form.push('\n');
    }
    form.push('\n');
    form.push_str(field);
    form.push_str(":\n");
    form.push_str(replacement);
    form
}
```

- [ ] **Step 6: Add workspace atomic delete builder**

In `src/tools/workspaces.rs`, add imports:

```rust
use crate::tools::params::{ModifyWorkspacesParams, WorkspaceModifyAction};
```

Append:

```rust
pub fn build_workspace_delete_invocation(params: &ModifyWorkspacesParams) -> Result<P4Invocation> {
    if params.action != WorkspaceModifyAction::Delete {
        return Err(P4McpError::InvalidInput {
            message: format!("unknown action: {}", params.action.as_str()),
        });
    }
    Ok(P4Invocation {
        args: vec!["client".to_string(), "-d".to_string(), params.workspace_name.clone()],
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}
```

- [ ] **Step 7: Migrate shelf and workspace server internals**

Replace `modify_shelves_inner` in `src/server.rs` with:

```rust
async fn modify_shelves_inner(
    &self,
    params: ModifyShelvesParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Shelves, "modify_shelves")
        .map_err(to_mcp_error)?;
    let invocation = build_shelf_modify_invocation(&params).map_err(to_mcp_error)?;
    let request = self.modify_shelves_approval_request(&params, &invocation);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }
    self.call_p4_tool(params.action.as_str(), invocation).await
}
```

Add:

```rust
fn modify_shelves_approval_request(
    &self,
    params: &ModifyShelvesParams,
    invocation: &P4Invocation,
) -> ApprovalRequest {
    let mut approval_params = params.clone();
    approval_params.approval_token = None;
    let mut targets = shelf_targets(&params.changelist_id);
    if let Some(files) = &params.file_paths {
        targets.extend(files.iter().cloned());
    }
    ApprovalRequest {
        tool: "modify_shelves".to_string(),
        action: params.action.as_str().to_string(),
        params: serde_json::to_value(approval_params)
            .expect("modify shelves params serialize to JSON"),
        preview: self.p4_approval_preview(P4ApprovalContext {
            tool: "modify_shelves",
            action: params.action.as_str(),
            targets,
            changelist: Some(params.changelist_id.clone()),
            workspace: None,
            stream: None,
            invocation,
        }),
    }
}
```

Replace `modify_workspaces_inner` with:

```rust
async fn modify_workspaces_inner(
    &self,
    params: ModifyWorkspacesParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Workspaces, "modify_workspaces")
        .map_err(to_mcp_error)?;
    let action = params.action.as_str();
    let preview_invocation = match params.action {
        WorkspaceModifyAction::Create | WorkspaceModifyAction::Update => json_invocation(
            vec!["client".to_string(), "-i".to_string()],
            Some(format!("Client: {}\n\n<patched after approval>\n", params.workspace_name)),
        ),
        WorkspaceModifyAction::Delete => build_workspace_delete_invocation(&params)
            .map_err(to_mcp_error)?,
        WorkspaceModifyAction::Switch => json_invocation(
            vec!["client".to_string(), "-s".to_string(), params.workspace_name.clone()],
            None,
        ),
    };
    let request = self.modify_workspaces_approval_request(&params, &preview_invocation);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }

    let invocation = match params.action {
        WorkspaceModifyAction::Create | WorkspaceModifyAction::Update => {
            let current = self
                .run_p4(text_invocation(vec![
                    "client".to_string(),
                    "-o".to_string(),
                    params.workspace_name.clone(),
                ]))
                .await?;
            let current_form = current
                .text
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let patch = WorkspaceFormPatch {
                root: params.workspace_root.clone(),
                description: params.workspace_description.clone(),
                options: params.workspace_options.clone(),
                line_end: params.workspace_line_end.clone(),
                view: params.workspace_view.clone(),
            };
            let patched = patch_workspace_form(current_form, &patch).map_err(to_mcp_error)?;
            json_invocation(vec!["client".to_string(), "-i".to_string()], Some(patched))
        }
        WorkspaceModifyAction::Delete => build_workspace_delete_invocation(&params)
            .map_err(to_mcp_error)?,
        WorkspaceModifyAction::Switch => json_invocation(
            vec!["client".to_string(), "-s".to_string(), params.workspace_name.clone()],
            None,
        ),
    };
    self.call_p4_tool(action, invocation).await
}
```

Add the helper:

```rust
fn modify_workspaces_approval_request(
    &self,
    params: &ModifyWorkspacesParams,
    invocation: &P4Invocation,
) -> ApprovalRequest {
    let mut approval_params = params.clone();
    approval_params.approval_token = None;
    ApprovalRequest {
        tool: "modify_workspaces".to_string(),
        action: params.action.as_str().to_string(),
        params: serde_json::to_value(approval_params)
            .expect("modify workspaces params serialize to JSON"),
        preview: self.p4_approval_preview(P4ApprovalContext {
            tool: "modify_workspaces",
            action: params.action.as_str(),
            targets: vec![params.workspace_name.clone()],
            changelist: None,
            workspace: Some(params.workspace_name.clone()),
            stream: None,
            invocation,
        }),
    }
}
```

Update public routes to use:

```rust
Parameters(params): Parameters<ModifyShelvesParams>,
```

and:

```rust
Parameters(params): Parameters<ModifyWorkspacesParams>,
```

- [ ] **Step 8: Run focused tests**

Run:

```bash
rtk cargo test modify_shelves_unshelve_to_changelist_uses_target_changelist --test tool_mapping_tests
rtk cargo test modify_shelves_force_shelve_uses_file_paths --test tool_mapping_tests
rtk cargo test modify_workspaces_delete_maps_to_client_delete --test tool_mapping_tests
rtk cargo test patch_workspace_form_preserves_view_when_view_is_not_supplied --test tool_mapping_tests
rtk cargo test modify_shelves_without_approval_does_not_call_executor
rtk cargo test modify_workspaces_without_approval_does_not_call_executor
```

Expected: all PASS after existing tests are migrated to `ModifyShelvesParams` and `ModifyWorkspacesParams`.

- [ ] **Step 9: Commit**

```bash
rtk git add src/tools/shelves.rs src/tools/workspaces.rs src/p4/forms.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align shelf and workspace write contracts"
```

---

### Task 5: Split Review Query And Modify Contracts

**Files:**
- Modify: `src/tools/reviews.rs`
- Modify: `src/server.rs`
- Modify: `tests/schema_contract_tests.rs`
- Modify: `tests/review_client_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add review split schema tests**

Append to `tests/schema_contract_tests.rs`:

```rust
use p4mcp_server_rs::tools::reviews::{
    ModifyReviewsParams, QueryReviewsParams, ReviewModifyAction, ReviewQueryAction,
};

#[test]
fn query_reviews_schema_matches_upstream_fields_and_omits_write_fields() {
    for field in [
        "action",
        "review_id",
        "fields",
        "comments_fields",
        "up_voters",
        "from_version",
        "to_version",
        "max_results",
        "after",
        "after_updated",
        "result_order",
        "projects",
        "state",
        "keywords",
        "keywords_fields",
        "include_transitions",
    ] {
        assert_has::<QueryReviewsParams>(field);
    }
    assert_omits::<QueryReviewsParams>("comment_id");
    assert_omits::<QueryReviewsParams>("approval_token");

    let params: QueryReviewsParams = serde_json::from_value(serde_json::json!({
        "action": "files",
        "review_id": 123,
        "from_version": 1,
        "to_version": 2
    }))
    .unwrap();
    assert_eq!(params.action, ReviewQueryAction::Files);
}

#[test]
fn modify_reviews_schema_matches_upstream_fields_and_omits_read_fields() {
    for field in [
        "action",
        "review_id",
        "change_id",
        "description",
        "reviewers",
        "required_reviewers",
        "reviewer_group_names",
        "reviewer_groups_required",
        "comment_file_path",
        "comment_left_line",
        "comment_right_line",
        "comment_version",
        "vote_value",
        "version",
        "transition",
        "jobs",
        "fix_status",
        "cleanup",
        "participant_user_names",
        "participant_users_required",
        "participant_group_names",
        "participant_groups_required",
        "body",
        "task_state",
        "notify",
        "comment_id",
        "not_updated_since",
        "max_reviews",
        "new_author",
        "new_description",
        "approval_token",
    ] {
        assert_has::<ModifyReviewsParams>(field);
    }
    assert_omits::<ModifyReviewsParams>("after");
    assert_omits::<ModifyReviewsParams>("fields");
    assert_omits::<ModifyReviewsParams>("confirmation");

    let params: ModifyReviewsParams = serde_json::from_value(serde_json::json!({
        "action": "reply_comment",
        "review_id": 123,
        "comment_id": 987,
        "body": "reply"
    }))
    .unwrap();
    assert_eq!(params.action, ReviewModifyAction::ReplyComment);
}
```

- [ ] **Step 2: Run failing review schema tests**

Run:

```bash
rtk cargo test query_reviews_schema_matches_upstream_fields_and_omits_write_fields --test schema_contract_tests
rtk cargo test modify_reviews_schema_matches_upstream_fields_and_omits_read_fields --test schema_contract_tests
```

Expected: FAIL because split review params do not exist.

- [ ] **Step 3: Replace review request types**

In `src/tools/reviews.rs`, replace `ReviewAction` and `ReviewRequest` with:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewQueryAction {
    List,
    Dashboard,
    Get,
    Transitions,
    FilesReadby,
    Files,
    Comments,
    Activity,
}

impl ReviewQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Dashboard => "dashboard",
            Self::Get => "get",
            Self::Transitions => "transitions",
            Self::FilesReadby => "files_readby",
            Self::Files => "files",
            Self::Comments => "comments",
            Self::Activity => "activity",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryReviewsParams {
    pub action: ReviewQueryAction,
    #[serde(default)]
    pub review_id: Option<u64>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default = "default_comments_fields")]
    pub comments_fields: Option<String>,
    #[serde(default)]
    pub up_voters: Option<Vec<String>>,
    #[serde(default)]
    pub from_version: Option<u64>,
    #[serde(default)]
    pub to_version: Option<u64>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub after_updated: Option<String>,
    #[serde(default)]
    pub result_order: Option<String>,
    #[serde(default)]
    pub projects: Option<Vec<String>>,
    #[serde(default)]
    pub state: Option<Vec<String>>,
    #[serde(default)]
    pub keywords: Option<String>,
    #[serde(default)]
    pub keywords_fields: Option<Vec<String>>,
    #[serde(default)]
    pub include_transitions: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewModifyAction {
    Create,
    RefreshProjects,
    Vote,
    Transition,
    AppendParticipants,
    AddComment,
    ReplyComment,
    AppendChange,
    ReplaceWithChange,
    Join,
    ArchiveInactive,
    MarkCommentRead,
    MarkCommentUnread,
    MarkAllCommentsRead,
    MarkAllCommentsUnread,
    UpdateAuthor,
    UpdateDescription,
    ReplaceParticipants,
    DeleteParticipants,
    Leave,
    Obliterate,
}

impl ReviewModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::RefreshProjects => "refresh_projects",
            Self::Vote => "vote",
            Self::Transition => "transition",
            Self::AppendParticipants => "append_participants",
            Self::AddComment => "add_comment",
            Self::ReplyComment => "reply_comment",
            Self::AppendChange => "append_change",
            Self::ReplaceWithChange => "replace_with_change",
            Self::Join => "join",
            Self::ArchiveInactive => "archive_inactive",
            Self::MarkCommentRead => "mark_comment_read",
            Self::MarkCommentUnread => "mark_comment_unread",
            Self::MarkAllCommentsRead => "mark_all_comments_read",
            Self::MarkAllCommentsUnread => "mark_all_comments_unread",
            Self::UpdateAuthor => "update_author",
            Self::UpdateDescription => "update_description",
            Self::ReplaceParticipants => "replace_participants",
            Self::DeleteParticipants => "delete_participants",
            Self::Leave => "leave",
            Self::Obliterate => "obliterate",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ModifyReviewsParams {
    pub action: ReviewModifyAction,
    #[serde(default)]
    pub review_id: Option<u64>,
    #[serde(default)]
    pub change_id: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub reviewers: Option<Vec<String>>,
    #[serde(default)]
    pub required_reviewers: Option<Vec<String>>,
    #[serde(default)]
    pub reviewer_group_names: Option<Vec<String>>,
    #[serde(default)]
    pub reviewer_groups_required: Option<Vec<String>>,
    #[serde(default)]
    pub comment_file_path: Option<String>,
    #[serde(default)]
    pub comment_left_line: Option<u64>,
    #[serde(default)]
    pub comment_right_line: Option<u64>,
    #[serde(default)]
    pub comment_version: Option<u64>,
    #[serde(default)]
    pub vote_value: Option<String>,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub transition: Option<String>,
    #[serde(default)]
    pub jobs: Option<Vec<String>>,
    #[serde(default)]
    pub fix_status: Option<String>,
    #[serde(default)]
    pub cleanup: Option<bool>,
    #[serde(default)]
    pub participant_user_names: Option<Vec<String>>,
    #[serde(default)]
    pub participant_users_required: Option<Vec<String>>,
    #[serde(default)]
    pub participant_group_names: Option<Vec<String>>,
    #[serde(default)]
    pub participant_groups_required: Option<Vec<String>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub task_state: Option<String>,
    #[serde(default)]
    pub notify: Option<String>,
    #[serde(default)]
    pub comment_id: Option<u64>,
    #[serde(default)]
    pub not_updated_since: Option<String>,
    #[serde(default)]
    pub max_reviews: u16,
    #[serde(default)]
    pub new_author: Option<String>,
    #[serde(default)]
    pub new_description: Option<String>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

fn default_comments_fields() -> Option<String> {
    Some("id,body,user,time".to_string())
}
```

- [ ] **Step 4: Replace review HTTP builders**

In `src/tools/reviews.rs`, replace `impl ReviewRequest` with:

```rust
impl QueryReviewsParams {
    pub fn to_http(&self) -> Result<BuiltReviewRequest> {
        let id = || required_id(self.review_id, "review_id");
        let built = match self.action {
            ReviewQueryAction::List => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews".into(),
                query: review_list_query(self),
                body: json!({}),
            },
            ReviewQueryAction::Dashboard => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews/dashboard".into(),
                query: vec![("max".into(), self.max_results.to_string())],
                body: json!({}),
            },
            ReviewQueryAction::Get => {
                let mut request = get(format!("/reviews/{}", id()?));
                add_repeated_query(&mut request.query, "fields[]", self.fields.as_deref());
                if self.include_transitions == Some(true) {
                    request.query.push(("transitions".into(), "true".into()));
                }
                request
            }
            ReviewQueryAction::Transitions => get(format!("/reviews/{}/transitions", id()?)),
            ReviewQueryAction::FilesReadby => get(format!("/reviews/{}/files/readby", id()?)),
            ReviewQueryAction::Files => {
                let mut request = get(format!("/reviews/{}/files", id()?));
                if let Some(from) = self.from_version {
                    request.query.push(("from".into(), from.to_string()));
                }
                if let Some(to) = self.to_version {
                    request.query.push(("to".into(), to.to_string()));
                }
                request
            }
            ReviewQueryAction::Comments => get(format!("/reviews/{}/comments", id()?)),
            ReviewQueryAction::Activity => {
                let mut request = get(format!("/reviews/{}/activity", id()?));
                request.query.push(("max".into(), self.max_results.to_string()));
                request
            }
        };
        Ok(built)
    }
}

impl ModifyReviewsParams {
    pub fn to_http(&self, username: Option<&str>) -> Result<BuiltReviewRequest> {
        let review_id = || required_id(self.review_id, "review_id");
        let change_id = || required_id(self.change_id, "change_id");
        let built = match self.action {
            ReviewModifyAction::Create => post("/reviews".into(), create_review_body(self, change_id()?)),
            ReviewModifyAction::RefreshProjects => post(format!("/reviews/{}/refreshProjects", review_id()?), json!({})),
            ReviewModifyAction::Vote => post(format!("/reviews/{}/vote", review_id()?), vote_body(self)?),
            ReviewModifyAction::Transition => post(format!("/reviews/{}/transitions", review_id()?), transition_body(self)?),
            ReviewModifyAction::AppendParticipants => post(format!("/reviews/{}/participants", review_id()?), participants_body(self)),
            ReviewModifyAction::AddComment => post(format!("/reviews/{}/comments", review_id()?), comment_body(self, None)?),
            ReviewModifyAction::ReplyComment => post(format!("/reviews/{}/comments", review_id()?), comment_body(self, self.comment_id)?),
            ReviewModifyAction::AppendChange => post(format!("/reviews/{}/appendchange", review_id()?), json!({"changeId": change_id()?})),
            ReviewModifyAction::ReplaceWithChange => post(format!("/reviews/{}/replacewithchange", review_id()?), json!({"changeId": change_id()?})),
            ReviewModifyAction::Join => post(format!("/reviews/{}/join", review_id()?), join_body(username)),
            ReviewModifyAction::ArchiveInactive => post("/reviews/archiveInactive".into(), archive_body(self)?),
            ReviewModifyAction::MarkCommentRead => post(format!("/comments/{}/read", required_id(self.comment_id, "comment_id")?), json!({})),
            ReviewModifyAction::MarkCommentUnread => post(format!("/comments/{}/unread", required_id(self.comment_id, "comment_id")?), json!({})),
            ReviewModifyAction::MarkAllCommentsRead => post(format!("/reviews/{}/comments/read", review_id()?), json!({})),
            ReviewModifyAction::MarkAllCommentsUnread => post(format!("/reviews/{}/comments/unread", review_id()?), json!({})),
            ReviewModifyAction::UpdateAuthor => post_put("PUT", format!("/reviews/{}/author", review_id()?), json!({"author": required_string(self.new_author.as_deref(), "new_author")?})),
            ReviewModifyAction::UpdateDescription => post_put("PUT", format!("/reviews/{}/description", review_id()?), json!({"description": required_string(self.new_description.as_deref(), "new_description")?})),
            ReviewModifyAction::ReplaceParticipants => post_put("PUT", format!("/reviews/{}/participants", review_id()?), participants_body(self)),
            ReviewModifyAction::DeleteParticipants => delete(format!("/reviews/{}/participants", review_id()?), participants_body(self)),
            ReviewModifyAction::Leave => delete(format!("/reviews/{}/leave", review_id()?), json!({})),
            ReviewModifyAction::Obliterate => delete(format!("/reviews/{}", review_id()?), json!({})),
        };
        Ok(built)
    }
}
```

Add helpers below the existing `get/post/put/delete` helpers:

```rust
fn post_put(method: &str, path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: method.into(),
        path,
        query: Vec::new(),
        body,
    }
}

fn required_id(value: Option<u64>, name: &str) -> Result<u64> {
    value.ok_or_else(|| P4McpError::InvalidInput {
        message: format!("{name} is required"),
    })
}

fn required_string(value: Option<&str>, name: &str) -> Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{name} is required"),
        })
}

fn review_list_query(params: &QueryReviewsParams) -> Vec<(String, String)> {
    let mut query = vec![("max".into(), params.max_results.to_string())];
    push_optional(&mut query, "after", params.after.as_deref());
    push_optional(&mut query, "afterUpdated", params.after_updated.as_deref());
    push_optional(&mut query, "resultOrder", params.result_order.as_deref());
    push_optional(&mut query, "keywords", params.keywords.as_deref());
    add_repeated_query(&mut query, "project[]", params.projects.as_deref());
    add_repeated_query(&mut query, "state[]", params.state.as_deref());
    add_repeated_query(&mut query, "keywordsFields[]", params.keywords_fields.as_deref());
    add_repeated_query(&mut query, "fields[]", params.fields.as_deref());
    query
}

fn push_optional(query: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        query.push((name.to_string(), value.to_string()));
    }
}

fn add_repeated_query(query: &mut Vec<(String, String)>, name: &str, values: Option<&[String]>) {
    if let Some(values) = values {
        query.extend(values.iter().map(|value| (name.to_string(), value.clone())));
    }
}

fn create_review_body(params: &ModifyReviewsParams, change_id: u64) -> Value {
    let mut body = json!({"change": change_id});
    insert_string(&mut body, "description", params.description.as_deref());
    insert_array(&mut body, "reviewers", params.reviewers.as_deref());
    insert_array(&mut body, "requiredReviewers", params.required_reviewers.as_deref());
    if params.reviewer_group_names.is_some() || params.reviewer_groups_required.is_some() {
        body["reviewerGroups"] = reviewer_groups_body(params);
    }
    body
}

fn vote_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body = json!({"vote": required_string(params.vote_value.as_deref(), "vote_value")?});
    if let Some(version) = params.version {
        body["version"] = json!(version);
    }
    Ok(body)
}

fn transition_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body = json!({"transition": required_string(params.transition.as_deref(), "transition")?});
    if let Some(jobs) = &params.jobs {
        body["jobs"] = json!(jobs);
    }
    insert_string(&mut body, "fixStatus", params.fix_status.as_deref());
    if let Some(cleanup) = params.cleanup {
        body["cleanup"] = json!(cleanup);
    }
    Ok(body)
}

fn comment_body(params: &ModifyReviewsParams, parent_comment: Option<u64>) -> Result<Value> {
    let mut body = json!({"body": required_string(params.body.as_deref(), "body")?});
    let mut context = json!({});
    insert_string(&mut context, "file", params.comment_file_path.as_deref());
    if let Some(line) = params.comment_left_line {
        context["leftLine"] = json!(line);
    }
    if let Some(line) = params.comment_right_line {
        context["rightLine"] = json!(line);
    }
    if let Some(version) = params.comment_version {
        context["version"] = json!(version);
    }
    if let Some(comment) = parent_comment {
        context["comment"] = json!(comment);
    }
    if context.as_object().is_some_and(|object| !object.is_empty()) {
        body["context"] = context;
    }
    insert_string(&mut body, "taskState", params.task_state.as_deref());
    Ok(body)
}

fn archive_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body = json!({
        "notUpdatedSince": required_string(params.not_updated_since.as_deref(), "not_updated_since")?,
        "description": params.description.as_deref().unwrap_or("Archiving inactive reviews")
    });
    if params.max_reviews > 0 {
        body["max"] = json!(params.max_reviews);
    }
    Ok(body)
}

fn participants_body(params: &ModifyReviewsParams) -> Value {
    json!({
        "participants": {
            "users": participant_users(params),
            "groups": participant_groups(params)
        }
    })
}

fn participant_users(params: &ModifyReviewsParams) -> Value {
    let mut users = serde_json::Map::new();
    if let Some(names) = &params.participant_user_names {
        for name in names {
            users.insert(name.clone(), json!({"required": "no"}));
        }
    }
    if let Some(names) = &params.participant_users_required {
        for name in names {
            users.insert(name.clone(), json!({"required": "yes"}));
        }
    }
    Value::Object(users)
}

fn participant_groups(params: &ModifyReviewsParams) -> Value {
    let mut groups = serde_json::Map::new();
    if let Some(names) = &params.participant_group_names {
        for name in names {
            groups.insert(name.clone(), json!({"required": "none"}));
        }
    }
    if let Some(names) = &params.participant_groups_required {
        for name in names {
            groups.insert(name.clone(), json!({"required": "all"}));
        }
    }
    Value::Object(groups)
}

fn reviewer_groups_body(params: &ModifyReviewsParams) -> Value {
    let mut groups = Vec::new();
    if let Some(names) = &params.reviewer_group_names {
        groups.extend(names.iter().map(|name| json!({"name": name, "required": "false"})));
    }
    if let Some(names) = &params.reviewer_groups_required {
        groups.extend(names.iter().map(|name| json!({"name": name, "required": "true"})));
    }
    Value::Array(groups)
}

fn insert_string(body: &mut Value, field: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        body[field] = json!(value);
    }
}

fn insert_array(body: &mut Value, field: &str, value: Option<&[String]>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        body[field] = json!(value);
    }
}

fn join_body(username: Option<&str>) -> Value {
    match username {
        Some(username) if !username.trim().is_empty() => {
            let mut users = serde_json::Map::new();
            users.insert(username.to_string(), json!([]));
            let mut participants = serde_json::Map::new();
            participants.insert("users".to_string(), Value::Object(users));
            let mut body = serde_json::Map::new();
            body.insert("participants".to_string(), Value::Object(participants));
            Value::Object(body)
        }
        _ => json!({}),
    }
}
```

- [ ] **Step 5: Update review HTTP client API**

In `ReviewHttpClient`, replace request-based methods with:

```rust
pub async fn execute(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
    if built.method != "GET" {
        anyhow::bail!("Review API write request requires MCP write approval");
    }
    self.send(built).await
}

pub async fn execute_approved(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
    self.send(built).await
}
```

- [ ] **Step 6: Update tests to use split params**

In `tests/review_client_tests.rs`, replace old `ReviewRequest` imports with:

```rust
use p4mcp_server_rs::tools::reviews::{
    ModifyReviewsParams, QueryReviewsParams, ReviewHttpClient, ReviewModifyAction,
    ReviewQueryAction,
};
```

Replace the list test body with:

```rust
let request = QueryReviewsParams {
    action: ReviewQueryAction::List,
    review_id: None,
    fields: None,
    comments_fields: Some("id,body,user,time".to_string()),
    up_voters: None,
    from_version: None,
    to_version: None,
    max_results: 25,
    after: None,
    after_updated: None,
    result_order: None,
    projects: None,
    state: None,
    keywords: None,
    keywords_fields: None,
    include_transitions: None,
};
let built = request.to_http().unwrap();
```

Replace the vote request body with:

```rust
let request = ModifyReviewsParams {
    action: ReviewModifyAction::Vote,
    review_id: Some(123),
    change_id: None,
    description: None,
    reviewers: None,
    required_reviewers: None,
    reviewer_group_names: None,
    reviewer_groups_required: None,
    comment_file_path: None,
    comment_left_line: None,
    comment_right_line: None,
    comment_version: None,
    vote_value: Some("up".to_string()),
    version: Some(2),
    transition: None,
    jobs: None,
    fix_status: None,
    cleanup: None,
    participant_user_names: None,
    participant_users_required: None,
    participant_group_names: None,
    participant_groups_required: None,
    body: None,
    task_state: None,
    notify: None,
    comment_id: None,
    not_updated_since: None,
    max_reviews: 0,
    new_author: None,
    new_description: None,
    approval_token: None,
};
let built = request.to_http(None).unwrap();
```

When executing the client, call:

```rust
let result = client.execute(built).await.unwrap();
```

for reads and:

```rust
let result = client.execute_approved(built).await.unwrap();
```

for approved writes.

- [ ] **Step 7: Run focused review tests**

Run:

```bash
rtk cargo test query_reviews_schema_matches_upstream_fields_and_omits_write_fields --test schema_contract_tests
rtk cargo test modify_reviews_schema_matches_upstream_fields_and_omits_read_fields --test schema_contract_tests
rtk cargo test --test review_client_tests
```

Expected: all PASS after imports and constructors are migrated.

- [ ] **Step 8: Commit**

```bash
rtk git add src/tools/reviews.rs tests/schema_contract_tests.rs tests/review_client_tests.rs
rtk git commit -m "fix: split review query and modify schemas"
```

---

### Task 6: Review Server Routing

**Files:**
- Modify: `src/server.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add smoke tests for split review routing**

In `tests/mcp_smoke_tests.rs`, add a query smoke that uses `QueryReviewsParams` with `fields`:

```rust
#[tokio::test]
async fn query_reviews_list_threads_upstream_filters_to_http() {
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(query_param("fields[]", "id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "reviews": [{"id": 123}]
        })))
        .expect(1)
        .mount(&swarm)
        .await;
    let executor = Arc::new(FakeExecutor::new(vec![
        Ok(output_records(vec![serde_json::json!({"userName": "alice", "serverAddress": "perforce:1666"})])),
        Ok(output_records(vec![serde_json::json!({"value": swarm.uri()})])),
        Ok(output_text("perforce:1666 (alice) ticket-123\n")),
    ]));
    let server = P4McpServer::with_executor(test_config(), executor);

    let response = server
        .query_reviews(Parameters(QueryReviewsParams {
            action: ReviewQueryAction::List,
            review_id: None,
            fields: Some(vec!["id".to_string()]),
            comments_fields: Some("id,body,user,time".to_string()),
            up_voters: None,
            from_version: None,
            to_version: None,
            max_results: 5,
            after: None,
            after_updated: None,
            result_order: None,
            projects: None,
            state: None,
            keywords: None,
            keywords_fields: None,
            include_transitions: None,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.action, "list");
}
```

In the existing `#[cfg(test)] mod tests` inside `src/server.rs`, add a modify smoke for `comment_id`. This route needs private approval helpers, so keep it as a server unit test instead of an external integration test:

```rust
#[tokio::test]
async fn modify_reviews_comment_read_uses_comment_id_after_approval() {
    let swarm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v11/comments/987/read"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "read": true
        })))
        .expect(1)
        .mount(&swarm)
        .await;
    let executor = Arc::new(FakeExecutor::new(vec![
        Ok(output_records(vec![serde_json::json!({"userName": "alice", "serverAddress": "perforce:1666"})])),
        Ok(output_records(vec![serde_json::json!({"value": swarm.uri()})])),
        Ok(output_text("perforce:1666 (alice) ticket-123\n")),
    ]));
    let approval = Arc::new(RecordingApprovalGate::approved());
    let server = P4McpServer::with_executor_and_approval(test_config(), executor, approval);

    let response = server
        .modify_reviews_inner(
            ModifyReviewsParams {
                action: ReviewModifyAction::MarkCommentRead,
                review_id: None,
                change_id: None,
                description: None,
                reviewers: None,
                required_reviewers: None,
                reviewer_group_names: None,
                reviewer_groups_required: None,
                comment_file_path: None,
                comment_left_line: None,
                comment_right_line: None,
                comment_version: None,
                vote_value: None,
                version: None,
                transition: None,
                jobs: None,
                fix_status: None,
                cleanup: None,
                participant_user_names: None,
                participant_users_required: None,
                participant_group_names: None,
                participant_groups_required: None,
                body: None,
                task_state: None,
                notify: None,
                comment_id: Some(987),
                not_updated_since: None,
                max_reviews: 0,
                new_author: None,
                new_description: None,
                approval_token: Some("approved-token".to_string()),
            },
            ApprovalChannel::FallbackOnly,
        )
        .await
        .unwrap();

    assert_eq!(response.0.action, "mark_comment_read");
}
```

- [ ] **Step 2: Run failing smoke tests**

Run:

```bash
rtk cargo test query_reviews_list_threads_upstream_filters_to_http --test mcp_smoke_tests
rtk cargo test modify_reviews_comment_read_uses_comment_id_after_approval
```

Expected: FAIL because server routes still take `ReviewRequest`.

- [ ] **Step 3: Update server review imports and helpers**

In `src/server.rs`, replace review imports with:

```rust
reviews::{
    BuiltReviewRequest, ModifyReviewsParams, QueryReviewsParams, ReviewApiConfig,
    ReviewHttpClient,
},
```

Replace `review_action_name` with two helpers:

```rust
fn review_query_action_name(params: &QueryReviewsParams) -> String {
    params.action.as_str().to_string()
}

fn review_modify_action_name(params: &ModifyReviewsParams) -> String {
    params.action.as_str().to_string()
}
```

- [ ] **Step 4: Update review server methods**

Replace `query_reviews` with:

```rust
#[tool(
    description = "Query P4 Code Review / Swarm reviews",
    annotations(read_only_hint = true)
)]
pub async fn query_reviews(
    &self,
    Parameters(params): Parameters<QueryReviewsParams>,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Read, Toolset::Reviews, "query_reviews")
        .map_err(to_mcp_error)?;
    let built = params.to_http().map_err(to_mcp_error)?;
    let action = review_query_action_name(&params);
    let client = self.review_http_client_from_p4().await?;
    let message = client.execute(built).await.map_err(review_api_error)?;
    Ok(Json(ToolResponse::success(&action, message)))
}
```

Replace `modify_reviews_inner` with:

```rust
async fn modify_reviews_inner(
    &self,
    params: ModifyReviewsParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Reviews, "modify_reviews")
        .map_err(to_mcp_error)?;
    let preview_built = params.to_http(None).map_err(to_mcp_error)?;
    let request = self.modify_reviews_approval_request(&params, &preview_built);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }
    let action = review_modify_action_name(&params);
    let client = self.review_http_client_from_p4().await?;
    let built = params
        .to_http(Some(&client.username()))
        .map_err(to_mcp_error)?;
    let message = client.execute_approved(built).await.map_err(review_api_error)?;
    Ok(Json(ToolResponse::success(&action, message)))
}
```

Add this method to `ReviewHttpClient` in `src/tools/reviews.rs`:

```rust
pub fn username(&self) -> &str {
    &self.config.username
}
```

Replace `modify_reviews` route params with:

```rust
Parameters(params): Parameters<ModifyReviewsParams>,
```

Update `modify_reviews_approval_request` to accept `&ModifyReviewsParams` and use `review_modify_action_name(params)`.

- [ ] **Step 5: Run focused smoke tests**

Run:

```bash
rtk cargo test query_reviews_list_threads_upstream_filters_to_http --test mcp_smoke_tests
rtk cargo test modify_reviews_comment_read_uses_comment_id_after_approval
rtk cargo test --test review_client_tests
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
rtk git add src/server.rs src/tools/reviews.rs tests/mcp_smoke_tests.rs tests/review_client_tests.rs
rtk git commit -m "fix: route split review contracts"
```

---

### Task 7: Stream Modify Contract And CLI Mapping

**Files:**
- Modify: `src/tools/streams.rs`
- Modify: `src/p4/forms.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add stream modify mapping tests**

Append to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::streams::build_stream_modify_command;

#[test]
fn modify_streams_edit_spec_uses_stream_spec_edit_command() {
    let params = ModifyStreamsParams {
        action: StreamModifyAction::EditSpec,
        stream_name: Some("//depot/main".to_string()),
        stream_type: None,
        parent: None,
        name: None,
        description: None,
        options: None,
        parent_view: None,
        paths: None,
        remapped: None,
        ignored: None,
        changelist: Some("12345".to_string()),
        resolve_mode: None,
        target_changelist: None,
        parent_stream: None,
        branch: None,
        file_paths: None,
        preview: false,
        force: false,
        reverse: false,
        quiet: false,
        max_files: None,
        output_base: false,
        virtual_stream: false,
        schedule_branch_resolve: false,
        integrate_around_deleted: false,
        skip_cherry_picked: false,
        source_path: None,
        target_path: None,
        workspace: None,
        workspace_name: None,
        root: None,
        host: None,
        alt_roots: None,
        approval_token: None,
    };

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(invocation.args, vec!["edit", "-So", "-c", "12345"]);
}

#[test]
fn modify_streams_copy_uses_upstream_stream_flags() {
    let params = ModifyStreamsParams {
        action: StreamModifyAction::Copy,
        stream_name: Some("//depot/dev".to_string()),
        stream_type: None,
        parent: None,
        name: None,
        description: None,
        options: None,
        parent_view: None,
        paths: None,
        remapped: None,
        ignored: None,
        changelist: Some("12345".to_string()),
        resolve_mode: None,
        target_changelist: None,
        parent_stream: Some("//depot/main".to_string()),
        branch: None,
        file_paths: Some(vec!["//depot/dev/...".to_string()]),
        preview: true,
        force: true,
        reverse: true,
        quiet: true,
        max_files: Some(25),
        output_base: false,
        virtual_stream: true,
        schedule_branch_resolve: false,
        integrate_around_deleted: false,
        skip_cherry_picked: false,
        source_path: None,
        target_path: None,
        workspace: None,
        workspace_name: None,
        root: None,
        host: None,
        alt_roots: None,
        approval_token: None,
    };

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "copy",
            "-n",
            "-F",
            "-v",
            "-q",
            "-c",
            "12345",
            "-m25",
            "-S",
            "//depot/dev",
            "-P",
            "//depot/main",
            "-r",
            "//depot/dev/..."
        ]
    );
}
```

- [ ] **Step 2: Run failing stream mapping tests**

Run:

```bash
rtk cargo test modify_streams_edit_spec_uses_stream_spec_edit_command --test tool_mapping_tests
rtk cargo test modify_streams_copy_uses_upstream_stream_flags --test tool_mapping_tests
```

Expected: FAIL because stream modify builder does not exist.

- [ ] **Step 3: Add stream modify command enum and builder**

In `src/tools/streams.rs`, extend imports:

```rust
tools::params::{ModifyStreamsParams, QueryStreamsParams, StreamModifyAction, StreamQueryAction},
```

Add this enum below `StreamQueryCommand`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamModifyCommand {
    Single(P4Invocation),
    CreateOrUpdate,
}

impl StreamModifyCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            Self::CreateOrUpdate => None,
        }
    }
}
```

Add builder:

```rust
pub fn build_stream_modify_command(params: &ModifyStreamsParams) -> Result<StreamModifyCommand> {
    let invocation = match params.action {
        StreamModifyAction::Create | StreamModifyAction::Update => {
            return Ok(StreamModifyCommand::CreateOrUpdate);
        }
        StreamModifyAction::Delete => json_invocation(vec![
            "stream".into(),
            "-d".into(),
            required(params.stream_name.as_deref(), "stream_name")?,
        ]),
        StreamModifyAction::EditSpec => {
            let mut args = vec!["edit".to_string(), "-So".to_string()];
            if let Some(changelist) = non_blank(params.changelist.as_deref()) {
                args.extend(["-c".to_string(), changelist.to_string()]);
            }
            json_invocation(args)
        }
        StreamModifyAction::ResolveSpec => {
            let mode = match params.resolve_mode.as_deref().unwrap_or("auto") {
                "auto" => "-am",
                "accept_theirs" => "-at",
                "accept_yours" => "-ay",
                "accept_safe" => "-as",
                other => {
                    return Err(P4McpError::InvalidInput {
                        message: format!("invalid resolve_mode: {other}"),
                    });
                }
            };
            json_invocation(vec!["resolve".into(), "-So".into(), mode.into()])
        }
        StreamModifyAction::RevertSpec => json_invocation(vec!["revert".into(), "-So".into()]),
        StreamModifyAction::ShelveSpec => json_invocation(vec![
            "shelve".into(),
            "-As".into(),
            "-c".into(),
            required(params.changelist.as_deref(), "changelist")?,
        ]),
        StreamModifyAction::UnshelveSpec => {
            let mut args = vec![
                "unshelve".to_string(),
                "-As".to_string(),
                "-s".to_string(),
                required(params.changelist.as_deref(), "changelist")?,
            ];
            if let Some(target) = non_blank(params.target_changelist.as_deref()) {
                args.extend(["-c".to_string(), target.to_string()]);
            }
            json_invocation(args)
        }
        StreamModifyAction::Copy => propagation_invocation("copy", params)?,
        StreamModifyAction::Merge => propagation_invocation("merge", params)?,
        StreamModifyAction::Integrate => propagation_invocation("integrate", params)?,
        StreamModifyAction::Populate => populate_invocation(params)?,
        StreamModifyAction::Switch => {
            let mut args = vec![
                "client".to_string(),
                "-s".to_string(),
                "-S".to_string(),
                required(params.stream_name.as_deref(), "stream_name")?,
            ];
            if let Some(workspace) = non_blank(params.workspace.as_deref()) {
                args.push(workspace.to_string());
            }
            json_invocation(args)
        }
        StreamModifyAction::CreateWorkspace => {
            return Ok(StreamModifyCommand::CreateOrUpdate);
        }
    };
    Ok(StreamModifyCommand::Single(invocation))
}
```

Add propagation helpers:

```rust
fn propagation_invocation(command: &str, params: &ModifyStreamsParams) -> Result<P4Invocation> {
    let mut args = vec![command.to_string()];
    if params.preview {
        args.push("-n".into());
    }
    if params.force {
        args.push("-F".into());
    }
    if command == "copy" && params.virtual_stream {
        args.push("-v".into());
    }
    if params.quiet {
        args.push("-q".into());
    }
    if let Some(changelist) = non_blank(params.changelist.as_deref()) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
        args.extend(["-P".into(), parent.into()]);
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
    }
    if params.reverse {
        args.push("-r".into());
    }
    if params.output_base && (command == "merge" || command == "integrate") {
        args.push("-Ob".into());
    }
    if command == "integrate" {
        if params.schedule_branch_resolve {
            args.push("-Rb".into());
        }
        if params.integrate_around_deleted {
            args.push("-Di".into());
        }
        if params.skip_cherry_picked {
            args.push("-Rs".into());
        }
    }
    if let Some(file_paths) = &params.file_paths {
        args.extend(file_paths.iter().cloned());
    }
    Ok(json_invocation(args))
}

fn populate_invocation(params: &ModifyStreamsParams) -> Result<P4Invocation> {
    let mut args = vec!["populate".to_string()];
    if params.preview {
        args.push("-n".into());
    }
    if params.force {
        args.push("-F".into());
    }
    if params.reverse {
        args.push("-r".into());
    }
    if params.output_base {
        args.push("-o".into());
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
    if let Some(description) = non_blank(params.description.as_deref()) {
        args.extend(["-d".into(), description.into()]);
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
        args.extend(["-P".into(), parent.into()]);
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
    }
    if let Some(source) = non_blank(params.source_path.as_deref()) {
        args.push(source.into());
    }
    if let Some(target) = non_blank(params.target_path.as_deref()) {
        args.push(target.into());
    }
    Ok(json_invocation(args))
}
```

- [ ] **Step 4: Add stream form patch helper**

Append to `src/p4/forms.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFormPatch {
    pub stream: Option<String>,
    pub stream_type: Option<String>,
    pub parent: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub options: Option<String>,
    pub parent_view: Option<String>,
    pub paths: Option<Vec<String>>,
    pub remapped: Option<Vec<String>>,
    pub ignored: Option<Vec<String>>,
}

pub fn patch_stream_form(existing: &str, patch: &StreamFormPatch) -> Result<String> {
    let mut form = existing.to_string();
    if let Some(stream) = &patch.stream {
        form = replace_single_line_field(&form, "Stream", stream)?;
    }
    if let Some(stream_type) = &patch.stream_type {
        form = replace_single_line_field(&form, "Type", stream_type)?;
    }
    if let Some(parent) = &patch.parent {
        form = replace_single_line_field(&form, "Parent", parent)?;
    }
    if let Some(name) = &patch.name {
        form = replace_single_line_field(&form, "Name", name)?;
    }
    if let Some(description) = &patch.description {
        form = upsert_indented_block(&form, "Description", &indent_lines(description));
    }
    if let Some(options) = &patch.options {
        form = replace_single_line_field(&form, "Options", options)?;
    }
    if let Some(parent_view) = &patch.parent_view {
        form = replace_single_line_field(&form, "ParentView", parent_view)?;
    }
    if let Some(paths) = &patch.paths {
        form = replace_list_block(&form, "Paths", paths)?;
    }
    if let Some(remapped) = &patch.remapped {
        form = replace_list_block(&form, "Remapped", remapped)?;
    }
    if let Some(ignored) = &patch.ignored {
        form = replace_list_block(&form, "Ignored", ignored)?;
    }
    Ok(form)
}

fn replace_list_block(existing: &str, field: &str, values: &[String]) -> Result<String> {
    let replacement = values
        .iter()
        .map(|line| format!("\t{line}\n"))
        .collect::<String>();
    replace_indented_block(existing, field, &replacement)
}
```

- [ ] **Step 5: Migrate stream server internals**

Replace `modify_streams_inner` in `src/server.rs` with:

```rust
async fn modify_streams_inner(
    &self,
    params: ModifyStreamsParams,
    channel: ApprovalChannel,
) -> McpResult<Json<ToolResponse>> {
    self.policy()
        .check(Access::Write, Toolset::Streams, "modify_streams")
        .map_err(to_mcp_error)?;
    let action = params.action.as_str();
    let command = build_stream_modify_command(&params).map_err(to_mcp_error)?;
    let preview_invocation = match &command {
        StreamModifyCommand::Single(invocation) => invocation.clone(),
        StreamModifyCommand::CreateOrUpdate => json_invocation(
            vec!["stream".to_string(), "-i".to_string()],
            Some(format!(
                "Stream: {}\n\n<patched after approval>\n",
                params.stream_name.as_deref().unwrap_or("<required>")
            )),
        ),
    };
    let request = self.modify_streams_approval_request(&params, &preview_invocation);
    if let Some(response) = self
        .require_write_approval(channel, request, params.approval_token.as_deref())
        .await?
    {
        return Ok(response);
    }
    match command {
        StreamModifyCommand::Single(invocation) => self.call_p4_tool(action, invocation).await,
        StreamModifyCommand::CreateOrUpdate => {
            let stream_name = require_non_blank(params.stream_name.as_deref(), "stream_name")
                .map_err(to_mcp_error)?;
            let current = self
                .run_p4(text_invocation(vec![
                    "stream".to_string(),
                    "-o".to_string(),
                    stream_name.clone(),
                ]))
                .await?;
            let current_form = current
                .text
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let patch = StreamFormPatch {
                stream: Some(stream_name),
                stream_type: params.stream_type.clone(),
                parent: params.parent.clone(),
                name: params.name.clone(),
                description: params.description.clone(),
                options: params.options.clone(),
                parent_view: params.parent_view.clone(),
                paths: params.paths.clone(),
                remapped: params.remapped.clone(),
                ignored: params.ignored.clone(),
            };
            let patched = patch_stream_form(current_form, &patch).map_err(to_mcp_error)?;
            self.call_p4_tool(
                action,
                json_invocation(vec!["stream".to_string(), "-i".to_string()], Some(patched)),
            )
            .await
        }
    }
}
```

Add:

```rust
fn modify_streams_approval_request(
    &self,
    params: &ModifyStreamsParams,
    invocation: &P4Invocation,
) -> ApprovalRequest {
    let mut approval_params = params.clone();
    approval_params.approval_token = None;
    ApprovalRequest {
        tool: "modify_streams".to_string(),
        action: params.action.as_str().to_string(),
        params: serde_json::to_value(approval_params)
            .expect("modify streams params serialize to JSON"),
        preview: self.p4_approval_preview(P4ApprovalContext {
            tool: "modify_streams",
            action: params.action.as_str(),
            targets: named_scope_targets(params.stream_name.as_deref(), "stream operation"),
            changelist: params.changelist.clone(),
            workspace: params.workspace.clone().or_else(|| params.workspace_name.clone()),
            stream: params.stream_name.clone(),
            invocation,
        }),
    }
}
```

Update the public route:

```rust
Parameters(params): Parameters<ModifyStreamsParams>,
```

- [ ] **Step 6: Run focused stream tests**

Run:

```bash
rtk cargo test modify_streams_edit_spec_uses_stream_spec_edit_command --test tool_mapping_tests
rtk cargo test modify_streams_copy_uses_upstream_stream_flags --test tool_mapping_tests
rtk cargo test modify_streams_without_approval_does_not_call_executor
```

Expected: all PASS after existing stream modify tests are migrated to `ModifyStreamsParams`.

- [ ] **Step 7: Commit**

```bash
rtk git add src/tools/streams.rs src/p4/forms.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align stream write contract"
```

---

### Task 8: Remove Common Request Types And Run Final Gates

**Files:**
- Modify: `src/tools/params.rs`
- Modify: `src/tools/reviews.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/review_client_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Remove `CommonModifyParams`**

Delete this block from `src/tools/params.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonModifyParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub approval_token: Option<String>,
}
```

- [ ] **Step 2: Remove `ReviewRequest`**

In `src/tools/reviews.rs`, confirm there is no `pub struct ReviewRequest` and no mixed `ReviewAction`. If any remain, delete them and update imports to use `QueryReviewsParams` and `ModifyReviewsParams`.

- [ ] **Step 3: Run grep gates**

Run:

```bash
rtk rg "CommonModifyParams|ReviewRequest" src tests
rtk rg "Parameters<CommonModifyParams>|Parameters<ReviewRequest>" src/server.rs
```

Expected: no output.

- [ ] **Step 4: Run formatting and lint**

Run:

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both PASS.

- [ ] **Step 5: Run full tests**

Run:

```bash
rtk cargo test
```

Expected: all tests PASS. If sandbox blocks WireMock localhost binding with `PermissionDenied`, rerun the same command with escalation in the main agent session and record that the first failure was sandbox-related.

- [ ] **Step 6: Inspect final diff**

Run:

```bash
rtk git status --short
rtk git diff --stat
rtk git diff -- src/tools/params.rs src/tools/reviews.rs src/server.rs
```

Expected:

- No unrelated files changed.
- `CommonModifyParams` is gone.
- `ReviewRequest` is gone.
- Public routes use tool-specific params.
- Approval previews are still built before any write-side P4 fetch or HTTP execution.

- [ ] **Step 7: Commit**

```bash
rtk git add src tests
rtk git commit -m "fix: remove common write request surfaces"
```

---

### Task 9: PR Review Follow-Up

**Files:**
- Modify: none unless GitHub replies are considered external state.

- [ ] **Step 1: Check unresolved review threads**

Run:

```bash
rtk gh api graphql -f owner='j3bit' -f name='p4mcp-server-rs' -F number=2 -f query='query($owner:String!, $name:String!, $number:Int!) { repository(owner:$owner, name:$name) { pullRequest(number:$number) { reviewThreads(first:100) { nodes { id isResolved isOutdated path line comments(first:20) { nodes { databaseId body url author { login } } } } } } } }' --jq '.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false) | {thread: .id, path, line, comments: [.comments.nodes[] | {databaseId, author: .author.login, body, url}]}'
```

Expected: the three known unresolved threads are still present unless already resolved by reviewers.

- [ ] **Step 2: Reply to the changelist thread**

Use the known changelist update comment database ID `3411589868`. Re-check Step 1 output before running this command; if GitHub shows a different unresolved comment, use the rechecked database ID instead.

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3411589868/replies -f body=$'Fixed in the latest push.\n\nThis was a true upstream-parity issue, not an isolated form bug. `modify_changelists` now uses an upstream-shaped public schema with `file_paths`, and description updates no longer synthesize a replacement full form from sparse inputs. After write approval succeeds, the Rust port fetches the existing changelist form with `p4 change -o`, patches only `Description`, and submits it with `p4 change -i`, preserving the existing `Files:` list like upstream `fetch_change()` / `save_change()`.'
```

Expected: GitHub API returns the created reply JSON.

- [ ] **Step 3: Reply to the job thread**

Use the known job comment database ID `3411589872`. Re-check Step 1 output before running this command; if GitHub shows a different unresolved comment, use the rechecked database ID instead.

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3411589872/replies -f body=$'Fixed in the latest push.\n\n`modify_jobs` now exposes the upstream public actions `link_job` and `unlink_job` with a first-class `job_id` field. The direct CLI implementation maps those actions to `p4 fix -c <changelist> <job>` and `p4 fix -d -c <changelist> <job>` after the write approval gate, so the external MCP contract matches upstream while keeping the Rust CLI backend.'
```

Expected: GitHub API returns the created reply JSON.

- [ ] **Step 4: Reply to the review comment-id thread**

Use the known review comment-id comment database ID `3411589878`. Re-check Step 1 output before running this command; if GitHub shows a different unresolved comment, use the rechecked database ID instead.

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3411589878/replies -f body=$'Fixed in the latest push.\n\nThe review API schema is now split into `QueryReviewsParams` and `ModifyReviewsParams`, matching upstream read/write contracts instead of sharing one mixed request type. `reply_comment`, `mark_comment_read`, and `mark_comment_unread` now expose and require `comment_id`; read actions no longer appear on the modify schema and write actions no longer appear on the query schema.'
```

Expected: GitHub API returns the created reply JSON.

- [ ] **Step 5: Resolve the threads**

Resolve the known thread IDs after re-checking Step 1 shows they are still the unresolved threads:

```bash
rtk gh api graphql -f query='mutation($thread:ID!) { resolveReviewThread(input:{threadId:$thread}) { thread { id isResolved } } }' -f thread='PRRT_kwDOS5FH5M6JfYhh'
rtk gh api graphql -f query='mutation($thread:ID!) { resolveReviewThread(input:{threadId:$thread}) { thread { id isResolved } } }' -f thread='PRRT_kwDOS5FH5M6JfYhk'
rtk gh api graphql -f query='mutation($thread:ID!) { resolveReviewThread(input:{threadId:$thread}) { thread { id isResolved } } }' -f thread='PRRT_kwDOS5FH5M6JfYho'
```

Expected: each response has `"isResolved": true`.

- [ ] **Step 6: Push branch**

Run:

```bash
rtk git status --short --branch
rtk git push
```

Expected: clean worktree before push, branch pushed to `origin/codex/add-gitignore`.

---

## Self-Review

Spec coverage:

- Public route `Parameters<CommonModifyParams>` removal: Tasks 2-4, 7, and 8.
- Public route `Parameters<ReviewRequest>` removal: Tasks 5, 6, and 8.
- Tool-specific modify schemas for changelists, shelves, workspaces, jobs, streams: Task 2 plus route tasks 3, 4, and 7.
- Review query/modify schema split and upstream fields: Tasks 5 and 6.
- Fetch-patch-save instead of synthetic update forms: Tasks 3, 4, and 7.
- Approval preview remains side-effect-free: Tasks 3, 4, 6, 7, and final diff check in Task 8.
- Upstream contract tests: Tasks 1, 2, and 5.
- Completion grep gate: Task 8.

Placeholder scan:

- The plan contains no placeholder markers, no unfinished task markers, and no deferred implementation steps.
- GitHub reply and resolve commands use the known review comment and thread IDs, with Task 9 Step 1 requiring a re-check before executing them.

Type consistency:

- Changelist write path uses `ModifyChangelistsParams` and `ChangelistModifyAction`.
- Shelf write path uses `ModifyShelvesParams` and `ShelfModifyAction`.
- Workspace write path uses `ModifyWorkspacesParams` and `WorkspaceModifyAction`.
- Job write path uses `ModifyJobsParams` and `JobModifyAction`.
- Stream write path uses `ModifyStreamsParams` and `StreamModifyAction`.
- Review query path uses `QueryReviewsParams`; review modify path uses `ModifyReviewsParams`.
