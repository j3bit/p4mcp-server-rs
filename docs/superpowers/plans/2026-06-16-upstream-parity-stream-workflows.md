# Stream Workflow Upstream Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align Rust stream query and modify execution with upstream service intent and Perforce CLI semantics instead of applying narrow review-thread patches.

**Architecture:** Keep the upstream-shaped public `QueryStreamsParams` and `ModifyStreamsParams` schema intact. Replace the over-broad stream command builders with action-specific builders and server-side workflows where upstream performs validation, current stream resolution, or template detection. Preserve the Rust write approval gate: write previews stay side-effect-free and P4 execution still happens only after approval.

**Tech Stack:** Rust, `rmcp`, direct `p4` CLI invocation through `P4Executor`, `serde_json`, existing `rtk cargo` verification commands.

---

## Upstream And CLI Facts

Use this baseline throughout the implementation:

- Upstream repo: `perforce/p4mcp-server` `v2026.2.2955897`
- Upstream commit: `a64efb07511b2a62db41aeed110ab96744c4076a`
- Local CLI evidence: `/Users/jeongsaebit/Dev/p4-dogfood`, `p4` and `p4d` `2026.1/2972966 (2026/06/10)`

Relevant facts from upstream and `p4 help`:

- `p4 copy`: `force` in upstream means stream flow override and maps to uppercase `-F`.
- `p4 merge`: `force` in upstream means stream flow override and maps to uppercase `-F`.
- `p4 integrate`: `-f` and `-F` are both valid but have different meanings. Upstream `force` maps to lowercase `-f`, which ignores integration history. `-F` means arbitrary stream view integration and must not be emitted by the upstream `force` field.
- `p4 populate`: force maps to lowercase `-f`. `p4 populate` does not list uppercase `-F`.
- Upstream service `get_stream(None)` resolves the current workspace stream with `p4 client -o`, but upstream public `QueryStreamsParams` currently rejects `action="get"` without `stream_name`. The product decision for this Rust port is to honor upstream service intent plus CLI behavior: resolve the current stream for stream clients, and return a clear side-effect-free error for classic clients.
- `p4 stream -o` without a stream name can work only when the current client is stream-based. In a classic client, it fails with `Must specify a full stream path if not currently using a stream client.`

## Root Cause Findings

The current Rust port already has upstream-shaped stream public params, so this is not another `CommonQueryParams` or `CommonModifyParams` issue. The remaining parity hole is the execution layer:

- `copy`, `merge`, and `integrate` share one `propagation_invocation()` helper. That helper treats `force`, `branch`, `parent_stream`, and `stream_name` as universally meaningful, even though upstream handlers and P4 CLI semantics differ by action.
- `populate_invocation()` has action-specific code, but it still uses the wrong force flag and can combine branch, stream, and direct file modes instead of following upstream mode precedence.
- `query_streams.get`, `children`, `list_workspaces`, and `get_workspace` still route too much through single CLI invocations. Upstream service workflows perform stream existence checks, current stream resolution, and workspace template detection before returning data.

## File Structure

- Modify `src/tools/streams.rs`
  - Keep `QueryStreamsParams` and `ModifyStreamsParams` untouched.
  - Extend `StreamQueryCommand` so `get`, `children`, `get_workspace`, and `list_workspaces` can use server-side workflows.
  - Replace shared `propagation_invocation()` with action-specific `copy_invocation()`, `merge_invocation()`, `integrate_invocation()`, and `populate_invocation()`.
  - Export small invocation helpers that `src/server.rs` can use after validation and current-stream resolution.

- Modify `src/server.rs`
  - Add `query_stream_get()` to resolve `stream_name` from the current stream client when omitted.
  - Add `resolve_current_stream()` with a classic-workspace error.
  - Route `children`, `get_workspace`, and `list_workspaces` through workflows that match upstream validation.
  - Keep write approval behavior unchanged.

- Modify `tests/tool_mapping_tests.rs`
  - Add direct builder tests for stream write flag semantics and mode precedence.
  - Replace the old `stream_get_blank_name_errors` expectation with a command-shape expectation that missing stream name is handled by the server workflow.

- Modify `tests/mcp_smoke_tests.rs`
  - Add server-to-executor tests for `query_streams.get` current stream resolution.
  - Add server-to-executor tests for stream existence checks and workspace template detection.

---

### Task 1: Split Stream Modify Builders By Action

**Files:**
- Modify: `src/tools/streams.rs:183-435`
- Test: `tests/tool_mapping_tests.rs:1165-1203`

- [ ] **Step 1: Add failing tests for write flag semantics and mode precedence**

In `tests/tool_mapping_tests.rs`, insert these tests immediately after `modify_streams_copy_uses_upstream_stream_flags`:

