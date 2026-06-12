use std::{collections::BTreeSet, env, fmt, path::PathBuf, str::FromStr};

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

    #[arg(
        long,
        default_value = "files,changelists,shelves,workspaces,jobs,reviews,streams"
    )]
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

    #[arg(long)]
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

        let ssl_verify = if let Some(path) = self.ca_bundle {
            SslVerify::CaBundle(path)
        } else if self.ssl_no_verify {
            SslVerify::Disabled
        } else if let Some(path) = env::var_os("P4MCP_CA_BUNDLE") {
            SslVerify::CaBundle(PathBuf::from(path))
        } else if env::var("P4MCP_SSL_VERIFY").is_ok_and(|value| value == "false") {
            SslVerify::Disabled
        } else {
            SslVerify::Enabled
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
