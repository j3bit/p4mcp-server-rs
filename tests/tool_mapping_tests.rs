use p4mcp_server_rs::{
    config::Toolset,
    p4::forms::{WorkspaceFormPatch, patch_change_description_form, patch_workspace_form},
    p4::runner::{OutputMode, P4Invocation},
    permissions::{Access, SafetyPolicy},
    tools::{
        changelists::{build_changelist_modify_invocation, build_changelist_query_invocation},
        files::{build_file_invocation, build_file_modify_invocation},
        jobs::{build_job_modify_invocation, build_job_query_invocation},
        params::{
            ChangelistModifyAction, ChangelistQueryAction, FileModifyAction, FileQueryAction,
            JobModifyAction, JobQueryAction, ModifyChangelistsParams, ModifyFilesParams,
            ModifyJobsParams, ModifyShelvesParams, ModifyWorkspacesParams, QueryChangelistsParams,
            QueryFilesParams, QueryJobsParams, QueryShelvesParams, QueryStreamsParams,
            QueryWorkspacesParams, ShelfModifyAction, ShelfQueryAction, StreamQueryAction,
            WorkspaceModifyAction, WorkspaceQueryAction,
        },
        server::{QueryServerParams, ServerQueryAction, build_server_invocation},
        shelves::{build_shelf_modify_invocation, build_shelf_query_invocation},
        streams::build_stream_query_command,
        workspaces::{build_workspace_delete_invocation, build_workspace_query_invocation},
    },
};
use schemars::JsonSchema;

fn schema_has_property<T: JsonSchema>(name: &str) -> bool {
    let schema = schemars::schema_for!(T);
    let schema = serde_json::to_value(schema).unwrap();
    schema["properties"].as_object().unwrap().contains_key(name)
}

#[test]
fn query_files_params_deserialize_content_action() {
    let params: QueryFilesParams = serde_json::from_value(serde_json::json!({
        "action": "content",
        "file_path": "//depot/main/README.md"
    }))
    .unwrap();

    assert_eq!(params.action.as_str(), "content");
    assert_eq!(params.file_path, "//depot/main/README.md");
}

#[test]
fn readonly_blocks_modify_files() {
    let policy = SafetyPolicy::new(true, [Toolset::Files].into_iter().collect());
    let result = policy.check(Access::Write, Toolset::Files, "modify_files");
    assert!(result.unwrap_err().to_string().contains("read-only"));
}

#[test]
fn disabled_toolset_blocks_call() {
    let policy = SafetyPolicy::new(false, [Toolset::Changelists].into_iter().collect());
    let result = policy.check(Access::Read, Toolset::Files, "query_files");
    assert!(result.unwrap_err().to_string().contains("toolset disabled"));
}

#[test]
fn modify_file_params_schema_exposes_approval_token() {
    assert!(schema_has_property::<ModifyFilesParams>("approval_token"));
}

#[test]
fn modify_file_params_schema_omits_confirmation() {
    assert!(!schema_has_property::<ModifyFilesParams>("confirmation"));
}

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
    assert!(schema_has_property::<QueryWorkspacesParams>(
        "workspace_name"
    ));
    assert!(schema_has_property::<QueryWorkspacesParams>("user"));
    assert!(!schema_has_property::<QueryWorkspacesParams>(
        "changelist_id"
    ));
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
fn upstream_query_action_enums_deserialize_and_render_as_str() {
    let shelf: ShelfQueryAction = serde_json::from_value(serde_json::json!("diff")).unwrap();
    let workspace: WorkspaceQueryAction =
        serde_json::from_value(serde_json::json!("type")).unwrap();
    let job: JobQueryAction = serde_json::from_value(serde_json::json!("get_job")).unwrap();

    assert_eq!(shelf, ShelfQueryAction::Diff);
    assert_eq!(shelf.as_str(), "diff");
    assert_eq!(workspace, WorkspaceQueryAction::Type);
    assert_eq!(workspace.as_str(), "type");
    assert_eq!(job, JobQueryAction::GetJob);
    assert_eq!(job.as_str(), "get_job");
}

