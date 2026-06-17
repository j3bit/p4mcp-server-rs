use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{
        ModifyStreamsParams, QueryStreamsParams, StreamModifyAction, StreamQueryAction,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamQueryCommand {
    Single(P4Invocation),
    Get {
        stream_name: Option<String>,
        view_without_edit: bool,
        at_change: Option<String>,
    },
    Children {
        stream_name: String,
    },
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
    GetWorkspace {
        workspace: Option<String>,
        stream_name: Option<String>,
        template: Option<String>,
    },
    ListWorkspaces {
        stream_name: Option<String>,
        user: Option<String>,
        unloaded: bool,
        max_results: u16,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamModifyCommand {
    Single(P4Invocation),
    Create {
        stream_name: String,
    },
    Update {
        stream_name: String,
    },
    Switch {
        stream_name: String,
        workspace: Option<String>,
        preview: bool,
    },
    CreateWorkspace {
        stream_name: String,
        workspace_name: String,
        root: String,
    },
}

impl StreamModifyCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            Self::Create { .. }
            | Self::Update { .. }
            | Self::Switch { .. }
            | Self::CreateWorkspace { .. } => None,
        }
    }
}

pub fn build_stream_query_command(params: &QueryStreamsParams) -> Result<StreamQueryCommand> {
    match &params.action {
        StreamQueryAction::List => Ok(StreamQueryCommand::Single(stream_list_invocation(params))),
        StreamQueryAction::Get => Ok(StreamQueryCommand::Get {
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            view_without_edit: params.view_without_edit,
            at_change: non_blank(params.at_change.as_deref()).map(str::to_string),
        }),
        StreamQueryAction::Children => Ok(StreamQueryCommand::Children {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Parent => Ok(StreamQueryCommand::Parent {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::Graph => Ok(StreamQueryCommand::Graph {
            stream_name: required(params.stream_name.as_deref(), "stream_name")?,
        }),
        StreamQueryAction::IntegrationStatus => {
            Ok(StreamQueryCommand::Single(stream_istat_invocation(params)))
        }
        StreamQueryAction::GetWorkspace => Ok(StreamQueryCommand::GetWorkspace {
            workspace: non_blank(params.workspace.as_deref()).map(str::to_string),
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            template: non_blank(params.template.as_deref()).map(str::to_string),
        }),
        StreamQueryAction::ListWorkspaces => Ok(StreamQueryCommand::ListWorkspaces {
            stream_name: non_blank(params.stream_name.as_deref()).map(str::to_string),
            user: non_blank(params.user.as_deref()).map(str::to_string),
            unloaded: params.unloaded,
            max_results: params.max_results,
        }),
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

pub fn build_stream_modify_command(params: &ModifyStreamsParams) -> Result<StreamModifyCommand> {
    let invocation = match &params.action {
        StreamModifyAction::Create => {
            let stream_name =
                required_for_action(params.stream_name.as_deref(), "stream_name", &params.action)?;
            required_for_action(params.stream_type.as_deref(), "stream_type", &params.action)?;
            return Ok(StreamModifyCommand::Create { stream_name });
        }
        StreamModifyAction::Update => {
            return Ok(StreamModifyCommand::Update {
                stream_name: required_for_action(
                    params.stream_name.as_deref(),
                    "stream_name",
                    &params.action,
                )?,
            });
        }
        StreamModifyAction::CreateWorkspace => {
            return Ok(StreamModifyCommand::CreateWorkspace {
                stream_name: required_for_action(
                    params.stream_name.as_deref(),
                    "stream_name",
                    &params.action,
                )?,
                workspace_name: required_for_action(
                    params.workspace_name.as_deref(),
                    "workspace_name",
                    &params.action,
                )?,
                root: required_for_action(params.root.as_deref(), "root", &params.action)?,
            });
        }
        StreamModifyAction::Delete => json_invocation(vec![
            "stream".into(),
            "-d".into(),
            required(params.stream_name.as_deref(), "stream_name")?,
        ]),
        StreamModifyAction::EditSpec => {
            let mut args = vec!["edit".to_string(), "-So".to_string()];
            if let Some(changelist) = non_blank(params.changelist.as_deref()) {
                args.extend(["-c".to_string(), changelist.to_string()]);
            }
            json_invocation(args)
        }
        StreamModifyAction::ResolveSpec => {
            let mode = match params.resolve_mode.as_deref().unwrap_or("auto") {
                "auto" => "-am",
                "accept_theirs" => "-at",
                "accept_yours" => "-ay",
                "accept_safe" => "-as",
                other => {
                    return Err(P4McpError::InvalidInput {
                        message: format!("invalid resolve_mode: {other}"),
                    });
                }
            };
            json_invocation(vec!["resolve".into(), "-So".into(), mode.into()])
        }
        StreamModifyAction::RevertSpec => json_invocation(vec!["revert".into(), "-So".into()]),
        StreamModifyAction::ShelveSpec => json_invocation(vec![
            "shelve".into(),
            "-As".into(),
            "-c".into(),
            required(params.changelist.as_deref(), "changelist")?,
        ]),
        StreamModifyAction::UnshelveSpec => {
            let mut args = vec![
                "unshelve".to_string(),
                "-As".to_string(),
                "-s".to_string(),
                required(params.changelist.as_deref(), "changelist")?,
            ];
            if let Some(target) = non_blank(params.target_changelist.as_deref()) {
                args.extend(["-c".to_string(), target.to_string()]);
            }
            json_invocation(args)
        }
        StreamModifyAction::Copy => copy_invocation(params),
        StreamModifyAction::Merge => merge_invocation(params),
        StreamModifyAction::Integrate => integrate_invocation(params),
        StreamModifyAction::Populate => populate_invocation(params),
        StreamModifyAction::Switch => {
            return Ok(StreamModifyCommand::Switch {
                stream_name: required(params.stream_name.as_deref(), "stream_name")?,
                workspace: non_blank(params.workspace.as_deref()).map(str::to_string),
                preview: params.preview,
            });
        }
    };
    Ok(StreamModifyCommand::Single(invocation))
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

pub fn stream_get_invocation(
    stream_name: &str,
    view_without_edit: bool,
    at_change: Option<&str>,
) -> P4Invocation {
    let mut args = vec!["stream".into(), "-o".into()];
    if view_without_edit {
        args.push("-v".into());
    }
    let specifier = match non_blank(at_change) {
        Some(change) => format!("{stream_name}@{change}"),
        None => stream_name.to_string(),
    };
    args.push(specifier);
    json_invocation(args)
}

pub fn stream_children_invocation(stream_name: &str) -> P4Invocation {
    json_invocation(vec![
        "streams".into(),
        "-F".into(),
        format!("Parent={stream_name}"),
    ])
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

pub fn stream_get_workspace_invocation(
    workspace: Option<&str>,
    stream_name: Option<&str>,
    template: Option<&str>,
) -> P4Invocation {
    let mut args = vec!["client".into(), "-o".into()];
    if let Some(stream_name) = non_blank(stream_name) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(template) = non_blank(template) {
        args.extend(["-t".into(), template.into()]);
    }
    if let Some(workspace) = non_blank(workspace) {
        args.push(workspace.into());
    }
    json_invocation(args)
}

pub fn stream_list_workspaces_invocation(
    stream_name: Option<&str>,
    user: Option<&str>,
    unloaded: bool,
    max_results: u16,
) -> P4Invocation {
    let mut args = vec!["clients".into()];
    if unloaded {
        args.push("-U".into());
    }
    if let Some(stream_name) = non_blank(stream_name) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(user) = non_blank(user) {
        args.extend(["-u".into(), user.into()]);
    }
    args.extend(["-m".into(), max_results.to_string()]);
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

fn copy_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["copy".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-F".into());
    }
    if params.virtual_stream {
        args.push("-v".into());
    }
    push_quiet_changelist_max(&mut args, params);
    push_stream_mode(&mut args, params);
    if params.reverse {
        args.push("-r".into());
    }
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn merge_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["merge".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-F".into());
    }
    push_quiet_changelist_max(&mut args, params);
    if params.output_base {
        args.push("-Ob".into());
    }
    push_stream_mode(&mut args, params);
    if params.reverse {
        args.push("-r".into());
    }
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn integrate_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["integrate".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-f".into());
    }
    if params.quiet {
        args.push("-q".into());
    }
    if params.output_base {
        args.push("-Ob".into());
    }
    push_changelist_max(&mut args, params);
    if params.integrate_around_deleted {
        args.push("-Di".into());
    }
    if params.schedule_branch_resolve {
        args.push("-Rb".into());
    }
    if params.skip_cherry_picked {
        args.push("-Rs".into());
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
    } else {
        push_stream_mode(&mut args, params);
    }
    if params.reverse {
        args.push("-r".into());
    }
    push_file_paths(&mut args, params);
    json_invocation(args)
}

fn populate_invocation(params: &ModifyStreamsParams) -> P4Invocation {
    let mut args = vec!["populate".to_string()];
    push_preview(&mut args, params);
    if params.force {
        args.push("-f".into());
    }
    if params.output_base {
        args.push("-o".into());
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
    if let Some(description) = non_blank(params.description.as_deref()) {
        args.extend(["-d".into(), description.into()]);
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
        if params.reverse {
            args.push("-r".into());
        }
    } else if non_blank(params.stream_name.as_deref()).is_some() {
        push_stream_mode(&mut args, params);
        if params.reverse {
            args.push("-r".into());
        }
    } else {
        if let (Some(source), Some(target)) = (
            non_blank(params.source_path.as_deref()),
            non_blank(params.target_path.as_deref()),
        ) {
            args.push(source.into());
            args.push(target.into());
        }
    }
    json_invocation(args)
}

fn push_preview(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if params.preview {
        args.push("-n".into());
    }
}

fn push_quiet_changelist_max(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if params.quiet {
        args.push("-q".into());
    }
    push_changelist_max(args, params);
}

fn push_changelist_max(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(changelist) = non_blank(params.changelist.as_deref()) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
}

fn push_stream_mode(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
        if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
            args.extend(["-P".into(), parent.into()]);
        }
    }
}

fn push_file_paths(args: &mut Vec<String>, params: &ModifyStreamsParams) {
    if let Some(file_paths) = &params.file_paths {
        args.extend(file_paths.iter().cloned());
    }
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

fn required_for_action(
    value: Option<&str>,
    name: &str,
    action: &StreamModifyAction,
) -> Result<String> {
    match non_blank(value) {
        Some(value) => Ok(value.to_string()),
        None => Err(P4McpError::InvalidInput {
            message: format!("{name} is required for {}", action.as_str()),
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
