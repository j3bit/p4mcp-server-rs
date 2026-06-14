use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{FileModifyAction, FileQueryAction, ModifyFilesParams, QueryFilesParams},
};

pub fn build_file_invocation(params: &QueryFilesParams) -> Result<P4Invocation> {
    let invocation = match params.action {
        FileQueryAction::Content => P4Invocation {
            args: vec!["print".into(), "-q".into(), params.file_path.clone()],
            stdin: None,
            mode: OutputMode::Text,
        },
        FileQueryAction::History => P4Invocation {
            args: vec![
                "filelog".into(),
                format!("-m{}", params.max_results),
                params.file_path.clone(),
            ],
            stdin: None,
            mode: OutputMode::JsonLines,
        },
        FileQueryAction::Info => P4Invocation {
            args: vec!["fstat".into(), params.file_path.clone()],
            stdin: None,
            mode: OutputMode::JsonLines,
        },
        FileQueryAction::Metadata => P4Invocation {
            args: vec!["fstat".into(), "-Oal".into(), params.file_path.clone()],
            stdin: None,
            mode: OutputMode::JsonLines,
        },
        FileQueryAction::Diff => {
            if params.diff2 {
                let file2 = params
                    .file2
                    .clone()
                    .ok_or_else(|| P4McpError::InvalidInput {
                        message: "file2 is required for diff2".to_string(),
                    })?;
                P4Invocation {
                    args: vec!["diff2".into(), params.file_path.clone(), file2],
                    stdin: None,
                    mode: OutputMode::Text,
                }
            } else {
                if params.file2.is_some() {
                    return Err(P4McpError::InvalidInput {
                        message: "file2 cannot be used for workspace diff".to_string(),
                    });
                }
                P4Invocation {
                    args: vec!["diff".into(), params.file_path.clone()],
                    stdin: None,
                    mode: OutputMode::Text,
                }
            }
        }
        FileQueryAction::Annotations => P4Invocation {
            args: vec!["annotate".into(), params.file_path.clone()],
            stdin: None,
            mode: OutputMode::JsonLines,
        },
        FileQueryAction::Search => {
            let pattern = params
                .pattern
                .clone()
                .ok_or_else(|| P4McpError::InvalidInput {
                    message: "pattern is required for search".to_string(),
                })?;
            P4Invocation {
                args: vec![
                    "files".into(),
                    "-m".into(),
                    params.max_results.to_string(),
                    format!("{}/{}", params.file_path.trim_end_matches('/'), pattern),
                ],
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
        FileQueryAction::Grep => {
            let pattern = params
                .pattern
                .clone()
                .ok_or_else(|| P4McpError::InvalidInput {
                    message: "pattern is required for grep".to_string(),
                })?;
            let mut args = vec!["grep".into(), "-n".into()];
            if params.case_insensitive {
                args.push("-i".into());
            }
            args.extend(["-e".into(), pattern, params.file_path.clone()]);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
    };
    Ok(invocation)
}

pub fn build_file_modify_invocation(params: &ModifyFilesParams) -> Result<P4Invocation> {
    let files = params.file_paths.clone().unwrap_or_default();
    let invocation = match params.action {
        FileModifyAction::Add => {
            require_files(&files, "add")?;
            with_files(vec!["add", "-c", &params.changelist], files)
        }
        FileModifyAction::Edit => {
            require_files(&files, "edit")?;
            with_files(vec!["edit", "-c", &params.changelist], files)
        }
        FileModifyAction::Delete => {
            require_files(&files, "delete")?;
            with_files(vec!["delete", "-c", &params.changelist], files)
        }
        FileModifyAction::Revert => {
            require_files(&files, "revert")?;
            with_files(vec!["revert", "-c", &params.changelist], files)
        }
        FileModifyAction::Reconcile => {
            with_files(vec!["reconcile", "-c", &params.changelist], files)
        }
        FileModifyAction::Sync => {
            let mut args = vec!["sync".to_string()];
            if params.force {
                args.push("-f".to_string());
            }
            args.extend(files);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
        FileModifyAction::Move => {
            let sources = params.source_paths.clone().unwrap_or_default();
            let targets = params.target_paths.clone().unwrap_or_default();
            if sources.len() != targets.len() {
                return Err(P4McpError::InvalidInput {
                    message: "source_paths and target_paths must have the same length".to_string(),
                });
            }
            if sources.len() != 1 {
                return Err(P4McpError::InvalidInput {
                    message: "move accepts exactly one source and one target per tool call"
                        .to_string(),
                });
            }
            P4Invocation {
                args: vec![
                    "move".into(),
                    "-c".into(),
                    params.changelist.clone(),
                    sources[0].clone(),
                    targets[0].clone(),
                ],
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
        FileModifyAction::Resolve => {
            let flag = match params.mode.as_str() {
                "auto" => "-am",
                "safe" => "-as",
                "preview" => "-n",
                "yours" => "-ay",
                "force" => "-af",
                "theirs" => "-at",
                other => {
                    return Err(P4McpError::InvalidInput {
                        message: format!("invalid resolve mode: {other}"),
                    });
                }
            };
            let mut args = vec!["resolve".to_string(), flag.to_string()];
            if params.changelist != "default" {
                args.extend(["-c".to_string(), params.changelist.clone()]);
            }
            args.extend(files);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
    };
    Ok(invocation)
}

fn require_files(files: &[String], action: &str) -> Result<()> {
    if files.is_empty() {
        return Err(P4McpError::InvalidInput {
            message: format!("file_paths is required for {action}"),
        });
    }
    Ok(())
}

fn with_files(prefix: Vec<&str>, files: Vec<String>) -> P4Invocation {
    let mut args: Vec<String> = prefix.into_iter().map(str::to_string).collect();
    args.extend(files);
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}