#[test]
fn query_streams_schema_matches_upstream_fields() {
    for field in [
        "action",
        "stream_name",
        "stream_path",
        "filter",
        "fields",
        "unloaded",
        "all_streams",
        "viewmatch",
        "view_without_edit",
        "at_change",
        "both_directions",
        "force_refresh",
        "workspace",
        "template",
        "user",
        "file_paths",
        "changelist",
        "reverse",
        "long_output",
        "limit",
        "max_results",
    ] {
        assert!(
            schema_has_property::<QueryStreamsParams>(field),
            "missing stream schema field {field}"
        );
    }

    assert!(!schema_has_property::<QueryStreamsParams>("stream"));
    assert!(!schema_has_property::<QueryStreamsParams>("owner"));
}

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
        "template": "//streams/template",
        "max_results": 25
    }))
    .unwrap();

    assert_eq!(params.action, StreamQueryAction::List);
    assert_eq!(params.action.as_str(), "list");
    assert_eq!(
        params.stream_path.as_deref(),
        Some(&["//depot/...".to_string()][..])
    );
    assert_eq!(params.filter.as_deref(), Some("Owner=alice"));
    assert_eq!(
        params.fields.as_deref(),
        Some(
            &[
                "Stream".to_string(),
                "Owner".to_string(),
                "Type".to_string()
            ][..]
        )
    );
    assert!(params.unloaded);
    assert!(params.all_streams);
    assert_eq!(params.viewmatch.as_deref(), Some("//depot/main/file.txt"));
    assert_eq!(params.template.as_deref(), Some("//streams/template"));
    assert_eq!(params.max_results, 25);
}

#[test]
fn query_streams_params_default_max_results_and_template() {
    let params: QueryStreamsParams = serde_json::from_value(serde_json::json!({
        "action": "list"
    }))
    .unwrap();

    assert_eq!(params.max_results, 50);
    assert_eq!(params.template, None);
}

#[test]
fn stream_query_action_multi_word_variants_deserialize_and_render_as_str() {
    for (value, expected) in [
        ("integration_status", StreamQueryAction::IntegrationStatus),
        ("get_workspace", StreamQueryAction::GetWorkspace),
        ("list_workspaces", StreamQueryAction::ListWorkspaces),
        ("validate_file", StreamQueryAction::ValidateFile),
        ("validate_submit", StreamQueryAction::ValidateSubmit),
        ("check_resolve", StreamQueryAction::CheckResolve),
    ] {
        let action: StreamQueryAction = serde_json::from_value(serde_json::json!(value)).unwrap();

        assert_eq!(action, expected);
        assert_eq!(action.as_str(), value);
    }
}

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

#[test]
fn query_file_content_uses_text_print() {
    let params = QueryFilesParams {
        action: FileQueryAction::Content,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation,
        P4Invocation {
            args: vec!["print".into(), "-q".into(), "//depot/main/file.txt".into()],
            stdin: None,
            mode: OutputMode::Text,
        }
    );
}

#[test]
fn query_file_grep_maps_pattern() {
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
        vec!["grep", "-n", "-i", "-e", "needle", "//depot/main/..."]
    );
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn query_file_diff2_requires_and_uses_second_depot_path() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: Some("//depot/dev/file.txt".to_string()),
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation,
        P4Invocation {
            args: vec![
                "diff2".into(),
                "//depot/main/file.txt".into(),
                "//depot/dev/file.txt".into()
            ],
            stdin: None,
            mode: OutputMode::Text,
        }
    );
}

#[test]
fn query_file_workspace_diff_uses_single_path() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: false,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation,
        P4Invocation {
            args: vec!["diff".into(), "//depot/main/file.txt".into()],
            stdin: None,
            mode: OutputMode::Text,
        }
    );
}

#[test]
fn query_file_workspace_diff_rejects_second_path() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: Some("//depot/dev/file.txt".to_string()),
        diff2: false,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("file2 cannot be used for workspace diff"));
}

#[test]
fn query_file_diff2_requires_second_path() {
    let params = QueryFilesParams {
        action: FileQueryAction::Diff,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("file2 is required for diff2"));
}

#[test]
fn query_file_search_requires_pattern() {
    let params = QueryFilesParams {
        action: FileQueryAction::Search,
        file_path: "//depot/main".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("pattern is required for search"));
}

#[test]
fn query_file_grep_requires_pattern() {
    let params = QueryFilesParams {
        action: FileQueryAction::Grep,
        file_path: "//depot/main/...".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let error = build_file_invocation(&params).unwrap_err().to_string();
    assert!(error.contains("pattern is required for grep"));
}

#[test]
fn modify_file_add_maps_changelist() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Add,
        file_paths: Some(vec!["src/new.rs".to_string()]),
        changelist: "123".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(invocation.args, vec!["add", "-c", "123", "src/new.rs"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn modify_file_delete_builds_without_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: Some(vec!["//depot/main/old.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["delete", "-c", "default", "//depot/main/old.rs"]
    );
}

#[test]
fn modify_file_resolve_safe_maps_to_as() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "safe".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["resolve", "-as", "//depot/main/file.rs"]
    );
}

#[test]
fn modify_file_add_requires_files() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Add,
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
    assert!(error.contains("file_paths is required for add"));
}

