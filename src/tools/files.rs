use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{FileQueryAction, QueryFilesParams},
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
                let file2 = params.file2.clone().ok_or_else(|| P4McpError::P4Command {
                    message: "file2 is required for diff2".to_string(),
                })?;
                P4Invocation {
                    args: vec!["diff2".into(), params.file_path.clone(), file2],
                    stdin: None,
                    mode: OutputMode::Text,
                }
            } else {
                if params.file2.is_some() {
                    return Err(P4McpError::P4Command {
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
                .ok_or_else(|| P4McpError::P4Command {
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
                .ok_or_else(|| P4McpError::P4Command {
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
