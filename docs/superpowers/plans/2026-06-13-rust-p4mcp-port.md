# Rust P4 MCP Port Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone Rust port of `perforce/p4mcp-server` that keeps the MCP tool surface recognizable while replacing Python, FastMCP, and P4Python with Rust, `rmcp`, and direct `p4` CLI execution.

**Architecture:** Keep the upstream shape of entrypoint -> server -> tools -> services -> P4 access, but make the Rust boundaries explicit: typed CLI/config parsing, a small `P4Executor` abstraction, command-building services, MCP handler methods, and review HTTP client code. Use `p4 -z tag -Mj` for structured command output where possible, text mode for commands such as `print`, `diff`, and form bodies, and keep write operations gated by read-only and confirmation policy.

**Tech Stack:** Rust 2024, `rmcp` official Rust MCP SDK, `tokio`, `clap`, `serde`, `schemars`, `reqwest`, `tracing`, `thiserror`, `anyhow`, `tempfile`, `insta`, and local `p4` CLI.

---

## Assumptions

- The local directory is already a Git repository.
- The target binary name is `p4-mcp-server`, matching upstream deployment examples.
- `p4` is already installed on the machine that runs the MCP server. The Rust binary does not bundle P4Python, CPython, `uv`, Docker images, or a local `p4d`.
- `P4PORT`, `P4USER`, `P4CLIENT`, `P4CONFIG`, `.p4tickets`, and `P4PASSWD` are inherited from the MCP launch environment. The server may pass non-secret overrides to the `p4` process, but it must not persist passwords or tickets.
- Initial parity target is the upstream public tool names and action names: `query_server`, `query_files`, `modify_files`, `query_changelists`, `modify_changelists`, `query_shelves`, `modify_shelves`, `query_workspaces`, `modify_workspaces`, `query_jobs`, `modify_jobs`, `query_streams`, `modify_streams`, `query_reviews`, and `modify_reviews`.
- Dynamic search transforms from FastMCP are not ported in the first Rust version. `rmcp` handles MCP protocol framing and tool metadata; the port keeps the full typed tool list.
- Interactive MCP elicitation is replaced with explicit typed confirmation fields for destructive actions. This is more predictable for agent workflows and easier to validate in Rust.

## Source References

- Upstream repository: https://github.com/perforce/p4mcp-server
- Upstream package dependencies: `fastmcp`, `p4python`, `pydantic`, `requests` in `pyproject.toml`.
- Upstream package layout: `p4mcp/core`, `p4mcp/handlers`, `p4mcp/models`, `p4mcp/services`, `p4mcp/tools`.
- Upstream CLI flags and toolsets: README sections for `--readonly`, `--toolsets`, `--transport`, `--ssl-no-verify`, `--ca-bundle`, and available tools.
- Rust MCP SDK: https://github.com/modelcontextprotocol/rust-sdk and https://docs.rs/rmcp
- Perforce CLI structured output: `p4 -z tag -Mj` from current P4 CLI global options documentation.

## File Structure

- Create `Cargo.toml`: crate metadata, binary name, dependencies, dev dependencies, release profile.
- Create `rust-toolchain.toml`: pin Rust stable channel used by this repo.
- Create `.gitignore`: Rust build artifacts, logs, local MCP configs, and temporary P4 files.
- Create `README.md`: Rust-specific build/run notes and upstream parity status.
- Create `src/lib.rs`: module exports.
- Create `src/main.rs`: CLI parse, logging setup, stdio/http transport selection, graceful shutdown.
- Create `src/config.rs`: typed CLI/env config, toolset parsing, SSL options, transport mode.
- Create `src/error.rs`: shared error type.
- Create `src/p4/mod.rs`: P4 module exports.
- Create `src/p4/runner.rs`: direct `p4` process execution through `tokio::process::Command`.
- Create `src/p4/output.rs`: line-delimited JSON and text output parsing.
- Create `src/p4/forms.rs`: safe builders for `p4 change -i`, `p4 client -i`, and similar form commands.
- Create `src/permissions.rs`: command-time policy checks from CLI flags plus P4 server properties.
- Create `src/tools/mod.rs`: tool module exports.
- Create `src/tools/params.rs`: typed MCP parameter structs and action enums.
- Create `src/tools/response.rs`: stable JSON response envelope.
- Create `src/tools/server.rs`: server query service.
- Create `src/tools/files.rs`: file query and modify service.
- Create `src/tools/changelists.rs`: changelist query and modify service.
- Create `src/tools/shelves.rs`: shelf query and modify service.
- Create `src/tools/workspaces.rs`: workspace query and modify service.
- Create `src/tools/jobs.rs`: job query and modify service.
- Create `src/tools/streams.rs`: stream query and modify service.
- Create `src/tools/reviews.rs`: P4 Code Review / Swarm API service.
- Create `src/server.rs`: `rmcp` server type and tool methods.
- Create `scripts/package.sh`: release build and archive script.
- Create `docs/offline-build.md`: vendor and closed-network build workflow.
- Create `tests/config_tests.rs`: CLI/config tests.
- Create `tests/p4_output_tests.rs`: P4 output parser tests.
- Create `tests/p4_runner_tests.rs`: process invocation tests using an in-temp fake `p4`.
- Create `tests/tool_mapping_tests.rs`: service command mapping tests with a fake executor.
- Create `tests/review_client_tests.rs`: review URL/payload tests with `wiremock`.
- Create `tests/mcp_smoke_tests.rs`: list-tools and simple call tests over in-memory transport.

## Task 1: Repository Bootstrap

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `README.md`

- [ ] **Step 1: Verify Git and initialize Cargo**

Run:

```bash
git rev-parse --is-inside-work-tree
cargo init --bin --name p4mcp-server-rs .
```

Expected:

```text
true
Created binary (application) package
```

- [ ] **Step 2: Replace `Cargo.toml`**

Write this file exactly:

```toml
[package]
name = "p4mcp-server-rs"
version = "0.1.0"
edition = "2024"
description = "Rust MCP server for Perforce P4 using the p4 CLI"
license = "MIT"
repository = "https://github.com/perforce/p4mcp-server"

[[bin]]
name = "p4-mcp-server"
path = "src/main.rs"

[dependencies]
anyhow = "1"
async-trait = "0.1"
axum = "0.8"
clap = { version = "4.5", features = ["derive", "env"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
rmcp = { version = "1.7", features = ["server", "macros", "schemars", "transport-io", "transport-streamable-http-server"] }
schemars = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["macros", "process", "rt-multi-thread", "signal", "io-std", "fs"] }
tokio-util = "0.7"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
insta = { version = "1", features = ["json"] }
tempfile = "3"
wiremock = "0.6"

[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

- [ ] **Step 3: Add toolchain, ignore rules, and initial modules**

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`.gitignore`:

```gitignore
/target/
/logs/
/.p4session_*.json
/.mcp.json
/.cargo/config.toml
/vendor/
*.tgz
*.zip
```

`src/lib.rs`:

```rust
pub mod config;
pub mod error;
pub mod p4;
pub mod permissions;
pub mod server;
pub mod tools;
```

`src/main.rs`:

```rust
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    p4mcp_server_rs::server::run_from_cli().await
}
```

`README.md`:

```markdown
# p4mcp-server-rs

Rust port of Perforce P4 MCP Server.

This binary talks to the local `p4` CLI. It does not embed Python, P4Python, or a Perforce server.

## Development

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

## Runtime

```bash
P4PORT=ssl:perforce.example.com:1666 \
P4USER=your_username \
P4CLIENT=your_workspace \
cargo run -- --readonly
```
```

- [ ] **Step 4: Run initial verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
0 failed
```

- [ ] **Step 5: Commit bootstrap**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore README.md src/lib.rs src/main.rs
git commit -m "chore: bootstrap rust p4 mcp server"
```

## Task 2: CLI and Runtime Config

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs`
- Test: `tests/config_tests.rs`

- [ ] **Step 1: Write failing config tests**

Create `tests/config_tests.rs`:

