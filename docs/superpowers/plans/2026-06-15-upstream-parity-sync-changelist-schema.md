# Upstream Parity Query Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align the unresolved review fixes and every remaining query tool public schema with upstream `perforce/p4mcp-server`, treating files, changelists, shelves, workspaces, jobs, and streams with equal severity.

**Architecture:** Public MCP tool schemas are compatibility surface, so query tools must use tool-specific parameter structs that mirror upstream field names instead of the Rust port's broad `CommonQueryParams`. Keep the existing small CLI builder modules, but expand stream query routing because upstream stream queries expose more actions and options than the current Rust builder. The write approval gate remains an approved Rust safety extension, but query schemas and command behavior should follow upstream unless the upstream behavior depends on P4Python-only internals that must be expressed with direct `p4` CLI calls.

**Tech Stack:** Rust, `rmcp` tool parameter schemas, `serde`, `schemars`, Tokio tests, direct `p4` CLI invocation builders.

---

## Upstream Baseline

- Baseline: `perforce/p4mcp-server` `v2026.2.2955897`, commit `a64efb07511b2a62db41aeed110ab96744c4076a`.
- `p4mcp/handlers/file_handlers.py::_handle_modify_files` rejects missing `file_paths` for `sync` in the actual tool execution path.
- `p4mcp/tools/changelist_tools.py::query_changelists` fields: `action`, `changelist_id`, `workspace_name`, `user`, `status`, `depot_path`, `max_results`.
- `p4mcp/tools/shelve_tools.py::query_shelves` fields: `action`, `changelist_id`, `user`, `max_results`.
- `p4mcp/tools/workspace_tools.py::query_workspaces` fields: `action`, `workspace_name`, `user`, `max_results`.
- `p4mcp/tools/job_tools.py::query_jobs` fields: `action`, `changelist_id`, `job_id`, `max_results`.
- `p4mcp/tools/stream_tools.py::query_streams` fields: `action`, `stream_name`, `stream_path`, `filter`, `fields`, `unloaded`, `all_streams`, `viewmatch`, `view_without_edit`, `at_change`, `both_directions`, `force_refresh`, `workspace`, `template`, `user`, `file_paths`, `changelist`, `reverse`, `long_output`, `limit`, `max_results`.

## File Structure

- Modify `src/tools/params.rs`
  - Add tool-specific query action enums and query params for changelists, shelves, workspaces, jobs, and streams.
  - Remove `CommonQueryParams` after all query tools stop using it.
- Modify `src/tools/files.rs`
  - Require `file_paths` for `FileModifyAction::Sync`.
- Modify `src/tools/changelists.rs`
  - Add `depot_path` support to `p4 changes`.
- Modify `src/tools/jobs.rs`
  - Remove the Rust-only `query_jobs` action `"list"`.
- Modify `src/tools/streams.rs`
  - Replace the current stream query builder with an upstream-shaped builder for single-command stream queries.
  - Add `StreamQueryCommand` to represent stream actions that need server-side multi-command handling.
- Modify `src/server.rs`
  - Switch `query_changelists`, `query_shelves`, `query_workspaces`, `query_jobs`, and `query_streams` to their tool-specific params.
  - Add stream helper methods for upstream query actions that cannot be represented as one `P4Invocation`.
- Modify `tests/tool_mapping_tests.rs`
  - Add public schema tests for every query params type.
  - Add direct builder tests for sync, changelist depot path, jobs action removal, and stream command mapping.
- Modify `tests/mcp_smoke_tests.rs`
  - Replace all query tool calls that use `CommonQueryParams`.
  - Add smoke coverage for the stream actions that need multi-command routing.

## Design Decisions

- Treat every query tool equally. Do not align only `query_changelists` while leaving `query_shelves`, `query_workspaces`, `query_jobs`, or `query_streams` with a generic public schema.
- Keep tool-specific params as the public schema and pass scalar arguments or params references to builders internally. Do not preserve a generic query DTO after this change.
- Remove Rust-only query action exposure in this PR. `query_jobs.action = "list"` is not upstream and should be removed.
- For `query_streams`, match upstream public actions. Existing Rust behavior that is narrower than upstream must be expanded, not deferred solely because streams are larger.

## Task 1: Require Files For Sync

**Files:**
- Modify: `src/tools/files.rs`
- Test: `tests/tool_mapping_tests.rs`
- Test: `src/server.rs`

- [ ] **Step 1: Add direct builder regression test for missing sync files**

In `tests/tool_mapping_tests.rs`, add this test immediately after `modify_file_delete_requires_files`:

```rust
#[test]
fn modify_file_sync_requires_files() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Sync,
        file_paths: None,
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("file_paths is required for sync"));
}
```

- [ ] **Step 2: Run the focused failing builder test**

Run:

```bash
rtk cargo test modify_file_sync_requires_files --test tool_mapping_tests
```

Expected: FAIL because the current builder returns plain `["sync"]`.

- [ ] **Step 3: Require files in the sync builder arm**

In `src/tools/files.rs`, replace the `FileModifyAction::Sync` arm with:

```rust
        FileModifyAction::Sync => {
            require_files(&files, "sync")?;
            let mut args = vec!["sync".to_string()];
            if params.force {
                args.push("-f".to_string());
            }
            args.extend(files);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
```

- [ ] **Step 4: Add server-level invalid sync test**

In `src/server.rs`, inside the `#[cfg(test)] mod tests` block, add this test immediately after `modify_files_without_approval_does_not_call_executor`:

```rust
    #[tokio::test]
    async fn modify_files_sync_without_files_rejects_before_approval() {
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
        params.file_paths = None;

        let err = match server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("sync without file_paths should be rejected"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("file_paths is required for sync"));
        assert!(executor.invocations().is_empty());
        assert!(approval_gate.calls().is_empty());
    }
```

- [ ] **Step 5: Run focused sync tests**

Run:

```bash
rtk cargo test modify_file_sync --test tool_mapping_tests
rtk cargo test modify_files_sync_without_files_rejects_before_approval
```

Expected: PASS.

- [ ] **Step 6: Commit sync parity fix**

Run:

```bash
rtk git add src/tools/files.rs tests/tool_mapping_tests.rs src/server.rs
rtk git commit -m "fix: require files for sync"
```

