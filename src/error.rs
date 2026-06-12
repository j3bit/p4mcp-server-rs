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