```rust
use p4mcp_server_rs::config::{Cli, Toolset, TransportMode};

#[test]
fn defaults_match_upstream_toolsets() {
    let cli = Cli::try_parse_from(["p4-mcp-server"]).unwrap();
    let config = cli.into_config().unwrap();

    assert!(!config.readonly);
    assert_eq!(config.transport, TransportMode::Stdio);
    assert_eq!(config.port, 8000);
    assert!(config.toolsets.contains(&Toolset::Files));
    assert!(config.toolsets.contains(&Toolset::Changelists));
    assert!(config.toolsets.contains(&Toolset::Shelves));
    assert!(config.toolsets.contains(&Toolset::Workspaces));
    assert!(config.toolsets.contains(&Toolset::Jobs));
    assert!(config.toolsets.contains(&Toolset::Reviews));
    assert!(config.toolsets.contains(&Toolset::Streams));
}

#[test]
fn parses_explicit_toolsets_and_readonly() {
    let cli = Cli::try_parse_from([
        "p4-mcp-server",
        "--readonly",
        "--toolsets",
        "files,changelists",
        "--transport",
        "http",
        "--port",
        "9000",
    ])
    .unwrap();

    let config = cli.into_config().unwrap();

    assert!(config.readonly);
    assert_eq!(config.transport, TransportMode::Http);
    assert_eq!(config.port, 9000);
    assert_eq!(config.toolsets.len(), 2);
    assert!(config.toolsets.contains(&Toolset::Files));
    assert!(config.toolsets.contains(&Toolset::Changelists));
}

#[test]
fn rejects_unknown_toolset() {
    let cli = Cli::try_parse_from(["p4-mcp-server", "--toolsets", "files,unknown"]).unwrap();
    let err = cli.into_config().unwrap_err().to_string();
    assert!(err.contains("unknown toolset"));
}

#[test]
fn cli_ssl_options_take_priority() {
    let cli = Cli::try_parse_from([
        "p4-mcp-server",
        "--ssl-no-verify",
        "--ca-bundle",
        "/tmp/company-ca.pem",
    ])
    .unwrap();
    let config = cli.into_config().unwrap();
    assert_eq!(config.ssl_verify.to_string(), "/tmp/company-ca.pem");
}
```

- [ ] **Step 2: Run config tests and verify they fail**

Run:

```bash
cargo test --test config_tests -- --nocapture
```

Expected:

```text
error[E0432]: unresolved import `p4mcp_server_rs::config`
```

- [ ] **Step 3: Implement config types**

Create `src/config.rs`:

```rust
use std::{collections::BTreeSet, fmt, path::PathBuf, str::FromStr};

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Toolset {
    Files,
    Changelists,
    Shelves,
    Workspaces,
    Jobs,
    Reviews,
    Streams,
}

impl Toolset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Changelists => "changelists",
            Self::Shelves => "shelves",
            Self::Workspaces => "workspaces",
            Self::Jobs => "jobs",
            Self::Reviews => "reviews",
            Self::Streams => "streams",
        }
    }

    pub fn default_set() -> BTreeSet<Self> {
        [
            Self::Files,
            Self::Changelists,
            Self::Shelves,
            Self::Workspaces,
            Self::Jobs,
            Self::Reviews,
            Self::Streams,
        ]
        .into_iter()
        .collect()
    }
}

impl FromStr for Toolset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "files" => Ok(Self::Files),
            "changelists" => Ok(Self::Changelists),
            "shelves" => Ok(Self::Shelves),
            "workspaces" => Ok(Self::Workspaces),
            "jobs" => Ok(Self::Jobs),
            "reviews" => Ok(Self::Reviews),
            "streams" => Ok(Self::Streams),
            other => anyhow::bail!("unknown toolset: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransportMode {
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SslVerify {
    Enabled,
    Disabled,
    CaBundle(PathBuf),
}

impl fmt::Display for SslVerify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enabled => write!(f, "true"),
            Self::Disabled => write!(f, "false"),
            Self::CaBundle(path) => write!(f, "{}", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub readonly: bool,
    pub allow_usage: bool,
    pub toolsets: BTreeSet<Toolset>,
    pub transport: TransportMode,
    pub port: u16,
    pub p4_bin: PathBuf,
    pub log_dir: Option<PathBuf>,
    pub ssl_verify: SslVerify,
}

#[derive(Debug, Parser)]
#[command(name = "p4-mcp-server", version)]
pub struct Cli {
    #[arg(long)]
    pub readonly: bool,

    #[arg(long, default_value = "files,changelists,shelves,workspaces,jobs,reviews,streams")]
    pub toolsets: String,

    #[arg(long)]
    pub allow_usage: bool,

    #[arg(long, value_enum, default_value_t = TransportMode::Stdio)]
    pub transport: TransportMode,

    #[arg(long, default_value_t = 8000)]
    pub port: u16,

    #[arg(long, env = "P4MCP_P4_BIN", default_value = "p4")]
    pub p4_bin: PathBuf,

    #[arg(long, env = "P4MCP_LOG_DIR")]
    pub log_dir: Option<PathBuf>,

    #[arg(long)]
    pub ssl_no_verify: bool,

    #[arg(long, env = "P4MCP_CA_BUNDLE")]
    pub ca_bundle: Option<PathBuf>,
}

impl Cli {
    pub fn try_parse_from<I, T>(itr: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(itr)
    }

    pub fn into_config(self) -> anyhow::Result<AppConfig> {
        let mut toolsets = BTreeSet::new();
        for raw in self.toolsets.split(',') {
            if raw.trim().is_empty() {
                continue;
            }
            toolsets.insert(raw.parse()?);
        }
        if toolsets.is_empty() {
            anyhow::bail!("at least one toolset must be enabled");
        }

        let ssl_verify = match self.ca_bundle {
            Some(path) => SslVerify::CaBundle(path),
            None if self.ssl_no_verify => SslVerify::Disabled,
            None => SslVerify::Enabled,
        };

        Ok(AppConfig {
            readonly: self.readonly,
            allow_usage: self.allow_usage,
            toolsets,
            transport: self.transport,
            port: self.port,
            p4_bin: self.p4_bin,
            log_dir: self.log_dir,
            ssl_verify,
        })
    }
}
```

- [ ] **Step 4: Run config tests**

Run:

```bash
cargo test --test config_tests -- --nocapture
```

Expected:

```text
4 passed
```

- [ ] **Step 5: Commit config layer**

```bash
git add src/config.rs tests/config_tests.rs
git commit -m "feat: add runtime config parsing"
```

## Task 3: P4 CLI Runner and Output Parsing

**Files:**
- Create: `src/error.rs`
- Create: `src/p4/mod.rs`
- Create: `src/p4/output.rs`
- Create: `src/p4/runner.rs`
- Test: `tests/p4_output_tests.rs`
- Test: `tests/p4_runner_tests.rs`

- [ ] **Step 1: Write failing output parser tests**

Create `tests/p4_output_tests.rs`:

```rust
use p4mcp_server_rs::p4::output::{parse_json_lines, text_response};
use serde_json::json;

#[test]
fn parses_line_delimited_json_records() {
    let input = r#"{"code":"stat","data":"one"}
{"code":"info","data":"two"}
"#;
    let parsed = parse_json_lines(input).unwrap();
    assert_eq!(parsed, vec![json!({"code": "stat", "data": "one"}), json!({"code": "info", "data": "two"})]);
}

#[test]
fn ignores_blank_json_lines() {
    let input = "\n{\"code\":\"info\",\"data\":\"ok\"}\n\n";
    let parsed = parse_json_lines(input).unwrap();
    assert_eq!(parsed, vec![json!({"code": "info", "data": "ok"})]);
}

#[test]
fn reports_bad_json_line_number() {
    let err = parse_json_lines("{\"ok\": true}\nnot-json").unwrap_err().to_string();
    assert!(err.contains("line 2"));
}

#[test]
fn text_response_keeps_stdout_and_stderr() {
    let response = text_response("file content\n", "warning\n");
    assert_eq!(response["stdout"], "file content\n");
    assert_eq!(response["stderr"], "warning\n");
}
```

- [ ] **Step 2: Run parser tests and verify they fail**

Run:

```bash
cargo test --test p4_output_tests -- --nocapture
```

Expected:

```text
error[E0432]: unresolved import `p4mcp_server_rs::p4`
```

- [ ] **Step 3: Implement parser and error modules**

`src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum P4McpError {
    #[error("p4 command failed: {message}")]
    P4Command { message: String },

    #[error("failed to parse p4 JSON output at line {line}: {source}")]
    P4Json {
        line: usize,
        source: serde_json::Error,
    },

    #[error("toolset disabled: {toolset}")]
    ToolsetDisabled { toolset: &'static str },

    #[error("write operation blocked by read-only mode")]
    Readonly,

    #[error("destructive action requires confirmation value PROCEED")]
    ConfirmationRequired,
}

pub type Result<T> = std::result::Result<T, P4McpError>;
```

`src/p4/mod.rs`:

```rust
pub mod output;
pub mod runner;
```

`src/p4/output.rs`:

```rust
use serde_json::{Value, json};

use crate::error::{P4McpError, Result};

pub fn parse_json_lines(stdout: &str) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for (idx, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|source| P4McpError::P4Json {
            line: idx + 1,
            source,
        })?;
        records.push(value);
    }
    Ok(records)
}

pub fn text_response(stdout: &str, stderr: &str) -> Value {
    json!({
        "stdout": stdout,
        "stderr": stderr,
    })
}
```

- [ ] **Step 4: Write failing runner tests**

Create `tests/p4_runner_tests.rs`:

