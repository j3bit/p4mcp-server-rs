use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{ModifyWorkspacesParams, WorkspaceModifyAction},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceModifyCommand {
    Create { workspace_name: String },
    Update { workspace_name: String },
    Delete(P4Invocation),
    Switch { workspace_name: String },
}

impl WorkspaceModifyCommand {
    pub fn preview_invocation(&self) -> Option<&P4Invocation> {
        match self {
            Self::Delete(invocation) => Some(invocation),
            Self::Create { .. } | Self::Update { .. } | Self::Switch { .. } => None,
        }
    }
}

pub fn build_workspace_modify_command(
    params: &ModifyWorkspacesParams,
) -> Result<WorkspaceModifyCommand> {
    let workspace_name = required_workspace_name(&params.workspace_name, params.action.as_str())?;
    match params.action {
        WorkspaceModifyAction::Create => {
            require_workspace_spec_fields(params, params.action.as_str())?;
            Ok(WorkspaceModifyCommand::Create { workspace_name })
        }
        WorkspaceModifyAction::Update => {
            require_workspace_spec_fields(params, params.action.as_str())?;
            Ok(WorkspaceModifyCommand::Update { workspace_name })
        }
        WorkspaceModifyAction::Delete => Ok(WorkspaceModifyCommand::Delete(
            build_workspace_delete_invocation(params)?,
        )),
        WorkspaceModifyAction::Switch => Ok(WorkspaceModifyCommand::Switch { workspace_name }),
    }
}

pub fn build_workspace_delete_invocation(params: &ModifyWorkspacesParams) -> Result<P4Invocation> {
    if params.action != WorkspaceModifyAction::Delete {
        return Err(P4McpError::InvalidInput {
            message: format!("unknown action: {}", params.action.as_str()),
        });
    }
    let workspace_name = required_workspace_name(&params.workspace_name, params.action.as_str())?;
    Ok(P4Invocation {
        args: vec!["client".into(), "-d".into(), workspace_name],
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

pub fn required_workspace_name(value: &str, action: &str) -> Result<String> {
    if value.trim().is_empty() {
        return Err(P4McpError::InvalidInput {
            message: format!("workspace_name is required for {action}"),
        });
    }
    Ok(value.to_string())
}

pub fn build_workspace_exists_invocation(workspace_name: &str) -> Result<P4Invocation> {
    Ok(P4Invocation {
        args: vec![
            "clients".into(),
            "-e".into(),
            required(Some(workspace_name), "workspace_name")?,
        ],
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

pub fn build_workspace_query_invocation(
    action: &str,
    workspace_name: Option<&str>,
    user: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list" => {
            let mut args = vec!["clients".into(), "-m".into(), max_results.to_string()];
            if let Some(user) = non_blank(user) {
                args.extend(["-u".into(), user.into()]);
            }
            args
        }
        "get" | "type" => vec![
            "client".into(),
            "-o".into(),
            required(workspace_name, "workspace_name")?,
        ],
        other => {
            return Err(P4McpError::InvalidInput {
                message: format!("unknown action: {other}"),
            });
        }
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn require_workspace_spec_fields(params: &ModifyWorkspacesParams, action: &str) -> Result<()> {
    let has_spec_fields = params
        .workspace_root
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty())
        || params
            .workspace_description
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        || params
            .workspace_view
            .as_ref()
            .is_some_and(|view| !view.is_empty());
    if !has_spec_fields {
        return Err(P4McpError::InvalidInput {
            message: format!("workspace specification fields are required for {action}"),
        });
    }
    Ok(())
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}
