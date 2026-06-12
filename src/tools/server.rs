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
