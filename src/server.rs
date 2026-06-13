use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use rmcp::{
    ErrorData, Json, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::{
    config::{AppConfig, Cli, Toolset, TransportMode},
    error::P4McpError,
    p4::{
        forms::{change_form, change_form_for},
        runner::{OutputMode, P4CommandOutput, P4Env, P4Executor, P4Invocation, TokioP4Executor},
    },
    permissions::{Access, SafetyPolicy},
    tools::{
        changelists::{build_changelist_modify_invocation, build_changelist_query_invocation},
        files::{build_file_invocation, build_file_modify_invocation},
        jobs::build_job_query_invocation,
        params::{CommonModifyParams, CommonQueryParams, ModifyFilesParams, QueryFilesParams},
        response::ToolResponse,
        reviews::ReviewRequest,
        server::{ServerQueryAction, build_server_invocation},
        shelves::build_shelf_query_invocation,
        streams::build_stream_query_invocation,
        workspaces::build_workspace_query_invocation,
    },
};

pub struct P4McpServer {
    config: Arc<AppConfig>,
    executor: Arc<dyn P4Executor>,
}

impl P4McpServer {
    pub fn new(config: AppConfig) -> Self {
        Self {
            executor: Arc::new(TokioP4Executor::new(config.p4_bin.clone())),
            config: Arc::new(config),
        }
    }

    pub fn with_executor(config: AppConfig, executor: Arc<dyn P4Executor>) -> Self {
        Self {
            config: Arc::new(config),
            executor,
        }
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn policy(&self) -> SafetyPolicy {
        SafetyPolicy::new(self.config.readonly, self.config.toolsets.clone())
    }

    pub fn tool_names() -> Vec<String> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect()
    }

    async fn run_p4(&self, invocation: P4Invocation) -> McpResult<P4CommandOutput> {
        self.executor
            .run(invocation, P4Env::new())
            .await
            .map_err(to_mcp_error)
    }

    async fn call_p4_tool(
        &self,
        action: &str,
        invocation: P4Invocation,
    ) -> McpResult<Json<ToolResponse>> {
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(action, output_message(output))))
    }
}

type McpResult<T> = std::result::Result<T, ErrorData>;

