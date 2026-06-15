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
        forms::{
            WorkspaceFormPatch, change_form, patch_change_description_form, patch_workspace_form,
        },
        runner::{OutputMode, P4CommandOutput, P4Env, P4Executor, P4Invocation, TokioP4Executor},
    },
    permissions::{Access, SafetyPolicy},
    tools::{
        changelists::{build_changelist_modify_invocation, build_changelist_query_invocation},
        files::{build_file_invocation, build_file_modify_invocation},
        jobs::{build_job_modify_invocation, build_job_query_invocation},
        params::{
            ChangelistModifyAction, CommonModifyParams, FileQueryAction, ModifyChangelistsParams,
            ModifyFilesParams, ModifyJobsParams, ModifyShelvesParams, ModifyWorkspacesParams,
            QueryChangelistsParams, QueryFilesParams, QueryJobsParams, QueryShelvesParams,
            QueryStreamsParams, QueryWorkspacesParams, WorkspaceModifyAction,
        },
        response::ToolResponse,
        reviews::{BuiltReviewRequest, ReviewApiConfig, ReviewHttpClient, ReviewRequest},
        server::{QueryServerParams, build_server_invocation},
        shelves::{build_shelf_modify_invocation, build_shelf_query_invocation},
        streams::{
            StreamQueryCommand, build_stream_query_command, client_spec_invocation,
            interchanges_invocation, opened_for_stream_validation_invocation,
            stream_resolve_preview_invocation, stream_spec_with_view_invocation,
        },
        workspaces::{build_workspace_delete_invocation, build_workspace_query_invocation},
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

    async fn query_stream_parent(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        self.require_existing_stream(stream_name).await?;
        let output = self
            .run_p4(json_invocation(
                vec!["stream".into(), "-o".into(), stream_name.into()],
                None,
            ))
            .await?;
        let parent = output
            .records
            .first()
            .and_then(|record| record.get("Parent"))
            .cloned()
            .unwrap_or(json!(null));
        Ok(Json(ToolResponse::success(action, parent)))
    }

    async fn query_stream_graph(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        self.require_existing_stream(stream_name).await?;
        let stream = self
            .run_p4(json_invocation(
                vec!["stream".into(), "-o".into(), stream_name.into()],
                None,
            ))
            .await?;
        let children = self
            .run_p4(json_invocation(
                vec![
                    "streams".into(),
                    "-F".into(),
                    format!("Parent={stream_name}"),
                ],
                None,
            ))
            .await?;
        let parent = stream
            .records
            .first()
            .and_then(|record| record.get("Parent"))
            .cloned()
            .unwrap_or(json!(null));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "stream": stream_name,
                "parent": parent,
                "children": children.records
            }),
        )))
    }

    async fn query_stream_validate_file(
        &self,
        action: &str,
        workspace: Option<&str>,
        file_paths: &[String],
    ) -> McpResult<Json<ToolResponse>> {
        let client = self.run_p4(client_spec_invocation(workspace)).await?;
        let stream = required_record_field(&client.records, "Stream").map_err(to_mcp_error)?;
        let stream_spec = self
            .run_p4(stream_spec_with_view_invocation(&stream))
            .await?;
        let paths = collect_record_strings(&stream_spec.records, "Paths");
        let ignored = collect_record_strings(&stream_spec.records, "Ignored");
        let results = file_paths
            .iter()
            .map(|file| classify_stream_file(file, &stream, &paths, &ignored))
            .collect::<Vec<_>>();
        let all_allowed = results
            .iter()
            .all(|result| result.get("allowed").and_then(Value::as_bool) == Some(true));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "all_allowed": all_allowed,
                "stream": stream,
                "workspace": workspace,
                "file_count": file_paths.len(),
                "results": results
            }),
        )))
    }

    async fn query_stream_validate_submit(
        &self,
        action: &str,
        workspace: Option<&str>,
        changelist: Option<&str>,
    ) -> McpResult<Json<ToolResponse>> {
        let client = self.run_p4(client_spec_invocation(workspace)).await?;
        let stream = required_record_field(&client.records, "Stream").map_err(to_mcp_error)?;
        let opened = self
            .run_p4(opened_for_stream_validation_invocation(
                workspace, changelist,
            ))
            .await?;
        let stream_spec = self
            .run_p4(stream_spec_with_view_invocation(&stream))
            .await?;
        let paths = collect_record_strings(&stream_spec.records, "Paths");
        let ignored = collect_record_strings(&stream_spec.records, "Ignored");
        let results = opened
            .records
            .iter()
            .filter_map(|record| record.get("depotFile").and_then(Value::as_str))
            .map(|file| classify_stream_file(file, &stream, &paths, &ignored))
            .collect::<Vec<_>>();
        let submittable = results
            .iter()
            .all(|result| result.get("allowed").and_then(Value::as_bool) == Some(true));
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "submittable": submittable,
                "stream": stream,
                "workspace": workspace,
                "file_count": results.len(),
                "results": results
            }),
        )))
    }

    async fn query_stream_check_resolve(
        &self,
        action: &str,
        stream_name: &str,
    ) -> McpResult<Json<ToolResponse>> {
        self.require_existing_stream(stream_name).await?;
        let preview = self.run_p4(stream_resolve_preview_invocation()).await?;
        Ok(Json(ToolResponse::success(
            action,
            json!({
                "status": "success",
                "resolve_needed": !preview.records.is_empty(),
                "stream": stream_name,
                "conflicts": preview.records
            }),
        )))
    }

    async fn query_stream_interchanges(
        &self,
        action: &str,
        stream_name: &str,
        reverse: bool,
        file_paths: &[String],
        long_output: bool,
        limit: Option<u16>,
    ) -> McpResult<Json<ToolResponse>> {
        let client = self.run_p4(client_spec_invocation(None)).await?;
        let workspace_stream = required_workspace_stream(&client.records).map_err(to_mcp_error)?;
        self.require_existing_stream(stream_name).await?;
        let output = self
            .run_p4(interchanges_invocation(
                stream_name,
                reverse,
                file_paths,
                long_output,
            ))
            .await?;
        let changelists = match limit {
            Some(limit) => output.records.into_iter().take(limit as usize).collect(),
            None => output.records,
        };
        let message =
            stream_interchanges_message(stream_name, &workspace_stream, reverse, changelists.len());
        let source_stream = if reverse {
            workspace_stream.clone()
        } else {
            stream_name.to_string()
        };
        let message = json!({
            "changelists": changelists,
            "count": message.0,
            "source_stream": source_stream,
            "workspace_stream": workspace_stream,
            "direction": if reverse { "reverse" } else { "forward" },
            "message": message.1
        });
        Ok(Json(ToolResponse::success(action, message)))
    }

    async fn require_existing_stream(&self, stream_name: &str) -> McpResult<()> {
        let active = self
            .run_p4(json_invocation(
                vec![
                    "streams".into(),
                    "-F".into(),
                    format!("Stream={stream_name}"),
                ],
                None,
            ))
            .await?;
        if !active.records.is_empty() {
            return Ok(());
        }

        let including_deleted = self
            .run_p4(json_invocation(
                vec![
                    "streams".into(),
                    "-a".into(),
                    "-F".into(),
                    format!("Stream={stream_name}"),
                ],
                None,
            ))
            .await?;
        if including_deleted.records.is_empty() {
            return Err(to_mcp_error(invalid_input(format!(
                "stream does not exist: {stream_name}"
            ))));
        }

        Ok(())
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
        params: ModifyChangelistsParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Changelists, "modify_changelists")
            .map_err(to_mcp_error)?;

        let action = params.action.as_str().to_string();
        let changelist_id = match params.action {
            ChangelistModifyAction::Create => Some("new".to_string()),
            ChangelistModifyAction::Update
            | ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => Some(required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &action,
            )?),
        };
        let stdin = match params.action {
            ChangelistModifyAction::Create => Some(change_form(
                params.description.as_deref().unwrap_or_default(),
                &[],
            )),
            ChangelistModifyAction::Update => {
                Some("Description:\n\tapproval preview placeholder\n".to_string())
            }
            ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => None,
        };
        let invocation =
            build_changelist_modify_invocation(&params, stdin).map_err(to_mcp_error)?;
        let request = self.modify_changelists_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_changelists",
                action: &action,
                targets: changelist_modify_targets(&params, changelist_id.as_deref()),
                changelist: changelist_id.clone(),
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

        let invocation = if params.action == ChangelistModifyAction::Update {
            let changelist_id = changelist_id.expect("update changelist id was required");
            let output = self
                .run_p4(text_invocation(vec![
                    "change".to_string(),
                    "-o".to_string(),
                    changelist_id,
                ]))
                .await?;
            let existing = output
                .text
                .get("stdout")
                .and_then(Value::as_str)
                .ok_or_else(|| to_mcp_error(invalid_input("p4 change -o did not return stdout")))?;
            let patched = patch_change_description_form(
                existing,
                params.description.as_deref().unwrap_or_default(),
            )
            .map_err(to_mcp_error)?;
            build_changelist_modify_invocation(&params, Some(patched)).map_err(to_mcp_error)?
        } else {
            invocation
        };
        self.call_p4_tool(&action, invocation).await
    }

    async fn modify_shelves_inner(
        &self,
        params: ModifyShelvesParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Shelves, "modify_shelves")
            .map_err(to_mcp_error)?;
        let invocation = build_shelf_modify_invocation(&params).map_err(to_mcp_error)?;
        let action = params.action.as_str().to_string();
        let request = self.modify_shelves_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_shelves",
                action: &action,
                targets: shelf_modify_targets(&params),
                changelist: Some(params.changelist_id.clone()),
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
        self.call_p4_tool(&action, invocation).await
    }

    async fn modify_workspaces_inner(
        &self,
        params: ModifyWorkspacesParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Workspaces, "modify_workspaces")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str().to_string();
        let invocation = match params.action {
            WorkspaceModifyAction::Create | WorkspaceModifyAction::Update => json_invocation(
                vec!["client".to_string(), "-i".to_string()],
                Some("workspace form will be fetched and patched after approval\n".to_string()),
            ),
            WorkspaceModifyAction::Delete => {
                build_workspace_delete_invocation(&params).map_err(to_mcp_error)?
            }
            WorkspaceModifyAction::Switch => json_invocation(
                vec![
                    "client".to_string(),
                    "-s".to_string(),
                    params.workspace_name.clone(),
                ],
                None,
            ),
        };
        let request = self.modify_workspaces_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_workspaces",
                action: &action,
                targets: vec![params.workspace_name.clone()],
                changelist: None,
                workspace: Some(params.workspace_name.clone()),
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
        let invocation = match params.action {
            WorkspaceModifyAction::Create | WorkspaceModifyAction::Update => {
                let output = self
                    .run_p4(text_invocation(vec![
                        "client".to_string(),
                        "-o".to_string(),
                        params.workspace_name.clone(),
                    ]))
                    .await?;
                let existing = output
                    .text
                    .get("stdout")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        to_mcp_error(invalid_input("p4 client -o did not return stdout"))
                    })?;
                let patch = WorkspaceFormPatch {
                    root: params.workspace_root.clone(),
                    description: params.workspace_description.clone(),
                    options: params.workspace_options.clone(),
                    line_end: params.workspace_line_end.clone(),
                    view: params.workspace_view.clone(),
                };
                let patched = patch_workspace_form(existing, &patch).map_err(to_mcp_error)?;
                json_invocation(vec!["client".to_string(), "-i".to_string()], Some(patched))
            }
            WorkspaceModifyAction::Delete | WorkspaceModifyAction::Switch => invocation,
        };
        self.call_p4_tool(&action, invocation).await
    }

    async fn modify_jobs_inner(
        &self,
        params: ModifyJobsParams,
        channel: ApprovalChannel,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Write, Toolset::Jobs, "modify_jobs")
            .map_err(to_mcp_error)?;
        let invocation = build_job_modify_invocation(&params).map_err(to_mcp_error)?;
        let action = params.action.as_str().to_string();
        let request = self.modify_jobs_approval_request(
            &params,
            P4ApprovalContext {
                tool: "modify_jobs",
                action: &action,
                targets: vec![
                    format!("job:{}", params.job_id),
                    format!("changelist:{}", params.changelist_id),
                ],
                changelist: Some(params.changelist_id.clone()),
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
        self.call_p4_tool(&action, invocation).await
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

    fn modify_changelists_approval_request(
        &self,
        params: &ModifyChangelistsParams,
        context: P4ApprovalContext<'_>,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("modify changelists params serialize to JSON"),
            preview: self.p4_approval_preview(context),
        }
    }

    fn modify_shelves_approval_request(
        &self,
        params: &ModifyShelvesParams,
        context: P4ApprovalContext<'_>,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("modify shelves params serialize to JSON"),
            preview: self.p4_approval_preview(context),
        }
    }

    fn modify_workspaces_approval_request(
        &self,
        params: &ModifyWorkspacesParams,
        context: P4ApprovalContext<'_>,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("modify workspaces params serialize to JSON"),
            preview: self.p4_approval_preview(context),
        }
    }

    fn modify_jobs_approval_request(
        &self,
        params: &ModifyJobsParams,
        context: P4ApprovalContext<'_>,
    ) -> ApprovalRequest {
        let mut approval_params = params.clone();
        approval_params.approval_token = None;

        ApprovalRequest {
            tool: context.tool.to_string(),
            action: context.action.to_string(),
            params: serde_json::to_value(approval_params)
                .expect("modify jobs params serialize to JSON"),
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
        Parameters(params): Parameters<QueryServerParams>,
    ) -> McpResult<Json<ToolResponse>> {
        let invocation = build_server_invocation(&params);
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
        Parameters(params): Parameters<QueryChangelistsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Changelists, "query_changelists")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_changelist_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.status.as_deref(),
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.depot_path.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(
        description = "Create, update, submit, or delete changelists",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_changelists(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyChangelistsParams>,
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
        Parameters(params): Parameters<QueryShelvesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Shelves, "query_shelves")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_shelf_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(
        description = "Shelve, unshelve, or delete shelved files",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_shelves(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyShelvesParams>,
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
        Parameters(params): Parameters<QueryWorkspacesParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Workspaces, "query_workspaces")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        if action == "type" {
            return self
                .query_workspace_type(params.workspace_name.as_deref())
                .await;
        }
        if action == "status" {
            return self
                .query_workspace_status(params.workspace_name.as_deref())
                .await;
        }
        let invocation = build_workspace_query_invocation(
            action,
            params.workspace_name.as_deref(),
            params.user.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(
        description = "Create, update, or delete workspaces using p4 client forms",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    pub async fn modify_workspaces(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyWorkspacesParams>,
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
        Parameters(params): Parameters<QueryJobsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Jobs, "query_jobs")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_job_query_invocation(
            action,
            params.changelist_id.as_deref(),
            params.job_id.as_deref(),
            params.max_results,
        )
        .map_err(to_mcp_error)?;
        self.call_p4_tool(action, invocation).await
    }

    #[tool(
        description = "Attach or detach jobs from changelists",
        annotations(read_only_hint = false)
    )]
    pub async fn modify_jobs(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<ModifyJobsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.modify_jobs_inner(params, ApprovalChannel::Elicitation(peer))
            .await
    }

    #[tool(
        description = "List streams, get stream specs, graph streams, validate stream files, check stream resolves, and inspect stream integration status",
        annotations(read_only_hint = true)
    )]
    pub async fn query_streams(
        &self,
        Parameters(params): Parameters<QueryStreamsParams>,
    ) -> McpResult<Json<ToolResponse>> {
        self.policy()
            .check(Access::Read, Toolset::Streams, "query_streams")
            .map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let command = build_stream_query_command(&params).map_err(to_mcp_error)?;
        match command {
            StreamQueryCommand::Single(invocation) => self.call_p4_tool(action, invocation).await,
            StreamQueryCommand::Parent { stream_name } => {
                self.query_stream_parent(action, &stream_name).await
            }
            StreamQueryCommand::Graph { stream_name } => {
                self.query_stream_graph(action, &stream_name).await
            }
            StreamQueryCommand::ValidateFile {
                workspace,
                file_paths,
            } => {
                self.query_stream_validate_file(action, workspace.as_deref(), &file_paths)
                    .await
            }
            StreamQueryCommand::ValidateSubmit {
                workspace,
                changelist,
            } => {
                self.query_stream_validate_submit(
                    action,
                    workspace.as_deref(),
                    changelist.as_deref(),
                )
                .await
            }
            StreamQueryCommand::CheckResolve { stream_name } => {
                self.query_stream_check_resolve(action, &stream_name).await
            }
            StreamQueryCommand::Interchanges {
                stream_name,
                reverse,
                file_paths,
                long_output,
                limit,
            } => {
                self.query_stream_interchanges(
                    action,
                    &stream_name,
                    reverse,
                    &file_paths,
                    long_output,
                    limit,
                )
                .await
            }
        }
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

fn required_record_field(records: &[Value], field: &str) -> crate::error::Result<String> {
    records
        .first()
        .and_then(|record| non_empty_string_field(record, field))
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{field} is required"),
        })
}