Expected: commit succeeds.

## Task 2: Define Tool-Specific Query Params

**Files:**
- Modify: `src/tools/params.rs`
- Test: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add schema tests for upstream query params**

In `tests/tool_mapping_tests.rs`, update the params import block to include all new params and action enums:

```rust
        params::{
            ChangelistQueryAction, CommonModifyParams, FileModifyAction, FileQueryAction,
            ModifyFilesParams, QueryChangelistsParams, QueryFilesParams, QueryJobsParams,
            QueryShelvesParams, QueryStreamsParams, QueryWorkspacesParams, StreamQueryAction,
        },
```

Then add these schema tests after `modify_file_params_schema_omits_confirmation`:

```rust
#[test]
fn query_changelists_schema_matches_upstream_fields() {
    assert!(schema_has_property::<QueryChangelistsParams>("depot_path"));
    assert!(!schema_has_property::<QueryChangelistsParams>("file_path"));
    assert!(!schema_has_property::<QueryChangelistsParams>("stream"));
    assert!(!schema_has_property::<QueryChangelistsParams>("owner"));
}

#[test]
fn query_shelves_schema_matches_upstream_fields() {
    assert!(schema_has_property::<QueryShelvesParams>("changelist_id"));
    assert!(schema_has_property::<QueryShelvesParams>("user"));
    assert!(!schema_has_property::<QueryShelvesParams>("workspace_name"));
    assert!(!schema_has_property::<QueryShelvesParams>("file_path"));
}

#[test]
fn query_workspaces_schema_matches_upstream_fields() {
    assert!(schema_has_property::<QueryWorkspacesParams>("workspace_name"));
    assert!(schema_has_property::<QueryWorkspacesParams>("user"));
    assert!(!schema_has_property::<QueryWorkspacesParams>("changelist_id"));
    assert!(!schema_has_property::<QueryWorkspacesParams>("file_path"));
}

#[test]
fn query_jobs_schema_matches_upstream_fields() {
    assert!(schema_has_property::<QueryJobsParams>("changelist_id"));
    assert!(schema_has_property::<QueryJobsParams>("job_id"));
    assert!(!schema_has_property::<QueryJobsParams>("workspace_name"));
    assert!(!schema_has_property::<QueryJobsParams>("file_path"));
}

#[test]
fn query_streams_schema_matches_upstream_fields() {
    assert!(schema_has_property::<QueryStreamsParams>("stream_name"));
    assert!(schema_has_property::<QueryStreamsParams>("stream_path"));
    assert!(schema_has_property::<QueryStreamsParams>("filter"));
    assert!(schema_has_property::<QueryStreamsParams>("fields"));
    assert!(schema_has_property::<QueryStreamsParams>("viewmatch"));
    assert!(schema_has_property::<QueryStreamsParams>("workspace"));
    assert!(schema_has_property::<QueryStreamsParams>("file_paths"));
    assert!(schema_has_property::<QueryStreamsParams>("changelist"));
    assert!(!schema_has_property::<QueryStreamsParams>("stream"));
    assert!(!schema_has_property::<QueryStreamsParams>("owner"));
}
```

Add these deserialization tests after the schema tests:

```rust
#[test]
fn query_changelists_params_deserialize_list_action_with_depot_path() {
    let params: QueryChangelistsParams = serde_json::from_value(serde_json::json!({
        "action": "list",
        "workspace_name": "ws-main",
        "user": "alice",
        "status": "pending",
        "depot_path": "//depot/main/...",
        "max_results": 7
    }))
    .unwrap();

    assert_eq!(params.action, ChangelistQueryAction::List);
    assert_eq!(params.action.as_str(), "list");
    assert_eq!(params.depot_path.as_deref(), Some("//depot/main/..."));
}

#[test]
fn query_streams_params_deserialize_upstream_list_fields() {
    let params: QueryStreamsParams = serde_json::from_value(serde_json::json!({
        "action": "list",
        "stream_path": ["//depot/..."],
        "filter": "Owner=alice",
        "fields": ["Stream", "Owner", "Type"],
        "unloaded": true,
        "all_streams": true,
        "viewmatch": "//depot/main/file.txt",
        "max_results": 25
    }))
    .unwrap();

    assert_eq!(params.action, StreamQueryAction::List);
    assert_eq!(params.action.as_str(), "list");
    assert_eq!(params.stream_path.as_deref(), Some(&["//depot/...".to_string()][..]));
    assert_eq!(params.filter.as_deref(), Some("Owner=alice"));
    assert_eq!(params.fields.as_deref(), Some(&["Stream".to_string(), "Owner".to_string(), "Type".to_string()][..]));
    assert!(params.unloaded);
    assert!(params.all_streams);
    assert_eq!(params.viewmatch.as_deref(), Some("//depot/main/file.txt"));
    assert_eq!(params.max_results, 25);
}
```

- [ ] **Step 2: Run the focused failing schema tests**

Run:

```bash
rtk cargo test query_changelists_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_streams_params_deserialize_upstream_list_fields --test tool_mapping_tests
```

Expected: FAIL because the new types do not exist yet.

- [ ] **Step 3: Add query params and action enums**

In `src/tools/params.rs`, insert this code immediately after `ModifyFilesParams`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangelistQueryAction {
    Get,
    List,
}

