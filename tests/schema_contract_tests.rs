use std::collections::BTreeSet;

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
        "action": "update",
        "workspace_name": "ws-main"
    }))
    .unwrap();

    assert_eq!(params.action, WorkspaceModifyAction::Update);
    assert_eq!(params.workspace_options, None);
    assert_eq!(params.workspace_line_end, None);
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
