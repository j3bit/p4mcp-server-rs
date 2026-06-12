use std::{collections::BTreeMap, path::Path, path::PathBuf, process::Stdio};

use async_trait::async_trait;
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{
    error::{P4McpError, Result},
    p4::output::{parse_json_lines, text_response},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    JsonLines,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P4Invocation {
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub mode: OutputMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct P4CommandOutput {
    pub records: Vec<Value>,
    pub text: Value,
}

pub type P4Env = BTreeMap<String, String>;

#[async_trait]
pub trait P4Executor: Send + Sync {
    async fn run(&self, invocation: P4Invocation, env: P4Env) -> Result<P4CommandOutput>;
}

#[derive(Debug, Clone)]
pub struct TokioP4Executor {
    p4_bin: PathBuf,
}

impl TokioP4Executor {
    pub fn new(p4_bin: impl Into<PathBuf>) -> Self {
        Self {
            p4_bin: p4_bin.into(),
        }
    }

    pub async fn run(&self, invocation: P4Invocation, env: P4Env) -> Result<P4CommandOutput> {
        run_command(&self.p4_bin, invocation, env).await
    }
}

#[async_trait]
impl P4Executor for TokioP4Executor {
    async fn run(&self, invocation: P4Invocation, env: P4Env) -> Result<P4CommandOutput> {
        run_command(&self.p4_bin, invocation, env).await
    }
}

async fn run_command(
    p4_bin: &Path,
    invocation: P4Invocation,
    env: P4Env,
) -> Result<P4CommandOutput> {
    let mut command = Command::new(p4_bin);
    if invocation.mode == OutputMode::JsonLines {
        command.arg("-z").arg("tag").arg("-Mj");
    }
    command.args(&invocation.args);
    command.stdin(if invocation.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    command.envs(env);

    let mut child = command.spawn().map_err(|source| P4McpError::P4Command {
        message: source.to_string(),
    })?;

    let mut stdin_write_error = None;
    if let Some(stdin) = invocation.stdin {
        if let Some(mut child_stdin) = child.stdin.take() {
            if let Err(source) = child_stdin.write_all(stdin.as_bytes()).await {
                stdin_write_error = Some(source.to_string());
            }
        } else {
            stdin_write_error = Some("failed to open p4 stdin".to_string());
        }
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|source| P4McpError::P4Command {
            message: source.to_string(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if let Some(write_error) = stdin_write_error {
        return Err(P4McpError::P4Command {
            message: format_command_error(
                &format!("failed to write p4 stdin: {write_error}"),
                output.status,
                &stdout,
                &stderr,
            ),
        });
    }

    if !output.status.success() {
        return Err(P4McpError::P4Command {
            message: format_command_error(
                "p4 exited with failure",
                output.status,
                &stdout,
                &stderr,
            ),
        });
    }

    let records = match invocation.mode {
        OutputMode::JsonLines => parse_json_lines(&stdout)?,
        OutputMode::Text => Vec::new(),
    };

    Ok(P4CommandOutput {
        records,
        text: text_response(&stdout, &stderr),
    })
}

fn format_command_error(
    message: &str,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
) -> String {
    format!("{message}; status: {status}; stdout: {stdout}; stderr: {stderr}")
}
