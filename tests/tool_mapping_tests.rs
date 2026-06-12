use p4mcp_server_rs::{
    config::Toolset,
    p4::runner::{OutputMode, P4Invocation},
    permissions::{Access, SafetyPolicy},
    tools::{
        files::build_file_invocation,
        params::{FileModifyAction, FileQueryAction, ModifyFilesParams, QueryFilesParams},
        server::{ServerQueryAction, build_server_invocation},
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