```rust
#[test]
fn modify_streams_copy_ignores_branch_like_upstream_handler() {
    let mut params = modify_streams_params(StreamModifyAction::Copy);
    params.stream_name = Some("//streams/dev".to_string());
    params.branch = Some("ignored_branch".to_string());
    params.force = true;

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(invocation.args, vec!["copy", "-F", "-S", "//streams/dev"]);
}

#[test]
fn modify_streams_merge_ignores_branch_like_upstream_handler() {
    let mut params = modify_streams_params(StreamModifyAction::Merge);
    params.stream_name = Some("//streams/dev".to_string());
    params.parent_stream = Some("//streams/main".to_string());
    params.branch = Some("ignored_branch".to_string());
    params.force = true;
    params.output_base = true;

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "merge",
            "-F",
            "-Ob",
            "-S",
            "//streams/dev",
            "-P",
            "//streams/main"
        ]
    );
}

#[test]
fn modify_streams_integrate_uses_lowercase_force_and_branch_precedence() {
    let mut params = modify_streams_params(StreamModifyAction::Integrate);
    params.stream_name = Some("//streams/dev".to_string());
    params.parent_stream = Some("//streams/main".to_string());
    params.branch = Some("dev_to_rel".to_string());
    params.file_paths = Some(vec!["//streams/dev/src/...".to_string()]);
    params.preview = true;
    params.force = true;
    params.reverse = true;
    params.quiet = true;
    params.max_files = Some(10);
    params.output_base = true;
    params.schedule_branch_resolve = true;
    params.integrate_around_deleted = true;
    params.skip_cherry_picked = true;
    params.changelist = Some("12345".to_string());

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "integrate",
            "-n",
            "-f",
            "-q",
            "-Ob",
            "-c",
            "12345",
            "-m10",
            "-Di",
            "-Rb",
            "-Rs",
            "-b",
            "dev_to_rel",
            "-r",
            "//streams/dev/src/..."
        ]
    );
}

#[test]
fn modify_streams_integrate_uses_stream_mode_when_branch_absent() {
    let mut params = modify_streams_params(StreamModifyAction::Integrate);
    params.stream_name = Some("//streams/dev".to_string());
    params.parent_stream = Some("//streams/main".to_string());
    params.force = true;
    params.reverse = true;

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "integrate",
            "-f",
            "-S",
            "//streams/dev",
            "-P",
            "//streams/main",
            "-r"
        ]
    );
}

#[test]
fn modify_streams_populate_uses_lowercase_force_and_branch_precedence() {
    let mut params = modify_streams_params(StreamModifyAction::Populate);
    params.stream_name = Some("//streams/dev".to_string());
    params.parent_stream = Some("//streams/main".to_string());
    params.branch = Some("seed_branch".to_string());
    params.source_path = Some("//depot/source/...".to_string());
    params.target_path = Some("//depot/target/...".to_string());
    params.preview = true;
    params.force = true;
    params.reverse = true;
    params.max_files = Some(20);
    params.output_base = true;
    params.description = Some("Seed stream".to_string());

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "populate",
            "-n",
            "-f",
            "-o",
            "-m20",
            "-d",
            "Seed stream",
            "-b",
            "seed_branch",
            "-r"
        ]
    );
}

#[test]
fn modify_streams_populate_uses_direct_paths_when_branch_and_stream_absent() {
    let mut params = modify_streams_params(StreamModifyAction::Populate);
    params.source_path = Some("//depot/source/...".to_string());
    params.target_path = Some("//depot/target/...".to_string());
    params.force = true;

    let command = build_stream_modify_command(&params).unwrap();
    let invocation = command.into_single_invocation().unwrap();

    assert_eq!(
        invocation.args,
        vec![
            "populate",
            "-f",
            "//depot/source/...",
            "//depot/target/..."
        ]
    );
}
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run:

```bash
rtk cargo test modify_streams_integrate_uses_lowercase_force_and_branch_precedence --test tool_mapping_tests
rtk cargo test modify_streams_populate_uses_lowercase_force_and_branch_precedence --test tool_mapping_tests
```

Expected:

- The integrate test fails because the current builder emits `-F` and combines `-S` with `-b`.
- The populate test fails because the current builder emits `-F` and combines branch, stream, and direct paths.

- [ ] **Step 3: Replace the shared propagation builder with action-specific builders**

In `src/tools/streams.rs`, change this match block:

```rust
        StreamModifyAction::Copy => propagation_invocation("copy", params)?,
        StreamModifyAction::Merge => propagation_invocation("merge", params)?,
        StreamModifyAction::Integrate => propagation_invocation("integrate", params)?,
        StreamModifyAction::Populate => populate_invocation(params)?,
