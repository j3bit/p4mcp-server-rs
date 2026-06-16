use std::{
    collections::VecDeque,
    env,
    ffi::OsString,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use p4mcp_server_rs::{
    config::{AppConfig, SslVerify, Toolset, TransportMode},
    error::P4McpError,
    p4::runner::{P4CommandOutput, P4Env, P4Executor, P4Invocation},
    server::P4McpServer,
    tools::{
        params::{
            ChangelistQueryAction, FileQueryAction, QueryChangelistsParams, QueryFilesParams,
            QueryStreamsParams, QueryWorkspacesParams, StreamQueryAction, WorkspaceQueryAction,
        },
        reviews::{QueryReviewsParams, ReviewQueryAction},
        server::{QueryServerParams, ServerQueryAction},
    },
};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::Tool;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

static REVIEW_AUTH_ENV_LOCK: Mutex<()> = Mutex::new(());

struct ReviewAuthEnvGuard {
    _lock: MutexGuard<'static, ()>,
    p4passwd: Option<OsString>,
    p4config: Option<OsString>,
}

impl ReviewAuthEnvGuard {
    fn new(p4passwd: &str) -> Self {
        let lock = REVIEW_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_p4passwd = env::var_os("P4PASSWD");
        let previous_p4config = env::var_os("P4CONFIG");
        unsafe {
            env::set_var("P4PASSWD", p4passwd);
            env::remove_var("P4CONFIG");
        }
        Self {
            _lock: lock,
            p4passwd: previous_p4passwd,
            p4config: previous_p4config,
        }
    }
}

impl Drop for ReviewAuthEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.p4passwd {
                Some(value) => env::set_var("P4PASSWD", value),
                None => env::remove_var("P4PASSWD"),
            }
            match &self.p4config {
                Some(value) => env::set_var("P4CONFIG", value),
                None => env::remove_var("P4CONFIG"),
            }
        }
    }
}

fn test_config() -> AppConfig {
    AppConfig {
        readonly: true,
        allow_usage: false,
        toolsets: Toolset::default_set(),
        transport: TransportMode::Stdio,
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 8000,
        p4_bin: "p4".into(),
        log_dir: None,
        ssl_verify: SslVerify::Enabled,
    }
}

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

fn query_reviews_params(action: ReviewQueryAction) -> QueryReviewsParams {
    QueryReviewsParams {
        action,
        review_id: None,
        fields: None,
        comments_fields: Some("id,body,user,time".to_string()),
        up_voters: None,
        from_version: None,
        to_version: None,
        max_results: 10,
        after: None,
        after_updated: None,
        result_order: None,
        projects: None,
        state: None,
        keywords: None,
        keywords_fields: None,
        include_transitions: None,
    }
}

#[test]
fn server_constructs_with_config() {
    let server = P4McpServer::new(test_config());
    assert!(server.config().readonly);
}

#[test]
fn initial_tool_names_are_registered() {
    let names = P4McpServer::tool_names();

    assert_eq!(
        names,
        [
            "modify_changelists",
            "modify_files",
            "modify_jobs",
            "modify_reviews",
            "modify_shelves",
            "modify_streams",
            "modify_workspaces",
            "query_changelists",
            "query_files",
            "query_jobs",
            "query_reviews",
            "query_server",
            "query_shelves",
            "query_streams",
            "query_workspaces",
        ]
    );
    assert_eq!(names.len(), 15);
}

#[test]
fn tool_metadata_marks_query_tools_read_only() {
    let tools = P4McpServer::tools();

    for tool in tools.iter().filter(|tool| tool.name.starts_with("query_")) {
        assert_eq!(
            annotations(tool).read_only_hint,
            Some(true),
            "{} should be marked read-only",
            tool.name
        );
    }
}

