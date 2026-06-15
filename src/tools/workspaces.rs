use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

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

fn required(value: Option<&str>, name: &str) -> Result<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}