```

to:

```rust
        StreamModifyAction::Copy => copy_invocation(params),
        StreamModifyAction::Merge => merge_invocation(params),
        StreamModifyAction::Integrate => integrate_invocation(params),
        StreamModifyAction::Populate => populate_invocation(params),
```

Then replace `propagation_invocation()` and `populate_invocation()` with this complete action-specific code:

```rust
fn copy_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["copy".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-F".into());
    }
    if params.virtual_stream {
        args.push("-v".into());
    }
    push_quiet_changelist_max(&mut args, params);
    push_stream_mode(&mut args, params);
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn merge_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["merge".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-F".into());
    }
    if params.quiet {
        args.push("-q".into());
    }
    if params.output_base {
        args.push("-Ob".into());
    }
    push_changelist_max(&mut args, params);
    push_stream_mode(&mut args, params);
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn integrate_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["integrate".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-f".into());
    }
    if params.quiet {
        args.push("-q".into());
    }
    if params.output_base {
        args.push("-Ob".into());
    }
    push_changelist_max(&mut args, params);
    if params.integrate_around_deleted {
        args.push("-Di".into());
    }
    if params.schedule_branch_resolve {
        args.push("-Rb".into());
    }
    if params.skip_cherry_picked {
        args.push("-Rs".into());
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
        if params.reverse {
            args.push("-r".into());
        }
    } else {
        push_stream_mode(&mut args, params);
    }
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn populate_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["populate".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-f".into());
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
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
        if params.reverse {
            args.push("-r".into());
        }
    } else if non_blank(params.stream_name.as_deref()).is_some() {
        push_stream_mode(&mut args, params);
    } else if let (Some(source), Some(target)) = (
        non_blank(params.source_path.as_deref()),
        non_blank(params.target_path.as_deref()),
    ) {
        args.push(source.into());
        args.push(target.into());
    }
    json_invocation(args)
}

fn push_preview(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if params.preview {
        args.push("-n".into());
    }
}

fn push_quiet_changelist_max(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if params.quiet {
        args.push("-q".into());
    }
    push_changelist_max(args, params);
}

fn push_changelist_max(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(changelist) = non_blank(params.changelist.as_deref()) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
}

fn push_stream_mode(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
        if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
            args.extend(["-P".into(), parent.into()]);
        }
        if params.reverse {
            args.push("-r".into());
        }
    }
}

fn push_file_paths(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(file_paths) = &params.file_paths {
        args.extend(file_paths.iter().cloned());
    }
}
```

This preserves the upstream public shape while preventing the Rust port from inventing cross-action behavior:

- `branch` affects `integrate` and `populate`, not `copy` or `merge`.
- `force` maps to `-F` for `copy` and `merge`.
- `force` maps to `-f` for `integrate` and `populate`.
- `integrate` and `populate` use branch mode before stream mode, matching upstream service control flow.

- [ ] **Step 4: Run the targeted tests to verify they pass**

Run:

```bash
rtk cargo test modify_streams_copy_ignores_branch_like_upstream_handler --test tool_mapping_tests
rtk cargo test modify_streams_merge_ignores_branch_like_upstream_handler --test tool_mapping_tests
rtk cargo test modify_streams_integrate_uses_lowercase_force_and_branch_precedence --test tool_mapping_tests
rtk cargo test modify_streams_integrate_uses_stream_mode_when_branch_absent --test tool_mapping_tests
rtk cargo test modify_streams_populate_uses_lowercase_force_and_branch_precedence --test tool_mapping_tests
rtk cargo test modify_streams_populate_uses_direct_paths_when_branch_and_stream_absent --test tool_mapping_tests
```

Expected: all six tests pass.

- [ ] **Step 5: Commit Task 1**

Run:

```bash
rtk git add src/tools/streams.rs tests/tool_mapping_tests.rs
rtk git commit -m "fix: align stream write command semantics"
```

Expected: one commit containing only `src/tools/streams.rs` and `tests/tool_mapping_tests.rs`.

### Task 2: Resolve Current Stream For `query_streams.get`

**Files:**
- Modify: `src/tools/streams.rs:10-110`
- Modify: `src/tools/streams.rs:227-239`
- Modify: `src/server.rs:52-57`
- Modify: `src/server.rs:242-467`
- Modify: `src/server.rs:1312-1314`
- Test: `tests/tool_mapping_tests.rs:1347-1375`
- Test: `tests/mcp_smoke_tests.rs:386-470`

- [ ] **Step 1: Replace the old get-builder error test**

In `tests/tool_mapping_tests.rs`, replace `stream_get_blank_name_errors` with this test:

```rust
#[test]
fn stream_get_without_name_is_server_side_current_stream_workflow() {
    let params = QueryStreamsParams {
        action: StreamQueryAction::Get,
        stream_name: Some(" ".to_string()),
        stream_path: None,
        filter: None,
        fields: None,
        unloaded: false,
        all_streams: false,
        viewmatch: None,
        view_without_edit: true,
        at_change: Some("12345".to_string()),
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

    let command = build_stream_query_command(&params).unwrap();

    assert!(matches!(
        command,
        p4mcp_server_rs::tools::streams::StreamQueryCommand::Get {
            stream_name: None,
            view_without_edit: true,
            at_change: Some(_),
        }
    ));
}
```

- [ ] **Step 2: Add server smoke tests for current stream resolution**

In `tests/mcp_smoke_tests.rs`, insert these tests immediately before `query_streams_interchanges_runs_upstream_command_and_limits_client_side`:

```rust
#[tokio::test]
async fn query_streams_get_resolves_current_stream_from_stream_client() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/current"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/current"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "Stream": "//streams/current",
                "Type": "development"
            })],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Get);
    params.stream_name = None;
    params.view_without_edit = true;

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "get");
    assert_eq!(
        response.0.message,
        json!([{"Stream": "//streams/current", "Type": "development"}])
    );
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].args, ["client", "-o"]);
    assert_eq!(
        invocations[1].args,
        ["streams", "-F", "Stream=//streams/current"]
    );
    assert_eq!(
        invocations[2].args,
        ["stream", "-o", "-v", "//streams/current"]
    );
}