fn required_workspace_stream(records: &[Value]) -> crate::error::Result<String> {
    required_record_field(records, "Stream").map_err(|_| P4McpError::InvalidInput {
        message: "interchanges requires a stream-based workspace".to_string(),
    })
}

fn collect_record_strings(records: &[Value], field: &str) -> Vec<String> {
    let mut values = Vec::new();

    for record in records {
        if let Some(value) = record.get(field) {
            push_record_strings(value, &mut values);
        }

        let Some(object) = record.as_object() else {
            continue;
        };
        let mut indexed = object
            .iter()
            .filter_map(|(key, value)| {
                let suffix = key.strip_prefix(field)?;
                if suffix.is_empty() {
                    return None;
                }
                Some((suffix.parse::<usize>().ok(), suffix.to_string(), value))
            })
            .collect::<Vec<_>>();
        indexed.sort_by(
            |(left_index, left_suffix, _), (right_index, right_suffix, _)| match (
                left_index,
                right_index,
            ) {
                (Some(left), Some(right)) => left.cmp(right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left_suffix.cmp(right_suffix),
            },
        );
        for (_, _, value) in indexed {
            push_record_strings(value, &mut values);
        }
    }

    values
}

fn push_record_strings(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            values.extend(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            );
        }
        Value::String(value) => {
            let value = value.trim();
            if !value.is_empty() {
                values.push(value.to_string());
            }
        }
        _ => {}
    }
}