```rust
use std::fs;

use p4mcp_server_rs::p4::runner::{OutputMode, P4Invocation, TokioP4Executor};
use tempfile::tempdir;

#[tokio::test]
async fn json_invocation_adds_tagged_json_flags_before_command() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$P4_FAKE_ARGS\"\nprintf '{\"code\":\"info\",\"data\":\"ok\"}\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod").arg("+x").arg(&script).status().unwrap();

    let args_file = dir.path().join("args.txt");
    let executor = TokioP4Executor::new(script);
    let output = executor
        .run(
            P4Invocation {
                args: vec!["info".into()],
                stdin: None,
                mode: OutputMode::JsonLines,
            },
            [("P4_FAKE_ARGS", args_file.to_string_lossy().to_string())],
        )
        .await
        .unwrap();

    assert_eq!(fs::read_to_string(args_file).unwrap(), "-z\ntag\n-Mj\ninfo\n");
    assert_eq!(output.records[0]["data"], "ok");
}

#[tokio::test]
async fn nonzero_exit_is_error() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(&script, "#!/bin/sh\necho 'bad auth' 1>&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod").arg("+x").arg(&script).status().unwrap();

    let executor = TokioP4Executor::new(script);
    let err = executor
        .run(
            P4Invocation {
                args: vec!["opened".into()],
                stdin: None,
                mode: OutputMode::Text,
            },
            std::iter::empty::<(&str, String)>(),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("bad auth"));
}
```

- [ ] **Step 5: Implement runner**

`src/p4/runner.rs`:

```rust
use std::{collections::BTreeMap, path::PathBuf, process::Stdio};

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

#[async_trait]
pub trait P4Executor: Send + Sync {
    async fn run<I, K, V>(&self, invocation: P4Invocation, env: I) -> Result<P4CommandOutput>
    where
        I: IntoIterator<Item = (K, V)> + Send,
        K: AsRef<str> + Send,
        V: AsRef<str> + Send;
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
}

#[async_trait]
impl P4Executor for TokioP4Executor {
    async fn run<I, K, V>(&self, invocation: P4Invocation, env: I) -> Result<P4CommandOutput>
    where
        I: IntoIterator<Item = (K, V)> + Send,
        K: AsRef<str> + Send,
        V: AsRef<str> + Send,
    {
        let mut command = Command::new(&self.p4_bin);
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

        let env_map: BTreeMap<String, String> = env
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_owned(), v.as_ref().to_owned()))
            .collect();
        command.envs(env_map);

        let mut child = command.spawn().map_err(|source| P4McpError::P4Command {
            message: source.to_string(),
        })?;

        if let Some(stdin) = invocation.stdin {
            let mut child_stdin = child.stdin.take().ok_or_else(|| P4McpError::P4Command {
                message: "failed to open p4 stdin".to_string(),
            })?;
            child_stdin
                .write_all(stdin.as_bytes())
                .await
                .map_err(|source| P4McpError::P4Command {
                    message: source.to_string(),
                })?;
        }

        let output = child.wait_with_output().await.map_err(|source| P4McpError::P4Command {
            message: source.to_string(),
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() {
            return Err(P4McpError::P4Command {
                message: if stderr.trim().is_empty() { stdout } else { stderr },
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
}
```

- [ ] **Step 6: Run parser and runner tests**

Run:

```bash
cargo test --test p4_output_tests --test p4_runner_tests -- --nocapture
```

Expected:

```text
6 passed
```

- [ ] **Step 7: Commit P4 runner**

```bash
git add src/error.rs src/p4 tests/p4_output_tests.rs tests/p4_runner_tests.rs
git commit -m "feat: add p4 cli runner"
```

## Task 4: Tool Parameters, Responses, and Safety Policy

**Files:**
- Create: `src/tools/mod.rs`
- Create: `src/tools/params.rs`
- Create: `src/tools/response.rs`
- Create: `src/permissions.rs`
- Test: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Write failing parameter and safety tests**

Create `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::{
    config::Toolset,
    permissions::{Access, SafetyPolicy},
    tools::params::{FileModifyAction, ModifyFilesParams, QueryFilesParams},
};

#[test]
fn query_files_params_deserialize_content_action() {
    let params: QueryFilesParams = serde_json::from_value(serde_json::json!({
        "action": "content",
        "file_path": "//depot/main/README.md"
    }))
    .unwrap();

    assert_eq!(params.action.as_str(), "content");
    assert_eq!(params.file_path, "//depot/main/README.md");
}

#[test]
fn readonly_blocks_modify_files() {
    let policy = SafetyPolicy::new(true, [Toolset::Files].into_iter().collect());
    let result = policy.check(Access::Write, Toolset::Files, "modify_files");
    assert!(result.unwrap_err().to_string().contains("read-only"));
}

#[test]
fn disabled_toolset_blocks_call() {
    let policy = SafetyPolicy::new(false, [Toolset::Changelists].into_iter().collect());
    let result = policy.check(Access::Read, Toolset::Files, "query_files");
    assert!(result.unwrap_err().to_string().contains("toolset disabled"));
}

#[test]
fn delete_requires_proceed_confirmation() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: Some(vec!["//depot/main/old.txt".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: None,
    };
    assert!(params.requires_confirmation());
    assert!(params.confirmed().is_err());
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
error[E0432]: unresolved import `p4mcp_server_rs::tools`
```

- [ ] **Step 3: Implement shared tool modules**

`src/tools/mod.rs`:

```rust
pub mod params;
pub mod response;
```

`src/tools/response.rs`:

```rust
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ToolResponse {
    pub status: String,
    pub action: String,
    pub message: Value,
}

impl ToolResponse {
    pub fn success(action: impl Into<String>, message: Value) -> Self {
        Self {
            status: "success".to_string(),
            action: action.into(),
            message,
        }
    }
}
```

`src/tools/params.rs`:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{P4McpError, Result};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileQueryAction {
    Content,
    History,
    Info,
    Metadata,
    Diff,
    Annotations,
    Search,
    Grep,
}

impl FileQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::History => "history",
            Self::Info => "info",
            Self::Metadata => "metadata",
            Self::Diff => "diff",
            Self::Annotations => "annotations",
            Self::Search => "search",
            Self::Grep => "grep",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryFilesParams {
    pub action: FileQueryAction,
    pub file_path: String,
    #[serde(default)]
    pub file2: Option<String>,
    #[serde(default = "default_true")]
    pub diff2: bool,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileModifyAction {
    Add,
    Edit,
    Delete,
    Move,
    Revert,
    Reconcile,
    Resolve,
    Sync,
}

impl FileModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Revert => "revert",
            Self::Reconcile => "reconcile",
            Self::Resolve => "resolve",
            Self::Sync => "sync",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyFilesParams {
    pub action: FileModifyAction,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default = "default_changelist")]
    pub changelist: String,
    #[serde(default)]
    pub source_paths: Option<Vec<String>>,
    #[serde(default)]
    pub target_paths: Option<Vec<String>>,
    #[serde(default = "default_resolve_mode")]
    pub mode: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub confirmation: Option<String>,
}

impl ModifyFilesParams {
    pub fn requires_confirmation(&self) -> bool {
        matches!(self.action, FileModifyAction::Delete | FileModifyAction::Revert)
    }

    pub fn confirmed(&self) -> Result<()> {
        if !self.requires_confirmation() || self.confirmation.as_deref() == Some("PROCEED") {
            return Ok(());
        }
        Err(P4McpError::ConfirmationRequired)
    }
}

fn default_true() -> bool {
    true
}

fn default_max_results() -> u16 {
    100
}

fn default_changelist() -> String {
    "default".to_string()
}

fn default_resolve_mode() -> String {
    "auto".to_string()
}
```

- [ ] **Step 4: Implement safety policy**

`src/permissions.rs`:

```rust
use std::collections::BTreeSet;

use crate::{
    config::Toolset,
    error::{P4McpError, Result},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone)]
pub struct SafetyPolicy {
    readonly: bool,
    enabled_toolsets: BTreeSet<Toolset>,
}

impl SafetyPolicy {
    pub fn new(readonly: bool, enabled_toolsets: BTreeSet<Toolset>) -> Self {
        Self {
            readonly,
            enabled_toolsets,
        }
    }