#[tokio::test]
async fn query_streams_get_rejects_classic_workspace_without_stream_name() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: vec![json!({"Client": "classic-ws"})],
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Get);
    params.stream_name = None;

    let err = match server.query_streams(Parameters(params)).await {
        Ok(_) => panic!("query_streams get should reject classic workspaces without stream_name"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(
        err.message
            .contains("No stream specified and current workspace is not stream-based")
    );
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["client", "-o"]);
}

#[tokio::test]
async fn query_streams_get_rejects_missing_explicit_stream_before_fetch() {
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
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Get);
    params.stream_name = Some("//streams/missing".to_string());

    let err = match server.query_streams(Parameters(params)).await {
        Ok(_) => panic!("query_streams get should validate explicit stream existence"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("stream does not exist: //streams/missing"));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0].args,
        ["streams", "-F", "Stream=//streams/missing"]
    );
    assert_eq!(
        invocations[1].args,
        ["streams", "-a", "-F", "Stream=//streams/missing"]
    );
}

#[tokio::test]
async fn query_streams_get_at_change_uses_resolved_stream_without_existence_lookup() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/current"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "Stream": "//streams/current",
                "Change": "12345"
            })],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Get);
    params.stream_name = None;
    params.at_change = Some("12345".to_string());

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(
        response.0.message,
        json!([{"Stream": "//streams/current", "Change": "12345"}])
    );
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].args, ["client", "-o"]);
    assert_eq!(invocations[1].args, ["stream", "-o", "//streams/current@12345"]);
}
```

- [ ] **Step 3: Run the get tests to verify they fail**

Run:

```bash
rtk cargo test stream_get_without_name_is_server_side_current_stream_workflow --test tool_mapping_tests
rtk cargo test query_streams_get_resolves_current_stream_from_stream_client --test mcp_smoke_tests
```

Expected:

- The tool mapping test fails because `build_stream_query_command()` still rejects blank `stream_name`.
- The smoke test fails because `query_streams.get` does not yet resolve current workspace stream.

- [ ] **Step 4: Extend `StreamQueryCommand` and query builder**

In `src/tools/streams.rs`, replace the beginning of `StreamQueryCommand` with this version:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamQueryCommand {
    Single(P4Invocation),
    Get {
        stream_name: Option<String>,
        view_without_edit: bool,
        at_change: Option<String>,
    },
    Children {
        stream_name: String,
    },
    Parent {
        stream_name: String,
    },
    Graph {
        stream_name: String,
    },
    GetWorkspace {
        workspace: Option<String>,
        stream_name: Option<String>,
        template: Option<String>,
    },
    ListWorkspaces {
        stream_name: Option<String>,
        user: Option<String>,
        unloaded: bool,
        max_results: u16,
    },
    ValidateFile {
        workspace: Option<String>,
        file_paths: Vec<String>,
    },
    ValidateSubmit {
        workspace: Option<String>,
        changelist: Option<String>,
    },
    CheckResolve {
        stream_name: String,
    },
    Interchanges {
        stream_name: String,
        reverse: bool,
        file_paths: Vec<String>,
        long_output: bool,
        limit: Option<u16>,
    },
}
```

Then update the relevant `build_stream_query_command()` arms:

