use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_job_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    job_id: Option<&str>,
    _max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list_jobs" => vec![
            "fixes".into(),
            "-c".into(),
            required(changelist_id, "changelist_id")?,
        ],
        "get_job" => vec!["job".into(), "-o".into(), required(job_id, "job_id")?],
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
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(P4McpError::InvalidInput {
            message: format!("{name} is required"),
        }),
    }
}
