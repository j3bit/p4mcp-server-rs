use std::collections::BTreeSet;

use p4mcp_server_rs::tools::{
    params::{
        ChangelistModifyAction, JobModifyAction, ModifyChangelistsParams, ModifyJobsParams,
        ModifyShelvesParams, ModifyStreamsParams, ModifyWorkspacesParams, ShelfModifyAction,
        StreamModifyAction, WorkspaceModifyAction,
    },
    server::QueryServerParams,
};
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

fn assert_has<T: JsonSchema>(field: &str) {
    assert!(
        schema_properties::<T>().contains(field),
        "schema missing field {field}"
    );
}

fn assert_omits<T: JsonSchema>(field: &str) {
    assert!(
        !schema_properties::<T>().contains(field),
        "schema unexpectedly includes field {field}"
    );
}

#[test]
fn query_server_schema_matches_upstream_params_object() {
    let props = schema_properties::<QueryServerParams>();
    assert!(props.contains("action"));
    assert_eq!(props.len(), 1);
}

#[test]
fn modify_changelists_schema_matches_upstream_fields() {
    for field in [
        "action",
        "changelist_id",
        "description",
        "file_paths",
        "approval_token",
    ] {
        assert_has::<ModifyChangelistsParams>(field);
    }

    for field in ["files", "form", "confirmation"] {
        assert_omits::<ModifyChangelistsParams>(field);
    }

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

    for field in ["files", "form", "confirmation"] {
        assert_omits::<ModifyShelvesParams>(field);
    }

    let params: ModifyShelvesParams = serde_json::from_value(serde_json::json!({
        "action": "unshelve_to_changelist",
        "changelist_id": "12345",
        "target_changelist": "54321",
        "force": true
    }))
    .unwrap();

    assert_eq!(params.action, ShelfModifyAction::UnshelveToChangelist);
    assert_eq!(params.target_changelist, "54321");
    assert!(params.force);
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

    for field in ["form", "confirmation"] {
        assert_omits::<ModifyWorkspacesParams>(field);
    }

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

    for field in ["files", "form", "confirmation"] {
        assert_omits::<ModifyJobsParams>(field);
    }

    let params: ModifyJobsParams = serde_json::from_value(serde_json::json!({
        "action": "link_job",
        "changelist_id": "12345",
        "job_id": "job000001"
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

    for field in ["stream", "form", "confirmation"] {
        assert_omits::<ModifyStreamsParams>(field);
    }

    let params: ModifyStreamsParams = serde_json::from_value(serde_json::json!({
        "action": "create_workspace"
    }))
    .unwrap();

    assert_eq!(params.action, StreamModifyAction::CreateWorkspace);
}