```rust
        StreamQueryAction::Get => Ok(StreamQueryCommand::Get {
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            view_without_edit: params.view_without_edit,
            at_change: non_blank(params.at_change.as_deref()).map(str::to_string),
        }),
        StreamQueryAction::Children => Ok(StreamQueryCommand::Children {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::GetWorkspace => Ok(StreamQueryCommand::GetWorkspace {
            workspace: non_blank(params.workspace.as_deref()).map(str::to_string),
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            template: non_blank(params.template.as_deref()).map(str::to_string),
        }),
        StreamQueryAction::ListWorkspaces => Ok(StreamQueryCommand::ListWorkspaces {
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            user: non_blank(params.user.as_deref()).map(str::to_string),
            unloaded: params.unloaded,
            max_results: params.max_results,
        }),
```

- [ ] **Step 5: Export get/list workspace invocation helpers**

In `src/tools/streams.rs`, replace `stream_get_invocation()`, `stream_children_invocation()`, `stream_get_workspace_invocation()`, and `stream_list_workspaces_invocation()` with this code:

```rust
pub fn stream_get_invocation(
    stream_name: &str,
    view_without_edit: bool,
    at_change: Option<&str>,
) -> P4Invocation {
    let mut args = vec!["stream".into(), "-o".into()];
    if view_without_edit {
        args.push("-v".into());
    }
    let specifier = match non_blank(at_change) {
        Some(change) => format!("{stream_name}@{change}"),
        None => stream_name.to_string(),
    };
    args.push(specifier);
    json_invocation(args)
}

pub fn stream_children_invocation(stream_name: &str) -> P4Invocation {
    json_invocation(vec![
        "streams".into(),
        "-F".into(),
        format!("Parent={stream_name}"),
    ])
}

pub fn stream_get_workspace_invocation(
    workspace: Option<&str>,
    stream_name: Option<&str>,
    template: Option<&str>,
) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(stream_name) = non_blank(stream_name) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(template) = non_blank(template) {
        args.extend(["-t".into(), template.into()]);
    }
    if let Some(workspace) = non_blank(workspace) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

pub fn stream_list_workspaces_invocation(
    stream_name: Option<&str>,
    user: Option<&str>,
    unloaded: bool,
    max_results: u16,
) -> P4Invocation {
    let mut args = vec!["clients".into()];
    if unloaded {
        args.push("-U".into());
    }
    if let Some(stream_name) = non_blank(stream_name) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(user) = non_blank(user) {
        args.extend(["-u".into(), user.into()]);
    }
    args.extend(["-m".into(), max_results.to_string()]);
    json_invocation(args)
}
```

- [ ] **Step 6: Route `query_streams.get` through server-side current-stream resolution**

In `src/server.rs`, update the stream imports to include the new helpers:

```rust
            opened_for_stream_validation_invocation, stream_children_invocation,
            stream_get_invocation, stream_get_workspace_invocation,
            stream_list_workspaces_invocation, stream_resolve_preview_invocation,
            stream_spec_with_view_invocation,
```

Add this method near `query_stream_parent()`:

```rust
    async fn query_stream_get(
        &self,
        action: &str,
        stream_name: Option<&str>,
        view_without_edit: bool,
        at_change: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let effective_stream = self.resolve_current_stream(stream_name).await?;
        if at_change.is_none() {
            self.require_existing_stream(&effective_stream).await?;
        }
        let output = self
            .run_p4(stream_get_invocation(
                &effective_stream,
                view_without_edit,
                at_change,
            ))
            .await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }

    async fn resolve_current_stream(&self, stream_name: Option<&str>) -> McpResult<String> {
        if let Some(stream_name) = stream_name
            .map(str::trim)
            .filter(|stream_name| !stream_name.is_empty())
        {
            return Ok(stream_name.to_string());
        }

        let client = self.run_p4(client_spec_invocation(None)).await?;
        required_record_field(&client.records, "Stream").map_err(|_| {
            to_mcp_error(invalid_input(
                "No stream specified and current workspace is not stream-based",
            ))
        })
    }
```

Then update the `query_streams()` match arm:

```rust
            StreamQueryCommand::Get {
                stream_name,
                view_without_edit,
                at_change,
            } => {
                self.query_stream_get(
                    action,
                    stream_name.as_deref(),
                    view_without_edit,
                    at_change.as_deref(),
                )
                .await
            }
```

- [ ] **Step 7: Run the get tests to verify they pass**

Run:

```bash
rtk cargo test stream_get_without_name_is_server_side_current_stream_workflow --test tool_mapping_tests
rtk cargo test query_streams_get_resolves_current_stream_from_stream_client --test mcp_smoke_tests
rtk cargo test query_streams_get_rejects_classic_workspace_without_stream_name --test mcp_smoke_tests
rtk cargo test query_streams_get_rejects_missing_explicit_stream_before_fetch --test mcp_smoke_tests
rtk cargo test query_streams_get_at_change_uses_resolved_stream_without_existence_lookup --test mcp_smoke_tests
```

Expected: all five tests pass.

