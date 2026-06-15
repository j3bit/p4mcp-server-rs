use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{QueryStreamsParams, StreamQueryAction},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamQueryCommand {
    Single(P4Invocation),
    Parent {
        stream_name: String,
    },
    Graph {
        stream_name: String,
    },
    ValidateFile {
        workspace: Option<String>,
        file_paths: Vec<String>,
    },
    ValidateSubmit {
        workspace: Option<String>,
        changelist: Option<String>,
    },
    CheckResolve {
        stream_name: String,
    },
    Interchanges {
        stream_name: String,
        reverse: bool,
        file_paths: Vec<String>,
        long_output: bool,
        limit: Option<u16>,
    },
}

impl StreamQueryCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            _ => None,
        }
    }
}

pub fn build_stream_query_command(params: &QueryStreamsParams) -> Result<StreamQueryCommand> {
    match &params.action {
        StreamQueryAction::List => Ok(StreamQueryCommand::Single(stream_list_invocation(params))),
        StreamQueryAction::Get => Ok(StreamQueryCommand::Single(stream_get_invocation(params)?)),
        StreamQueryAction::Children => Ok(StreamQueryCommand::Single(stream_children_invocation(
            params,
        )?)),
        StreamQueryAction::Parent => Ok(StreamQueryCommand::Parent {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Graph => Ok(StreamQueryCommand::Graph {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::IntegrationStatus => {
            Ok(StreamQueryCommand::Single(stream_istat_invocation(params)))
        }
        StreamQueryAction::GetWorkspace => Ok(StreamQueryCommand::Single(
            stream_get_workspace_invocation(params),
        )),
        StreamQueryAction::ListWorkspaces => Ok(StreamQueryCommand::Single(
            stream_list_workspaces_invocation(params),
        )),
        StreamQueryAction::ValidateFile => Ok(StreamQueryCommand::ValidateFile {
            workspace: params.workspace.clone(),
            file_paths: required_files(params.file_paths.as_deref(), "file_paths")?,
        }),
        StreamQueryAction::ValidateSubmit => Ok(StreamQueryCommand::ValidateSubmit {
            workspace: params.workspace.clone(),
            changelist: params.changelist.clone(),
        }),
        StreamQueryAction::CheckResolve => Ok(StreamQueryCommand::CheckResolve {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Interchanges => Ok(StreamQueryCommand::Interchanges {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
            reverse: params.reverse,
            file_paths: params.file_paths.clone().unwrap_or_default(),
            long_output: params.long_output,
            limit: params.limit,
        }),
    }
}

pub fn build_stream_query_invocation(
    action: &str,
    stream: Option<&str>,
    owner: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    match action {
        "parent" => Ok(json_invocation(vec![
            "stream".into(),
            "-o".into(),
            required(stream, "stream")?,
        ])),
        "graph" => Ok(json_invocation(vec![
            "streams".into(),
            "-T".into(),
            "Stream,Parent,Type,Name,Owner".into(),
        ])),
        _ => {
            let params = QueryStreamsParams {
                action: legacy_stream_action(action)?,
                stream_name: stream.map(str::to_string),
                stream_path: None,
                filter: owner.map(|owner| format!("Owner={owner}")),
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
                max_results,
            };
            match build_stream_query_command(&params)? {
                StreamQueryCommand::Single(invocation) => Ok(invocation),
                _ => Err(P4McpError::InvalidInput {
                    message: format!("action requires server routing: {action}"),
                }),
            }
        }
    }
}

fn legacy_stream_action(action: &str) -> Result<StreamQueryAction> {
    match action {
        "list" => Ok(StreamQueryAction::List),
        "get" => Ok(StreamQueryAction::Get),
        "children" => Ok(StreamQueryAction::Children),
        "parent" => Ok(StreamQueryAction::Parent),
        "graph" => Ok(StreamQueryAction::Graph),
        "integration_status" => Ok(StreamQueryAction::IntegrationStatus),
        "get_workspace" => Ok(StreamQueryAction::GetWorkspace),
        "list_workspaces" => Ok(StreamQueryAction::ListWorkspaces),
        other => Err(P4McpError::InvalidInput {
            message: format!("unknown action: {other}"),
        }),
    }
}

fn stream_list_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["streams".into()];
    if params.unloaded {
        args.push("-U".into());
    }
    if params.all_streams {
        args.push("-a".into());
    }
    if let Some(filter) = non_blank(params.filter.as_deref()) {
        args.extend(["-F".into(), filter.into()]);
    }
    if let Some(fields) = params.fields.as_deref().filter(|fields| !fields.is_empty()) {
        args.extend(["-T".into(), fields.join(",")]);
    }
    args.extend(["-m".into(), params.max_results.to_string()]);
    if let Some(viewmatch) = non_blank(params.viewmatch.as_deref()) {
        args.extend(["--viewmatch".into(), viewmatch.into()]);
    }
    if let Some(stream_path) = params.stream_path.as_deref() {
        args.extend(stream_path.iter().cloned());
    }
    json_invocation(args)
}

fn stream_get_invocation(params: &QueryStreamsParams) -> Result<P4Invocation> {
    let stream_name = required(params.stream_name.as_deref(), "stream_name")?;
    let mut args = vec!["stream".into(), "-o".into()];
    if params.view_without_edit {
        args.push("-v".into());
    }
    let specifier = match non_blank(params.at_change.as_deref()) {
        Some(change) => format!("{stream_name}@{change}"),
        None => stream_name,
    };
    args.push(specifier);
    Ok(json_invocation(args))
}

fn stream_children_invocation(params: &QueryStreamsParams) -> Result<P4Invocation> {
    Ok(json_invocation(vec![
        "streams".into(),
        "-F".into(),
        format!(
            "Parent={}",
            required(params.stream_name.as_deref(), "stream_name")?
        ),
    ]))
}

fn stream_istat_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["istat".into()];
    if params.both_directions {
        args.push("-a".into());
    }
    if params.force_refresh {
        args.push("-c".into());
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.push(stream_name.into());
    }
    json_invocation(args)
}

fn stream_get_workspace_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(template) = non_blank(params.template.as_deref()) {
        args.extend(["-t".into(), template.into()]);
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(workspace) = non_blank(params.workspace.as_deref()) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

fn stream_list_workspaces_invocation(params: &QueryStreamsParams) -> P4Invocation {
    let mut args = vec!["clients".into()];
    if params.unloaded {
        args.push("-U".into());
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(user) = non_blank(params.user.as_deref()) {
        args.extend(["-u".into(), user.into()]);
    }
    args.extend(["-m".into(), params.max_results.to_string()]);
    json_invocation(args)
}

pub fn interchanges_invocation(
    stream_name: &str,
    reverse: bool,
    file_paths: &[String],
    long_output: bool,
) -> P4Invocation {
    let mut args = vec!["interchanges".into(), "-S".into(), stream_name.to_string()];
    if reverse {
        args.push("-r".into());
    }
    if long_output {
        args.push("-l".into());
    }
    args.extend(file_paths.iter().cloned());
    json_invocation(args)
}

pub fn opened_for_stream_validation_invocation(
    workspace: Option<&str>,
    changelist: Option<&str>,
) -> P4Invocation {
    let mut args = vec!["opened".into()];
    if let Some(changelist) = non_blank(changelist) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(workspace) = non_blank(workspace) {
        args.extend(["-C".into(), workspace.into()]);
    }
    json_invocation(args)
}

pub fn client_spec_invocation(workspace: Option<&str>) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(workspace) = non_blank(workspace) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

pub fn stream_spec_with_view_invocation(stream_name: &str) -> P4Invocation {
    json_invocation(vec![
        "stream".into(),
        "-o".into(),
        "-v".into(),
        stream_name.into(),
    ])
}

pub fn stream_resolve_preview_invocation() -> P4Invocation {
    json_invocation(vec!["stream".into(), "resolve".into(), "-n".into()])
}

fn json_invocation(args: Vec<String>) -> P4Invocation {
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    match non_blank(value) {
        Some(value) => Ok(value.to_string()),
        None => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}

fn required_files(files: Option<&[String]>, name: &str) -> Result<Vec<String>> {
    match files {
        Some(files) if !files.is_empty() => Ok(files.to_vec()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}
