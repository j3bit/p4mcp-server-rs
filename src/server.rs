use std::{path::Path, sync::Arc};

use anyhow::Result;
use clap::Parser;
use rmcp::{
    ErrorData, Json, Peer, RoleServer, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::Tool,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    approval::{
        ApprovalChannel, ApprovalDecision, ApprovalPreview, ApprovalRequest,
        DefaultWriteApprovalGate, HttpPreview, WriteApprovalGate,
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
        params::{
            CommonModifyParams, CommonQueryParams, FileQueryAction, ModifyFilesParams,
            QueryFilesParams,
        },
        response::ToolResponse,
        reviews::{BuiltReviewRequest, ReviewApiConfig, ReviewHttpClient, ReviewRequest},
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

struct P4ApprovalContext<'a> {
    tool: &'static str,
    action: &'a str,
    targets: Vec<String>,
    changelist: Option<String>,
    workspace: Option<String>,
    stream: Option<String>,
    invocation: &'a P4Invocation,
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
        Self::tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect()
    }

    pub fn tools() -> Vec<Tool> {
        Self::tool_router().list_all()
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

    async fn review_http_client_from_p4(&self) -> McpResult<ReviewHttpClient> {
        let info = self
            .run_p4(json_invocation(vec!["info".to_string()], None))
            .await?;
        let swarm_property = self
            .run_p4(json_invocation(
                vec![
                    "property".to_string(),
                    "-l".to_string(),
                    "-n".to_string(),
                    "P4.Swarm.URL".to_string(),
                ],
                None,
            ))
            .await?;
        let tickets = self
            .run_p4(text_invocation(vec!["tickets".to_string()]))
            .await?;
        let tickets_stdout = tickets
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let api_config =
            ReviewApiConfig::from_p4(&info.records, &swarm_property.records, tickets_stdout)
                .map_err(to_mcp_error)?;

        ReviewHttpClient::new_with_ssl_verify(
            api_config.api_base,
            api_config.username,
            api_config.ticket,
            &self.config.ssl_verify,
        )
        .map_err(review_api_error)
    }

    async fn query_workspace_type(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let invocation = build_workspace_query_invocation("type", workspace_name, None, 100)
            .map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        Ok(Json(ToolResponse::success(
            "type",
            json!({"workspace_type": workspace_type_from_records(&output.records)}),
        )))
    }

    async fn query_workspace_status(
        &self,
        workspace_name: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let workspace_name =
            require_non_blank(workspace_name, "workspace_name").map_err(to_mcp_error)?;

        let workspace_spec = self
            .run_workspace_status_command(
                json_invocation(vec!["client".into(), "-o".into(), workspace_name], None),
                None,
            )
            .await?;
        let opened = self
            .run_workspace_status_command(json_invocation(vec!["opened".into()], None), None)
            .await?;
        let out_of_sync = self
            .run_workspace_status_command(
                json_invocation(vec!["sync".into(), "-n".into()], None),
                Some("File(s) up-to-date"),
            )
            .await?;
        let pending_resolves = self
            .run_workspace_status_command(
                json_invocation(vec!["resolve".into(), "-n".into()], None),
                Some("No file(s) to resolve"),
            )
            .await?;
        let synced_changes = self
            .run_workspace_status_command(
                json_invocation(vec!["changes".into(), "-m1".into(), "#have".into()], None),
                None,
            )
            .await?;

        Ok(Json(ToolResponse::success(
            "status",
            workspace_status_message(
                &workspace_spec.records,
                &opened.records,
                &out_of_sync.records,
                &pending_resolves.records,
                &synced_changes.records,
            ),
        )))
    }

    async fn run_workspace_status_command(
        &self,
        invocation: P4Invocation,
        benign_empty_message: Option<&str>,
    ) -> McpResult<P4CommandOutput> {
        match self.executor.run(invocation, P4Env::new()).await {
            Ok(output) => Ok(output),
            Err(error) => {
                if benign_empty_message.is_some_and(|message| error.to_string().contains(message)) {
                    Ok(empty_p4_output())
                } else {
                    Err(to_mcp_error(error))
                }
            }
        }
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
            preview: self.p4_approval_preview(P4ApprovalContext {
                tool: "modify_files",
                action: &action,
                targets,
                changelist: Some(params.changelist.clone()),
                workspace: None,
                stream: None,
                invocation,
            }),
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
            "update" | "submit" | "delete" | "move_files" => required_option(
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
        let invocation = build_changelist_modify_invocation(
            &params.action,
            &changelist_id,
            stdin,
            &params.files,
        )
        .map_err(to_mcp_error)?;
        let request = self.common_modify_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_changelists",
                action: &params.action,
                targets: changelist_modify_targets(&params.action, &changelist_id, &params.files),
                changelist: Some(changelist_id),
                workspace: None,
                stream: None,
                invocation: &invocation,
            },
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
        let mut args = match params.action.as_str() {
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
        args.extend(params.files.iter().cloned());
        let invocation = json_invocation(args, None);
        let request = self.common_modify_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_shelves",
                action: &params.action,
                targets: shelf_targets(&change),
                changelist: Some(change),
                workspace: None,
                stream: None,
                invocation: &invocation,
            },
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
        let targets = named_scope_targets(workspace.as_deref(), "workspace form");
        let request = self.common_modify_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_workspaces",
                action: &params.action,
                targets,
                changelist: None,
                workspace,
                stream: None,
                invocation: &invocation,
            },
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
            &params,
            P4ApprovalContext {
                tool: "modify_jobs",
                action: &params.action,
                targets: params.files.clone(),
                changelist: Some(change),
                workspace: None,
                stream: None,
                invocation: &invocation,
            },
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
        let targets = named_scope_targets(stream.as_deref(), "stream form");
        let request = self.common_modify_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_streams",
                action: &params.action,
                targets,
                changelist: None,
                workspace: None,
                stream,
                invocation: &invocation,
            },
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
        params: &CommonModifyParams,
        context: P4ApprovalContext<'_>,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("common modify params serialize to JSON"),
            preview: self.p4_approval_preview(context),
        }
    }

    fn p4_approval_preview(&self, context: P4ApprovalContext<'_>) -> ApprovalPreview {
        let summary = approval_summary(context.action, &context.targets);
        ApprovalPreview {
            summary,
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            targets: context.targets,
            changelist: context.changelist,
            workspace: context.workspace,
            stream: context.stream,
            review: None,
            command: Some(command_preview(&self.config.p4_bin, context.invocation)),
            request: None,
        }
    }

    async fn modify_reviews_inner(
        &self,
        params: ReviewRequest,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Reviews, "modify_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        if built.method == "GET" {
            return Err(to_mcp_error(invalid_input(
                "modify_reviews only supports write review actions",
            )));
        }
        let request = self.modify_reviews_approval_request(&params, &built);
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        let action = review_action_name(&params);
        let client = self.review_http_client_from_p4().await?;
        let message = client
            .execute_approved(&params)
            .await
            .map_err(review_api_error)?;
        Ok(Json(ToolResponse::success(&action, message)))
    }

    fn modify_reviews_approval_request(
        &self,
        params: &ReviewRequest,
        built: &BuiltReviewRequest,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;
        let action = review_action_name(params);
        let review = params.review_id.map(|review_id| review_id.to_string());
        let targets = review
            .as_ref()
            .map(|review| vec![format!("review:{review}")])
            .unwrap_or_else(|| vec![review_target_from_path(&built.path)]);

        ApprovalRequest {
            tool: "modify_reviews".to_string(),
            action: action.clone(),
            params: serde_json::to_value(approval_params).expect("review params serialize to JSON"),
            preview: ApprovalPreview {
                summary: format!("Review API {} {}", built.method, built.path),
                tool: "modify_reviews".to_string(),
                action,
                targets,
                changelist: None,
                workspace: None,
                stream: None,
                review,
                command: None,
                request: Some(HttpPreview {
                    method: built.method.clone(),
                    path: built.path.clone(),
                }),
            },
        }
    }
}