- [ ] **Step 8: Commit Task 2**

Run:

```bash
rtk git add src/tools/streams.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: resolve current stream for stream get"
```

Expected: one commit containing the stream get workflow change.

### Task 3: Move Remaining Stream Query Validation Into Server Workflows

**Files:**
- Modify: `src/server.rs:242-467`
- Modify: `src/server.rs:1312-1355`
- Test: `tests/mcp_smoke_tests.rs:386-620`

- [ ] **Step 1: Add failing tests for children, list_workspaces, and get_workspace workflows**

In `tests/mcp_smoke_tests.rs`, insert these tests after `query_streams_get_at_change_uses_resolved_stream_without_existence_lookup`:

```rust
#[tokio::test]
async fn query_streams_children_validates_parent_stream_before_listing_children() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/child", "Parent": "//streams/dev"})],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_streams(Parameters(stream_query_params(StreamQueryAction::Children)))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "children");
    assert_eq!(
        response.0.message,
        json!([{"Stream": "//streams/child", "Parent": "//streams/dev"}])
    );
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0].args,
        ["streams", "-F", "Stream=//streams/dev"]
    );
    assert_eq!(
        invocations[1].args,
        ["streams", "-F", "Parent=//streams/dev"]
    );
}

#[tokio::test]
async fn query_streams_children_rejects_missing_stream_before_listing_children() {
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
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_streams(Parameters(stream_query_params(StreamQueryAction::Children)))
        .await
    {
        Ok(_) => panic!("query_streams children should reject missing parent streams"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("stream does not exist: //streams/dev"));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0].args,
        ["streams", "-F", "Stream=//streams/dev"]
    );
    assert_eq!(
        invocations[1].args,
        ["streams", "-a", "-F", "Stream=//streams/dev"]
    );
}

#[tokio::test]
async fn query_streams_list_workspaces_validates_stream_before_listing_clients() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"client": "stream-ws"})],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::ListWorkspaces);
    params.user = Some("alice".to_string());
    params.unloaded = true;
    params.max_results = 5;

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list_workspaces");
    assert_eq!(response.0.message, json!([{"client": "stream-ws"}]));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        invocations[0].args,
        ["streams", "-F", "Stream=//streams/dev"]
    );
    assert_eq!(
        invocations[1].args,
        ["clients", "-U", "-S", "//streams/dev", "-u", "alice", "-m", "5"]
    );
}

#[tokio::test]
async fn query_streams_get_workspace_rejects_template_for_named_workspace() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: vec![json!({"Client": "missing-ws"})],
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::GetWorkspace);
    params.workspace = Some("missing-ws".to_string());
    params.stream_name = None;

    let err = match server.query_streams(Parameters(params)).await {
        Ok(_) => panic!("query_streams get_workspace should reject template specs"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("Workspace 'missing-ws' does not exist"));
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["client", "-o", "missing-ws"]);
}

#[tokio::test]
async fn query_streams_get_workspace_returns_existing_named_workspace() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: vec![json!({
            "Client": "stream-ws",
            "Update": "2026/06/16",
            "Stream": "//streams/dev"
        })],
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::GetWorkspace);
    params.workspace = Some("stream-ws".to_string());
    params.stream_name = Some("//streams/dev".to_string());
    params.template = Some("template-ws".to_string());

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "get_workspace");
    assert_eq!(
        response.0.message,
        json!([{
            "Client": "stream-ws",
            "Update": "2026/06/16",
            "Stream": "//streams/dev"
        }])
    );
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0].args,
        ["client", "-o", "-S", "//streams/dev", "-t", "template-ws", "stream-ws"]
    );
}
```

- [ ] **Step 2: Run one representative failing test for each workflow**

Run:

```bash
rtk cargo test query_streams_children_validates_parent_stream_before_listing_children --test mcp_smoke_tests
rtk cargo test query_streams_list_workspaces_validates_stream_before_listing_clients --test mcp_smoke_tests
rtk cargo test query_streams_get_workspace_rejects_template_for_named_workspace --test mcp_smoke_tests
```

Expected:

- `children` fails because it currently lists children without validating the parent stream.
- `list_workspaces` fails because it currently calls `clients` directly.
- `get_workspace` fails because it returns the template spec for a missing named workspace.

- [ ] **Step 3: Add server workflow methods**

In `src/server.rs`, add these methods near the existing stream query methods:

```rust
    async fn query_stream_children(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        self.require_existing_stream(stream_name).await?;
        let output = self.run_p4(stream_children_invocation(stream_name)).await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }

    async fn query_stream_get_workspace(
        &self,
        action: &str,
        workspace: Option<&str>,
        stream_name: Option<&str>,
        template: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self
            .run_p4(stream_get_workspace_invocation(
                workspace,
                stream_name,
                template,
            ))
            .await?;
        if let Some(workspace) = workspace {
            if !client_spec_is_existing_workspace(&output.records) {
                return Err(to_mcp_error(invalid_input(format!(
                    "Workspace '{workspace}' does not exist"
                ))));
            }
        }
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }

    async fn query_stream_list_workspaces(
        &self,
        action: &str,
        stream_name: Option<&str>,
        user: Option<&str>,
        unloaded: bool,
        max_results: u16,
    ) -> McpResult<Json<ToolResponse>> {
        if let Some(stream_name) = stream_name {
            self.require_existing_stream(stream_name).await?;
        }
        let output = self
            .run_p4(stream_list_workspaces_invocation(
                stream_name,
                user,
                unloaded,
                max_results,
            ))
            .await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }
```

Add this helper near `form_has_non_empty_field()`:

```rust
fn client_spec_is_existing_workspace(records: &[Value]) -> bool {
    records.first().is_some_and(|record| {
        non_empty_string_field(record, "Update").is_some()
            || non_empty_string_field(record, "Access").is_some()
    })
}
```

- [ ] **Step 4: Route the new `StreamQueryCommand` variants**

In `src/server.rs`, update the `query_streams()` match:

```rust
            StreamQueryCommand::Children { stream_name } => {
                self.query_stream_children(action, &stream_name).await
            }
            StreamQueryCommand::GetWorkspace {
                workspace,
                stream_name,
                template,
            } => {
                self.query_stream_get_workspace(
                    action,
                    workspace.as_deref(),
                    stream_name.as_deref(),
                    template.as_deref(),
                )
                .await
            }
            StreamQueryCommand::ListWorkspaces {
                stream_name,
                user,
                unloaded,
                max_results,
            } => {
                self.query_stream_list_workspaces(
                    action,
                    stream_name.as_deref(),
                    user.as_deref(),
                    unloaded,
                    max_results,
                )
                .await
            }
```

Keep the existing `Parent`, `Graph`, `ValidateFile`, `ValidateSubmit`, `CheckResolve`, and `Interchanges` arms, because they already use server workflows.

- [ ] **Step 5: Run the workflow tests to verify they pass**

Run:

```bash
rtk cargo test query_streams_children_validates_parent_stream_before_listing_children --test mcp_smoke_tests
rtk cargo test query_streams_children_rejects_missing_stream_before_listing_children --test mcp_smoke_tests
rtk cargo test query_streams_list_workspaces_validates_stream_before_listing_clients --test mcp_smoke_tests
rtk cargo test query_streams_get_workspace_rejects_template_for_named_workspace --test mcp_smoke_tests
rtk cargo test query_streams_get_workspace_returns_existing_named_workspace --test mcp_smoke_tests
```

Expected: all five tests pass.

- [ ] **Step 6: Commit Task 3**

Run:

```bash
rtk git add src/server.rs src/tools/streams.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: align stream query workflows"
```

Expected: one commit containing stream query workflow validation.

### Task 4: Preserve Approval-Gate Side-Effect Boundaries

**Files:**
- Modify: `src/server.rs:775-845`
- Test: `src/server.rs:2560-3065`

- [ ] **Step 1: Add server tests proving write previews use corrected commands before approval**

In `src/server.rs`, inside the existing `#[cfg(test)] mod tests`, add these tests near the existing `modify_streams_*` tests:

```rust
    #[tokio::test]
    async fn modify_streams_integrate_preview_uses_lowercase_force_without_executor_call() {
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
        let mut params = modify_streams_params(StreamModifyAction::Integrate);
        params.stream_name = Some("//streams/dev".to_string());
        params.force = true;

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .unwrap();

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "integrate".to_string(),
                "-f".to_string(),
                "-S".to_string(),
                "//streams/dev".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_streams_populate_preview_uses_lowercase_force_without_executor_call() {
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
        let mut params = modify_streams_params(StreamModifyAction::Populate);
        params.stream_name = Some("//streams/dev".to_string());
        params.force = true;

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .unwrap();

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "populate".to_string(),
                "-f".to_string(),
                "-S".to_string(),
                "//streams/dev".to_string(),
            ])
        );
    }
```

- [ ] **Step 2: Run the approval preview tests**

Run:

```bash
rtk cargo test modify_streams_integrate_preview_uses_lowercase_force_without_executor_call
rtk cargo test modify_streams_populate_preview_uses_lowercase_force_without_executor_call
```

Expected: both tests pass. The executor invocation list remains empty in both tests.

