use std::{
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
        params::{CommonModifyParams, CommonQueryParams, FileQueryAction, QueryFilesParams},
        reviews::{ReviewAction, ReviewRequest},
        server::ServerQueryAction,
    },
};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

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

fn write_config() -> AppConfig {
    AppConfig {
        readonly: false,
        ..test_config()
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
            user: None,
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
        ["changes", "-m", "7", "-s", "pending", "-c", "ws-main"]
    );
}

#[tokio::test]
async fn modify_workspaces_rejects_delete_without_workspace_name() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(write_config(), executor.clone());

    let err = match server
        .modify_workspaces(Parameters(CommonModifyParams {
            action: "delete".to_string(),
            changelist_id: None,
            workspace_name: None,
            stream: None,
            description: None,
            files: Vec::new(),
            form: None,
            confirmation: Some("PROCEED".to_string()),
        }))
        .await
    {
        Ok(_) => panic!("modify_workspaces should reject delete without a workspace name"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(
        err.message
            .contains("workspace_name is required for delete")
    );
    assert!(executor.invocations().is_empty());
}

#[tokio::test]
async fn modify_changelists_update_requires_changelist_id() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(write_config(), executor.clone());

    let err = match server
        .modify_changelists(Parameters(CommonModifyParams {
            action: "update".to_string(),
            changelist_id: None,
            workspace_name: None,
            stream: None,
            description: Some("update description".to_string()),
            files: Vec::new(),
            form: None,
            confirmation: None,
        }))
        .await
    {
        Ok(_) => panic!("modify_changelists should reject update without changelist_id"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("changelist_id is required for update"));
    assert!(executor.invocations().is_empty());
}

#[tokio::test]
async fn modify_changelists_update_form_uses_requested_changelist() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(write_config(), executor.clone());

    let response = server
        .modify_changelists(Parameters(CommonModifyParams {
            action: "update".to_string(),
            changelist_id: Some("123".to_string()),
            workspace_name: None,
            stream: None,
            description: Some("update description".to_string()),
            files: vec!["//depot/main/file.rs".to_string()],
            form: None,
            confirmation: None,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args, ["change", "-i"]);
    assert!(
        invocations[0]
            .stdin
            .as_deref()
            .is_some_and(|stdin| stdin.contains("Change: 123"))
    );
}

#[tokio::test]
async fn query_reviews_returns_dry_run_request_metadata() {
    let server = P4McpServer::new(test_config());

    let response = server
        .query_reviews(Parameters(ReviewRequest {
            action: ReviewAction::List,
            review_id: None,
            max_results: 5,
            body: json!({}),
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "dry_run");
    assert_eq!(response.0.action, "query_reviews");
    assert_eq!(response.0.message["method"], "GET");
    assert_eq!(response.0.message["path"], "/reviews");
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