impl ChangelistQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::List => "list",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryChangelistsParams {
    pub action: ChangelistQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub depot_path: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShelfQueryAction {
    List,
    Diff,
    Files,
}

impl ShelfQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Diff => "diff",
            Self::Files => "files",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryShelvesParams {
    pub action: ShelfQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceQueryAction {
    List,
    Get,
    Type,
    Status,
}

impl WorkspaceQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Get => "get",
            Self::Type => "type",
            Self::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryWorkspacesParams {
    pub action: WorkspaceQueryAction,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobQueryAction {
    ListJobs,
    GetJob,
}

impl JobQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ListJobs => "list_jobs",
            Self::GetJob => "get_job",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryJobsParams {
    pub action: JobQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamQueryAction {
    List,
    Get,
    Children,
    Parent,
    Graph,
    IntegrationStatus,
    GetWorkspace,
    ListWorkspaces,
    ValidateFile,
    ValidateSubmit,
    CheckResolve,
    Interchanges,
}

impl StreamQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Get => "get",
            Self::Children => "children",
            Self::Parent => "parent",
            Self::Graph => "graph",
            Self::IntegrationStatus => "integration_status",
            Self::GetWorkspace => "get_workspace",
            Self::ListWorkspaces => "list_workspaces",
            Self::ValidateFile => "validate_file",
            Self::ValidateSubmit => "validate_submit",
            Self::CheckResolve => "check_resolve",
            Self::Interchanges => "interchanges",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryStreamsParams {
    pub action: StreamQueryAction,
    #[serde(default)]
    pub stream_name: Option<String>,
    #[serde(default)]
    pub stream_path: Option<Vec<String>>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub unloaded: bool,
    #[serde(default)]
    pub all_streams: bool,
    #[serde(default)]
    pub viewmatch: Option<String>,
    #[serde(default)]
    pub view_without_edit: bool,
    #[serde(default)]
    pub at_change: Option<String>,
    #[serde(default)]
    pub both_directions: bool,
    #[serde(default)]
    pub force_refresh: bool,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub changelist: Option<String>,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub long_output: bool,
    #[serde(default)]
    pub limit: Option<u16>,
    #[serde(default = "default_stream_max_results")]
    pub max_results: u16,
}
```

Then add this helper after `default_max_results`:

```rust
fn default_stream_max_results() -> u16 {
    50
}
```

- [ ] **Step 4: Remove CommonQueryParams after server migration**

Do not delete `CommonQueryParams` in this task. Delete it in Task 6 after all server methods and tests stop importing it.

- [ ] **Step 5: Run schema tests**

Run:

```bash
rtk cargo test query_changelists_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_shelves_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_workspaces_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_jobs_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_streams_schema_matches_upstream_fields --test tool_mapping_tests
rtk cargo test query_streams_params_deserialize_upstream_list_fields --test tool_mapping_tests
```

Expected: PASS after the types are added.

- [ ] **Step 6: Commit query params**

Run:

```bash
rtk git add src/tools/params.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: add upstream query params"
```

Expected: commit succeeds.

## Task 3: Align Changelists, Shelves, Workspaces, And Jobs

**Files:**
- Modify: `src/tools/changelists.rs`
- Modify: `src/tools/jobs.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add direct mapping tests**

In `tests/tool_mapping_tests.rs`, update existing `build_changelist_query_invocation` calls to include the new `depot_path` argument. For example:

```rust
let invocation =
    build_changelist_query_invocation("get", Some("default"), None, None, None, None, 10)
        .unwrap();
```

Add this test after `changelist_get_blank_id_errors`:

```rust
#[test]
fn changelist_list_appends_depot_path_filter() {
    let invocation = build_changelist_query_invocation(
        "list",
        None,
        Some("pending"),
        Some("ws-main"),
        Some("alice"),
        Some("//depot/main/..."),
        7,
    )
    .unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "changes",
            "-m",
            "7",
            "-s",
            "pending",
            "-c",
            "ws-main",
            "-u",
            "alice",
            "//depot/main/..."
        ]
    );
}
```

Add this test after `job_get_blank_id_errors`:

```rust
#[test]
fn job_query_rejects_non_upstream_list_action() {
    let error = build_job_query_invocation("list", None, None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown action: list"));
}
```

- [ ] **Step 2: Run focused failing mapping tests**

Run:

```bash
rtk cargo test changelist_list_appends_depot_path_filter --test tool_mapping_tests
rtk cargo test job_query_rejects_non_upstream_list_action --test tool_mapping_tests
```

Expected: FAIL because `depot_path` is not supported and `query_jobs.list` is still accepted.

- [ ] **Step 3: Extend changelist query builder**

In `src/tools/changelists.rs`, replace the query builder signature with:

```rust
pub fn build_changelist_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    status: Option<&str>,
    workspace_name: Option<&str>,
    user: Option<&str>,
    depot_path: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
```

Then replace the `"list"` arm with:

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
            if let Some(depot_path) = depot_path {
                args.push(depot_path.into());
            }
            args
        }
```

- [ ] **Step 4: Remove non-upstream jobs list arm**

In `src/tools/jobs.rs`, delete this arm from `build_job_query_invocation`:

```rust
        "list" => vec!["jobs".into(), "-m".into(), max_results.to_string()],
```

Keep the `max_results` parameter because the function signature is still used by the public `QueryJobsParams`; prefix it in the signature to avoid an unused warning:

```rust
pub fn build_job_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    job_id: Option<&str>,
    _max_results: u16,
) -> Result<P4Invocation> {
```

- [ ] **Step 5: Switch server methods to tool-specific params**

In `src/server.rs`, replace the params import with:

```rust
        params::{
            CommonModifyParams, CommonQueryParams, FileQueryAction, ModifyFilesParams,
            QueryChangelistsParams, QueryFilesParams, QueryJobsParams, QueryShelvesParams,
            QueryWorkspacesParams,
        },
```

Replace `query_changelists` with:

```rust
    pub async fn query_changelists(
        &self,
        Parameters(params): Parameters<QueryChangelistsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Changelists, "query_changelists")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_changelist_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.status.as_deref(),
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.depot_path.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }
```

Replace `query_shelves` with:

```rust
    pub async fn query_shelves(
        &self,
        Parameters(params): Parameters<QueryShelvesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Shelves, "query_shelves")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_shelf_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }
```

Replace `query_workspaces` with:

```rust
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<QueryWorkspacesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        if action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
        if action == "status" {
            return self
                .query_workspace_status(params.workspace_name.as_deref())
                .await;
        }
        let invocation = build_workspace_query_invocation(
            action,
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }
```

Replace `query_jobs` with:

```rust
    pub async fn query_jobs(
        &self,
        Parameters(params): Parameters<QueryJobsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Jobs, "query_jobs")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_job_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.job_id.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }
```

- [ ] **Step 6: Update smoke test imports and calls**

In `tests/mcp_smoke_tests.rs`, replace the params import with:

```rust
        params::{
            ChangelistQueryAction, FileQueryAction, QueryChangelistsParams, QueryFilesParams,
            QueryWorkspacesParams, WorkspaceQueryAction,
        },
