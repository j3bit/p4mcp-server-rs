use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
    tools::params::{JobModifyAction, ModifyJobsParams},
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

pub fn build_job_modify_invocation(params: &ModifyJobsParams) -> Result<P4Invocation> {
    let change = required(Some(params.changelist_id.as_str()), "changelist_id")?;
    let job = required(Some(params.job_id.as_str()), "job_id")?;
    let args = match params.action {
        JobModifyAction::LinkJob => vec!["fix".into(), "-c".into(), change, job],
        JobModifyAction::UnlinkJob => vec!["fix".into(), "-d".into(), "-c".into(), change, job],
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
