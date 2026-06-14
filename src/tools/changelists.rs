use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_changelist_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    status: Option<&str>,
    workspace_name: Option<&str>,
    user: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "get" if changelist_id == Some("default") => {
            vec!["opened".into(), "-c".into(), "default".into()]
        }
        "get" => vec![
            "describe".into(),
            "-s".into(),
            required(changelist_id, "changelist_id")?,
        ],
        "list" => {
            let mut args = vec!["changes".into(), "-m".into(), max_results.to_string()];
            if let Some(status) = status {
                args.extend(["-s".into(), status.into()]);
            }
            if let Some(workspace) = workspace_name {
                args.extend(["-c".into(), workspace.into()]);
            }
            if let Some(user) = user {
                args.extend(["-u".into(), user.into()]);
            }
            args
        }
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

pub fn build_changelist_modify_invocation(
    action: &str,
    changelist_id: &str,
    stdin: Option<String>,
) -> Result<P4Invocation> {
    let (args, stdin) = match action {
        "create" => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "create")?),
        ),
        "update" => (
            vec!["change".into(), "-i".into()],
            Some(required_stdin(stdin, "update")?),
        ),
        "submit" => (
            vec![
                "submit".into(),
                "-c".into(),
                required_value(changelist_id, "changelist_id", "submit")?,
            ],
            None,
        ),
        "delete" => (
            vec![
                "change".into(),
                "-d".into(),
                required_value(changelist_id, "changelist_id", "delete")?,
            ],
            None,
        ),
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin,
        mode: OutputMode::JsonLines,
    })
}

fn required_stdin(stdin: Option<String>, action: &str) -> Result<String> {
    match stdin {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(P4McpError::InvalidInput {
            message: format!("stdin is required for {action}"),
        }),
    }
}

fn required_value(value: &str, name: &str, action: &str) -> Result<String> {
    if value.trim().is_empty() {
        Err(P4McpError::InvalidInput {
            message: format!("{name} is required for {action}"),
        })
    } else {
        Ok(value.to_string())
    }
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}

fn unknown<T>(action: &str) -> Result<T> {
    Err(P4McpError::InvalidInput {
        message: format!("unknown action: {action}"),
    })
}