```

Update `query_changelists_calls_injected_executor` to use `QueryChangelistsParams`:

```rust
    let response = server
        .query_changelists(Parameters(QueryChangelistsParams {
            action: ChangelistQueryAction::List,
            changelist_id: None,
            workspace_name: Some("ws-main".to_string()),
            user: Some("alice".to_string()),
            status: Some("pending".to_string()),
            depot_path: Some("//depot/main/...".to_string()),
            max_results: 7,
        }))
        .await
        .unwrap();
```

Update every `query_workspaces` test to use `QueryWorkspacesParams`. For example, in `query_workspaces_list_by_user_calls_injected_executor`:

```rust
    let response = server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::List,
            workspace_name: None,
            user: Some("alice".to_string()),
            max_results: 7,
        }))
        .await
        .unwrap();
```

- [ ] **Step 7: Run focused small query tests**

Run:

```bash
rtk cargo test changelist_list_appends_depot_path_filter --test tool_mapping_tests
rtk cargo test job_query_rejects_non_upstream_list_action --test tool_mapping_tests
rtk cargo test query_changelists_calls_injected_executor --test mcp_smoke_tests
rtk cargo test query_workspaces --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 8: Commit small query tool alignment**

Run:

```bash
rtk git add src/tools/changelists.rs src/tools/jobs.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align core query schemas"
```

Expected: commit succeeds.

## Task 4: Align Stream Query Public Schema And Single-Command Actions

**Files:**
- Modify: `src/tools/streams.rs`
- Modify: `src/server.rs`
- Modify: `tests/tool_mapping_tests.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add stream builder tests**

In `tests/tool_mapping_tests.rs`, update stream builder tests to construct `QueryStreamsParams`.

Update the streams import from:

```rust
        streams::build_stream_query_invocation,
```

to:

```rust
        streams::build_stream_query_command,
