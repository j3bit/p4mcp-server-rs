use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

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
    value
        .map(str::to_string)
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{name} is required"),
        })
}

fn unknown<T>(action: &str) -> Result<T> {
    Err(P4McpError::InvalidInput {
        message: format!("unknown action: {action}"),
    })
}