#[tool_router]
impl P4McpServer {
    #[tool(description = "Query Perforce server metadata")]
    pub async fn query_server(
        &self,
        Parameters(action): Parameters<ServerQueryAction>,
    ) -> McpResult<Json<ToolResponse>> {
        let invocation = build_server_invocation(action);
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(
            "query_server",
            Value::Array(output.records),
        )))
    }

    #[tool(description = "Query Perforce files")]
    pub async fn query_files(
        &self,
        Parameters(params): Parameters<QueryFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Files, "query_files")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_file_invocation(&params).map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(description = "Modify Perforce files")]
    pub async fn modify_files(
        &self,
        Parameters(params): Parameters<ModifyFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Files, "modify_files")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_file_modify_invocation(&params).map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(description = "Get changelist information or list changelists")]
    pub async fn query_changelists(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Changelists, "query_changelists")
            .map_err(to_mcp_error)?;
        let invocation = build_changelist_query_invocation(
            &params.action,
            params.changelist_id.as_deref(),
            params.status.as_deref(),
            params.workspace_name.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "Create, update, submit, or delete changelists")]
    pub async fn modify_changelists(
        &self,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Changelists, "modify_changelists")
            .map_err(to_mcp_error)?;
        let changelist_id = match params.action.as_str() {
            "create" => "new".to_string(),
            "update" | "submit" | "delete" => required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &params.action,
            )?,
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        if params.action == "delete" {
            require_confirmation(params.confirmation.as_deref())?;
        }
        let stdin = params.form.clone().or_else(|| {
            params.description.as_ref().map(|description| {
                if params.action == "create" {
                    change_form(description, &params.files)
                } else {
                    change_form_for(&changelist_id, description, &params.files)
                }
            })
        });
        let invocation = build_changelist_modify_invocation(&params.action, &changelist_id, stdin)
            .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "List shelves, show shelf diff, or list shelf files")]
    pub async fn query_shelves(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Shelves, "query_shelves")
            .map_err(to_mcp_error)?;
        let invocation = build_shelf_query_invocation(
            &params.action,
            params.changelist_id.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "Shelve, unshelve, or delete shelved files")]
    pub async fn modify_shelves(
        &self,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Shelves, "modify_shelves")
            .map_err(to_mcp_error)?;
        let change = required_option(
            params.changelist_id.as_deref(),
            "changelist_id",
            &params.action,
        )?;
        let args = match params.action.as_str() {
            "shelve" => vec!["shelve".to_string(), "-c".to_string(), change],
            "unshelve" => vec!["unshelve".to_string(), "-s".to_string(), change],
            "delete" => {
                require_confirmation(params.confirmation.as_deref())?;
                vec![
                    "shelve".to_string(),
                    "-d".to_string(),
                    "-c".to_string(),
                    change,
                ]
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        self.call_p4_tool(&params.action, json_invocation(args, None))
            .await
    }

    #[tool(description = "List, get, map, or inspect workspaces")]
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        let invocation = build_workspace_query_invocation(
            &params.action,
            params.workspace_name.as_deref(),
            params.file_path.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "Create, update, or delete workspaces using p4 client forms")]
    pub async fn modify_workspaces(
        &self,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Workspaces, "modify_workspaces")
            .map_err(to_mcp_error)?;
        let (args, stdin) = match params.action.as_str() {
            "create" | "update" => (
                vec!["client".to_string(), "-i".to_string()],
                Some(required_form(params.form.as_deref(), &params.action)?),
            ),
            "delete" => {
                require_confirmation(params.confirmation.as_deref())?;
                let workspace_name =
                    required_option(params.workspace_name.as_deref(), "workspace_name", "delete")?;
                (
                    vec!["client".to_string(), "-d".to_string(), workspace_name],
                    None,
                )
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        self.call_p4_tool(&params.action, json_invocation(args, stdin))
            .await
    }

    #[tool(description = "List or get jobs and fixes")]
    pub async fn query_jobs(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Jobs, "query_jobs")
            .map_err(to_mcp_error)?;
        let invocation = build_job_query_invocation(
            &params.action,
            params.changelist_id.as_deref(),
            params.job_id.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "Attach or detach jobs from changelists")]
    pub async fn modify_jobs(
        &self,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Jobs, "modify_jobs")
            .map_err(to_mcp_error)?;
        let change = required_option(
            params.changelist_id.as_deref(),
            "changelist_id",
            &params.action,
        )?;
        let job = params
            .files
            .first()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| to_mcp_error(invalid_input("files[0] must contain the job id")))?;
        let args = match params.action.as_str() {
            "fix" => vec!["fix".to_string(), "-c".to_string(), change, job],
            "unfix" => vec![
                "fix".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                change,
                job,
            ],
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        self.call_p4_tool(&params.action, json_invocation(args, None))
            .await
    }

    #[tool(
        description = "List streams, get stream specs, graph streams, and inspect stream integration status"
    )]
    pub async fn query_streams(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Streams, "query_streams")
            .map_err(to_mcp_error)?;
        let invocation = build_stream_query_invocation(
            &params.action,
            params.stream.as_deref(),
            params.owner.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(description = "Create, update, or delete stream specs")]
    pub async fn modify_streams(
        &self,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Streams, "modify_streams")
            .map_err(to_mcp_error)?;
        let (args, stdin) = match params.action.as_str() {
            "create" | "update" => (
                vec!["stream".to_string(), "-i".to_string()],
                Some(required_form(params.form.as_deref(), &params.action)?),
            ),
            "delete" => {
                require_confirmation(params.confirmation.as_deref())?;
                let stream = required_option(params.stream.as_deref(), "stream", "delete")?;
                (vec!["stream".to_string(), "-d".to_string(), stream], None)
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        self.call_p4_tool(&params.action, json_invocation(args, stdin))
            .await
    }

    #[tool(description = "Query P4 Code Review / Swarm reviews")]
    pub async fn query_reviews(
        &self,
        Parameters(params): Parameters<ReviewRequest>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Reviews, "query_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        Ok(Json(ToolResponse::dry_run(
            "query_reviews",
            review_message(built),
        )))
    }

    #[tool(description = "Modify P4 Code Review / Swarm reviews")]
    pub async fn modify_reviews(
        &self,
        Parameters(params): Parameters<ReviewRequest>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Reviews, "modify_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        Ok(Json(ToolResponse::dry_run(
            "modify_reviews",
            review_message(built),
        )))
    }
}

