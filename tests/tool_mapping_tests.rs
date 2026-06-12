use p4mcp_server_rs::{
    config::Toolset,
    error::P4McpError,
    p4::runner::{OutputMode, P4Invocation},
    permissions::{Access, SafetyPolicy},
    tools::{
        changelists::{build_changelist_modify_invocation, build_changelist_query_invocation},
        files::{build_file_invocation, build_file_modify_invocation},
        jobs::build_job_query_invocation,
        params::{FileModifyAction, FileQueryAction, ModifyFilesParams, QueryFilesParams},
        server::{ServerQueryAction, build_server_invocation},
        shelves::build_shelf_query_invocation,
        streams::build_stream_query_invocation,
        workspaces::build_workspace_query_invocation,
    },
};

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
fn delete_requires_proceed_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: Some(vec!["//depot/main/old.txt".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: None,
    };
    assert!(params.requires_confirmation());
    assert!(params.confirmed().is_err());
}

#[test]
fn query_server_info_maps_to_info() {
    let invocation = build_server_invocation(ServerQueryAction::ServerInfo);
    assert_eq!(invocation.args, vec!["info"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn query_current_user_maps_to_user_output() {
    let invocation = build_server_invocation(ServerQueryAction::CurrentUser);
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
        confirmation: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(invocation.args, vec!["add", "-c", "123", "src/new.rs"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn modify_file_delete_requires_confirmation_before_command() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: Some(vec!["//depot/main/old.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: None,
    };

    assert!(build_file_modify_invocation(&params).is_err());
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
        confirmation: None,
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
        confirmation: None,
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("file_paths is required for add"));
}

#[test]
fn modify_file_confirmed_delete_requires_files_after_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: None,
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: Some("PROCEED".to_string()),
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("file_paths is required for delete"));
}

#[test]
fn modify_file_confirmed_revert_maps_changelist_and_file() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Revert,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "456".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: Some("PROCEED".to_string()),
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
        confirmation: None,
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
        confirmation: None,
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
        confirmation: None,
    };

    let error = build_file_modify_invocation(&params)
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid resolve mode: bad"));
}

#[test]
fn modify_file_resolve_theirs_requires_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "theirs".to_string(),
        force: false,
        confirmation: None,
    };

    assert!(matches!(
        build_file_modify_invocation(&params),
        Err(P4McpError::ConfirmationRequired)
    ));
}

#[test]
fn modify_file_resolve_force_requires_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "force".to_string(),
        force: false,
        confirmation: None,
    };

    assert!(matches!(
        build_file_modify_invocation(&params),
        Err(P4McpError::ConfirmationRequired)
    ));
}

#[test]
fn modify_file_resolve_theirs_confirmed_maps_to_at() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "theirs".to_string(),
        force: false,
        confirmation: Some("PROCEED".to_string()),
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["resolve", "-at", "//depot/main/file.rs"]
    );
}

#[test]
fn default_changelist_get_uses_opened_not_describe() {
    let invocation =
        build_changelist_query_invocation("get", Some("default"), None, None, 10).unwrap();
    assert_eq!(invocation.args, vec!["opened", "-c", "default"]);
}

#[test]
fn numbered_changelist_get_uses_describe() {
    let invocation = build_changelist_query_invocation("get", Some("123"), None, None, 10).unwrap();
    assert_eq!(invocation.args, vec!["describe", "-s", "123"]);
}

#[test]
fn changelist_submit_uses_numbered_change() {
    let invocation = build_changelist_modify_invocation("submit", "123", None).unwrap();
    assert_eq!(invocation.args, vec!["submit", "-c", "123"]);
}

#[test]
fn changelist_create_without_stdin_errors() {
    let error = build_changelist_modify_invocation("create", "", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdin is required for create"));
}

#[test]
fn changelist_update_without_stdin_errors() {
    let error = build_changelist_modify_invocation("update", "123", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdin is required for update"));
}

#[test]
fn changelist_submit_empty_id_errors() {
    let error = build_changelist_modify_invocation("submit", " ", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changelist_id is required for submit"));
}

#[test]
fn changelist_delete_empty_id_errors() {
    let error = build_changelist_modify_invocation("delete", "", None)
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
fn workspace_where_maps_file_argument() {
    let invocation =
        build_workspace_query_invocation("where", None, Some("//depot/main/file.rs"), 10).unwrap();
    assert_eq!(invocation.args, vec!["where", "//depot/main/file.rs"]);
}

#[test]
fn job_list_for_changelist_uses_fixes() {
    let invocation = build_job_query_invocation("list_jobs", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["fixes", "-c", "123"]);
}

#[test]
fn stream_list_with_owner_uses_owner_filter() {
    let invocation = build_stream_query_invocation("list", None, Some("alice"), 25).unwrap();
    assert_eq!(
        invocation.args,
        vec!["streams", "-m", "25", "-F", "Owner=alice"]
    );
}

#[test]
fn stream_integration_status_uses_istat() {
    let invocation =
        build_stream_query_invocation("integration_status", Some("//streams/dev"), None, 10)
            .unwrap();
    assert_eq!(invocation.args, vec!["istat", "-s", "//streams/dev"]);
}