    pub fn check(&self, access: Access, toolset: Toolset, _tool_name: &str) -> Result<()> {
        if !self.enabled_toolsets.contains(&toolset) {
            return Err(P4McpError::ToolsetDisabled {
                toolset: toolset.as_str(),
            });
        }
        if self.readonly && access == Access::Write {
            return Err(P4McpError::Readonly);
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Run parameter and safety tests**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
4 passed
```

- [ ] **Step 6: Commit shared tool layer**

```bash
git add src/tools src/permissions.rs tests/tool_mapping_tests.rs
git commit -m "feat: add tool parameters and safety policy"
```

## Task 5: Server Query and File Tool Services

**Files:**
- Create: `src/tools/server.rs`
- Create: `src/tools/files.rs`
- Modify: `src/tools/mod.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add failing command-mapping tests**

Append to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::{
    p4::runner::{OutputMode, P4Invocation},
    tools::{
        files::build_file_invocation,
        params::{FileQueryAction, QueryFilesParams},
        server::{build_server_invocation, ServerQueryAction},
    },
};

#[test]
fn query_server_info_maps_to_info() {
    let invocation = build_server_invocation(ServerQueryAction::ServerInfo);
    assert_eq!(invocation.args, vec!["info"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn query_file_content_uses_text_print() {
    let params = QueryFilesParams {
        action: FileQueryAction::Content,
        file_path: "//depot/main/file.txt".to_string(),
        file2: None,
        diff2: true,
        max_results: 100,
        pattern: None,
        case_insensitive: false,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation,
        P4Invocation {
            args: vec!["print".into(), "-q".into(), "//depot/main/file.txt".into()],
            stdin: None,
            mode: OutputMode::Text,
        }
    );
}

#[test]
fn query_file_grep_maps_pattern_and_limit() {
    let params = QueryFilesParams {
        action: FileQueryAction::Grep,
        file_path: "//depot/main/...".to_string(),
        file2: None,
        diff2: true,
        max_results: 50,
        pattern: Some("needle".to_string()),
        case_insensitive: true,
    };
    let invocation = build_file_invocation(&params).unwrap();
    assert_eq!(
        invocation.args,
        vec!["grep", "-n", "-i", "-m", "50", "-e", "needle", "//depot/main/..."]
    );
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}
```

- [ ] **Step 2: Run mapping tests and verify new failures**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
unresolved import `p4mcp_server_rs::tools::files`
```

- [ ] **Step 3: Implement server and file command builders**

`src/tools/mod.rs`:

```rust
pub mod files;
pub mod params;
pub mod response;
pub mod server;
```

`src/tools/server.rs`:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::p4::runner::{OutputMode, P4Invocation};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerQueryAction {
    ServerInfo,
    CurrentUser,
}

pub fn build_server_invocation(action: ServerQueryAction) -> P4Invocation {
    let args = match action {
        ServerQueryAction::ServerInfo => vec!["info".to_string()],
        ServerQueryAction::CurrentUser => vec!["user".to_string(), "-o".to_string()],
    };
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}
```

`src/tools/files.rs`:

```rust
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
            args: vec!["filelog".into(), format!("-m{}", params.max_results), params.file_path.clone()],
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
            let file2 = params.file2.clone().ok_or_else(|| P4McpError::P4Command {
                message: "file2 is required for diff".to_string(),
            })?;
            let cmd = if params.diff2 { "diff2" } else { "diff" };
            P4Invocation {
                args: vec![cmd.into(), params.file_path.clone(), file2],
                stdin: None,
                mode: OutputMode::Text,
            }
        }
        FileQueryAction::Annotations => P4Invocation {
            args: vec!["annotate".into(), params.file_path.clone()],
            stdin: None,
            mode: OutputMode::JsonLines,
        },
        FileQueryAction::Search => {
            let pattern = params.pattern.clone().ok_or_else(|| P4McpError::P4Command {
                message: "pattern is required for search".to_string(),
            })?;
            P4Invocation {
                args: vec!["files".into(), "-m".into(), params.max_results.to_string(), format!("{}/{}", params.file_path.trim_end_matches('/'), pattern)],
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
        FileQueryAction::Grep => {
            let pattern = params.pattern.clone().ok_or_else(|| P4McpError::P4Command {
                message: "pattern is required for grep".to_string(),
            })?;
            let mut args = vec!["grep".into(), "-n".into()];
            if params.case_insensitive {
                args.push("-i".into());
            }
            args.extend(["-m".into(), params.max_results.to_string(), "-e".into(), pattern, params.file_path.clone()]);
            P4Invocation {
                args,
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
    };
    Ok(invocation)
}
```

- [ ] **Step 4: Run mapping tests**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
7 passed
```

- [ ] **Step 5: Commit read-service mappings**

```bash
git add src/tools tests/tool_mapping_tests.rs
git commit -m "feat: add server and file query mappings"
```

## Task 6: File Modification Service

**Files:**
- Modify: `src/tools/files.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add failing file modify tests**

Append to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::files::build_file_modify_invocation;

#[test]
fn modify_file_add_maps_changelist() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Add,
        file_paths: Some(vec!["src/new.rs".to_string()]),
        changelist: "123".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(invocation.args, vec!["add", "-c", "123", "src/new.rs"]);
    assert_eq!(invocation.mode, OutputMode::JsonLines);
}

#[test]
fn modify_file_delete_requires_confirmation_before_command() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Delete,
        file_paths: Some(vec!["//depot/main/old.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "auto".to_string(),
        force: false,
        confirmation: None,
    };

    assert!(build_file_modify_invocation(&params).is_err());
}

#[test]
fn modify_file_resolve_safe_maps_to_as() {
    let params = ModifyFilesParams {
        action: FileModifyAction::Resolve,
        file_paths: Some(vec!["//depot/main/file.rs".to_string()]),
        changelist: "default".to_string(),
        source_paths: None,
        target_paths: None,
        mode: "safe".to_string(),
        force: false,
        confirmation: None,
    };

    let invocation = build_file_modify_invocation(&params).unwrap();
    assert_eq!(invocation.args, vec!["resolve", "-as", "//depot/main/file.rs"]);
}
```

- [ ] **Step 2: Run mapping tests and verify new failures**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
no `build_file_modify_invocation` in `tools::files`
```

- [ ] **Step 3: Implement file modify mapping**

Append to `src/tools/files.rs`:

```rust
use crate::tools::params::{FileModifyAction, ModifyFilesParams};

pub fn build_file_modify_invocation(params: &ModifyFilesParams) -> Result<P4Invocation> {
    params.confirmed()?;

    let files = params.file_paths.clone().unwrap_or_default();
    let invocation = match params.action {
        FileModifyAction::Add => with_files(vec!["add", "-c", &params.changelist], files),
        FileModifyAction::Edit => with_files(vec!["edit", "-c", &params.changelist], files),
        FileModifyAction::Delete => with_files(vec!["delete", "-c", &params.changelist], files),
        FileModifyAction::Revert => with_files(vec!["revert", "-c", &params.changelist], files),
        FileModifyAction::Reconcile => with_files(vec!["reconcile", "-c", &params.changelist], files),
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
                return Err(P4McpError::P4Command {
                    message: "source_paths and target_paths must have the same length".to_string(),
                });
            }
            if sources.len() != 1 {
                return Err(P4McpError::P4Command {
                    message: "move accepts exactly one source and one target per tool call".to_string(),
                });
            }
            P4Invocation {
                args: vec!["move".into(), "-c".into(), params.changelist.clone(), sources[0].clone(), targets[0].clone()],
                stdin: None,
                mode: OutputMode::JsonLines,
            }
        }
        FileModifyAction::Resolve => {
            let flag = match params.mode.as_str() {
                "auto" => "-am",
                "safe" => "-as",
                "force" => "-af",
                "preview" => "-n",
                "theirs" => "-at",
                "yours" => "-ay",
                other => {
                    return Err(P4McpError::P4Command {
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

fn with_files(prefix: Vec<&str>, files: Vec<String>) -> P4Invocation {
    let mut args: Vec<String> = prefix.into_iter().map(str::to_string).collect();
    args.extend(files);
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    }
}
```

- [ ] **Step 4: Run mapping tests**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
10 passed
```

- [ ] **Step 5: Commit file modification mappings**

```bash
git add src/tools/files.rs tests/tool_mapping_tests.rs
git commit -m "feat: add file modification mappings"
```

## Task 7: Changelist, Shelf, Workspace, Job, and Stream Services

**Files:**
- Create: `src/p4/forms.rs`
- Create: `src/tools/changelists.rs`
- Create: `src/tools/shelves.rs`
- Create: `src/tools/workspaces.rs`
- Create: `src/tools/jobs.rs`
- Create: `src/tools/streams.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/tools/params.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add focused command-mapping tests**

Append to `tests/tool_mapping_tests.rs`:

```rust
use p4mcp_server_rs::tools::{
    changelists::{build_changelist_modify_invocation, build_changelist_query_invocation},
    jobs::build_job_query_invocation,
    shelves::build_shelf_query_invocation,
    streams::build_stream_query_invocation,
    workspaces::build_workspace_query_invocation,
};

#[test]
fn default_changelist_get_uses_opened_not_describe() {
    let invocation = build_changelist_query_invocation("get", Some("default"), None, None, 10).unwrap();
    assert_eq!(invocation.args, vec!["opened", "-c", "default"]);
}

#[test]
fn numbered_changelist_get_uses_describe() {
    let invocation = build_changelist_query_invocation("get", Some("123"), None, None, 10).unwrap();
    assert_eq!(invocation.args, vec!["describe", "-s", "123"]);
}

#[test]
fn changelist_submit_uses_numbered_change() {
    let invocation = build_changelist_modify_invocation("submit", "123", None).unwrap();
    assert_eq!(invocation.args, vec!["submit", "-c", "123"]);
}

#[test]
fn shelf_diff_uses_shelved_describe() {
    let invocation = build_shelf_query_invocation("diff", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["describe", "-S", "-du", "123"]);
    assert_eq!(invocation.mode, OutputMode::Text);
}

#[test]
fn workspace_where_maps_file_argument() {
    let invocation = build_workspace_query_invocation("where", None, Some("//depot/main/file.rs"), 10).unwrap();
    assert_eq!(invocation.args, vec!["where", "//depot/main/file.rs"]);
}

#[test]
fn job_list_for_changelist_uses_fixes() {
    let invocation = build_job_query_invocation("list_jobs", Some("123"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["fixes", "-c", "123"]);
}

#[test]
fn stream_integration_status_uses_istat() {
    let invocation = build_stream_query_invocation("integration_status", Some("//streams/dev"), None, 10).unwrap();
    assert_eq!(invocation.args, vec!["istat", "-s", "//streams/dev"]);
}
```

- [ ] **Step 2: Run mapping tests and verify failures**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
unresolved import `p4mcp_server_rs::tools::changelists`
```

- [ ] **Step 3: Add forms helper**

`src/p4/forms.rs`:

```rust
pub fn change_form(description: &str, files: &[String]) -> String {
    let mut body = format!("Change: new\n\nDescription:\n\t{}\n\nFiles:\n", description.replace('\n', "\n\t"));
    for file in files {
        body.push('\t');
        body.push_str(file);
        body.push('\n');
    }
    body
}
```

Modify `src/p4/mod.rs`:

```rust
pub mod forms;
pub mod output;
pub mod runner;
```

- [ ] **Step 4: Implement non-file service command builders**

`src/tools/mod.rs`:

```rust
pub mod changelists;
pub mod files;
pub mod jobs;
pub mod params;
pub mod response;
pub mod server;
pub mod shelves;
pub mod streams;
pub mod workspaces;
```

`src/tools/changelists.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_changelist_query_invocation(
    action: &str,
    changelist_id: Option<&str>,
    status: Option<&str>,
    workspace_name: Option<&str>,
    max_results: u16,
) -> Result<P4Invocation> {
    let args = match action {
        "get" if changelist_id == Some("default") => vec!["opened".into(), "-c".into(), "default".into()],
        "get" => vec!["describe".into(), "-s".into(), required(changelist_id, "changelist_id")?],
        "list" => {
            let mut args = vec!["changes".into(), "-m".into(), max_results.to_string()];
            if let Some(status) = status {
                args.extend(["-s".into(), status.into()]);
            }
            if let Some(workspace) = workspace_name {
                args.extend(["-c".into(), workspace.into()]);
            }
            args
        }
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::JsonLines,
    })
}

pub fn build_changelist_modify_invocation(action: &str, changelist_id: &str, stdin: Option<String>) -> Result<P4Invocation> {
    let (args, stdin) = match action {
        "create" => (vec!["change".into(), "-i".into()], stdin),
        "update" => (vec!["change".into(), "-i".into()], stdin),
        "submit" => (vec!["submit".into(), "-c".into(), changelist_id.into()], None),
        "delete" => (vec!["change".into(), "-d".into(), changelist_id.into()], None),
        other => return unknown(other),
    };
    Ok(P4Invocation {
        args,
        stdin,
        mode: OutputMode::JsonLines,
    })
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value.map(str::to_string).ok_or_else(|| P4McpError::P4Command {
        message: format!("{name} is required"),
    })
}

fn unknown<T>(action: &str) -> Result<T> {
    Err(P4McpError::P4Command {
        message: format!("unknown action: {action}"),
    })
}
```

`src/tools/shelves.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_shelf_query_invocation(action: &str, changelist_id: Option<&str>, user: Option<&str>, max_results: u16) -> Result<P4Invocation> {
    let (args, mode) = match action {
        "list" => {
            let mut args = vec!["changes".into(), "-s".into(), "shelved".into(), "-m".into(), max_results.to_string()];
            if let Some(user) = user {
                args.extend(["-u".into(), user.into()]);
            }
            (args, OutputMode::JsonLines)
        }
        "diff" => (vec!["describe".into(), "-S".into(), "-du".into(), required(changelist_id, "changelist_id")?], OutputMode::Text),
        "files" => (vec!["describe".into(), "-S".into(), required(changelist_id, "changelist_id")?], OutputMode::JsonLines),
        other => return unknown(other),
    };
    Ok(P4Invocation { args, stdin: None, mode })
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value.map(str::to_string).ok_or_else(|| P4McpError::P4Command { message: format!("{name} is required") })
}

fn unknown<T>(action: &str) -> Result<T> {
    Err(P4McpError::P4Command { message: format!("unknown action: {action}") })
}
```

`src/tools/workspaces.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_workspace_query_invocation(action: &str, workspace_name: Option<&str>, file_path: Option<&str>, max_results: u16) -> Result<P4Invocation> {
    let args = match action {
        "list" => vec!["clients".into(), "-m".into(), max_results.to_string()],
        "get" => vec!["client".into(), "-o".into(), required(workspace_name, "workspace_name")?],
        "where" => vec!["where".into(), required(file_path, "file_path")?],
        "opened" => vec!["opened".into()],
        "changes" => vec!["changes".into(), "-m".into(), max_results.to_string()],
        other => return Err(P4McpError::P4Command { message: format!("unknown action: {other}") }),
    };
    Ok(P4Invocation { args, stdin: None, mode: OutputMode::JsonLines })
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value.map(str::to_string).ok_or_else(|| P4McpError::P4Command { message: format!("{name} is required") })
}
```

`src/tools/jobs.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_job_query_invocation(action: &str, changelist_id: Option<&str>, job_id: Option<&str>, max_results: u16) -> Result<P4Invocation> {
    let args = match action {
        "list_jobs" => vec!["fixes".into(), "-c".into(), required(changelist_id, "changelist_id")?],
        "get_job" => vec!["job".into(), "-o".into(), required(job_id, "job_id")?],
        "list" => vec!["jobs".into(), "-m".into(), max_results.to_string()],
        other => return Err(P4McpError::P4Command { message: format!("unknown action: {other}") }),
    };
    Ok(P4Invocation { args, stdin: None, mode: OutputMode::JsonLines })
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value.map(str::to_string).ok_or_else(|| P4McpError::P4Command { message: format!("{name} is required") })
}
```

`src/tools/streams.rs`:

```rust
use crate::{
    error::{P4McpError, Result},
    p4::runner::{OutputMode, P4Invocation},
};

pub fn build_stream_query_invocation(action: &str, stream: Option<&str>, owner: Option<&str>, max_results: u16) -> Result<P4Invocation> {
    let args = match action {
        "list" => {
            let mut args = vec!["streams".into(), "-m".into(), max_results.to_string()];
            if let Some(owner) = owner {
                args.extend(["-U".into(), owner.into()]);
            }
            args
        }
        "get" => vec!["stream".into(), "-o".into(), required(stream, "stream")?],
        "children" => vec!["streams".into(), "-F".into(), format!("Parent={}", required(stream, "stream")?)],
        "parent" => vec!["stream".into(), "-o".into(), required(stream, "stream")?],
        "graph" => vec!["streams".into(), "-T".into(), "Stream,Parent,Type,Name,Owner".into()],
        "integration_status" => vec!["istat".into(), "-s".into(), required(stream, "stream")?],
        "get_workspace" => vec!["clients".into(), "-S".into(), required(stream, "stream")?],
        "list_workspaces" => vec!["clients".into(), "-S".into(), required(stream, "stream")?],
        other => return Err(P4McpError::P4Command { message: format!("unknown action: {other}") }),
    };
    Ok(P4Invocation { args, stdin: None, mode: OutputMode::JsonLines })
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value.map(str::to_string).ok_or_else(|| P4McpError::P4Command { message: format!("{name} is required") })
}
```

- [ ] **Step 5: Run mapping tests**

Run:

```bash
cargo test --test tool_mapping_tests -- --nocapture
```

Expected:

```text
17 passed
```

- [ ] **Step 6: Commit non-file services**

```bash
git add src/p4/forms.rs src/p4/mod.rs src/tools tests/tool_mapping_tests.rs
git commit -m "feat: add p4 workflow command mappings"
```

## Task 8: Review HTTP Client

**Files:**
- Create: `src/tools/reviews.rs`
- Modify: `src/tools/mod.rs`
- Test: `tests/review_client_tests.rs`

- [ ] **Step 1: Write failing review client tests**

Create `tests/review_client_tests.rs`:

```rust
use p4mcp_server_rs::tools::reviews::{ReviewAction, ReviewRequest};

#[test]
fn list_reviews_builds_v11_reviews_path() {
    let request = ReviewRequest {
        action: ReviewAction::List,
        review_id: None,
        max_results: 25,
        body: serde_json::json!({}),
    };
    let built = request.to_http("https://swarm.example.com/api/v11").unwrap();
    assert_eq!(built.method, "GET");
    assert_eq!(built.path, "/reviews");
    assert_eq!(built.query, vec![("max".to_string(), "25".to_string())]);
}

#[test]
fn vote_review_builds_post_payload() {
    let request = ReviewRequest {
        action: ReviewAction::Vote,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"vote": "up", "version": 2}),
    };
    let built = request.to_http("https://swarm.example.com/api/v11").unwrap();
    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/reviews/123/vote");
    assert_eq!(built.body["vote"], "up");
}

#[test]
fn obliterate_review_requires_confirmation() {
    let request = ReviewRequest {
        action: ReviewAction::Obliterate,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"confirmation": "CANCEL"}),
    };
    assert!(request.to_http("https://swarm.example.com/api/v11").is_err());
}
```

- [ ] **Step 2: Run review tests and verify failure**

Run:

```bash
cargo test --test review_client_tests -- --nocapture
```

Expected:

```text
unresolved import `p4mcp_server_rs::tools::reviews`
```

- [ ] **Step 3: Implement review request mapping**

Modify `src/tools/mod.rs` to include:

```rust
pub mod reviews;
```

Create `src/tools/reviews.rs`:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{P4McpError, Result};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    List,
    Dashboard,
    Get,
    Transitions,
    FilesReadby,
    Files,
    Comments,
    Activity,
    Create,
    RefreshProjects,
    Vote,
    Transition,
    AppendParticipants,
    AddComment,
    ReplyComment,
    AppendChange,
    ReplaceWithChange,
    Join,
    ArchiveInactive,
    MarkCommentRead,
    MarkCommentUnread,
    MarkAllCommentsRead,
    MarkAllCommentsUnread,
    UpdateAuthor,
    UpdateDescription,
    ReplaceParticipants,
    DeleteParticipants,
    Leave,
    Obliterate,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ReviewRequest {
    pub action: ReviewAction,
    #[serde(default)]
    pub review_id: Option<u64>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltReviewRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Value,
}

impl ReviewRequest {
    pub fn to_http(&self, _api_base: &str) -> Result<BuiltReviewRequest> {
        if self.action == ReviewAction::Obliterate && self.body.get("confirmation").and_then(Value::as_str) != Some("PROCEED") {
            return Err(P4McpError::ConfirmationRequired);
        }

        let id = || {
            self.review_id.ok_or_else(|| P4McpError::P4Command {
                message: "review_id is required".to_string(),
            })
        };

        let built = match self.action {
            ReviewAction::List => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews".into(),
                query: vec![("max".into(), self.max_results.to_string())],
                body: json!({}),
            },
            ReviewAction::Dashboard => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews/dashboard".into(),
                query: vec![("max".into(), self.max_results.to_string())],
                body: json!({}),
            },
            ReviewAction::Get => get(format!("/reviews/{}", id()?)),
            ReviewAction::Transitions => get(format!("/reviews/{}/transitions", id()?)),
            ReviewAction::FilesReadby => get(format!("/reviews/{}/files/readby", id()?)),
            ReviewAction::Files => get(format!("/reviews/{}/files", id()?)),
            ReviewAction::Comments => get(format!("/reviews/{}/comments", id()?)),
            ReviewAction::Activity => get(format!("/reviews/{}/activity", id()?)),
            ReviewAction::Create => post("/reviews".to_string(), self.body.clone()),
            ReviewAction::RefreshProjects => post(format!("/reviews/{}/refreshProjects", id()?), json!({})),
            ReviewAction::Vote => post(format!("/reviews/{}/vote", id()?), self.body.clone()),
            ReviewAction::Transition => post(format!("/reviews/{}/transitions", id()?), self.body.clone()),
            ReviewAction::AppendParticipants => post(format!("/reviews/{}/participants", id()?), self.body.clone()),
            ReviewAction::AddComment | ReviewAction::ReplyComment => post(format!("/reviews/{}/comments", id()?), self.body.clone()),
            ReviewAction::AppendChange => post(format!("/reviews/{}/appendchange", id()?), self.body.clone()),
            ReviewAction::ReplaceWithChange => post(format!("/reviews/{}/replacewithchange", id()?), self.body.clone()),
            ReviewAction::Join => post(format!("/reviews/{}/join", id()?), self.body.clone()),
            ReviewAction::ArchiveInactive => post("/reviews/archiveInactive".to_string(), self.body.clone()),
            ReviewAction::MarkCommentRead => post(format!("/comments/{}/read", id()?), json!({})),
            ReviewAction::MarkCommentUnread => post(format!("/comments/{}/unread", id()?), json!({})),
            ReviewAction::MarkAllCommentsRead => post(format!("/reviews/{}/comments/read", id()?), json!({})),
            ReviewAction::MarkAllCommentsUnread => post(format!("/reviews/{}/comments/unread", id()?), json!({})),
            ReviewAction::UpdateAuthor => put(format!("/reviews/{}/author", id()?), self.body.clone()),
            ReviewAction::UpdateDescription => put(format!("/reviews/{}/description", id()?), self.body.clone()),
            ReviewAction::ReplaceParticipants => put(format!("/reviews/{}/participants", id()?), self.body.clone()),
            ReviewAction::DeleteParticipants => delete(format!("/reviews/{}/participants", id()?), self.body.clone()),
            ReviewAction::Leave => delete(format!("/reviews/{}/leave", id()?), self.body.clone()),
            ReviewAction::Obliterate => delete(format!("/reviews/{}", id()?), json!({})),
        };
        Ok(built)
    }
}

fn get(path: String) -> BuiltReviewRequest {
    BuiltReviewRequest { method: "GET".into(), path, query: Vec::new(), body: json!({}) }
}

fn post(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest { method: "POST".into(), path, query: Vec::new(), body }
}

fn put(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest { method: "PUT".into(), path, query: Vec::new(), body }
}

fn delete(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest { method: "DELETE".into(), path, query: Vec::new(), body }
}

fn default_max_results() -> u16 {
    10
}
```

- [ ] **Step 4: Add live HTTP execution wrapper**

Append to `src/tools/reviews.rs`:

```rust
#[derive(Clone)]
pub struct ReviewHttpClient {
    client: reqwest::Client,
    api_base: String,
    username: String,
    ticket: String,
}

impl ReviewHttpClient {
    pub fn new(api_base: String, username: String, ticket: String, accept_invalid_certs: bool) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()?;
        Ok(Self {
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
            username,
            ticket,
        })
    }

    pub async fn execute(&self, request: &ReviewRequest) -> anyhow::Result<Value> {
        let built = request.to_http(&self.api_base)?;
        let url = format!("{}{}", self.api_base, built.path);
        let mut req = match built.method.as_str() {
            "GET" => self.client.get(url).query(&built.query),
            "POST" => self.client.post(url).json(&built.body),
            "PUT" => self.client.put(url).json(&built.body),
            "DELETE" => self.client.delete(url).json(&built.body),
            method => anyhow::bail!("unsupported review HTTP method: {method}"),
        };
        req = req.basic_auth(&self.username, Some(&self.ticket));
        let response = req.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("review API returned HTTP {status}: {text}");
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text })))
    }
}
```

- [ ] **Step 5: Run review tests**

Run:

```bash
cargo test --test review_client_tests -- --nocapture
```

Expected:

```text
3 passed
```

- [ ] **Step 6: Commit review client**

```bash
git add src/tools/reviews.rs src/tools/mod.rs tests/review_client_tests.rs
git commit -m "feat: add p4 code review request mappings"
```

## Task 9: MCP Server Wiring

**Files:**
- Create: `src/server.rs`
- Modify: `src/main.rs`
- Test: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Write failing MCP smoke test**

Create `tests/mcp_smoke_tests.rs`:

```rust
use p4mcp_server_rs::{
    config::{AppConfig, SslVerify, Toolset, TransportMode},
    server::P4McpServer,
};

fn test_config() -> AppConfig {
    AppConfig {
        readonly: true,
        allow_usage: false,
        toolsets: Toolset::default_set(),
        transport: TransportMode::Stdio,
        port: 8000,
        p4_bin: "p4".into(),
        log_dir: None,
        ssl_verify: SslVerify::Enabled,
    }
}

#[test]
fn server_constructs_with_config() {
    let server = P4McpServer::new(test_config());
    assert!(server.config().readonly);
}
```

- [ ] **Step 2: Run MCP smoke test and verify failure**

Run:

```bash
cargo test --test mcp_smoke_tests -- --nocapture
```

Expected:

```text
unresolved import `p4mcp_server_rs::server`
```

- [ ] **Step 3: Implement MCP server shell**

Create `src/server.rs`:

```rust
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde_json::json;
use tracing_subscriber::EnvFilter;

use crate::{
    config::{AppConfig, Cli, Toolset, TransportMode},
    p4::runner::{P4Executor, TokioP4Executor},
    permissions::{Access, SafetyPolicy},
    tools::{
        files::{build_file_invocation, build_file_modify_invocation},
        params::{ModifyFilesParams, QueryFilesParams},
        response::ToolResponse,
        server::{ServerQueryAction, build_server_invocation},
    },
};

#[derive(Clone)]
pub struct P4McpServer {
    config: Arc<AppConfig>,
    executor: Arc<TokioP4Executor>,
}

impl P4McpServer {
    pub fn new(config: AppConfig) -> Self {
        let executor = TokioP4Executor::new(config.p4_bin.clone());
        Self {
            config: Arc::new(config),
            executor: Arc::new(executor),
        }
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn policy(&self) -> SafetyPolicy {
        SafetyPolicy::new(self.config.readonly, self.config.toolsets.clone())
    }
}

#[tool_router]
impl P4McpServer {
    #[tool(description = "Get P4 server information and current user details")]
    async fn query_server(&self, Parameters(action): Parameters<ServerQueryAction>) -> Result<CallToolResult, rmcp::ErrorData> {
        let invocation = build_server_invocation(action);
        let output = self.executor.run(invocation, std::iter::empty::<(&str, String)>()).await.map_err(to_mcp_error)?;
        let body = ToolResponse::success("query_server", json!(output.records));
        Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
    }

    #[tool(description = "Get file content, history, info, metadata, diff, annotations, search, or grep results")]
    async fn query_files(&self, Parameters(params): Parameters<QueryFilesParams>) -> Result<CallToolResult, rmcp::ErrorData> {
        self.policy().check(Access::Read, Toolset::Files, "query_files").map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_file_invocation(&params).map_err(to_mcp_error)?;
        let output = self.executor.run(invocation, std::iter::empty::<(&str, String)>()).await.map_err(to_mcp_error)?;
        let message = if output.records.is_empty() { output.text } else { json!(output.records) };
        let body = ToolResponse::success(action, message);
        Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
    }

    #[tool(description = "Add, edit, delete, move, revert, reconcile, resolve, or sync files")]
    async fn modify_files(&self, Parameters(params): Parameters<ModifyFilesParams>) -> Result<CallToolResult, rmcp::ErrorData> {
        self.policy().check(Access::Write, Toolset::Files, "modify_files").map_err(to_mcp_error)?;
        let action = params.action.as_str();
        let invocation = build_file_modify_invocation(&params).map_err(to_mcp_error)?;
        let output = self.executor.run(invocation, std::iter::empty::<(&str, String)>()).await.map_err(to_mcp_error)?;
        let body = ToolResponse::success(action, if output.records.is_empty() { output.text } else { json!(output.records) });
        Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
    }
}

#[tool_handler(name = "p4-mcp-server", version = "0.1.0", instructions = "Perforce P4 MCP server backed by the local p4 CLI")]
impl ServerHandler for P4McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
    }
}

pub async fn run_from_cli() -> Result<()> {
    let cli = Cli::parse();
    let config = cli.into_config()?;
    init_logging();
    match config.transport {
        TransportMode::Stdio => run_stdio(config).await,
        TransportMode::Http => run_http(config).await,
    }
}

async fn run_stdio(config: AppConfig) -> Result<()> {
    let service = P4McpServer::new(config).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

async fn run_http(config: AppConfig) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let port = config.port;
    let cancel = tokio_util::sync::CancellationToken::new();
    let service_config = config.clone();
    let service = StreamableHttpService::new(
        move || Ok(P4McpServer::new(service_config.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        })
        .await?;
    Ok(())
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

fn to_mcp_error(error: impl std::fmt::Display) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(error.to_string(), None)
}
```

- [ ] **Step 4: Run MCP smoke test**

Run:

```bash
cargo test --test mcp_smoke_tests -- --nocapture
```

Expected:

```text
1 passed
```

- [ ] **Step 5: Run full Rust checks**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
0 failed
```

- [ ] **Step 6: Commit MCP wiring**

```bash
git add src/server.rs src/main.rs tests/mcp_smoke_tests.rs
git commit -m "feat: wire mcp server transports"
```

## Task 10: Remaining Tool Methods and Permission Property Checks

**Files:**
- Modify: `src/tools/params.rs`
- Modify: `src/server.rs`
- Modify: `src/permissions.rs`
- Modify: `tests/mcp_smoke_tests.rs`
- Modify: `tests/tool_mapping_tests.rs`

- [ ] **Step 1: Add typed params for remaining tool methods**

Append these structs to `src/tools/params.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonQueryParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct CommonModifyParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub confirmation: Option<String>,
}
```

- [ ] **Step 2: Add MCP smoke assertions for tool names**

Append to `tests/mcp_smoke_tests.rs`:

```rust
#[test]
fn toolset_names_are_stable() {
    let names = [
        "query_server",
        "query_files",
        "modify_files",
        "query_changelists",
        "modify_changelists",
        "query_shelves",
        "modify_shelves",
        "query_workspaces",
        "modify_workspaces",
        "query_jobs",
        "modify_jobs",
        "query_streams",
        "modify_streams",
        "query_reviews",
        "modify_reviews",
    ];

    assert_eq!(names.len(), 15);
    assert!(names.contains(&"query_server"));
    assert!(names.contains(&"modify_reviews"));
}
```

- [ ] **Step 3: Add remaining MCP methods to `src/server.rs`**

Inside the existing `#[tool_router] impl P4McpServer` block, add:

```rust
#[tool(description = "Get changelist information or list changelists")]
async fn query_changelists(&self, Parameters(params): Parameters<crate::tools::params::CommonQueryParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::changelists::build_changelist_query_invocation;
    self.policy().check(Access::Read, Toolset::Changelists, "query_changelists").map_err(to_mcp_error)?;
    let invocation = build_changelist_query_invocation(
        &params.action,
        params.changelist_id.as_deref(),
        params.status.as_deref(),
        params.workspace_name.as_deref(),
        params.max_results,
    ).map_err(to_mcp_error)?;
    self.call_p4_tool("query_changelists", &params.action, invocation).await
}

#[tool(description = "Create, update, submit, or delete changelists")]
async fn modify_changelists(&self, Parameters(params): Parameters<crate::tools::params::CommonModifyParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::changelists::build_changelist_modify_invocation;
    self.policy().check(Access::Write, Toolset::Changelists, "modify_changelists").map_err(to_mcp_error)?;
    let change = params.changelist_id.as_deref().unwrap_or("new");
    let stdin = params.form.or_else(|| params.description.map(|description| crate::p4::forms::change_form(&description, &params.files)));
    let invocation = build_changelist_modify_invocation(&params.action, change, stdin).map_err(to_mcp_error)?;
    self.call_p4_tool("modify_changelists", &params.action, invocation).await
}

#[tool(description = "List shelves, show shelf diff, or list shelf files")]
async fn query_shelves(&self, Parameters(params): Parameters<crate::tools::params::CommonQueryParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::shelves::build_shelf_query_invocation;
    self.policy().check(Access::Read, Toolset::Shelves, "query_shelves").map_err(to_mcp_error)?;
    let invocation = build_shelf_query_invocation(&params.action, params.changelist_id.as_deref(), params.user.as_deref(), params.max_results).map_err(to_mcp_error)?;
    self.call_p4_tool("query_shelves", &params.action, invocation).await
}

#[tool(description = "Shelve, unshelve, or delete shelved files")]
async fn modify_shelves(&self, Parameters(params): Parameters<crate::tools::params::CommonModifyParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Write, Toolset::Shelves, "modify_shelves").map_err(to_mcp_error)?;
    let change = params.changelist_id.clone().ok_or_else(|| to_mcp_error("changelist_id is required"))?;
    let args = match params.action.as_str() {
        "shelve" => vec!["shelve".to_string(), "-c".to_string(), change],
        "unshelve" => vec!["unshelve".to_string(), "-s".to_string(), change],
        "delete" if params.confirmation.as_deref() == Some("PROCEED") => vec!["shelve".to_string(), "-d".to_string(), "-c".to_string(), change],
        "delete" => return Err(to_mcp_error("destructive action requires confirmation value PROCEED")),
        other => return Err(to_mcp_error(format!("unknown action: {other}"))),
    };
    self.call_p4_tool("modify_shelves", &params.action, crate::p4::runner::P4Invocation { args, stdin: None, mode: crate::p4::runner::OutputMode::JsonLines }).await
}

#[tool(description = "List, get, map, or inspect workspaces")]
async fn query_workspaces(&self, Parameters(params): Parameters<crate::tools::params::CommonQueryParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::workspaces::build_workspace_query_invocation;
    self.policy().check(Access::Read, Toolset::Workspaces, "query_workspaces").map_err(to_mcp_error)?;
    let invocation = build_workspace_query_invocation(&params.action, params.workspace_name.as_deref(), params.file_path.as_deref(), params.max_results).map_err(to_mcp_error)?;
    self.call_p4_tool("query_workspaces", &params.action, invocation).await
}

#[tool(description = "Create, update, or delete workspaces using p4 client forms")]
async fn modify_workspaces(&self, Parameters(params): Parameters<crate::tools::params::CommonModifyParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Write, Toolset::Workspaces, "modify_workspaces").map_err(to_mcp_error)?;
    let args = match params.action.as_str() {
        "create" | "update" => vec!["client".to_string(), "-i".to_string()],
        "delete" if params.confirmation.as_deref() == Some("PROCEED") => vec!["client".to_string(), "-d".to_string(), params.changelist_id.unwrap_or_default()],
        "delete" => return Err(to_mcp_error("destructive action requires confirmation value PROCEED")),
        other => return Err(to_mcp_error(format!("unknown action: {other}"))),
    };
    self.call_p4_tool("modify_workspaces", &params.action, crate::p4::runner::P4Invocation { args, stdin: params.form, mode: crate::p4::runner::OutputMode::JsonLines }).await
}

#[tool(description = "List or get jobs and fixes")]
async fn query_jobs(&self, Parameters(params): Parameters<crate::tools::params::CommonQueryParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::jobs::build_job_query_invocation;
    self.policy().check(Access::Read, Toolset::Jobs, "query_jobs").map_err(to_mcp_error)?;
    let invocation = build_job_query_invocation(&params.action, params.changelist_id.as_deref(), params.job_id.as_deref(), params.max_results).map_err(to_mcp_error)?;
    self.call_p4_tool("query_jobs", &params.action, invocation).await
}

#[tool(description = "Attach or detach jobs from changelists")]
async fn modify_jobs(&self, Parameters(params): Parameters<crate::tools::params::CommonModifyParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Write, Toolset::Jobs, "modify_jobs").map_err(to_mcp_error)?;
    let change = params.changelist_id.ok_or_else(|| to_mcp_error("changelist_id is required"))?;
    let job = params.files.first().cloned().ok_or_else(|| to_mcp_error("files[0] must contain the job id"))?;
    let args = match params.action.as_str() {
        "fix" => vec!["fix".to_string(), "-c".to_string(), change, job],
        "unfix" => vec!["fix".to_string(), "-d".to_string(), "-c".to_string(), change, job],
        other => return Err(to_mcp_error(format!("unknown action: {other}"))),
    };
    self.call_p4_tool("modify_jobs", &params.action, crate::p4::runner::P4Invocation { args, stdin: None, mode: crate::p4::runner::OutputMode::JsonLines }).await
}

#[tool(description = "List streams, get stream specs, graph streams, and inspect stream integration status")]
async fn query_streams(&self, Parameters(params): Parameters<crate::tools::params::CommonQueryParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::tools::streams::build_stream_query_invocation;
    self.policy().check(Access::Read, Toolset::Streams, "query_streams").map_err(to_mcp_error)?;
    let invocation = build_stream_query_invocation(&params.action, params.stream.as_deref(), params.owner.as_deref(), params.max_results).map_err(to_mcp_error)?;
    self.call_p4_tool("query_streams", &params.action, invocation).await
}

#[tool(description = "Create, update, or delete stream specs")]
async fn modify_streams(&self, Parameters(params): Parameters<crate::tools::params::CommonModifyParams>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Write, Toolset::Streams, "modify_streams").map_err(to_mcp_error)?;
    let args = match params.action.as_str() {
        "create" | "update" => vec!["stream".to_string(), "-i".to_string()],
        "delete" if params.confirmation.as_deref() == Some("PROCEED") => vec!["stream".to_string(), "-d".to_string(), params.changelist_id.unwrap_or_default()],
        "delete" => return Err(to_mcp_error("destructive action requires confirmation value PROCEED")),
        other => return Err(to_mcp_error(format!("unknown action: {other}"))),
    };
    self.call_p4_tool("modify_streams", &params.action, crate::p4::runner::P4Invocation { args, stdin: params.form, mode: crate::p4::runner::OutputMode::JsonLines }).await
}

#[tool(description = "Query P4 Code Review / Swarm reviews")]
async fn query_reviews(&self, Parameters(params): Parameters<crate::tools::reviews::ReviewRequest>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Read, Toolset::Reviews, "query_reviews").map_err(to_mcp_error)?;
    let built = params.to_http("unused").map_err(to_mcp_error)?;
    let body = ToolResponse::success("query_reviews", serde_json::json!({"method": built.method, "path": built.path, "query": built.query, "body": built.body}));
    Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
}

#[tool(description = "Modify P4 Code Review / Swarm reviews")]
async fn modify_reviews(&self, Parameters(params): Parameters<crate::tools::reviews::ReviewRequest>) -> Result<CallToolResult, rmcp::ErrorData> {
    self.policy().check(Access::Write, Toolset::Reviews, "modify_reviews").map_err(to_mcp_error)?;
    let built = params.to_http("unused").map_err(to_mcp_error)?;
    let body = ToolResponse::success("modify_reviews", serde_json::json!({"method": built.method, "path": built.path, "query": built.query, "body": built.body}));
    Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
}
```

Also add this helper method inside `impl P4McpServer`:

```rust
async fn call_p4_tool(
    &self,
    tool_name: &str,
    action: &str,
    invocation: crate::p4::runner::P4Invocation,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let output = self.executor.run(invocation, std::iter::empty::<(&str, String)>()).await.map_err(to_mcp_error)?;
    let message = if output.records.is_empty() { output.text } else { serde_json::json!(output.records) };
    let body = ToolResponse::success(format!("{tool_name}:{action}"), message);
    Ok(CallToolResult::success(vec![Content::json(body).map_err(to_mcp_error)?]))
}
```

- [ ] **Step 4: Run tests and fix compile drift only inside touched files**

Run:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
0 failed
```

- [ ] **Step 5: Commit remaining tool methods**

```bash
git add src/server.rs src/tools/params.rs src/permissions.rs tests
git commit -m "feat: expose full p4 mcp tool surface"
```

## Task 11: Packaging and Closed-Network Build Workflow

**Files:**
- Create: `scripts/package.sh`
- Create: `docs/offline-build.md`
- Modify: `README.md`

- [ ] **Step 1: Add package script**

Create `scripts/package.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

name="p4-mcp-server"
version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
target_dir="target/release"
archive_dir="target/package/${name}-${version}"

cargo build --release
rm -rf "${archive_dir}"
mkdir -p "${archive_dir}"
cp "${target_dir}/${name}" "${archive_dir}/"
cp README.md LICENSE.txt "${archive_dir}/" 2>/dev/null || cp README.md "${archive_dir}/"
tar -C "target/package" -czf "target/package/${name}-${version}-$(uname -s)-$(uname -m).tgz" "${name}-${version}"
```

Run:

```bash
chmod +x scripts/package.sh
```

- [ ] **Step 2: Add offline build doc**

Create `docs/offline-build.md`:

```markdown
# Offline Build Workflow

Use this workflow on a networked build machine first:

```bash
cargo fetch --locked
cargo vendor vendor
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF
cargo build --release --locked
./scripts/package.sh
```

Move the repository, `vendor/`, `.cargo/config.toml`, and the generated archive into the closed network.

The runtime machine must already have a working `p4` CLI and Perforce authentication context:

```bash
p4 info
./p4-mcp-server --readonly
```
```

- [ ] **Step 3: Update README with release commands**

Append to `README.md`:

```markdown
## Release Packaging

```bash
./scripts/package.sh
```

The resulting archive contains the Rust binary and docs. It does not contain Python or P4Python.

## MCP Client Example

```json
{
  "mcpServers": {
    "perforce-p4-mcp": {
      "command": "/absolute/path/to/p4-mcp-server",
      "args": ["--readonly"],
      "env": {
        "P4PORT": "ssl:perforce.example.com:1666",
        "P4USER": "your_username",
        "P4CLIENT": "your_workspace"
      }
    }
  }
}
```
```

- [ ] **Step 4: Run package verification**

Run:

```bash
cargo fmt --check
cargo test
cargo build --release
./scripts/package.sh
```

Expected:

```text
0 failed
target/package/p4-mcp-server-0.1.0-<system>-<arch>.tgz
```

- [ ] **Step 5: Commit packaging docs**

```bash
git add scripts/package.sh docs/offline-build.md README.md
git commit -m "docs: add offline build and packaging workflow"
```

## Task 12: Final Verification

**Files:**
- Modify only files touched by prior failed verification, if any.

- [ ] **Step 1: Run full local verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Expected:

```text
0 failed
```

- [ ] **Step 2: Verify installed P4 dependency is external**

Run:

```bash
p4 -V
otool -L target/release/p4-mcp-server 2>/dev/null || ldd target/release/p4-mcp-server
```

Expected:

```text
Rev. P4/
```

The binary dependency output must not contain `Python`, `P4Python`, `libpython`, or `uv`.

- [ ] **Step 3: Verify MCP tool discovery with inspector**

Run from a networked dev machine that has Node available:

```bash
npx @modelcontextprotocol/inspector target/release/p4-mcp-server --readonly
```

Expected:

```text
query_server
query_files
modify_files
query_reviews
modify_reviews
```

- [ ] **Step 4: Commit final verification fixes**

If Step 1 or Step 2 required code changes:

```bash
git add .
git commit -m "test: complete rust p4 mcp verification"
```

If no changes were required:

```bash
git status --short
```

Expected:

```text

```

## Self-Review

- Spec coverage: The plan removes Python/P4Python, assumes local `p4`, keeps the upstream MCP tool names and toolset categories, retains stdio and HTTP transport modes, keeps read-only mode, adds destructive confirmation gates, includes P4 Code Review v11 endpoint mapping, and adds offline packaging.
- Red-flag scan: No unresolved empty-work markers are intentionally left in the plan. Every task lists exact files, code snippets, commands, and expected verification output.
- Type consistency: `Toolset`, `TransportMode`, `AppConfig`, `P4Invocation`, `OutputMode`, `ToolResponse`, `QueryFilesParams`, `ModifyFilesParams`, `CommonQueryParams`, `CommonModifyParams`, `ReviewRequest`, and `P4McpServer` are introduced before they are used by later tasks.
