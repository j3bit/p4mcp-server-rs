use std::{
    env,
    ffi::OsString,
    net::{IpAddr, Ipv4Addr},
    sync::{Mutex, MutexGuard},
};

use p4mcp_server_rs::config::{Cli, SslVerify, Toolset, TransportMode};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    p4_bin: Option<OsString>,
    host: Option<OsString>,
    log_dir: Option<OsString>,
    ca_bundle: Option<OsString>,
    ssl_verify: Option<OsString>,
}

impl EnvGuard {
    fn new() -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let p4_bin = env::var_os("P4MCP_P4_BIN");
        let host = env::var_os("P4MCP_HOST");
        let log_dir = env::var_os("P4MCP_LOG_DIR");
        let ca_bundle = env::var_os("P4MCP_CA_BUNDLE");
        let ssl_verify = env::var_os("P4MCP_SSL_VERIFY");

        unsafe {
            env::remove_var("P4MCP_P4_BIN");
            env::remove_var("P4MCP_HOST");
            env::remove_var("P4MCP_LOG_DIR");
            env::remove_var("P4MCP_CA_BUNDLE");
            env::remove_var("P4MCP_SSL_VERIFY");
        }

        Self {
            _lock: lock,
            p4_bin,
            host,
            log_dir,
            ca_bundle,
            ssl_verify,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.p4_bin {
                Some(value) => env::set_var("P4MCP_P4_BIN", value),
                None => env::remove_var("P4MCP_P4_BIN"),
            }

            match &self.host {
                Some(value) => env::set_var("P4MCP_HOST", value),
                None => env::remove_var("P4MCP_HOST"),
            }

            match &self.log_dir {
                Some(value) => env::set_var("P4MCP_LOG_DIR", value),
                None => env::remove_var("P4MCP_LOG_DIR"),
            }

            match &self.ca_bundle {
                Some(value) => env::set_var("P4MCP_CA_BUNDLE", value),
                None => env::remove_var("P4MCP_CA_BUNDLE"),
            }

            match &self.ssl_verify {
                Some(value) => env::set_var("P4MCP_SSL_VERIFY", value),
                None => env::remove_var("P4MCP_SSL_VERIFY"),
            }
        }
    }
}

#[test]
fn defaults_match_upstream_toolsets() {
    let _env = EnvGuard::new();
    let cli = Cli::try_parse_from(["p4-mcp-server"]).unwrap();
    let config = cli.into_config().unwrap();

    assert!(!config.readonly);
    assert_eq!(config.transport, TransportMode::Stdio);
    assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(config.port, 8000);
    assert_eq!(config.toolsets, Toolset::default_set());
}

#[test]
fn parses_explicit_toolsets_and_readonly() {
    let _env = EnvGuard::new();
    let cli = Cli::try_parse_from([
        "p4-mcp-server",
        "--readonly",
        "--toolsets",
        "files,changelists",
        "--transport",
        "http",
        "--host",
        "0.0.0.0",
        "--port",
        "9000",
    ])
    .unwrap();

    let config = cli.into_config().unwrap();

    assert!(config.readonly);
    assert_eq!(config.transport, TransportMode::Http);
    assert_eq!(config.host, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    assert_eq!(config.port, 9000);
    assert_eq!(config.toolsets.len(), 2);
    assert!(config.toolsets.contains(&Toolset::Files));
    assert!(config.toolsets.contains(&Toolset::Changelists));
}

#[test]
fn rejects_unknown_toolset() {
    let _env = EnvGuard::new();
    let cli = Cli::try_parse_from(["p4-mcp-server", "--toolsets", "files,unknown"]).unwrap();
    let err = cli.into_config().unwrap_err().to_string();
    assert!(err.contains("unknown toolset"));
}

#[test]
fn cli_ssl_options_take_priority() {
    let _env = EnvGuard::new();
    unsafe {
        env::set_var("P4MCP_CA_BUNDLE", "/tmp/env-ca.pem");
        env::set_var("P4MCP_SSL_VERIFY", "false");
    }

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

#[test]
fn cli_ssl_no_verify_beats_env_ca_bundle() {
    let _env = EnvGuard::new();
    unsafe {
        env::set_var("P4MCP_CA_BUNDLE", "/tmp/env-ca.pem");
    }

    let cli = Cli::try_parse_from(["p4-mcp-server", "--ssl-no-verify"]).unwrap();
    let config = cli.into_config().unwrap();

    assert_eq!(config.ssl_verify, SslVerify::Disabled);
}

#[test]
fn env_ca_bundle_beats_env_ssl_verify_false() {
    let _env = EnvGuard::new();
    unsafe {
        env::set_var("P4MCP_CA_BUNDLE", "/tmp/env-ca.pem");
        env::set_var("P4MCP_SSL_VERIFY", "false");
    }

    let cli = Cli::try_parse_from(["p4-mcp-server"]).unwrap();
    let config = cli.into_config().unwrap();

    assert_eq!(config.ssl_verify.to_string(), "/tmp/env-ca.pem");
}

#[test]
fn env_ssl_verify_false_disables_verification() {
    let _env = EnvGuard::new();
    unsafe {
        env::set_var("P4MCP_SSL_VERIFY", "false");
    }

    let cli = Cli::try_parse_from(["p4-mcp-server"]).unwrap();
    let config = cli.into_config().unwrap();

    assert_eq!(config.ssl_verify, SslVerify::Disabled);
}