fn stream_interchanges_message(
    stream_name: &str,
    workspace_stream: &str,
    reverse: bool,
    count: usize,
) -> (usize, String) {
    if count == 0 {
        return (
            count,
            format!(
                "Streams are in sync; no outstanding changelists between '{stream_name}' and '{workspace_stream}'"
            ),
        );
    }

    let message = if reverse {
        format!(
            "{count} outstanding changelist(s) in '{workspace_stream}' not yet propagated to '{stream_name}'"
        )
    } else {
        format!(
            "{count} outstanding changelist(s) in '{stream_name}' not yet merged into '{workspace_stream}'"
        )
    };

    (count, message)
}

fn stream_rule_result(path_type: &str) -> (bool, String) {
    match path_type {
        "exclude" => (false, "excluded".to_string()),
        "import" => (false, "import_readonly".to_string()),
        "share" | "isolate" | "import+" => (true, path_type.to_string()),
        other => (false, other.to_string()),
    }
}

fn p4_pattern_matches(pattern: &str, file: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/...") {
        return file == prefix || file.starts_with(&format!("{prefix}/"));
    }
    wildcard_match(pattern.as_bytes(), file.as_bytes())
}

fn wildcard_match(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern.starts_with(b"...") {
        return wildcard_match(&pattern[3..], text)
            || (!text.is_empty() && wildcard_match(pattern, &text[1..]));
    }
    if pattern[0] == b'*' {
        return wildcard_match(&pattern[1..], text)
            || (!text.is_empty() && text[0] != b'/' && wildcard_match(pattern, &text[1..]));
    }
    if text.first() == Some(&pattern[0]) {
        return wildcard_match(&pattern[1..], &text[1..]);
    }
    false
}

