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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamModifyCommand {
    Single(P4Invocation),
    CreateOrUpdate,
}

impl StreamModifyCommand {
    pub fn into_single_invocation(self) -> Option<P4Invocation> {
        match self {
            Self::Single(invocation) => Some(invocation),
            Self::CreateOrUpdate => None,
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

pub fn build_stream_modify_command(params: &ModifyStreamsParams) -> Result<StreamModifyCommand> {
    let invocation = match &params.action {
        StreamModifyAction::Create
        | StreamModifyAction::Update
        | StreamModifyAction::CreateWorkspace => return Ok(StreamModifyCommand::CreateOrUpdate),
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
        StreamModifyAction::Copy => propagation_invocation("copy", params)?,
        StreamModifyAction::Merge => propagation_invocation("merge", params)?,
        StreamModifyAction::Integrate => propagation_invocation("integrate", params)?,
        StreamModifyAction::Populate => populate_invocation(params)?,
        StreamModifyAction::Switch => {
            let mut args = vec![
                "client".to_string(),
                "-s".to_string(),
                "-S".to_string(),
                required(params.stream_name.as_deref(), "stream_name")?,
            ];
            if let Some(workspace) = non_blank(params.workspace.as_deref()) {
                args.push(workspace.to_string());
            }
            json_invocation(args)
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

fn propagation_invocation(command: &str, params: &ModifyStreamsParams) -> Result<P4Invocation> {
    let mut args = vec![command.to_string()];
    if params.preview {
        args.push("-n".into());
    }
    if params.force {
        args.push("-F".into());
    }
    if command == "copy" && params.virtual_stream {
        args.push("-v".into());
    }
    if params.quiet {
        args.push("-q".into());
    }
    if let Some(changelist) = non_blank(params.changelist.as_deref()) {
        args.extend(["-c".into(), changelist.into()]);
    }
    if let Some(max_files) = params.max_files {
        args.push(format!("-m{max_files}"));
    }
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
        args.extend(["-P".into(), parent.into()]);
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
    }
    if params.reverse {
        args.push("-r".into());
    }
    if params.output_base && matches!(command, "merge" | "integrate") {
        args.push("-Ob".into());
    }
    if command == "integrate" {
        if params.schedule_branch_resolve {
            args.push("-Rb".into());
        }
        if params.integrate_around_deleted {
            args.push("-Di".into());
        }
        if params.skip_cherry_picked {
            args.push("-Rs".into());
        }
    }
    if let Some(file_paths) = &params.file_paths {
        args.extend(file_paths.iter().cloned());
    }
    Ok(json_invocation(args))
}

fn populate_invocation(params: &ModifyStreamsParams) -> Result<P4Invocation> {
    let mut args = vec!["populate".to_string()];
    if params.preview {
        args.push("-n".into());
    }
    if params.force {
        args.push("-F".into());
    }
    if params.reverse {
        args.push("-r".into());
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
    if let Some(stream_name) = non_blank(params.stream_name.as_deref()) {
        args.extend(["-S".into(), stream_name.into()]);
    }
    if let Some(parent) = non_blank(params.parent_stream.as_deref()) {
        args.extend(["-P".into(), parent.into()]);
    }
    if let Some(branch) = non_blank(params.branch.as_deref()) {
        args.extend(["-b".into(), branch.into()]);
    }
    if let Some(source) = non_blank(params.source_path.as_deref()) {
        args.push(source.into());
    }
    if let Some(target) = non_blank(params.target_path.as_deref()) {
        args.push(target.into());
    }
    Ok(json_invocation(args))
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
