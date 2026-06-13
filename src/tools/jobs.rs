use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_job_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    job_id: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "list_jobs" => vec![
            "fixes".into(),
            "-c".into(),
            required(changelist_id, "changelist_id")?,
        ],
        "get_job" => vec!["job".into(), "-o".into(), required(job_id, "job_id")?],
        "list" => vec!["jobs".into(), "-m".into(), max_results.to_string()],
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