#[test]
fn tool_metadata_marks_modify_tools_writable() {
    let tools = P4McpServer::tools();

    for tool in tools.iter().filter(|tool| tool.name.starts_with("modify_")) {
        assert_eq!(
            annotations(tool).read_only_hint,
            Some(false),
            "{} should be marked writable",
            tool.name
        );
    }
}

#[test]
fn tool_metadata_marks_destructive_modify_tools() {
    let tools = P4McpServer::tools();
    let destructive_tools = [
        "modify_changelists",
        "modify_files",
        "modify_reviews",
        "modify_shelves",
        "modify_streams",
        "modify_workspaces",
    ];

    for tool_name in destructive_tools {
        let tool = tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("{tool_name} should be registered"));
        assert_eq!(
            annotations(tool).destructive_hint,
            Some(true),
            "{tool_name} should be marked destructive"
        );
    }
}

#[tokio::test]
async fn query_server_calls_injected_executor() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"serverRoot": "/p4"})],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_server(Parameters(QueryServerParams {
            action: ServerQueryAction::ServerInfo,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "query_server");
    assert_eq!(response.0.message, json!([{"serverRoot": "/p4"}]));
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(executor.invocations()[0].args, ["info"]);
}

#[tokio::test]
async fn query_changelists_calls_injected_executor() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"change": "123"})],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

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

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
    assert_eq!(response.0.message, json!([{"change": "123"}]));
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(
        executor.invocations()[0].args,
        [
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

#[tokio::test]
async fn query_workspaces_list_by_user_calls_injected_executor() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({"client": "ws-main", "Owner": "alice"})],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::List,
            workspace_name: None,
            user: Some("alice".to_string()),
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

#[tokio::test]
async fn query_workspaces_type_classifies_stream_workspace() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![json!({
            "Client": "ws-stream",
            "Stream": "//streams/main"
        })],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Type,
            workspace_name: Some("ws-stream".to_string()),
            user: None,
            max_results: 10,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "type");
    assert_eq!(response.0.message, json!({"workspace_type": "stream"}));
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(
        executor.invocations()[0].args,
        ["client", "-o", "ws-stream"]
    );
}

#[tokio::test]
async fn query_workspaces_status_runs_upstream_status_commands() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Client": "ws-main", "View0": "//depot/... //ws-main/..."})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/open.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/out-of-sync.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"fromFile": "//depot/main/base.rs"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"change": "42"})],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Status,
            workspace_name: Some("ws-main".to_string()),
            user: None,
            max_results: 10,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "status");
    assert_eq!(
        response.0.message,
        json!({
            "opened_files": ["//depot/main/open.rs"],
            "out_of_sync_files": ["//depot/main/out-of-sync.rs"],
            "sync_warnings": [],
            "pending_resolves": ["//depot/main/base.rs"],
            "last_synced_cl": "42"
        })
    );

    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 5);
    assert_eq!(invocations[0].args, ["client", "-o", "ws-main"]);
    assert_eq!(invocations[1].args, ["opened"]);
    assert_eq!(invocations[2].args, ["sync", "-n"]);
    assert_eq!(invocations[3].args, ["resolve", "-n"]);
    assert_eq!(invocations[4].args, ["changes", "-m1", "#have"]);
}

#[tokio::test]
async fn query_workspaces_status_requires_workspace_name() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_workspaces(Parameters(QueryWorkspacesParams {
            action: WorkspaceQueryAction::Status,
            workspace_name: None,
            user: None,
            max_results: 10,
        }))
        .await
    {
        Ok(_) => panic!("query_workspaces status should require workspace_name"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("workspace_name is required"));
    assert!(executor.invocations().is_empty());
}

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
    assert!(
        err.message
            .contains("stream does not exist: //streams/missing")
    );
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
    assert_eq!(
        invocations[1].args,
        ["stream", "-o", "//streams/current@12345"]
    );
}

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
        [
            "clients",
            "-U",
            "-S",
            "//streams/dev",
            "-u",
            "alice",
            "-m",
            "5"
        ]
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
    assert!(
        err.message
            .contains("Workspace 'missing-ws' does not exist")
    );
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
        [
            "client",
            "-o",
            "-S",
            "//streams/dev",
            "-t",
            "template-ws",
            "stream-ws"
        ]
    );
}

