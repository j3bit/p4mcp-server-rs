use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_workspace_query_invocation(
    action: &str,
    workspace_name: Option<&str>,
    file_path: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list" => vec!["clients".into(), "-m".into(), max_results.to_string()],
        "get" => vec![
            "client".into(),
            "-o".into(),
            required(workspace_name, "workspace_name")?,
        ],
        "where" => vec!["where".into(), required(file_path, "file_path")?],
        "opened" => vec!["opened".into()],
        "changes" => vec!["changes".into(), "-m".into(), max_results.to_string()],
        other => {
            return Err(P4McpError::P4Command {
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

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value
        .map(str::to_string)
        .ok_or_else(|| P4McpError::P4Command {
            message: format!("{name} is required"),
        })
}