#[test]
fn modify_file_delete_requires_files() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
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
    assert!(error.contains("file_paths is required for delete"));
}

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

#[test]
fn modify_file_revert_maps_changelist_and_file() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Revert,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "456".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["revert", "-c", "456", "//depot/main/file.rs"]
    );
}

#[test]
fn modify_file_move_source_target_mismatch_errors() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Move,
        file_paths: None,
        changelist: "default".to_string(),
        source_paths: Some(vec!["//depot/main/source.rs".to_string()]),
        target_paths: Some(vec![]),
        mode: "auto".to_string(),
        force: false,
        approval_token: None,
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("source_paths and target_paths must have the same length"));
}

#[test]
fn modify_file_sync_force_maps_flag_and_file() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Sync,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: true,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(invocation.args, vec!["sync", "-f", "//depot/main/file.rs"]);
}

#[test]
fn modify_file_invalid_resolve_mode_errors() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "bad".to_string(),
        force: false,
        approval_token: None,
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid resolve mode: bad"));
}

#[test]
fn modify_file_resolve_theirs_builds_without_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "theirs".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["resolve", "-at", "//depot/main/file.rs"]
    );
}

#[test]
fn modify_file_resolve_force_maps_to_af() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "force".to_string(),
        force: false,
        approval_token: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["resolve", "-af", "//depot/main/file.rs"]
    );
}

#[test]
fn default_changelist_get_uses_opened_not_describe() {
    let invocation =
        build_changelist_query_invocation("get", Some("default"), None, None, None, None, 10)
            .unwrap();
    assert_eq!(invocation.args, vec!["opened", "-c", "default"]);
}

#[test]
fn numbered_changelist_get_uses_describe() {
    let invocation =
        build_changelist_query_invocation("get", Some("123"), None, None, None, None, 10).unwrap();
    assert_eq!(invocation.args, vec!["describe", "-s", "123"]);
}