type McpResult<T> = std::result::Result<T, ErrorData>;

#[tool_router]
impl P4McpServer {
    #[tool(
        description = "Query Perforce server metadata",
        annotations(read_only_hint = true)
    )]
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

    #[tool(
        description = "Query Perforce files",
        annotations(read_only_hint = true)
    )]
    pub async fn query_files(
        &self,
        Parameters(params): Parameters<QueryFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Files, "query_files")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let grep_max_results =
            (params.action == FileQueryAction::Grep).then_some(params.max_results as usize);
        let invocation = build_file_invocation(&params).map_err(to_mcp_error)?;
        let output = self.run_p4(invocation).await?;
        let message = if let Some(max_results) = grep_max_results {
            output_message_with_record_limit(output, max_results)
        } else {
            output_message(output)
        };
        Ok(Json(ToolResponse::success(action, message)))
    }

    #[tool(
        description = "Modify Perforce files",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_files(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyFilesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_files_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "Get changelist information or list changelists",
        annotations(read_only_hint = true)
    )]
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
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(
        description = "Create, update, submit, or delete changelists",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_changelists(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_changelists_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "List shelves, show shelf diff, or list shelf files",
        annotations(read_only_hint = true)
    )]
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

    #[tool(
        description = "Shelve, unshelve, or delete shelved files",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_shelves(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_shelves_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "List, get, classify, or inspect workspace status",
        annotations(read_only_hint = true)
    )]
    pub async fn query_workspaces(
        &self,
        Parameters(params): Parameters<CommonQueryParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        if params.action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
        if params.action == "status" {
            return self
                .query_workspace_status(params.workspace_name.as_deref())
                .await;
        }
        let invocation = build_workspace_query_invocation(
            &params.action,
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(&params.action, invocation).await
    }

    #[tool(
        description = "Create, update, or delete workspaces using p4 client forms",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_workspaces(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_workspaces_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "List or get jobs and fixes",
        annotations(read_only_hint = true)
    )]
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

    #[tool(
        description = "Attach or detach jobs from changelists",
        annotations(read_only_hint = false)
    )]
    pub async fn modify_jobs(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_jobs_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "List streams, get stream specs, graph streams, and inspect stream integration status",
        annotations(read_only_hint = true)
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

    #[tool(
        description = "Create, update, or delete stream specs",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_streams(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<CommonModifyParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_streams_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "Query P4 Code Review / Swarm reviews",
        annotations(read_only_hint = true)
    )]
    pub async fn query_reviews(
        &self,
        Parameters(params): Parameters<ReviewRequest>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Reviews, "query_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        if built.method != "GET" {
            return Err(to_mcp_error(invalid_input(
                "query_reviews only supports read review actions",
            )));
        }
        let action = review_action_name(&params);
        let client = self.review_http_client_from_p4().await?;
        let message = client.execute(&params).await.map_err(review_api_error)?;
        Ok(Json(ToolResponse::success(&action, message)))
    }

    #[tool(
        description = "Modify P4 Code Review / Swarm reviews",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_reviews(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ReviewRequest>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_reviews_inner(params, ApprovalChannel::Elicitation(peer))
            .await
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
    let _logging_guard = init_logging(config.log_dir.as_deref())?;

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

fn workspace_type_from_records(records: &[Value]) -> &'static str {
    if records
        .iter()
        .any(|record| non_empty_string_field(record, "Stream").is_some())
    {
        return "stream";
    }

    if records.iter().any(record_has_depot_view) {
        return "standard";
    }

    "custom"
}