```

Replace `stream_list_with_owner_uses_owner_filter` with:

```rust
#[test]
fn stream_list_uses_upstream_filters() {
    let params = QueryStreamsParams {
        action: StreamQueryAction::List,
        stream_name: None,
        stream_path: Some(vec!["//depot/...".to_string()]),
        filter: Some("Owner=alice".to_string()),
        fields: Some(vec!["Stream".to_string(), "Owner".to_string(), "Type".to_string()]),
        unloaded: true,
        all_streams: true,
        viewmatch: Some("//depot/main/file.txt".to_string()),
        view_without_edit: false,
        at_change: None,
        both_directions: false,
        force_refresh: false,
        workspace: None,
        template: None,
        user: None,
        file_paths: None,
        changelist: None,
        reverse: false,
        long_output: false,
        limit: None,
        max_results: 25,
    };

    let command = build_stream_query_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();
    assert_eq!(
        invocation.args,
        vec![
            "streams",
            "-U",
            "-a",
            "-F",
            "Owner=alice",
            "-T",
            "Stream,Owner,Type",
            "-m",
            "25",
            "--viewmatch",
            "//depot/main/file.txt",
            "//depot/..."
        ]
    );
}
```

Replace `stream_integration_status_uses_istat` with:

```rust
#[test]
fn stream_integration_status_uses_upstream_flags() {
    let params = QueryStreamsParams {
        action: StreamQueryAction::IntegrationStatus,
        stream_name: Some("//streams/dev".to_string()),
        stream_path: None,
        filter: None,
        fields: None,
        unloaded: false,
        all_streams: false,
        viewmatch: None,
        view_without_edit: false,
        at_change: None,
        both_directions: true,
        force_refresh: true,
        workspace: None,
        template: None,
        user: None,
        file_paths: None,
        changelist: None,
        reverse: false,
        long_output: false,
        limit: None,
        max_results: 10,
    };

    let command = build_stream_query_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();
    assert_eq!(invocation.args, vec!["istat", "-a", "-c", "//streams/dev"]);
}
```

Replace `stream_get_blank_name_errors` with:

```rust
#[test]
fn stream_get_blank_name_errors() {
    let params = QueryStreamsParams {
        action: StreamQueryAction::Get,
        stream_name: Some(" ".to_string()),
        stream_path: None,
        filter: None,
        fields: None,
        unloaded: false,
        all_streams: false,
        viewmatch: None,
        view_without_edit: false,
        at_change: None,
        both_directions: false,
        force_refresh: false,
        workspace: None,
        template: None,
        user: None,
        file_paths: None,
        changelist: None,
        reverse: false,
        long_output: false,
        limit: None,
        max_results: 10,
    };

    let error = build_stream_query_command(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stream_name is required"));
}
```

- [ ] **Step 2: Run focused failing stream builder tests**

Run:

```bash
rtk cargo test stream_list_uses_upstream_filters --test tool_mapping_tests
rtk cargo test stream_integration_status_uses_upstream_flags --test tool_mapping_tests
rtk cargo test stream_get_blank_name_errors --test tool_mapping_tests
```

Expected: FAIL because `build_stream_query_command` does not exist yet.

- [ ] **Step 3: Replace stream query builder API**

In `src/tools/streams.rs`, replace the file with this implementation:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{QueryStreamsParams, StreamQueryAction},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamQueryCommand {
    Single(P4Invocation),
    Parent { stream_name: String },
    Graph { stream_name: String },
    ValidateFile {
        workspace: Option<String>,
        file_paths: Vec<String>,
    },
    ValidateSubmit {
        workspace: Option<String>,
        changelist: Option<String>,
    },
    CheckResolve { stream_name: String },
    Interchanges {
        stream_name: String,
        reverse: bool,
        file_paths: Vec<String>,
        long_output: bool,
        limit: Option<u16>,
    },
}

impl StreamQueryCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            _ => None,
        }
    }
}

pub fn build_stream_query_command(params: &QueryStreamsParams) -> Result<StreamQueryCommand> {
    match params.action {
        StreamQueryAction::List => Ok(StreamQueryCommand::Single(stream_list_invocation(params))),
        StreamQueryAction::Get => Ok(StreamQueryCommand::Single(stream_get_invocation(params)?)),
        StreamQueryAction::Children => Ok(StreamQueryCommand::Single(stream_children_invocation(params)?)),
        StreamQueryAction::Parent => Ok(StreamQueryCommand::Parent {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Graph => Ok(StreamQueryCommand::Graph {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::IntegrationStatus => Ok(StreamQueryCommand::Single(stream_istat_invocation(params))),
        StreamQueryAction::GetWorkspace => Ok(StreamQueryCommand::Single(stream_get_workspace_invocation(params))),
        StreamQueryAction::ListWorkspaces => Ok(StreamQueryCommand::Single(stream_list_workspaces_invocation(params))),
        StreamQueryAction::ValidateFile => Ok(StreamQueryCommand::ValidateFile {
            workspace: params.workspace.clone(),
            file_paths: required_files(params.file_paths.as_deref(), "file_paths")?,
        }),
        StreamQueryAction::ValidateSubmit => Ok(StreamQueryCommand::ValidateSubmit {
            workspace: params.workspace.clone(),
            changelist: params.changelist.clone(),
        }),
        StreamQueryAction::CheckResolve => Ok(StreamQueryCommand::CheckResolve {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Interchanges => Ok(StreamQueryCommand::Interchanges {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
            reverse: params.reverse,
            file_paths: params.file_paths.clone().unwrap_or_default(),
            long_output: params.long_output,
            limit: params.limit,
        }),
    }
}

pub fn build_stream_query_invocation(
    action: &str,
    stream: Option<&str>,
    owner: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let params = QueryStreamsParams {
        action: legacy_stream_action(action)?,
        stream_name: stream.map(str::to_string),
        stream_path: None,
        filter: owner.map(|owner| format!("Owner={owner}")),
        fields: None,
        unloaded: false,
        all_streams: false,
        viewmatch: None,
        view_without_edit: false,
        at_change: None,
        both_directions: false,
        force_refresh: false,
        workspace: None,
        template: None,
        user: None,
        file_paths: None,
        changelist: None,
        reverse: false,
        long_output: false,
        limit: None,
        max_results,
    };
    match build_stream_query_command(&params)? {
        StreamQueryCommand::Single(invocation) => Ok(invocation),
        _ => Err(P4McpError::InvalidInput {
            message: format!("action requires server routing: {action}"),
        }),
    }
}

fn legacy_stream_action(action: &str) -> Result<StreamQueryAction> {
    match action {
        "list" => Ok(StreamQueryAction::List),
        "get" => Ok(StreamQueryAction::Get),
        "children" => Ok(StreamQueryAction::Children),
        "parent" => Ok(StreamQueryAction::Parent),
        "graph" => Ok(StreamQueryAction::Graph),
        "integration_status" => Ok(StreamQueryAction::IntegrationStatus),
        "get_workspace" => Ok(StreamQueryAction::GetWorkspace),
        "list_workspaces" => Ok(StreamQueryAction::ListWorkspaces),
        other => Err(P4McpError::InvalidInput {
            message: format!("unknown action: {other}"),
        }),
    }
}

fn stream_list_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["streams".into()];
    if params.unloaded {
        args.push("-U".into());
    }
    if params.all_streams {
        args.push("-a".into());
    }
    if let Some(filter) = non_blank(params.filter.as_deref()) {
        args.extend(["-F".into(), filter.into()]);
    }
    if let Some(fields) = params.fields.as_deref().filter(|fields| !fields.is_empty()) {
        args.extend(["-T".into(), fields.join(",")]);
    }
    args.extend(["-m".into(), params.max_results.to_string()]);
    if let Some(viewmatch) = non_blank(params.viewmatch.as_deref()) {
        args.extend(["--viewmatch".into(), viewmatch.into()]);
    }
    if let Some(stream_path) = params.stream_path.as_deref() {
        args.extend(stream_path.iter().cloned());
    }
    json_invocation(args)
}

fn stream_get_invocation(params: &QueryStreamsParams) -> Result<P4Invocation> {
    let stream_name = required(params.stream_name.as_deref(), "stream_name")?;
    let mut args = vec!["stream".into(), "-o".into()];
    if params.view_without_edit {
        args.push("-v".into());
    }
    let specifier = match non_blank(params.at_change.as_deref()) {
        Some(change) => format!("{stream_name}@{change}"),
        None => stream_name,
    };
    args.push(specifier);
    Ok(json_invocation(args))
}

fn stream_children_invocation(params: &QueryStreamsParams) -> Result<P4Invocation> {
    Ok(json_invocation(vec![
        "streams".into(),
        "-F".into(),
        format!("Parent={}", required(params.stream_name.as_deref(), "stream_name")?),
    ]))
}

fn stream_istat_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["istat".into()];
    if params.both_directions {
        args.push("-a".into());
    }
    if params.force_refresh {
        args.push("-c".into());
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.push(stream_name.into());
    }
    json_invocation(args)
}

fn stream_get_workspace_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(template) = non_blank(params.template.as_deref()) {
        args.extend(["-t".into(), template.into()]);
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(workspace) = non_blank(params.workspace.as_deref()) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

fn stream_list_workspaces_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["clients".into()];
    if params.unloaded {
        args.push("-U".into());
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(user) = non_blank(params.user.as_deref()) {
        args.extend(["-u".into(), user.into()]);
    }
    args.extend(["-m".into(), params.max_results.to_string()]);
    json_invocation(args)
}

pub fn interchanges_invocation(
    stream_name: &str,
    reverse: bool,
    file_paths: &[String],
    long_output: bool,
) -> P4Invocation {
    let mut args = vec!["interchanges".into(), "-S".into(), stream_name.to_string()];
    if reverse {
        args.push("-r".into());
    }
    if long_output {
        args.push("-l".into());
    }
    args.extend(file_paths.iter().cloned());
    json_invocation(args)
}

pub fn opened_for_stream_validation_invocation(workspace: Option<&str>, changelist: Option<&str>) -> P4Invocation {
    let mut args = vec!["opened".into()];
    if let Some(changelist) = non_blank(changelist) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(workspace) = non_blank(workspace) {
        args.extend(["-C".into(), workspace.into()]);
    }
    json_invocation(args)
}

pub fn client_spec_invocation(workspace: Option<&str>) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(workspace) = non_blank(workspace) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

pub fn stream_spec_with_view_invocation(stream_name: &str) -> P4Invocation {
    json_invocation(vec!["stream".into(), "-o".into(), "-v".into(), stream_name.into()])
}

pub fn stream_resolve_preview_invocation() -> P4Invocation {
    json_invocation(vec!["stream".into(), "resolve".into(), "-n".into()])
}

fn json_invocation(args: Vec<String>) -> P4Invocation {
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    match non_blank(value) {
        Some(value) => Ok(value.to_string()),
        None => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}

fn required_files(files: Option<&[String]>, name: &str) -> Result<Vec<String>> {
    match files {
        Some(files) if !files.is_empty() => Ok(files.to_vec()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}
```

- [ ] **Step 4: Keep server compiling before stream routing migration**

Do not change `src/server.rs` in this task. The temporary `build_stream_query_invocation` wrapper in `src/tools/streams.rs` keeps the existing server route compiling until Task 5 replaces `query_streams`.

- [ ] **Step 5: Run focused stream builder tests**

Run:

```bash
rtk cargo test stream_list_uses_upstream_filters --test tool_mapping_tests
rtk cargo test stream_integration_status_uses_upstream_flags --test tool_mapping_tests
rtk cargo test stream_get_blank_name_errors --test tool_mapping_tests
```

Expected: PASS.

- [ ] **Step 6: Commit stream builder alignment**

Run:

```bash
rtk git add src/tools/streams.rs src/server.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: align stream query builder"
```

Expected: commit succeeds.

## Task 5: Route Stream Multi-Command Actions

**Files:**
- Modify: `src/server.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add stream smoke tests**

In `tests/mcp_smoke_tests.rs`, update params imports to include stream params:

```rust
            ChangelistQueryAction, FileQueryAction, QueryChangelistsParams, QueryFilesParams,
            QueryStreamsParams, QueryWorkspacesParams, StreamQueryAction, WorkspaceQueryAction,
```

Add this helper near existing test helper functions:

```rust
fn stream_query_params(action: StreamQueryAction) -> QueryStreamsParams {
    QueryStreamsParams {
        action,
        stream_name: Some("//streams/dev".to_string()),
        stream_path: None,
        filter: None,
        fields: None,
        unloaded: false,
        all_streams: false,
        viewmatch: None,
        view_without_edit: false,
        at_change: None,
        both_directions: false,
        force_refresh: false,
        workspace: None,
        template: None,
        user: None,
        file_paths: None,
        changelist: None,
        reverse: false,
        long_output: false,
        limit: None,
        max_results: 50,
    }
}
```

Add this test after `query_workspaces_status_requires_workspace_name`:

```rust
#[tokio::test]
async fn query_streams_interchanges_runs_upstream_command_and_limits_client_side() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![
            json!({"change": "101"}),
            json!({"change": "102"}),
            json!({"change": "103"}),
        ],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Interchanges);
    params.reverse = true;
    params.long_output = true;
    params.limit = Some(2);
    params.file_paths = Some(vec!["//streams/dev/src/...".to_string()]);

    let response = server
        .query_streams(Parameters(params))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "interchanges");
    assert_eq!(
        response.0.message,
        json!([{"change": "101"}, {"change": "102"}])
    );
    assert_eq!(
        executor.invocations()[0].args,
        ["interchanges", "-S", "//streams/dev", "-r", "-l", "//streams/dev/src/..."]
    );
}
```

Add this test after the interchanges test:

```rust
#[tokio::test]
async fn query_streams_check_resolve_runs_preview_after_stream_check() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"resolve": "needed"})],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_streams(Parameters(stream_query_params(StreamQueryAction::CheckResolve)))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "check_resolve");
    assert_eq!(response.0.message["resolve_needed"], json!(true));
    let invocations = executor.invocations();
    assert_eq!(invocations[0].args, ["streams", "-F", "Stream=//streams/dev"]);
    assert_eq!(invocations[1].args, ["stream", "resolve", "-n"]);
}
```

- [ ] **Step 2: Run focused failing stream smoke tests**

Run:

```bash
rtk cargo test query_streams_interchanges_runs_upstream_command_and_limits_client_side --test mcp_smoke_tests
rtk cargo test query_streams_check_resolve_runs_preview_after_stream_check --test mcp_smoke_tests
```

Expected: FAIL because `query_streams` still routes through the old single-invocation builder.

- [ ] **Step 3: Replace query_streams routing**

In `src/server.rs`, update the params import by removing `CommonQueryParams` and adding `QueryStreamsParams`:

```rust
        params::{
            CommonModifyParams, FileQueryAction, ModifyFilesParams, QueryChangelistsParams,
            QueryFilesParams, QueryJobsParams, QueryShelvesParams, QueryStreamsParams,
            QueryWorkspacesParams,
        },