#[test]
fn changelist_get_blank_id_errors() {
    let error = build_changelist_query_invocation("get", Some(" "), None, None, None, None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changelist_id is required"));
}

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

fn modify_changelists_params(action: ChangelistModifyAction) -> ModifyChangelistsParams {
    ModifyChangelistsParams {
        action,
        changelist_id: None,
        description: None,
        file_paths: None,
        approval_token: None,
    }
}

#[test]
fn modify_changelists_move_files_uses_file_paths() {
    let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
    params.changelist_id = Some("12345".to_string());
    params.file_paths = Some(vec!["//depot/main/a.txt".to_string()]);

    let invocation = build_changelist_modify_invocation(&params, None).unwrap();

    assert_eq!(
        invocation.args,
        vec!["reopen", "-c", "12345", "//depot/main/a.txt"]
    );
    assert_eq!(invocation.stdin, None);
}

#[test]
fn patch_change_description_preserves_existing_files() {
    let existing = "\
Change: 12345

Description:
\told description
\tcontinued

Files:
\t//depot/main/a.txt
\t//depot/main/b.txt
";

    let patched = patch_change_description_form(existing, "new description\nsecond line").unwrap();

    assert_eq!(
        patched,
        "\
Change: 12345

Description:
\tnew description
\tsecond line

Files:
\t//depot/main/a.txt
\t//depot/main/b.txt
"
    );
}

#[test]
fn changelist_submit_uses_numbered_change() {
    let mut params = modify_changelists_params(ChangelistModifyAction::Submit);
    params.changelist_id = Some("123".to_string());

    let invocation = build_changelist_modify_invocation(&params, None).unwrap();

    assert_eq!(invocation.args, vec!["submit", "-c", "123"]);
}

#[test]
fn changelist_move_files_uses_reopen() {
    let files = vec![
        "//depot/main/a.rs".to_string(),
        "//depot/main/b.rs".to_string(),
    ];

    let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
    params.changelist_id = Some("123".to_string());
    params.file_paths = Some(files);

    let invocation = build_changelist_modify_invocation(&params, None).unwrap();

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
    let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
    params.changelist_id = Some("123".to_string());

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();

    assert!(error.contains("file_paths is required for move_files"));
}

#[test]
fn changelist_move_files_empty_id_errors() {
    let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
    params.changelist_id = Some(" ".to_string());
    params.file_paths = Some(vec!["//depot/main/a.rs".to_string()]);

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();

    assert!(error.contains("changelist_id is required for move_files"));
}

#[test]
fn changelist_create_without_stdin_errors() {
    let params = modify_changelists_params(ChangelistModifyAction::Create);

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdin is required for create"));
}

#[test]
fn changelist_update_without_stdin_errors() {
    let mut params = modify_changelists_params(ChangelistModifyAction::Update);
    params.changelist_id = Some("123".to_string());

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdin is required for update"));
}

#[test]
fn changelist_submit_empty_id_errors() {
    let mut params = modify_changelists_params(ChangelistModifyAction::Submit);
    params.changelist_id = Some(" ".to_string());

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changelist_id is required for submit"));
}

#[test]
fn changelist_delete_empty_id_errors() {
    let mut params = modify_changelists_params(ChangelistModifyAction::Delete);
    params.changelist_id = Some("".to_string());

    let error = build_changelist_modify_invocation(&params, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changelist_id is required for delete"));
}

#[test]
fn shelf_diff_uses_shelved_describe() {
    let invocation = build_shelf_query_invocation("diff", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["describe", "-S", "-du", "123"]);
    assert_eq!(invocation.mode, OutputMode::Text);
}

#[test]
fn shelf_diff_blank_changelist_errors() {
    let error = build_shelf_query_invocation("diff", Some(" "), None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changelist_id is required"));
}

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

    assert_eq!(
        invocation.args,
        vec!["unshelve", "-s", "12345", "-c", "54321"]
    );
    assert_eq!(invocation.mode, OutputMode::JsonLines);
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
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn modify_workspaces_delete_maps_to_client_delete() {
    let params = ModifyWorkspacesParams {
        action: WorkspaceModifyAction::Delete,
        workspace_name: "ws-main".to_string(),
        workspace_root: None,
        workspace_description: None,
        workspace_options: None,
        workspace_line_end: None,
        workspace_view: None,
        approval_token: None,
    };

    let invocation = build_workspace_delete_invocation(&params).unwrap();

    assert_eq!(invocation.args, vec!["client", "-d", "ws-main"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn patch_workspace_form_preserves_view_when_view_is_not_supplied() {
    let existing = "\
Client: ws-main
Root: /old/root
Options: noallwrite noclobber nocompress unlocked nomodtime normdir
LineEnd: local

View:
\t//depot/main/... //ws-main/main/...
";
    let patch = WorkspaceFormPatch {
        root: Some("/new/root".to_string()),
        description: Some("new description".to_string()),
        options: None,
        line_end: None,
        view: None,
    };

    let patched = patch_workspace_form(existing, &patch).unwrap();

    assert!(patched.contains("Root: /new/root"));
    assert!(patched.contains("Description:\n\tnew description"));
    assert!(
        patched.contains("Options: noallwrite noclobber nocompress unlocked nomodtime normdir")
    );
    assert!(patched.contains("LineEnd: local"));
    assert!(patched.contains("View:\n\t//depot/main/... //ws-main/main/..."));
}

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

#[test]
fn workspace_list_by_user_uses_user_filter() {
    let invocation = build_workspace_query_invocation("list", None, Some("alice"), 7).unwrap();
    assert_eq!(invocation.args, vec!["clients", "-m", "7", "-u", "alice"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn workspace_type_uses_client_spec() {
    let invocation = build_workspace_query_invocation("type", Some("ws-stream"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["client", "-o", "ws-stream"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn workspace_get_blank_name_errors() {
    let error = build_workspace_query_invocation("get", Some(" "), None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("workspace_name is required"));
}

#[test]
fn job_list_for_changelist_uses_fixes() {
    let invocation = build_job_query_invocation("list_jobs", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["fixes", "-c", "123"]);
}

#[test]
fn job_get_blank_id_errors() {
    let error = build_job_query_invocation("get_job", None, Some(" "), 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("job_id is required"));
}

#[test]
fn job_query_rejects_non_upstream_list_action() {
    let error = build_job_query_invocation("list", None, None, 10)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown action: list"));
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

#[test]
fn stream_list_uses_upstream_filters() {
    let params = QueryStreamsParams {
        action: StreamQueryAction::List,
        stream_name: None,
        stream_path: Some(vec!["//depot/...".to_string()]),
        filter: Some("Owner=alice".to_string()),
        fields: Some(vec![
            "Stream".to_string(),
            "Owner".to_string(),
            "Type".to_string(),
        ]),
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

    let error = build_stream_query_command(&params).unwrap_err().to_string();
    assert!(error.contains("stream_name is required"));
}
