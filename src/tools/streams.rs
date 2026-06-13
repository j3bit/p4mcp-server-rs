use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_stream_query_invocation(
    action: &str,
    stream: Option<&str>,
    owner: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list" => {
            let mut args = vec!["streams".into(), "-m".into(), max_results.to_string()];
            if let Some(owner) = owner {
                args.extend(["-F".into(), format!("Owner={owner}")]);
            }
            args
        }
        "get" => vec!["stream".into(), "-o".into(), required(stream, "stream")?],
        "children" => vec![
            "streams".into(),
            "-F".into(),
            format!("Parent={}", required(stream, "stream")?),
        ],
        "parent" => vec!["stream".into(), "-o".into(), required(stream, "stream")?],
        "graph" => vec![
            "streams".into(),
            "-T".into(),
            "Stream,Parent,Type,Name,Owner".into(),
        ],
        "integration_status" => vec!["istat".into(), "-s".into(), required(stream, "stream")?],
        "get_workspace" => vec!["clients".into(), "-S".into(), required(stream, "stream")?],
        "list_workspaces" => vec!["clients".into(), "-S".into(), required(stream, "stream")?],
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

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value
        .map(str::to_string)
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{name} is required"),
        })
}
