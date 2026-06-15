use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use p4mcp_server_rs::{
    config::{AppConfig, SslVerify, Toolset, TransportMode},
    error::P4McpError,
    p4::runner::{P4CommandOutput, P4Env, P4Executor, P4Invocation},
    server::P4McpServer,
    tools::{
        params::{CommonQueryParams, FileQueryAction, QueryFilesParams},
        reviews::{ReviewAction, ReviewRequest},
        server::ServerQueryAction,
    },
};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::Tool;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        .query_server(Parameters(ServerQueryAction::ServerInfo))
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
        .query_changelists(Parameters(CommonQueryParams {
            action: "list".to_string(),
            changelist_id: None,
            workspace_name: Some("ws-main".to_string()),
            file_path: None,
            user: Some("alice".to_string()),
            status: Some("pending".to_string()),
            job_id: None,
            stream: None,
            owner: None,
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
            "changes", "-m", "7", "-s", "pending", "-c", "ws-main", "-u", "alice"
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
        .query_workspaces(Parameters(CommonQueryParams {
            action: "list".to_string(),
            changelist_id: None,
            workspace_name: None,
            file_path: None,
            user: Some("alice".to_string()),
            status: None,
            job_id: None,
            stream: None,
            owner: None,
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
        .query_workspaces(Parameters(CommonQueryParams {
            action: "type".to_string(),
            changelist_id: None,
            workspace_name: Some("ws-stream".to_string()),
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
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
        .query_workspaces(Parameters(CommonQueryParams {
            action: "status".to_string(),
            changelist_id: None,
            workspace_name: Some("ws-main".to_string()),
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
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
        .query_workspaces(Parameters(CommonQueryParams {
            action: "status".to_string(),
            changelist_id: None,
            workspace_name: None,
            file_path: None,
            user: None,
            status: None,
            job_id: None,
            stream: None,
            owner: None,
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
        .query_reviews(Parameters(ReviewRequest {
            action: ReviewAction::List,
            review_id: None,
            max_results: 5,
            body: json!({}),
            approval_token: None,
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
async fn query_reviews_rejects_write_actions_before_p4_discovery() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_reviews(Parameters(ReviewRequest {
            action: ReviewAction::Vote,
            review_id: Some(123),
            max_results: 10,
            body: json!({"vote": "up"}),
            approval_token: None,
        }))
        .await
    {
        Ok(_) => panic!("query_reviews should reject write review actions"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(
        err.message
            .contains("query_reviews only supports read review actions")
    );
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
        .query_server(Parameters(ServerQueryAction::ServerInfo))
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
