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
        params::{FileQueryAction, QueryFilesParams},
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

#[test]
fn server_constructs_with_config() {
    let server = P4McpServer::new(test_config());
    assert!(server.config().readonly);
}

#[test]
fn initial_tool_names_are_registered() {
    let names = P4McpServer::tool_names();

    assert_eq!(names, ["modify_files", "query_files", "query_server"]);
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