```

Then replace `query_streams` with:

```rust
    pub async fn query_streams(
        &self,
        Parameters(params): Parameters<QueryStreamsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Streams, "query_streams")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let command = build_stream_query_command(&params).map_err(to_mcp_error)?;
        match command {
            StreamQueryCommand::Single(invocation) => self.call_p4_tool(action, invocation).await,
            StreamQueryCommand::Parent { stream_name } => {
                self.query_stream_parent(action, &stream_name).await
            }
            StreamQueryCommand::Graph { stream_name } => {
                self.query_stream_graph(action, &stream_name).await
            }
            StreamQueryCommand::ValidateFile {
                workspace,
                file_paths,
            } => {
                self.query_stream_validate_file(action, workspace.as_deref(), &file_paths)
                    .await
            }
            StreamQueryCommand::ValidateSubmit {
                workspace,
                changelist,
            } => {
                self.query_stream_validate_submit(action, workspace.as_deref(), changelist.as_deref())
                    .await
            }
            StreamQueryCommand::CheckResolve { stream_name } => {
                self.query_stream_check_resolve(action, &stream_name).await
            }
            StreamQueryCommand::Interchanges {
                stream_name,
                reverse,
                file_paths,
                long_output,
                limit,
            } => {
                self.query_stream_interchanges(
                    action,
                    &stream_name,
                    reverse,
                    &file_paths,
                    long_output,
                    limit,
                )
                .await
            }
        }
    }