fn stream_pattern_matches(file: &str, stream: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    let depot_pattern = if pattern.starts_with("//") {
        pattern.to_string()
    } else {
        format!(
            "{}/{}",
            stream.trim_end_matches('/'),
            pattern.trim_start_matches('/')
        )
    };
    p4_pattern_matches(&depot_pattern, file)
}

fn classify_stream_file(file: &str, stream: &str, paths: &[String], ignored: &[String]) -> Value {
    if ignored
        .iter()
        .any(|pattern| stream_pattern_matches(file, stream, pattern))
    {
        return json!({
            "file": file,
            "allowed": false,
            "rule": "ignored"
        });
    }

    let mut matched_rule = None;
    for path in paths {
        let mut parts = path.split_whitespace();
        let path_type = parts.next().unwrap_or_default();
        let pattern = parts.next().unwrap_or_default();
        if stream_pattern_matches(file, stream, pattern) {
            matched_rule = Some(stream_rule_result(path_type));
        }
    }

    if let Some((allowed, rule)) = matched_rule {
        return json!({
            "file": file,
            "allowed": allowed,
            "rule": rule
        });
    }

    json!({
        "file": file,
        "allowed": false,
        "rule": "outside_view"
    })
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

fn changelist_modify_targets(
    params: &ModifyChangelistsParams,
    changelist_id: Option<&str>,
) -> Vec<String> {
    let mut targets = changelist_id.map(changelist_targets).unwrap_or_default();
    if params.action == ChangelistModifyAction::MoveFiles {
        if let Some(file_paths) = &params.file_paths {
            targets.extend(file_paths.iter().cloned());
        }
    }
    targets
}

fn shelf_targets(changelist_id: &str) -> Vec<String> {
    vec![format!("shelf:{changelist_id}")]
}

fn shelf_modify_targets(params: &ModifyShelvesParams) -> Vec<String> {
    let mut targets = shelf_targets(&params.changelist_id);
    if let Some(file_paths) = &params.file_paths {
        targets.extend(file_paths.iter().cloned());
    }
    targets
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
        tools::params::{
            ChangelistModifyAction, FileModifyAction, JobModifyAction, ShelfModifyAction,
            WorkspaceModifyAction,
        },
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

    fn modify_changelists_params(action: ChangelistModifyAction) -> ModifyChangelistsParams {
        ModifyChangelistsParams {
            action,
            changelist_id: None,
            description: None,
            file_paths: None,
            approval_token: None,
        }
    }

    fn modify_shelves_params(action: ShelfModifyAction) -> ModifyShelvesParams {
        ModifyShelvesParams {
            action,
            changelist_id: "123".to_string(),
            file_paths: None,
            target_changelist: "default".to_string(),
            force: false,
            approval_token: None,
        }
    }

    fn modify_workspaces_params(action: WorkspaceModifyAction) -> ModifyWorkspacesParams {
        ModifyWorkspacesParams {
            action,
            workspace_name: "ws-main".to_string(),
            workspace_root: None,
            workspace_description: None,
            workspace_options: None,
            workspace_line_end: None,
            workspace_view: None,
            approval_token: None,
        }
    }

    fn modify_jobs_params(action: JobModifyAction) -> ModifyJobsParams {
        ModifyJobsParams {
            action,
            changelist_id: "123".to_string(),
            job_id: "job000001".to_string(),
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
    async fn modify_files_sync_without_files_rejects_before_approval() {
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
        let mut params = modify_files_params(None);
        params.file_paths = None;

        let err = match server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("sync without file_paths should be rejected"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("file_paths is required for sync"));
        assert!(executor.invocations().is_empty());
        assert!(approval_gate.calls().is_empty());
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
        let mut params = modify_changelists_params(ChangelistModifyAction::Submit);
        params.changelist_id = Some("123".to_string());

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
        let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
        params.changelist_id = Some("123".to_string());
        params.file_paths = Some(vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ]);

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
        let mut params = modify_changelists_params(ChangelistModifyAction::MoveFiles);
        params.changelist_id = Some("123".to_string());
        params.file_paths = Some(vec![
            "//depot/main/a.rs".to_string(),
            "//depot/main/b.rs".to_string(),
        ]);
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
    async fn modify_changelists_update_without_approval_does_not_fetch_form() {
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
        let mut params = modify_changelists_params(ChangelistModifyAction::Update);
        params.changelist_id = Some("123".to_string());
        params.description = Some("new description".to_string());

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
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
                "change".to_string(),
                "-i".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn modify_changelists_update_after_approval_patches_existing_form() {
        let existing_form = "\
Change: 123

Description:
\told description

Files:
\t//depot/main/a.rs
";
        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: Vec::new(),
                text: json!({"stdout": existing_form, "stderr": ""}),
            },
            P4CommandOutput {
                records: vec![json!({"change": "123"})],
                text: json!({}),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = modify_changelists_params(ChangelistModifyAction::Update);
        params.changelist_id = Some("123".to_string());
        params.description = Some("new description".to_string());

        let response = server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approved update should succeed");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "update");
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].args, ["change", "-o", "123"]);
        assert_eq!(invocations[0].mode, OutputMode::Text);
        assert_eq!(invocations[1].args, ["change", "-i"]);
        assert_eq!(
            invocations[1].stdin.as_deref(),
            Some(
                "\
Change: 123

Description:
\tnew description

Files:
\t//depot/main/a.rs
"
            )
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
        let mut params = modify_shelves_params(ShelfModifyAction::Delete);
        params.file_paths = Some(vec!["//depot/main/file.txt".to_string()]);

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
        assert_eq!(
            calls[0].request.preview.targets,
            ["shelf:123", "//depot/main/file.txt"]
        );
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
        let mut params = modify_shelves_params(ShelfModifyAction::Delete);
        params.file_paths = Some(vec![
            "//depot/main/file.txt".to_string(),
            "//depot/main/other.txt".to_string(),
        ]);

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
        let params = modify_workspaces_params(WorkspaceModifyAction::Delete);

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
    async fn modify_workspaces_update_preview_uses_workspace_name() {
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
        let mut params = modify_workspaces_params(WorkspaceModifyAction::Update);
        params.workspace_root = Some("/workspace/root".to_string());

        let response = server
            .modify_workspaces_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("approval response should be returned");

        assert_eq!(response.0.status, "approval_required");
        assert!(executor.invocations().is_empty());
        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].request.preview.targets, ["ws-main"]);
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
        let params = modify_jobs_params(JobModifyAction::LinkJob);

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
        assert_eq!(calls[0].request.action, "link_job");
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
        assert_eq!(
            calls[0].request.preview.targets,
            ["job:job000001", "changelist:123"]
        );
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