#[tool_handler(
    name = "p4-mcp-server",
    version = "0.1.0",
    instructions = "MCP server for Perforce P4."
)]
impl ServerHandler for P4McpServer {}

pub async fn run_from_cli() -> Result<()> {
    let config = Cli::parse().into_config()?;
    init_logging();

    match config.transport {
        TransportMode::Stdio => run_stdio(config).await,
        TransportMode::Http => run_http(config).await,
    }
}

pub async fn run_stdio(config: AppConfig) -> Result<()> {
    let service = P4McpServer::new(config)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

pub async fn run_http(config: AppConfig) -> Result<()> {
    let host = config.host;
    let port = config.port;
    let config = Arc::new(config);
    let ct = CancellationToken::new();
    let service: StreamableHttpService<P4McpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(P4McpServer::new((*config).clone())),
            Default::default(),
            StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
        );

    let router = axum::Router::new().nest_service("/mcp", service);
    let addr = (host, port);
    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            ct.cancel();
        })
        .await?;
    Ok(())
}

fn output_message(output: P4CommandOutput) -> Value {
    if output.records.is_empty() {
        output.text
    } else {
        Value::Array(output.records)
    }
}

fn json_invocation(args: Vec<String>, stdin: Option<String>) -> P4Invocation {
    P4Invocation {
        args,
        stdin,
        mode: OutputMode::JsonLines,
    }
}

fn review_message(built: crate::tools::reviews::BuiltReviewRequest) -> Value {
    serde_json::json!({
        "method": built.method,
        "path": built.path,
        "query": built.query,
        "body": built.body,
    })
}

fn require_confirmation(confirmation: Option<&str>) -> McpResult<()> {
    if confirmation == Some("PROCEED") {
        return Ok(());
    }
    Err(to_mcp_error(P4McpError::ConfirmationRequired))
}

fn required_option(value: Option<&str>, name: &str, action: &str) -> McpResult<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(to_mcp_error(P4McpError::InvalidInput {
            message: format!("{name} is required for {action}"),
        })),
    }
}

fn required_form(value: Option<&str>, action: &str) -> McpResult<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(to_mcp_error(P4McpError::InvalidInput {
            message: format!("form is required for {action}"),
        })),
    }
}

fn unknown_action(action: &str) -> P4McpError {
    invalid_input(format!("unknown action: {action}"))
}

fn invalid_input(message: impl Into<String>) -> P4McpError {
    P4McpError::InvalidInput {
        message: message.into(),
    }
}

fn to_mcp_error(error: P4McpError) -> ErrorData {
    match error {
        P4McpError::InvalidInput { .. }
        | P4McpError::ToolsetDisabled { .. }
        | P4McpError::Readonly
        | P4McpError::ConfirmationRequired => ErrorData::invalid_params(error.to_string(), None),
        P4McpError::P4Command { .. } | P4McpError::P4Json { .. } => {
            ErrorData::internal_error(error.to_string(), None)
        }
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