fn workspace_status_message(
    _workspace_spec: &[Value],
    opened: &[Value],
    out_of_sync: &[Value],
    pending_resolves: &[Value],
    synced_changes: &[Value],
) -> Value {
    json!({
        "opened_files": collect_string_field(opened, "depotFile"),
        "out_of_sync_files": collect_string_field(out_of_sync, "depotFile"),
        "sync_warnings": collect_string_values(out_of_sync),
        "pending_resolves": collect_string_field(pending_resolves, "fromFile"),
        "last_synced_cl": synced_changes
            .first()
            .and_then(|record| non_empty_string_field(record, "change")),
    })
}

fn collect_string_field(records: &[Value], field: &str) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| non_empty_string_field(record, field))
        .collect()
}

fn collect_string_values(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn empty_p4_output() -> P4CommandOutput {
    P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }
}

fn require_non_blank(value: Option<&str>, name: &str) -> std::result::Result<String, P4McpError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(invalid_input(format!("{name} is required"))),
    }
}

fn record_has_depot_view(record: &Value) -> bool {
    let Some(object) = record.as_object() else {
        return false;
    };

    object.iter().any(|(key, value)| {
        (key == "View" || key.starts_with("View"))
            && value.as_str().is_some_and(|view| view.contains("//depot/"))
    })
}