- [ ] **Step 3: Commit Task 4**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "test: cover stream write approval previews"
```

Expected: one commit containing approval-boundary regression tests.

### Task 5: Full Verification And PR Review Follow-Up

**Files:**
- Read: `src/tools/streams.rs`
- Read: `src/server.rs`
- Read: `tests/tool_mapping_tests.rs`
- Read: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Run formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: command exits 0. If it fails, run `rtk cargo fmt`, inspect the diff, then rerun `rtk cargo fmt --check`.

- [ ] **Step 2: Run stream-focused tests**

Run:

```bash
rtk cargo test stream_ --test tool_mapping_tests
rtk cargo test query_streams_ --test mcp_smoke_tests
rtk cargo test modify_streams_
```

Expected: all stream-focused tests pass.

- [ ] **Step 3: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected: command exits 0.

- [ ] **Step 4: Run full test suite**

Run:

```bash
rtk cargo test
```

Expected: command exits 0. If WireMock or localhost binding fails with sandbox permission errors, rerun the same command outside the sandbox with escalation and record that the failure mode was sandbox-localhost binding.

- [ ] **Step 5: Inspect the final diff**

Run:

```bash
rtk git diff --stat
rtk git diff -- src/tools/streams.rs src/server.rs tests/tool_mapping_tests.rs tests/mcp_smoke_tests.rs
```

Expected:

- No changes to stream public params.
- No changes to write approval policy.
- `integrate` and `populate` use lowercase `-f` for the public `force` field.
- `copy` and `merge` still use uppercase `-F` for the public `force` field.
- `query_streams.get` resolves the current stream through `client -o` when `stream_name` is omitted.
- Classic workspace get fails before any `stream -o` call.
- Stream existence checks happen before `get`, `children`, and stream-filtered `list_workspaces`.

- [ ] **Step 6: Push the branch**

Run:

```bash
rtk git status --short --branch
rtk git push
```

Expected:

- `git status` shows only intentional committed state and pre-existing unrelated untracked P4D files if they are still present.
- `git push` updates PR #2 branch.

- [ ] **Step 7: Reply to the three PR review threads**

Use thread replies, not top-level PR comments. The exact comment IDs must be re-read before replying because GitHub thread state can change:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected: three unresolved non-outdated stream review threads are still present.

Reply content for the integrate force thread:

```text
Fixed in the latest push.

`p4 integrate -F` is a valid Helix Core option, but it is not the upstream `force` semantics for this MCP field. `p4 help integrate` distinguishes lowercase `-f` as "ignore integration history" and uppercase `-F` as arbitrary stream-view integration. Upstream `integrate_stream(force=True)` maps to lowercase `-f`, so the Rust port now does the same.

This was fixed by splitting the previous shared propagation builder into action-specific stream builders. `copy` and `merge` still use uppercase `-F`, while `integrate` uses lowercase `-f` and follows upstream branch-vs-stream mode precedence.
```

Reply content for the populate force thread:

```text
Fixed in the latest push.

`p4 help populate` lists lowercase `-f` for force and does not list uppercase `-F`. The Rust port now maps `modify_streams.populate force=true` to `p4 populate -f`, matching upstream `populate_stream(force=True)` and the CLI reference.

The populate builder now also follows upstream mode precedence: branch mode first, stream mode next, and direct source/target paths only when neither branch nor stream mode is selected.
```

Reply content for the stream get thread:

```text
Fixed in the latest push.

Upstream has a mismatch here: the public `QueryStreamsParams` validator requires `stream_name` for `get`, but the upstream `get_stream()` service implementation explicitly resolves the current workspace stream when no stream name is supplied. The Rust port now follows the service intent and the Perforce CLI behavior.

When `query_streams.get` omits `stream_name`, the server runs `p4 client -o`, reads the current client's `Stream`, validates the resolved stream when not querying an `@change`, and then runs `p4 stream -o`. In a classic workspace, it returns a clear invalid-params error before any `stream -o` call.
```

- [ ] **Step 8: Resolve the threads**

Resolve only the three stream threads after replies are posted and the branch is pushed.

Expected: a fresh review-thread read shows zero unresolved non-outdated review threads for PR #2.

---

## Self-Review

Spec coverage:

- Integrate force semantics are covered by Task 1 direct tests and Task 4 approval-preview tests.
- Populate force semantics are covered by Task 1 direct tests and Task 4 approval-preview tests.
- The requested decision for `query_streams.get` without `stream_name` is covered by Task 2.
- The broader root-cause fix is covered by action-specific modify builders in Task 1 and server-side query workflows in Tasks 2 and 3.
- The write approval gate remains covered by Task 4.

Placeholder scan:

- The plan contains exact files, test functions, commands, expected results, and code snippets.
- The plan does not rely on unspecified implementation steps.

Type consistency:

- New `StreamQueryCommand` variants are matched in `src/server.rs`.
- New helper names imported in `src/server.rs` match the exported functions in `src/tools/streams.rs`.
- Test helpers use existing `stream_query_params()` and `modify_streams_params()` constructors.