```

- [ ] **Step 4: Add stream server helpers**

In `src/server.rs`, add these helper methods inside `impl P4McpServer`, immediately after `query_workspace_status`:

```rust
    async fn query_stream_parent(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self
            .run_p4(json_invocation(vec![
                "stream".into(),
                "-o".into(),
                stream_name.into(),
            ], None))
            .await?;
        let parent = output
            .records
            .first()
            .and_then(|record| record.get("Parent"))
            .cloned()
            .unwrap_or(json!(null));
        Ok(Json(ToolResponse::success(action, parent)))
    }

    async fn query_stream_graph(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        let stream = self
            .run_p4(json_invocation(vec![
                "stream".into(),
                "-o".into(),
                stream_name.into(),
            ], None))
            .await?;
        let children = self
            .run_p4(json_invocation(vec![
                "streams".into(),
                "-F".into(),
                format!("Parent={stream_name}"),
            ], None))
            .await?;
        let parent = stream
            .records
            .first()
            .and_then(|record| record.get("Parent"))
            .cloned()
            .unwrap_or(json!(null));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "stream": stream_name,
                "parent": parent,
                "children": children.records
            }),
        )))
    }

    async fn query_stream_validate_file(
        &self,
        action: &str,
        workspace: Option<&str>,
        file_paths: &[String],
    ) -> McpResult<Json<ToolResponse>> {
        let client = self.run_p4(client_spec_invocation(workspace)).await?;
        let stream = required_record_field(&client.records, "Stream")
            .map_err(to_mcp_error)?;
        let stream_spec = self
            .run_p4(stream_spec_with_view_invocation(&stream))
            .await?;
        let paths = collect_record_strings(&stream_spec.records, "Paths");
        let ignored = collect_record_strings(&stream_spec.records, "Ignored");
        let results = file_paths
            .iter()
            .map(|file| classify_stream_file(file, &stream, &paths, &ignored))
            .collect::<Vec<_>>();
        let all_allowed = results
            .iter()
            .all(|result| result.get("allowed").and_then(|value| value.as_bool()) == Some(true));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "all_allowed": all_allowed,
                "stream": stream,
                "workspace": workspace,
                "file_count": file_paths.len(),
                "results": results
            }),
        )))
    }

    async fn query_stream_validate_submit(
        &self,
        action: &str,
        workspace: Option<&str>,
        changelist: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let client = self.run_p4(client_spec_invocation(workspace)).await?;
        let stream = required_record_field(&client.records, "Stream")
            .map_err(to_mcp_error)?;
        let opened = self
            .run_p4(opened_for_stream_validation_invocation(workspace, changelist))
            .await?;
        let stream_spec = self
            .run_p4(stream_spec_with_view_invocation(&stream))
            .await?;
        let paths = collect_record_strings(&stream_spec.records, "Paths");
        let ignored = collect_record_strings(&stream_spec.records, "Ignored");
        let results = opened
            .records
            .iter()
            .filter_map(|record| record.get("depotFile").and_then(|value| value.as_str()))
            .map(|file| classify_stream_file(file, &stream, &paths, &ignored))
            .collect::<Vec<_>>();
        let submittable = results
            .iter()
            .all(|result| result.get("allowed").and_then(|value| value.as_bool()) == Some(true));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "submittable": submittable,
                "stream": stream,
                "workspace": workspace,
                "file_count": results.len(),
                "results": results
            }),
        )))
    }

    async fn query_stream_check_resolve(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        self.run_p4(json_invocation(vec![
            "streams".into(),
            "-F".into(),
            format!("Stream={stream_name}"),
        ], None))
        .await?;
        let preview = self.run_p4(stream_resolve_preview_invocation()).await?;
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "resolve_needed": !preview.records.is_empty(),
                "stream": stream_name,
                "conflicts": preview.records
            }),
        )))
    }

    async fn query_stream_interchanges(
        &self,
        action: &str,
        stream_name: &str,
        reverse: bool,
        file_paths: &[String],
        long_output: bool,
        limit: Option<u16>,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self
            .run_p4(interchanges_invocation(
                stream_name,
                reverse,
                file_paths,
                long_output,
            ))
            .await?;
        let message = match limit {
            Some(limit) => {
                Value::Array(output.records.into_iter().take(limit as usize).collect())
            }
            None => output_message(output),
        };
        Ok(Json(ToolResponse::success(action, message)))
    }
```

- [ ] **Step 5: Add helper functions outside impl**

In `src/server.rs`, add these helper functions near `collect_string_field`:

```rust
fn required_record_field(records: &[Value], field: &str) -> crate::error::Result<String> {
    records
        .first()
        .and_then(|record| record.get(field))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{field} is required"),
        })
}

fn collect_record_strings(records: &[Value], field: &str) -> Vec<String> {
    records
        .iter()
        .flat_map(|record| match record.get(field) {
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>(),
            Some(Value::String(value)) => vec![value.clone()],
            _ => Vec::new(),
        })
        .collect()
}

fn classify_stream_file(file: &str, stream: &str, paths: &[String], ignored: &[String]) -> Value {
    if ignored.iter().any(|pattern| stream_pattern_matches(file, stream, pattern)) {
        return json!({
            "file": file,
            "allowed": false,
            "rule": "ignored"
        });
    }
    for path in paths {
        let mut parts = path.split_whitespace();
        let path_type = parts.next().unwrap_or_default();
        let pattern = parts.next().unwrap_or_default();
        if stream_pattern_matches(file, stream, pattern) {
            let allowed = matches!(path_type, "share" | "isolate" | "import+");
            return json!({
                "file": file,
                "allowed": allowed,
                "rule": path_type
            });
        }
    }
    json!({
        "file": file,
        "allowed": false,
        "rule": "outside_view"
    })
}

fn stream_pattern_matches(file: &str, stream: &str, pattern: &str) -> bool {
    let depot_pattern = if pattern.starts_with("//") {
        pattern.to_string()
    } else {
        format!("{}/{}", stream.trim_end_matches('/'), pattern.trim_start_matches('/'))
    };
    let prefix = depot_pattern.trim_end_matches("/...");
    file.starts_with(prefix)
}
```

- [ ] **Step 6: Run focused stream smoke tests**

Run:

```bash
rtk cargo test query_streams_interchanges_runs_upstream_command_and_limits_client_side --test mcp_smoke_tests
rtk cargo test query_streams_check_resolve_runs_preview_after_stream_check --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 7: Commit stream routing**