#[tokio::test]
async fn query_streams_interchanges_runs_upstream_command_and_limits_client_side() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/workspace"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![
                json!({"change": "101"}),
                json!({"change": "102"}),
                json!({"change": "103"}),
            ],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::Interchanges);
    params.reverse = true;
    params.long_output = true;
    params.limit = Some(2);
    params.file_paths = Some(vec!["//streams/dev/src/...".to_string()]);

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "interchanges");
    assert_eq!(
        response.0.message,
        json!({
            "changelists": [{"change": "101"}, {"change": "102"}],
            "count": 2,
            "source_stream": "//streams/workspace",
            "workspace_stream": "//streams/workspace",
            "direction": "reverse",
            "message": "2 outstanding changelist(s) in '//streams/workspace' not yet propagated to '//streams/dev'"
        })
    );
    let invocations = executor.invocations();
    assert_eq!(invocations[0].args, ["client", "-o"]);
    assert_eq!(
        invocations[1].args,
        ["streams", "-F", "Stream=//streams/dev"]
    );
    assert_eq!(
        invocations[2].args,
        [
            "interchanges",
            "-S",
            "//streams/dev",
            "-r",
            "-l",
            "//streams/dev/src/..."
        ]
    );
}

#[tokio::test]
async fn query_streams_interchanges_rejects_classic_workspace_before_stream_lookup() {
    let executor = Arc::new(QueuedExecutor::success(vec![P4CommandOutput {
        records: vec![json!({"Client": "classic-ws"})],
        text: json!({}),
    }]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_streams(Parameters(stream_query_params(
            StreamQueryAction::Interchanges,
        )))
        .await
    {
        Ok(_) => panic!("query_streams interchanges should require a stream workspace"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("stream-based workspace"));
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(executor.invocations()[0].args, ["client", "-o"]);
}

#[tokio::test]
async fn query_streams_validate_file_reads_indexed_stream_rules_and_later_exclude_wins() {
    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({
                "Stream": "//streams/dev",
                "Update": "2026/06/15"
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "Paths0": "share ...",
                "Paths1": "exclude src/private/...",
                "Ignored0": "*.tmp"
            })],
            text: json!({}),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());
    let mut params = stream_query_params(StreamQueryAction::ValidateFile);
    params.file_paths = Some(vec![
        "//streams/dev/src/private/secret.rs".to_string(),
        "//streams/dev/src/main.rs".to_string(),
        "//streams/dev/build.tmp".to_string(),
    ]);

    let response = server.query_streams(Parameters(params)).await.unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "validate_file");
    assert_eq!(response.0.message["all_allowed"], json!(false));
    assert_eq!(response.0.message["results"][0]["allowed"], json!(false));
    assert_eq!(response.0.message["results"][0]["rule"], json!("excluded"));
    assert_eq!(response.0.message["results"][1]["allowed"], json!(true));
    assert_eq!(response.0.message["results"][1]["rule"], json!("share"));
    assert_eq!(response.0.message["results"][2]["allowed"], json!(false));
    assert_eq!(response.0.message["results"][2]["rule"], json!("ignored"));
    let invocations = executor.invocations();
    assert_eq!(invocations[0].args, ["client", "-o"]);
    assert_eq!(invocations[1].args, ["stream", "-o", "-v", "//streams/dev"]);
}

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
        .query_streams(Parameters(stream_query_params(
            StreamQueryAction::CheckResolve,
        )))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "check_resolve");
    assert_eq!(response.0.message["resolve_needed"], json!(true));
    let invocations = executor.invocations();
    assert_eq!(
        invocations[0].args,
        ["streams", "-F", "Stream=//streams/dev"]
    );
    assert_eq!(invocations[1].args, ["stream", "resolve", "-n"]);
}

