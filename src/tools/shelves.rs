use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{ModifyShelvesParams, ShelfModifyAction},
};

pub fn build_shelf_modify_invocation(params: &ModifyShelvesParams) -> Result<P4Invocation> {
    let mut args = match params.action {
        ShelfModifyAction::Shelve => {
            let mut args = vec!["shelve".into()];
            if params.force {
                args.push("-f".into());
            }
            args.extend(["-c".into(), params.changelist_id.clone()]);
            args.extend(required_files(params, params.action.as_str())?);
            args
        }
        ShelfModifyAction::Unshelve => {
            let mut args = vec!["unshelve".into()];
            if params.force {
                args.push("-f".into());
            }
            args.extend(["-s".into(), params.changelist_id.clone()]);
            if let Some(file_paths) = &params.file_paths {
                args.extend(file_paths.iter().cloned());
            }
            args
        }
        ShelfModifyAction::Update => {
            let mut args = vec!["shelve".into()];
            if params.force {
                args.push("-f".into());
            }
            args.extend(["-c".into(), params.changelist_id.clone()]);
            args.extend(required_files(params, params.action.as_str())?);
            args
        }
        ShelfModifyAction::Delete => {
            let mut args = vec!["shelve".into(), "-d".into(), "-c".into()];
            args.push(params.changelist_id.clone());
            if let Some(file_paths) = &params.file_paths {
                args.extend(file_paths.iter().cloned());
            }
            args
        }
        ShelfModifyAction::UnshelveToChangelist => {
            let mut args = vec![
                "unshelve".into(),
                "-s".into(),
                params.changelist_id.clone(),
                "-c".into(),
                params.target_changelist.clone(),
            ];
            if let Some(file_paths) = &params.file_paths {
                args.extend(file_paths.iter().cloned());
            }
            args
        }
    };
    Ok(P4Invocation {
        args: std::mem::take(&mut args),
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

pub fn build_shelf_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    user: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let (args, mode) = match action {
        "list" => {
            let mut args = vec![
                "changes".into(),
                "-s".into(),
                "shelved".into(),
                "-m".into(),
                max_results.to_string(),
            ];
            if let Some(user) = user {
                args.extend(["-u".into(), user.into()]);
            }
            (args, OutputMode::JsonLines)
        }
        "diff" => (
            vec![
                "describe".into(),
                "-S".into(),
                "-du".into(),
                required(changelist_id, "changelist_id")?,
            ],
            OutputMode::Text,
        ),
        "files" => (
            vec![
                "describe".into(),
                "-S".into(),
                required(changelist_id, "changelist_id")?,
            ],
            OutputMode::JsonLines,
        ),
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode,
    })
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

fn required_files(params: &ModifyShelvesParams, action: &str) -> Result<Vec<String>> {
    match &params.file_paths {
        Some(file_paths) if !file_paths.is_empty() => Ok(file_paths.clone()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("file_paths is required for {action}"),
        }),
    }
}