Run:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: route stream query actions"
```

Expected: commit succeeds.

## Task 6: Remove CommonQueryParams And Finish Migration

**Files:**
- Modify: `src/tools/params.rs`
- Modify: `src/tools/streams.rs`
- Modify: `tests/mcp_smoke_tests.rs`
- Verify: `src/server.rs`, `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Search for remaining CommonQueryParams uses**

Run:

```bash
rtk rg -n "CommonQueryParams" src tests
```

Expected before deletion: only the struct definition remains in `src/tools/params.rs`, or no matches if an earlier task already removed it.

- [ ] **Step 2: Delete CommonQueryParams**

In `src/tools/params.rs`, delete this struct:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonQueryParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}
```

- [ ] **Step 3: Verify no CommonQueryParams remains**

Run:

```bash
rtk rg -n "CommonQueryParams" src tests
```

Expected: no output.

- [ ] **Step 4: Delete the temporary legacy stream wrapper**

In `src/tools/streams.rs`, delete the `build_stream_query_invocation` function and the `legacy_stream_action` function that were kept only to preserve compilation before `query_streams` routing was migrated.

Then verify no references remain:

```bash
rtk rg -n "build_stream_query_invocation|legacy_stream_action" src tests
```

Expected: no output.

- [ ] **Step 5: Run all query mapping and smoke tests**

Run:

```bash
rtk cargo test query_ --test tool_mapping_tests
rtk cargo test query_ --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 6: Commit removal**

Run:

```bash
rtk git add src/tools/params.rs src/tools/streams.rs tests/mcp_smoke_tests.rs src/server.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: remove common query params"
```

Expected: commit succeeds if the previous tasks did not already commit the same deletion. If there are no staged changes, skip this commit and continue to verification.

## Task 7: Verification And PR Follow-Up

**Files:**
- Verify: full repository
- External: GitHub PR #2 review threads

- [ ] **Step 1: Run formatting and lints**

Run:

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
```

Expected: PASS. If formatting fails, run `rtk cargo fmt`, then rerun `rtk cargo fmt --check`.

- [ ] **Step 2: Run full tests**

Run:

```bash
rtk cargo test
```

Expected: PASS. If WireMock or localhost binding fails inside sandbox with `PermissionDenied`, rerun `rtk cargo test` with escalated permissions and expect PASS.

- [ ] **Step 3: Inspect diff and history**

Run:

```bash
rtk git status --short --branch
rtk git log --oneline --decorate -6
rtk git diff --stat origin/codex/add-gitignore..HEAD
```

Expected:

- Branch is ahead of origin by the new plan and fix commits.
- Diff touches query params, query builders, server routing, and tests for the planned scope.

- [ ] **Step 4: Push the branch**

Run:

```bash
rtk git push origin codex/add-gitignore
```

Expected: push succeeds.

- [ ] **Step 5: Reply to the sync review thread**

Run:

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3410779210/replies -f body=$'Fixed in the latest push.\n\n`modify_files.sync` now follows the upstream handler control flow and rejects missing `file_paths` before write approval and before any `p4` invocation is constructed. This prevents an underspecified approved operation from becoming plain `p4 sync` over the full client view. Added direct builder coverage and a server-level approval-gate regression test.'
```

Expected: GitHub returns the created reply JSON.

- [ ] **Step 6: Reply to the query schema review thread**

Run:

```bash
rtk gh api repos/j3bit/p4mcp-server-rs/pulls/2/comments/3410779212/replies -f body=$'Fixed in the latest push.\n\nThe query tools now use upstream-shaped public params instead of the previous broad `CommonQueryParams`. `query_changelists` exposes `depot_path` and appends it to `p4 changes`; shelves, workspaces, jobs, and streams now expose tool-specific fields matching upstream terminology. The Rust-only `query_jobs.list` action was removed, and stream query routing was expanded so the upstream query actions are not treated as lower priority.'
```

Expected: GitHub returns the created reply JSON.

- [ ] **Step 7: Resolve both review threads**

Run:

```bash
rtk gh api graphql -f threadId='PRRT_kwDOS5FH5M6JdHuU' -f query='mutation($threadId:ID!) { resolveReviewThread(input:{threadId:$threadId}) { thread { id isResolved } } }'
rtk gh api graphql -f threadId='PRRT_kwDOS5FH5M6JdHuW' -f query='mutation($threadId:ID!) { resolveReviewThread(input:{threadId:$threadId}) { thread { id isResolved } } }'
```

Expected: each mutation returns `isResolved: true`.

- [ ] **Step 8: Confirm no unresolved review threads remain**

Run:

```bash
rtk gh api graphql -f owner=j3bit -f name=p4mcp-server-rs -F number=2 -f query='query($owner:String!, $name:String!, $number:Int!) { repository(owner:$owner, name:$name) { pullRequest(number:$number) { reviewThreads(first:100) { nodes { id isResolved path comments(first:1) { nodes { databaseId body } } } } } } }' --jq '[.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false)]'
```

Expected: `[]`.

## Self-Review

- Spec coverage:
  - Sync path requirement is covered by Task 1.
  - Changelists, shelves, workspaces, jobs, and streams public query schemas are covered by Task 2.
  - Changelist `depot_path`, shelves/workspaces/jobs tool-specific routing, and jobs action removal are covered by Task 3.
  - Stream public schema and single-command actions are covered by Task 4.
  - Stream multi-command actions are covered by Task 5.
  - `CommonQueryParams` and the temporary stream wrapper removal are covered by Task 6.
  - Verification and PR replies are covered by Task 7.
- Placeholder scan:
  - No banned marker text or unspecified code steps remain.
  - Runtime GitHub ids are pinned to the two unresolved threads observed during triage.
- Type consistency:
  - `QueryChangelistsParams`, `QueryShelvesParams`, `QueryWorkspacesParams`, `QueryJobsParams`, and `QueryStreamsParams` are defined before use.
  - `StreamQueryCommand` is defined before server routing uses it.
  - `build_stream_query_command` replaces `build_stream_query_invocation` consistently.