#[tokio::test]
async fn query_streams_check_resolve_rejects_missing_stream_before_preview() {
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
        .query_streams(Parameters(stream_query_params(
            StreamQueryAction::CheckResolve,
        )))
        .await
    {
        Ok(_) => panic!("query_streams check_resolve should reject missing streams"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("does not exist"));
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
async fn query_streams_parent_rejects_missing_stream_before_fetch() {
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
        .query_streams(Parameters(stream_query_params(StreamQueryAction::Parent)))
        .await
    {
        Ok(_) => panic!("query_streams parent should reject missing streams"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("does not exist"));
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
async fn query_files_grep_caps_records_by_max_results() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: vec![
            json!({"depotFile": "//depot/main/a.rs", "line": "1"}),
            json!({"depotFile": "//depot/main/b.rs", "line": "2"}),
            json!({"depotFile": "//depot/main/c.rs", "line": "3"}),
        ],
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_files(Parameters(QueryFilesParams {
            action: FileQueryAction::Grep,
            file_path: "//depot/main/...".to_string(),
            file2: None,
            diff2: true,
            max_results: 2,
            pattern: Some("needle".to_string()),
            case_insensitive: true,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "grep");
    assert_eq!(
        response.0.message,
        json!([
            {"depotFile": "//depot/main/a.rs", "line": "1"},
            {"depotFile": "//depot/main/b.rs", "line": "2"},
        ])
    );
    assert_eq!(executor.invocations().len(), 1);
    assert_eq!(
        executor.invocations()[0].args,
        ["grep", "-n", "-i", "-e", "needle", "//depot/main/..."]
    );
}

#[tokio::test]
async fn query_reviews_executes_review_api_request() {
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(header("authorization", "Basic YWxpY2U6dGlja2V0LTEyMw=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reviews": [123]
        })))
        .expect(1)
        .mount(&swarm)
        .await;

    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({
                "userName": "alice",
                "serverAddress": "perforce:1666"
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "value": swarm.uri()
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "perforce:1666 (alice) ticket-123\n",
                "stderr": ""
            }),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_reviews(Parameters(QueryReviewsParams {
            max_results: 5,
            ..query_reviews_params(ReviewQueryAction::List)
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
    assert_eq!(response.0.message, json!({ "reviews": [123] }));

    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].args, ["info"]);
    assert_eq!(
        invocations[1].args,
        ["property", "-l", "-n", "P4.Swarm.URL"]
    );
    assert_eq!(invocations[2].args, ["tickets"]);
}

#[tokio::test(flavor = "current_thread")]
async fn query_reviews_uses_p4passwd_when_tickets_are_empty() {
    let _env = ReviewAuthEnvGuard::new("password-123");
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(header("authorization", "Basic YWxpY2U6cGFzc3dvcmQtMTIz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reviews": [123]
        })))
        .expect(1)
        .mount(&swarm)
        .await;

    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({
                "userName": "alice",
                "serverAddress": "perforce:1666"
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "value": swarm.uri()
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "",
                "stderr": ""
            }),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_reviews(Parameters(QueryReviewsParams {
            max_results: 5,
            ..query_reviews_params(ReviewQueryAction::List)
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
    assert_eq!(response.0.message, json!({ "reviews": [123] }));

    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].args, ["info"]);
    assert_eq!(
        invocations[1].args,
        ["property", "-l", "-n", "P4.Swarm.URL"]
    );
    assert_eq!(invocations[2].args, ["tickets"]);
}

#[tokio::test]
async fn query_reviews_list_threads_upstream_filters_to_http() {
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(query_param("fields[]", "id"))
        .and(header("authorization", "Basic YWxpY2U6dGlja2V0LTEyMw=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reviews": [{"id": 123}]
        })))
        .expect(1)
        .mount(&swarm)
        .await;

    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({
                "userName": "alice",
                "serverAddress": "perforce:1666"
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "value": swarm.uri()
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "perforce:1666 (alice) ticket-123\n",
                "stderr": ""
            }),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor);

    let response = server
        .query_reviews(Parameters(QueryReviewsParams {
            fields: Some(vec!["id".to_string()]),
            max_results: 5,
            ..query_reviews_params(ReviewQueryAction::List)
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
}

#[tokio::test]
async fn query_reviews_rejects_missing_review_id_before_p4_discovery() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_reviews(Parameters(query_reviews_params(ReviewQueryAction::Get)))
        .await
    {
        Ok(_) => panic!("query_reviews should reject missing review_id"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("review_id is required"));
    assert!(executor.invocations().is_empty());
}

#[tokio::test]
async fn invalid_tool_params_return_invalid_params_error() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor);

    let err = match server
        .query_files(Parameters(QueryFilesParams {
            action: FileQueryAction::Diff,
            file_path: "//depot/main.c".to_string(),
            file2: None,
            diff2: true,
            max_results: 10,
            pattern: None,
            case_insensitive: false,
        }))
        .await
    {
        Ok(_) => panic!("query_files should reject missing file2 for diff2"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("file2 is required for diff2"));
}

#[tokio::test]
async fn executor_failures_return_internal_error() {
    let executor = Arc::new(FakeExecutor::failure(P4McpError::P4Command {
        message: "spawn failed".to_string(),
    }));
    let server = P4McpServer::with_executor(test_config(), executor);

    let err = match server
        .query_server(Parameters(QueryServerParams {
            action: ServerQueryAction::ServerInfo,
        }))
        .await
    {
        Ok(_) => panic!("query_server should surface fake executor failure"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::internal_error("", None).code);
    assert!(err.message.contains("p4 command failed"));
}

struct FakeExecutor {
    result: FakeResult,
    invocations: Mutex<Vec<P4Invocation>>,
}

impl FakeExecutor {
    fn success(output: P4CommandOutput) -> Self {
        Self {
            result: FakeResult::Success(output),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn failure(error: P4McpError) -> Self {
        Self {
            result: FakeResult::Failure(error),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn invocations(&self) -> Vec<P4Invocation> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

enum FakeResult {
    Success(P4CommandOutput),
    Failure(P4McpError),
}

fn annotations(tool: &Tool) -> &rmcp::model::ToolAnnotations {
    tool.annotations
        .as_ref()
        .unwrap_or_else(|| panic!("{} should have annotations", tool.name))
}

#[async_trait]
impl P4Executor for FakeExecutor {
    async fn run(
        &self,
        invocation: P4Invocation,
        _env: P4Env,
    ) -> p4mcp_server_rs::error::Result<P4CommandOutput> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(invocation);
        match &self.result {
            FakeResult::Success(output) => Ok(output.clone()),
            FakeResult::Failure(P4McpError::P4Command { message }) => Err(P4McpError::P4Command {
                message: message.clone(),
            }),
            FakeResult::Failure(_) => unreachable!("fake executor only emits p4 command failures"),
        }
    }
}

struct QueuedExecutor {
    outputs: Mutex<VecDeque<Result<P4CommandOutput, P4McpError>>>,
    invocations: Mutex<Vec<P4Invocation>>,
}

impl QueuedExecutor {
    fn success(outputs: Vec<P4CommandOutput>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn invocations(&self) -> Vec<P4Invocation> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl P4Executor for QueuedExecutor {
    async fn run(
        &self,
        invocation: P4Invocation,
        _env: P4Env,
    ) -> Result<P4CommandOutput, P4McpError> {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(invocation);

        self.outputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                Err(P4McpError::P4Command {
                    message: "queued executor exhausted".to_string(),
                })
            })
    }
}