fn non_empty_string_field(record: &Value, field: &str) -> Option<String> {
    record
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn output_message_with_record_limit(mut output: P4CommandOutput, max_records: usize) -> Value {
    if output.records.is_empty() {
        output.text
    } else {
        output.records.truncate(max_records);
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

fn text_invocation(args: Vec<String>) -> P4Invocation {
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::Text,
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

fn changelist_targets(changelist_id: &str) -> Vec<String> {
    vec![format!("changelist:{changelist_id}")]
}

fn changelist_modify_targets(action: &str, changelist_id: &str, files: &[String]) -> Vec<String> {
    let mut targets = changelist_targets(changelist_id);
    if action == "move_files" {
        targets.extend(files.iter().cloned());
    }
    targets
}

fn shelf_targets(changelist_id: &str) -> Vec<String> {
    vec![format!("shelf:{changelist_id}")]
}

fn named_scope_targets(name: Option<&str>, fallback: &str) -> Vec<String> {
    match name.filter(|value| !value.trim().is_empty()) {
        Some(name) => vec![name.to_string()],
        None => vec![fallback.to_string()],
    }
}

fn approval_summary(action: &str, targets: &[String]) -> String {
    if targets.is_empty() {
        format!("Run p4 {action}")
    } else {
        format!("Run p4 {action} on {}", targets.join(", "))
    }
}

fn review_action_name(params: &ReviewRequest) -> String {
    serde_json::to_value(&params.action)
        .expect("review action serializes to JSON")
        .as_str()
        .expect("review action serializes to a string")
        .to_string()
}

fn review_target_from_path(path: &str) -> String {
    path.trim_start_matches('/').to_string()
}

fn command_preview(p4_bin: &std::path::Path, invocation: &P4Invocation) -> Vec<String> {
    let mut command = vec![p4_bin.to_string_lossy().into_owned()];
    command.extend(invocation.args.iter().cloned());
    command
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

fn review_api_error(error: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("review API request failed: {error}"), None)
}

#[must_use]
struct LoggingGuard {
    _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

fn init_logging(log_dir: Option<&Path>) -> Result<LoggingGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    if let Some(log_dir) = log_dir {
        let file_appender = configured_log_file(log_dir)?;
        let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file_writer);

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init();

        Ok(LoggingGuard {
            _file_guard: Some(file_guard),
        })
    } else {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .try_init();

        Ok(LoggingGuard { _file_guard: None })
    }
}

fn configured_log_file(log_dir: &Path) -> Result<tracing_appender::rolling::RollingFileAppender> {
    std::fs::create_dir_all(log_dir)?;
    Ok(tracing_appender::rolling::never(log_dir, "p4mcp.log"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::Write,
        net::{IpAddr, Ipv4Addr},
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{
        approval::{ApprovalChannel, ApprovalDecision, ApprovalRequest, WriteApprovalGate},
        config::{SslVerify, TransportMode},
        p4::runner::{P4CommandOutput, P4Env},
        tools::params::FileModifyAction,
        tools::reviews::ReviewAction,
    };

    use super::*;

    #[test]
    fn configured_log_file_writes_to_p4mcp_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = configured_log_file(dir.path()).unwrap();

        writeln!(writer, "log-dir smoke").unwrap();
        drop(writer);

        let contents = std::fs::read_to_string(dir.path().join("p4mcp.log")).unwrap();
        assert!(contents.contains("log-dir smoke"));
    }

    #[test]
    fn workspace_type_from_records_classifies_standard_workspace() {
        let records = vec![json!({
            "Client": "ws-standard",
            "View0": "//depot/main/... //ws-standard/main/..."
        })];

        assert_eq!(workspace_type_from_records(&records), "standard");
    }

    #[test]
    fn workspace_type_from_records_classifies_custom_workspace() {
        let records = vec![json!({
            "Client": "ws-custom",
            "View0": "//streams/main/... //ws-custom/main/..."
        })];

        assert_eq!(workspace_type_from_records(&records), "custom");
    }

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

    fn review_modify_params(approval_token: Option<&str>) -> ReviewRequest {
        ReviewRequest {
            action: ReviewAction::Vote,
            review_id: Some(123),
            max_results: 10,
            body: json!({"vote": "up", "version": 2}),
            approval_token: approval_token.map(str::to_string),
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
        assert_eq!(calls[0].request.preview.targets, ["changelist:123"]);
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
    async fn modify_changelists_move_files_without_approval_does_not_call_executor() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/a.rs"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("move_files");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ];

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
        assert_eq!(calls[0].request.action, "move_files");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(
            calls[0].request.preview.targets,
            ["changelist:123", "//depot/main/a.rs", "//depot/main/b.rs",]
        );
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "reopen".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/a.rs".to_string(),
                "//depot/main/b.rs".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_changelists_move_files_after_approval_reopens_files() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"depotFile": "//depot/main/a.rs"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("move_files");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ];
        params.approval_token = Some("approved-token".to_string());

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved write should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "move_files");
        assert_eq!(
            response.0.message,
            json!([{"depotFile": "//depot/main/a.rs"}])
        );

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].args,
            [
                "reopen",
                "-c",
                "123",
                "//depot/main/a.rs",
                "//depot/main/b.rs",
            ]
        );

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].approval_token.as_deref(), Some("approved-token"));
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
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
        assert_eq!(calls[0].request.preview.targets, ["shelf:123"]);
        assert_eq!(calls[0].request.preview.changelist.as_deref(), Some("123"));
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/file.txt".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_shelves_delete_after_approval_passes_file_paths_to_p4() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: vec![json!({"change": "123"})],
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = common_modify_params("delete");
        params.changelist_id = Some("123".to_string());
        params.files = vec![
            "//depot/main/file.txt".to_string(),
            "//depot/main/other.txt".to_string(),
        ];

        let response = server
            .modify_shelves_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved shelf delete should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "delete");

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].args,
            [
                "shelve",
                "-d",
                "-c",
                "123",
                "//depot/main/file.txt",
                "//depot/main/other.txt",
            ]
        );

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].request.preview.command,
            Some(vec![
                "p4".to_string(),
                "shelve".to_string(),
                "-d".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/file.txt".to_string(),
                "//depot/main/other.txt".to_string(),
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
    async fn modify_workspaces_update_preview_uses_form_scope_when_name_absent() {
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
        let mut params = common_modify_params("update");
        params.form = Some("Client: ws-main\n".to_string());

        let response = server
            .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].request.preview.targets, ["workspace form"]);
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
    async fn modify_streams_update_preview_uses_form_scope_when_stream_absent() {
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
        let mut params = common_modify_params("update");
        params.form = Some("Stream: //streams/dev\n".to_string());

        let response = server
            .modify_streams_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].request.preview.targets, ["stream form"]);
    }

    #[tokio::test]
    async fn modify_reviews_without_approval_does_not_return_write_dry_run() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_reviews_inner(review_modify_params(None), ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert_ne!(response.0.status, "dry_run");
        assert!(response.0.message.get("method").is_none());
        assert!(response.0.message.get("path").is_none());
        assert!(executor.invocations().is_empty());

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].fallback_only);
        assert_eq!(calls[0].approval_token, None);
        assert_eq!(calls[0].request.tool, "modify_reviews");
        assert_eq!(calls[0].request.action, "vote");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
    }

    #[tokio::test]
    async fn modify_reviews_after_approval_executes_review_api_request() {
        let swarm = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v11/reviews/123/vote"))
            .and(header("authorization", "Basic YWxpY2U6dGlja2V0LTEyMw=="))
            .and(body_json(json!({"vote": "up", "version": 2})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "vote": "recorded"
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
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_reviews_inner(
                review_modify_params(Some("approved-token")),
                ApprovalChannel::FallbackOnly,
            )
            .await
            .expect("approved review write should execute");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "vote");
        assert_eq!(response.0.message, json!({"vote": "recorded"}));

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 3);
        assert_eq!(invocations[0].args, ["info"]);
        assert_eq!(
            invocations[1].args,
            ["property", "-l", "-n", "P4.Swarm.URL"]
        );
        assert_eq!(invocations[2].args, ["tickets"]);

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].approval_token.as_deref(), Some("approved-token"));
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
    }

    #[tokio::test]
    async fn modify_reviews_approval_preview_uses_method_and_path() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_reviews_inner(review_modify_params(None), ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].request.preview.tool, "modify_reviews");
        assert_eq!(calls[0].request.preview.action, "vote");
        assert_eq!(calls[0].request.preview.targets, ["review:123"]);
        assert_eq!(calls[0].request.preview.review.as_deref(), Some("123"));
        assert_eq!(calls[0].request.preview.command, None);
        assert_eq!(
            calls[0]
                .request
                .preview
                .request
                .as_ref()
                .map(|request| (request.method.as_str(), request.path.as_str(),)),
            Some(("POST", "/reviews/123/vote"))
        );
    }

    #[tokio::test]
    async fn modify_reviews_rejects_read_actions_after_policy_before_approval() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let err = match server
            .modify_reviews_inner(
                ReviewRequest {
                    action: ReviewAction::List,
                    review_id: None,
                    max_results: 10,
                    body: json!({}),
                    approval_token: Some("approved-token".to_string()),
                },
                ApprovalChannel::FallbackOnly,
            )
            .await
        {
            Ok(_) => panic!("modify_reviews should reject read review actions"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(
            err.message
                .contains("modify_reviews only supports write review actions")
        );
        assert!(approval_gate.calls().is_empty());
        assert!(executor.invocations().is_empty());
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

    struct QueuedExecutor {
        outputs: Mutex<VecDeque<crate::error::Result<P4CommandOutput>>>,
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
        ) -> crate::error::Result<P4CommandOutput> {
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
