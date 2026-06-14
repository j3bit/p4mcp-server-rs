use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use rmcp::{
    ErrorData, Json, Peer, RoleServer, ServerHandler, ServiceExt,
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
    approval::{
        ApprovalChannel, ApprovalDecision, ApprovalPreview, ApprovalRequest,
        DefaultWriteApprovalGate, WriteApprovalGate,
    },
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
    approval_gate: Arc<dyn WriteApprovalGate>,
}

impl P4McpServer {
    pub fn new(config: AppConfig) -> Self {
        let executor = Arc::new(TokioP4Executor::new(config.p4_bin.clone()));
        Self::with_executor_and_approval(
            config,
            executor,
            Arc::new(DefaultWriteApprovalGate::new()),
        )
    }

    pub fn with_executor(config: AppConfig, executor: Arc<dyn P4Executor>) -> Self {
        Self::with_executor_and_approval(
            config,
            executor,
            Arc::new(DefaultWriteApprovalGate::new()),
        )
    }

    pub fn with_executor_and_approval(
        config: AppConfig,
        executor: Arc<dyn P4Executor>,
        approval_gate: Arc<dyn WriteApprovalGate>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            executor,
            approval_gate,
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

    async fn require_write_approval(
        &self,
        channel: ApprovalChannel,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> McpResult<Option<Json<ToolResponse>>> {
        match self
            .approval_gate
            .approve(channel, request, approval_token)
            .await
            .map_err(to_mcp_error)?
        {
            ApprovalDecision::Approved => Ok(None),
            ApprovalDecision::Response(response) => Ok(Some(Json(response))),
        }
    }

    async fn modify_files_inner(
        &self,
        params: ModifyFilesParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Files, "modify_files")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_file_modify_invocation(&params).map_err(to_mcp_error)?;
        let request = self.modify_files_approval_request(&params, &invocation);
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(action, invocation).await
    }

    fn modify_files_approval_request(
        &self,
        params: &ModifyFilesParams,
        invocation: &P4Invocation,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;
        let targets = modify_files_targets(params);
        let action = params.action.as_str().to_string();

        ApprovalRequest {
            tool: "modify_files".to_string(),
            action: action.clone(),
            params: serde_json::to_value(approval_params)
                .expect("modify files params serialize to JSON"),
            preview: self.p4_approval_preview(
                "modify_files",
                &action,
                targets,
                Some(params.changelist.clone()),
                None,
                None,
                invocation,
            ),
        }
    }

    async fn modify_changelists_inner(
        &self,
        params: CommonModifyParams,
        channel: ApprovalChannel,
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
        let request = self.common_modify_approval_request(
            "modify_changelists",
            &params,
            params.files.clone(),
            Some(changelist_id),
            None,
            None,
            &invocation,
        );
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(&params.action, invocation).await
    }

    async fn modify_shelves_inner(
        &self,
        params: CommonModifyParams,
        channel: ApprovalChannel,
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
            "shelve" => vec!["shelve".to_string(), "-c".to_string(), change.clone()],
            "unshelve" => vec!["unshelve".to_string(), "-s".to_string(), change.clone()],
            "delete" => {
                vec![
                    "shelve".to_string(),
                    "-d".to_string(),
                    "-c".to_string(),
                    change.clone(),
                ]
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        let invocation = json_invocation(args, None);
        let request = self.common_modify_approval_request(
            "modify_shelves",
            &params,
            params.files.clone(),
            Some(change),
            None,
            None,
            &invocation,
        );
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(&params.action, invocation).await
    }

    async fn modify_workspaces_inner(
        &self,
        params: CommonModifyParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Workspaces, "modify_workspaces")
            .map_err(to_mcp_error)?;
        let (args, stdin, workspace) = match params.action.as_str() {
            "create" | "update" => (
                vec!["client".to_string(), "-i".to_string()],
                Some(required_form(params.form.as_deref(), &params.action)?),
                params.workspace_name.clone(),
            ),
            "delete" => {
                let workspace_name =
                    required_option(params.workspace_name.as_deref(), "workspace_name", "delete")?;
                (
                    vec![
                        "client".to_string(),
                        "-d".to_string(),
                        workspace_name.clone(),
                    ],
                    None,
                    Some(workspace_name),
                )
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        let invocation = json_invocation(args, stdin);
        let targets = workspace.iter().cloned().collect();
        let request = self.common_modify_approval_request(
            "modify_workspaces",
            &params,
            targets,
            None,
            workspace,
            None,
            &invocation,
        );
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(&params.action, invocation).await
    }

    async fn modify_jobs_inner(
        &self,
        params: CommonModifyParams,
        channel: ApprovalChannel,
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
            "fix" => vec!["fix".to_string(), "-c".to_string(), change.clone(), job],
            "unfix" => vec![
                "fix".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                change.clone(),
                job,
            ],
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        let invocation = json_invocation(args, None);
        let request = self.common_modify_approval_request(
            "modify_jobs",
            &params,
            params.files.clone(),
            Some(change),
            None,
            None,
            &invocation,
        );
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(&params.action, invocation).await
    }

    async fn modify_streams_inner(
        &self,
        params: CommonModifyParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Streams, "modify_streams")
            .map_err(to_mcp_error)?;
        let (args, stdin, stream) = match params.action.as_str() {
            "create" | "update" => (
                vec!["stream".to_string(), "-i".to_string()],
                Some(required_form(params.form.as_deref(), &params.action)?),
                params.stream.clone(),
            ),
            "delete" => {
                let stream = required_option(params.stream.as_deref(), "stream", "delete")?;
                (
                    vec!["stream".to_string(), "-d".to_string(), stream.clone()],
                    None,
                    Some(stream),
                )
            }
            other => return Err(to_mcp_error(unknown_action(other))),
        };
        let invocation = json_invocation(args, stdin);
        let targets = stream.iter().cloned().collect();
        let request = self.common_modify_approval_request(
            "modify_streams",
            &params,
            targets,
            None,
            None,
            stream,
            &invocation,
        );
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        self.call_p4_tool(&params.action, invocation).await
    }

    fn common_modify_approval_request(
        &self,
        tool: &str,
        params: &CommonModifyParams,
        targets: Vec<String>,
        changelist: Option<String>,
        workspace: Option<String>,
        stream: Option<String>,
        invocation: &P4Invocation,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;
        let action = params.action.clone();

        ApprovalRequest {
            tool: tool.to_string(),
            action: action.clone(),
            params: serde_json::to_value(approval_params)
                .expect("common modify params serialize to JSON"),
            preview: self.p4_approval_preview(
                tool, &action, targets, changelist, workspace, stream, invocation,
            ),
        }
    }

    fn p4_approval_preview(
        &self,
        tool: &str,
        action: &str,
        targets: Vec<String>,
        changelist: Option<String>,
        workspace: Option<String>,
        stream: Option<String>,
        invocation: &P4Invocation,
    ) -> ApprovalPreview {
        ApprovalPreview {
            summary: approval_summary(action, &targets),
            tool: tool.to_string(),
            action: action.to_string(),
            targets,
            changelist,
            workspace,
            stream,
            review: None,
            command: Some(command_preview(&self.config.p4_bin, invocation)),
            request: None,
        }
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_files_inner(params, ApprovalChannel::Elicitation(peer))
            .await
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_changelists_inner(params, ApprovalChannel::Elicitation(peer))
            .await
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_shelves_inner(params, ApprovalChannel::Elicitation(peer))
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_workspaces_inner(params, ApprovalChannel::Elicitation(peer))
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_jobs_inner(params, ApprovalChannel::Elicitation(peer))
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
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_streams_inner(params, ApprovalChannel::Elicitation(peer))
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

fn modify_files_targets(params: &ModifyFilesParams) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(file_paths) = &params.file_paths {
        targets.extend(file_paths.iter().cloned());
    }
    if let Some(source_paths) = &params.source_paths {
        targets.extend(source_paths.iter().cloned());
    }
    if let Some(target_paths) = &params.target_paths {
        targets.extend(target_paths.iter().cloned());
    }
    targets
}

fn approval_summary(action: &str, targets: &[String]) -> String {
    if targets.is_empty() {
        format!("Run p4 {action}")
    } else {
        format!("Run p4 {action} on {}", targets.join(", "))
    }
}

fn command_preview(p4_bin: &std::path::Path, invocation: &P4Invocation) -> Vec<String> {
    let mut command = vec![p4_bin.to_string_lossy().into_owned()];
    command.extend(invocation.args.iter().cloned());
    command
}

fn review_message(built: crate::tools::reviews::BuiltReviewRequest) -> Value {
    serde_json::json!({
        "method": built.method,
        "path": built.path,
        "query": built.query,
        "body": built.body,
    })
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
        | P4McpError::Readonly => ErrorData::invalid_params(error.to_string(), None),
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

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use serde_json::json;

    use crate::{
        approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest, WriteApprovalGate},
        config::{SslVerify, TransportMode},
        p4::runner::{P4CommandOutput, P4Env},
        tools::params::FileModifyAction,
    };

    use super::*;

    fn test_config(readonly: bool) -> AppConfig {
        AppConfig {
            readonly,
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

    fn modify_files_params(approval_token: Option<&str>) -> ModifyFilesParams {
        ModifyFilesParams {
            action: FileModifyAction::Sync,
            file_paths: Some(vec!["//depot/main/file.txt".to_string()]),
            changelist: "default".to_string(),
            source_paths: None,
            target_paths: None,
            mode: "auto".to_string(),
            force: true,
            approval_token: approval_token.map(str::to_string),
        }
    }

    fn common_modify_params(action: &str) -> CommonModifyParams {
        CommonModifyParams {
            action: action.to_string(),
            changelist_id: None,
            workspace_name: None,
            stream: None,
            description: None,
            files: Vec::new(),
            form: None,
            approval_token: None,
        }
    }

    #[tokio::test]
    async fn modify_files_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/file.txt"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_files_inner(modify_files_params(None), ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert_eq!(response.0.action, "sync");
        assert!(executor.invocations().is_empty());

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].approval_token, None);
        assert_eq!(calls[0].request.tool, "modify_files");
        assert_eq!(calls[0].request.action, "sync");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.tool, "modify_files");
        assert_eq!(calls[0].request.preview.action, "sync");
        assert_eq!(calls[0].request.preview.targets, ["//depot/main/file.txt"]);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "sync".to_string(),
                "-f".to_string(),
                "//depot/main/file.txt".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_files_after_approval_calls_executor_once() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/file.txt"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_files_inner(
                modify_files_params(Some("approved-token")),
                ApprovalChannel::FallbackOnly,
            )
            .await
            .expect("approved write should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "sync");

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].args, ["sync", "-f", "//depot/main/file.txt"]);

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].approval_token.as_deref(), Some("approved-token"));
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
    }

    #[tokio::test]
    async fn modify_files_edit_preview_includes_p4_edit() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/file.txt"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = modify_files_params(None);
        params.action = FileModifyAction::Edit;
        params.force = false;

        let response = server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "edit".to_string(),
                "-c".to_string(),
                "default".to_string(),
                "//depot/main/file.txt".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_changelists_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"change": "123"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("submit");
        params.changelist_id = Some("123".to_string());
        params.files = vec!["//depot/main/file.txt".to_string()];

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_changelists");
        assert_eq!(calls[0].request.action, "submit");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.tool, "modify_changelists");
        assert_eq!(calls[0].request.preview.action, "submit");
        assert_eq!(calls[0].request.preview.targets, ["//depot/main/file.txt"]);
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "submit".to_string(),
                "-c".to_string(),
                "123".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_shelves_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"change": "123"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("delete");
        params.changelist_id = Some("123".to_string());
        params.files = vec!["//depot/main/file.txt".to_string()];

        let response = server
            .modify_shelves_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_shelves");
        assert_eq!(calls[0].request.action, "delete");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.targets, ["//depot/main/file.txt"]);
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                "123".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_workspaces_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"client": "ws-main"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("delete");
        params.workspace_name = Some("ws-main".to_string());

        let response = server
            .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_workspaces");
        assert_eq!(calls[0].request.action, "delete");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.targets, ["ws-main"]);
        assert_eq!(
            calls[0].request.preview.workspace.as_deref(),
            Some("ws-main")
        );
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "client".to_string(),
                "-d".to_string(),
                "ws-main".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_jobs_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"Job": "job000001"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("fix");
        params.changelist_id = Some("123".to_string());
        params.files = vec!["job000001".to_string()];

        let response = server
            .modify_jobs_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_jobs");
        assert_eq!(calls[0].request.action, "fix");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.targets, ["job000001"]);
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "fix".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "job000001".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_streams_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"Stream": "//streams/dev"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("delete");
        params.stream = Some("//streams/dev".to_string());

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].request.tool, "modify_streams");
        assert_eq!(calls[0].request.action, "delete");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(calls[0].request.preview.targets, ["//streams/dev"]);
        assert_eq!(
            calls[0].request.preview.stream.as_deref(),
            Some("//streams/dev")
        );
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "stream".to_string(),
                "-d".to_string(),
                "//streams/dev".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn readonly_blocks_before_approval_gate() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/file.txt"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(true),
            executor.clone(),
            approval_gate.clone(),
        );

        let err = match server
            .modify_files_inner(modify_files_params(None), ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("readonly mode should reject the write"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("read-only mode"));
        assert!(approval_gate.calls().is_empty());
        assert!(executor.invocations().is_empty());
    }

    #[derive(Clone)]
    struct FakeApprovalCall {
        fallback_only: bool,
        request: ApprovalRequest,
        approval_token: Option<String>,
    }

    struct FakeApprovalGate {
        decision: ApprovalDecision,
        calls: Mutex<Vec<FakeApprovalCall>>,
    }

    impl FakeApprovalGate {
        fn approval_required() -> Self {
            Self {
                decision: ApprovalDecision::Response(ToolResponse::approval_required(
                    "sync",
                    json!({"reason": "approval required"}),
                )),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn approved() -> Self {
            Self {
                decision: ApprovalDecision::Approved,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<FakeApprovalCall> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl WriteApprovalGate for FakeApprovalGate {
        async fn approve(
            &self,
            channel: ApprovalChannel,
            request: ApprovalRequest,
            approval_token: Option<&str>,
        ) -> crate::error::Result<ApprovalDecision> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(FakeApprovalCall {
                    fallback_only: matches!(channel, ApprovalChannel::FallbackOnly),
                    request,
                    approval_token: approval_token.map(str::to_string),
                });
            Ok(self.decision.clone())
        }
    }

    struct FakeExecutor {
        output: P4CommandOutput,
        invocations: Mutex<Vec<P4Invocation>>,
    }

    impl FakeExecutor {
        fn success(output: P4CommandOutput) -> Self {
            Self {
                output,
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
    impl P4Executor for FakeExecutor {
        async fn run(
            &self,
            invocation: P4Invocation,
            _env: P4Env,
        ) -> crate::error::Result<P4CommandOutput> {
            self.invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(invocation);
            Ok(self.output.clone())
        }
    }
}
