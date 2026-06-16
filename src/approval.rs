use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{
    error::{P4McpError, Result},
    tools::response::ToolResponse,
};
use async_trait::async_trait;
use rmcp::{Peer, RoleServer, ServiceError, service::ElicitationError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const APPROVAL_TOKEN_TTL_SECONDS: u64 = 300;

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ApprovalPreview {
    pub summary: String,
    pub tool: String,
    pub action: String,
    pub targets: Vec<String>,
    pub changelist: Option<String>,
    pub workspace: Option<String>,
    pub stream: Option<String>,
    pub review: Option<String>,
    pub command: Option<Vec<String>>,
    pub commands: Option<Vec<Vec<String>>>,
    pub request: Option<HttpPreview>,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct HttpPreview {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct ApprovalRequest {
    pub tool: String,
    pub action: String,
    pub params: Value,
    pub preview: ApprovalPreview,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WriteApprovalChoice {
    pub decision: WriteApprovalDecision,
}

rmcp::elicit_safe!(WriteApprovalChoice);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WriteApprovalDecision {
    Proceed,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    Response(ToolResponse),
}

#[derive(Debug, Clone)]
pub enum ApprovalChannel {
    Elicitation(Peer<RoleServer>),
    FallbackOnly,
}

#[async_trait]
pub trait WriteApprovalGate: Send + Sync {
    async fn approve(
        &self,
        channel: ApprovalChannel,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> Result<ApprovalDecision>;
}

pub struct DefaultWriteApprovalGate {
    ttl: Duration,
    tokens: Mutex<HashMap<String, ApprovalTokenRecord>>,
}

struct ApprovalTokenRecord {
    digest: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
enum WriteApprovalElicitationDecision {
    Approved,
    Response(ToolResponse),
    FallbackRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WriteApprovalElicitationOutcome {
    Accepted(WriteApprovalChoice),
    Declined,
    Cancelled,
    Timeout,
    NoContent,
    InvalidContent,
    TransportError,
    CapabilityNotSupported,
}

impl DefaultWriteApprovalGate {
    pub fn new() -> Self {
        Self::with_ttl(Duration::from_secs(APPROVAL_TOKEN_TTL_SECONDS))
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            tokens: Mutex::new(HashMap::new()),
        }
    }

    pub fn digest(&self, request: &ApprovalRequest) -> String {
        let value = approval_digest_value(request);
        let mut canonical = String::new();
        write_canonical_json(&value, &mut canonical);

        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        format!("sha256:{}", hex_encode(&hasher.finalize()))
    }

    #[cfg(test)]
    fn token_count(&self) -> usize {
        self.tokens
            .lock()
            .expect("approval token store mutex poisoned")
            .len()
    }

    fn approve_fallback(
        &self,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> Result<ApprovalDecision> {
        let digest = self.digest(&request);

        if let Some(token) = approval_token {
            return self.consume_token(token, &digest, &request.action);
        }

        self.require_approval(request.action, request.preview, digest)
    }

    fn require_approval(
        &self,
        action: String,
        preview: ApprovalPreview,
        digest: String,
    ) -> Result<ApprovalDecision> {
        let approval_token = generate_token().map_err(|error| P4McpError::InvalidInput {
            message: format!("failed to generate approval token: {error}"),
        })?;

        let now = Instant::now();
        let expires_at = now + self.ttl;
        let mut tokens = self.tokens.lock().map_err(|_| P4McpError::InvalidInput {
            message: "approval token store mutex poisoned".to_string(),
        })?;
        prune_expired_tokens(&mut tokens, now);
        tokens.insert(
            approval_token.clone(),
            ApprovalTokenRecord {
                digest: digest.clone(),
                expires_at,
            },
        );

        let ttl_seconds = self.ttl.as_secs();
        let instruction = format!(
            "Approval required. Re-run the same request with approval_token \"{approval_token}\" within {ttl_seconds} seconds to approve digest {digest}.",
        );

        Ok(ApprovalDecision::Response(ToolResponse::approval_required(
            action,
            json!({
                "preview": preview,
                "digest": digest,
                "approval_token": approval_token,
                "ttl_seconds": ttl_seconds,
                "instruction": instruction,
            }),
        )))
    }

    fn consume_token(&self, token: &str, digest: &str, action: &str) -> Result<ApprovalDecision> {
        let mut tokens = self.tokens.lock().map_err(|_| P4McpError::InvalidInput {
            message: "approval token store mutex poisoned".to_string(),
        })?;
        prune_expired_tokens(&mut tokens, Instant::now());
        let record = tokens.remove(token);

        let Some(record) = record else {
            return Ok(cancelled(
                action,
                "approval token is unknown or already used",
            ));
        };

        if Instant::now() >= record.expires_at {
            return Ok(cancelled(action, "approval token expired"));
        }

        if record.digest != digest {
            return Ok(cancelled(
                action,
                "approval token does not match request digest",
            ));
        }

        Ok(ApprovalDecision::Approved)
    }

    async fn approve_elicitation(
        &self,
        peer: Peer<RoleServer>,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> Result<ApprovalDecision> {
        let action = request.action.clone();
        let message = format_elicitation_message(&request.preview);
        let result = peer
            .elicit_with_timeout::<WriteApprovalChoice>(
                message,
                Some(Duration::from_secs(APPROVAL_TOKEN_TTL_SECONDS)),
            )
            .await;
        let decision = decide_write_approval_from_elicitation(&action, elicitation_outcome(result));

        match decision {
            WriteApprovalElicitationDecision::Approved => Ok(ApprovalDecision::Approved),
            WriteApprovalElicitationDecision::Response(response) => {
                Ok(ApprovalDecision::Response(response))
            }
            WriteApprovalElicitationDecision::FallbackRequired => {
                self.approve_fallback(request, approval_token)
            }
        }
    }
}

impl Default for DefaultWriteApprovalGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WriteApprovalGate for DefaultWriteApprovalGate {
    async fn approve(
        &self,
        channel: ApprovalChannel,
        request: ApprovalRequest,
        approval_token: Option<&str>,
    ) -> Result<ApprovalDecision> {
        match channel {
            ApprovalChannel::Elicitation(peer) => {
                self.approve_elicitation(peer, request, approval_token)
                    .await
            }
            ApprovalChannel::FallbackOnly => self.approve_fallback(request, approval_token),
        }
    }
}

fn format_elicitation_message(preview: &ApprovalPreview) -> String {
    let mut lines = vec![
        "Approve this Perforce write?".to_string(),
        format!("Summary: {}", preview.summary),
        format!("Tool: {}", preview.tool),
        format!("Action: {}", preview.action),
    ];

    if !preview.targets.is_empty() {
        lines.push(format!("Targets: {}", preview.targets.join(", ")));
    }
    if let Some(changelist) = &preview.changelist {
        lines.push(format!("Changelist: {changelist}"));
    }
    if let Some(workspace) = &preview.workspace {
        lines.push(format!("Workspace: {workspace}"));
    }
    if let Some(stream) = &preview.stream {
        lines.push(format!("Stream: {stream}"));
    }
    if let Some(review) = &preview.review {
        lines.push(format!("Review: {review}"));
    }
    if let Some(commands) = &preview.commands {
        for (index, command) in commands.iter().enumerate() {
            lines.push(format!("Command {}: {}", index + 1, format_command(command)));
        }
    } else if let Some(command) = &preview.command {
        lines.push(format!("Command: {}", format_command(command)));
    }
    if let Some(request) = &preview.request {
        lines.push(format!("Request: {} {}", request.method, request.path));
    }

    lines.push("Choose PROCEED to execute this write or CANCEL to leave it unchanged.".to_string());
    lines.join("\n")
}

fn format_command(command: &[String]) -> String {
    command
        .iter()
        .map(|argument| format_command_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_command_argument(argument: &str) -> String {
    if !needs_shell_quote(argument) {
        return argument.to_string();
    }

    format!("'{}'", argument.replace('\'', "'\\''"))
}

fn needs_shell_quote(argument: &str) -> bool {
    argument.is_empty()
        || argument
            .chars()
            .any(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, '/' | '.' | '_' | '-' | ':'))
}

fn elicitation_outcome(
    result: std::result::Result<Option<WriteApprovalChoice>, ElicitationError>,
) -> WriteApprovalElicitationOutcome {
    match result {
        Ok(Some(choice)) => WriteApprovalElicitationOutcome::Accepted(choice),
        Ok(None) | Err(ElicitationError::NoContent) => WriteApprovalElicitationOutcome::NoContent,
        Err(ElicitationError::UserDeclined) => WriteApprovalElicitationOutcome::Declined,
        Err(ElicitationError::UserCancelled) => WriteApprovalElicitationOutcome::Cancelled,
        Err(ElicitationError::ParseError { .. }) => WriteApprovalElicitationOutcome::InvalidContent,
        Err(ElicitationError::CapabilityNotSupported) => {
            WriteApprovalElicitationOutcome::CapabilityNotSupported
        }
        Err(ElicitationError::Service(ServiceError::Timeout { .. })) => {
            WriteApprovalElicitationOutcome::Timeout
        }
        Err(ElicitationError::Service(_)) | Err(_) => {
            WriteApprovalElicitationOutcome::TransportError
        }
    }
}

fn decide_write_approval_from_elicitation(
    action: &str,
    outcome: WriteApprovalElicitationOutcome,
) -> WriteApprovalElicitationDecision {
    match outcome {
        WriteApprovalElicitationOutcome::Accepted(WriteApprovalChoice {
            decision: WriteApprovalDecision::Proceed,
        }) => WriteApprovalElicitationDecision::Approved,
        WriteApprovalElicitationOutcome::Accepted(WriteApprovalChoice {
            decision: WriteApprovalDecision::Cancel,
        }) => WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
            action.to_string(),
            json!({ "reason": "write approval cancelled by user" }),
        )),
        WriteApprovalElicitationOutcome::Declined => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval declined by user" }),
            ))
        }
        WriteApprovalElicitationOutcome::Cancelled => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval cancelled by user" }),
            ))
        }
        WriteApprovalElicitationOutcome::Timeout => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval timed out" }),
            ))
        }
        WriteApprovalElicitationOutcome::NoContent => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval returned no content" }),
            ))
        }
        WriteApprovalElicitationOutcome::InvalidContent => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval response was invalid" }),
            ))
        }
        WriteApprovalElicitationOutcome::TransportError => {
            WriteApprovalElicitationDecision::Response(ToolResponse::cancelled(
                action.to_string(),
                json!({ "reason": "write approval request failed" }),
            ))
        }
        WriteApprovalElicitationOutcome::CapabilityNotSupported => {
            WriteApprovalElicitationDecision::FallbackRequired
        }
    }
}

fn cancelled(action: &str, reason: &str) -> ApprovalDecision {
    ApprovalDecision::Response(ToolResponse::cancelled(
        action.to_string(),
        json!({ "reason": reason }),
    ))
}

fn prune_expired_tokens(tokens: &mut HashMap<String, ApprovalTokenRecord>, now: Instant) {
    tokens.retain(|_, record| now < record.expires_at);
}

fn generate_token() -> std::result::Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn approval_digest_value(request: &ApprovalRequest) -> Value {
    json!({
        "tool": request.tool,
        "action": request.action,
        "params": sanitize_params_for_digest(&request.params),
        "preview": sanitize_value_for_digest(
            &serde_json::to_value(&request.preview).expect("approval preview serializes to JSON")
        ),
    })
}

fn sanitize_params_for_digest(value: &Value) -> Value {
    match value {
        Value::Object(values) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in values {
                if key == "approval_token" {
                    continue;
                }
                sanitized.insert(key.clone(), sanitize_field_for_digest(key, value));
            }
            Value::Object(sanitized)
        }
        other => sanitize_value_for_digest(other),
    }
}

fn sanitize_value_for_digest(value: &Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.iter().map(sanitize_value_for_digest).collect())
        }
        Value::Object(values) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in values {
                sanitized.insert(key.clone(), sanitize_field_for_digest(key, value));
            }
            Value::Object(sanitized)
        }
        other => other.clone(),
    }
}

fn sanitize_field_for_digest(field: &str, value: &Value) -> Value {
    if is_secret_field(field) {
        return json!({
            "redacted_sha256": digest_value(value),
        });
    }
    sanitize_value_for_digest(value)
}

fn digest_value(value: &Value) -> String {
    let mut canonical = String::new();
    write_canonical_json(value, &mut canonical);

    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("sha256:{}", hex_encode(&hasher.finalize()))
}

fn write_canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => out.push_str(&value.to_string()),
        Value::String(value) => {
            out.push_str(&serde_json::to_string(value).expect("string serializes to JSON"))
        }
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(value, out);
            }
            out.push(']');
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();

            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("object key serializes to JSON"));
                out.push(':');
                write_canonical_json(&values[*key], out);
            }
            out.push('}');
        }
    }
}

fn is_secret_field(field: &str) -> bool {
    matches!(
        field,
        "approval_token"
            | "password"
            | "ticket"
            | "authorization"
            | "Authorization"
            | "P4PASSWD"
            | "P4TICKETS"
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::tools::response::ToolResponse;
    use serde_json::json;

    use super::*;

    fn sample_request() -> ApprovalRequest {
        ApprovalRequest {
            tool: "modify_files".to_string(),
            action: "sync".to_string(),
            params: json!({
                "approval_token": null,
                "files": ["//depot/main/a.txt", "//depot/main/b.txt"],
                "force": true
            }),
            preview: ApprovalPreview {
                summary: "sync two files".to_string(),
                tool: "modify_files".to_string(),
                action: "sync".to_string(),
                targets: vec![
                    "//depot/main/a.txt".to_string(),
                    "//depot/main/b.txt".to_string(),
                ],
                changelist: Some("12345".to_string()),
                workspace: Some("ws-main".to_string()),
                stream: Some("//stream/main".to_string()),
                review: Some("review-7".to_string()),
                command: Some(vec![
                    "p4".to_string(),
                    "sync".to_string(),
                    "//depot/main/a.txt".to_string(),
                    "//depot/main/b.txt".to_string(),
                ]),
                commands: None,
                request: Some(HttpPreview {
                    method: "POST".to_string(),
                    path: "/mcp/tools/modify_files".to_string(),
                }),
            },
        }
    }

    #[test]
    fn format_elicitation_message_lists_multiple_commands() {
        let mut preview = sample_request().preview;
        preview.command = None;
        preview.commands = Some(vec![
            vec![
                "p4".to_string(),
                "move".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/a.txt".to_string(),
                "//depot/dev/a.txt".to_string(),
            ],
            vec![
                "p4".to_string(),
                "move".to_string(),
                "-c".to_string(),
                "123".to_string(),
                "//depot/main/b.txt".to_string(),
                "//depot/dev/b.txt".to_string(),
            ],
        ]);

        let message = format_elicitation_message(&preview);

        assert!(message.contains(
            "Command 1: p4 move -c 123 //depot/main/a.txt //depot/dev/a.txt"
        ));
        assert!(message.contains(
            "Command 2: p4 move -c 123 //depot/main/b.txt //depot/dev/b.txt"
        ));
    }

    #[test]
    fn format_elicitation_message_prefers_commands_over_command() {
        let mut preview = sample_request().preview;
        preview.command = Some(vec![
            "p4".to_string(),
            "sync".to_string(),
            "//depot/main/single.txt".to_string(),
        ]);
        preview.commands = Some(vec![vec![
            "p4".to_string(),
            "move".to_string(),
            "-c".to_string(),
            "123".to_string(),
            "//depot/main/a.txt".to_string(),
            "//depot/dev/a.txt".to_string(),
        ]]);

        let message = format_elicitation_message(&preview);

        assert!(message.contains(
            "Command 1: p4 move -c 123 //depot/main/a.txt //depot/dev/a.txt"
        ));
        assert!(!message.contains("Command: p4 sync //depot/main/single.txt"));
    }

    #[test]
    fn format_elicitation_message_quotes_spaced_command_arguments() {
        let mut preview = sample_request().preview;
        preview.command = None;
        preview.commands = Some(vec![vec![
            "p4".to_string(),
            "move".to_string(),
            "-c".to_string(),
            "123".to_string(),
            "//depot/main/file with space.txt".to_string(),
            "//depot/dev/file with space.txt".to_string(),
        ]]);

        let message = format_elicitation_message(&preview);

        assert!(message.contains(
            "Command 1: p4 move -c 123 '//depot/main/file with space.txt' '//depot/dev/file with space.txt'"
        ));
    }

    async fn require_approval(
        gate: &DefaultWriteApprovalGate,
        request: ApprovalRequest,
    ) -> (String, String) {
        match gate
            .approve(ApprovalChannel::FallbackOnly, request, None)
            .await
            .expect("approval check succeeds")
        {
            ApprovalDecision::Response(response) => {
                assert_eq!(response.status, "approval_required");
                approval_fields(&response)
            }
            other => panic!("expected approval required, got {other:?}"),
        }
    }

    fn approval_fields(response: &ToolResponse) -> (String, String) {
        let digest = response.message["digest"]
            .as_str()
            .expect("approval response includes digest")
            .to_string();
        let approval_token = response.message["approval_token"]
            .as_str()
            .expect("approval response includes token")
            .to_string();
        (digest, approval_token)
    }

    fn assert_json_schema<T: schemars::JsonSchema>() {}

    fn accepted_choice(decision: WriteApprovalDecision) -> WriteApprovalElicitationOutcome {
        WriteApprovalElicitationOutcome::Accepted(WriteApprovalChoice { decision })
    }

    fn assert_cancelled_response(decision: WriteApprovalElicitationDecision) {
        match decision {
            WriteApprovalElicitationDecision::Response(response) => {
                assert_eq!(response.status, "cancelled");
                assert_eq!(response.action, "sync");
            }
            other => panic!("expected cancelled response, got {other:?}"),
        }
    }

    fn invalid_choice_parse_error() -> ElicitationError {
        let data = json!({ "decision": "INVALID" });
        let error = serde_json::from_value::<WriteApprovalChoice>(data.clone())
            .expect_err("invalid choice should fail to parse");
        ElicitationError::ParseError { error, data }
    }

    #[test]
    fn elicitation_outcome_maps_accepted_choice() {
        assert_eq!(
            elicitation_outcome(Ok(Some(WriteApprovalChoice {
                decision: WriteApprovalDecision::Proceed,
            }))),
            accepted_choice(WriteApprovalDecision::Proceed)
        );
    }

    #[test]
    fn elicitation_outcome_maps_no_content() {
        assert_eq!(
            elicitation_outcome(Ok(None)),
            WriteApprovalElicitationOutcome::NoContent
        );
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::NoContent)),
            WriteApprovalElicitationOutcome::NoContent
        );
    }

    #[test]
    fn elicitation_outcome_maps_parse_error_to_invalid_content() {
        assert_eq!(
            elicitation_outcome(Err(invalid_choice_parse_error())),
            WriteApprovalElicitationOutcome::InvalidContent
        );
    }

    #[test]
    fn elicitation_outcome_maps_timeout() {
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::Service(ServiceError::Timeout {
                timeout: Duration::from_secs(300),
            }))),
            WriteApprovalElicitationOutcome::Timeout
        );
    }

    #[test]
    fn elicitation_outcome_maps_generic_service_errors_to_transport_error() {
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::Service(
                ServiceError::UnexpectedResponse,
            ))),
            WriteApprovalElicitationOutcome::TransportError
        );
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::Service(
                ServiceError::TransportClosed
            ))),
            WriteApprovalElicitationOutcome::TransportError
        );
    }

    #[test]
    fn elicitation_outcome_maps_user_and_capability_errors() {
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::UserDeclined)),
            WriteApprovalElicitationOutcome::Declined
        );
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::UserCancelled)),
            WriteApprovalElicitationOutcome::Cancelled
        );
        assert_eq!(
            elicitation_outcome(Err(ElicitationError::CapabilityNotSupported)),
            WriteApprovalElicitationOutcome::CapabilityNotSupported
        );
    }

    #[test]
    fn accepted_proceed_approves() {
        assert_eq!(
            decide_write_approval_from_elicitation(
                "sync",
                accepted_choice(WriteApprovalDecision::Proceed)
            ),
            WriteApprovalElicitationDecision::Approved
        );
    }

    #[test]
    fn accepted_cancel_returns_cancelled() {
        assert_cancelled_response(decide_write_approval_from_elicitation(
            "sync",
            accepted_choice(WriteApprovalDecision::Cancel),
        ));
    }

    #[test]
    fn decline_returns_cancelled() {
        assert_cancelled_response(decide_write_approval_from_elicitation(
            "sync",
            WriteApprovalElicitationOutcome::Declined,
        ));
    }

    #[test]
    fn cancel_returns_cancelled() {
        assert_cancelled_response(decide_write_approval_from_elicitation(
            "sync",
            WriteApprovalElicitationOutcome::Cancelled,
        ));
    }

    #[test]
    fn timeout_returns_cancelled() {
        assert_cancelled_response(decide_write_approval_from_elicitation(
            "sync",
            WriteApprovalElicitationOutcome::Timeout,
        ));
    }

    #[test]
    fn capability_not_supported_returns_fallback_required() {
        assert_eq!(
            decide_write_approval_from_elicitation(
                "sync",
                WriteApprovalElicitationOutcome::CapabilityNotSupported,
            ),
            WriteApprovalElicitationDecision::FallbackRequired
        );
    }

    #[tokio::test]
    async fn fallback_without_token_returns_approval_required() {
        assert_json_schema::<ApprovalPreview>();
        assert_json_schema::<HttpPreview>();

        let gate = DefaultWriteApprovalGate::new();
        let request = sample_request();

        let decision = gate
            .approve(ApprovalChannel::FallbackOnly, request.clone(), None)
            .await
            .expect("approval check succeeds");

        match decision {
            ApprovalDecision::Response(response) => {
                assert_eq!(response.status, "approval_required");
                assert_eq!(response.action, request.action);
                assert_eq!(
                    response.message["preview"],
                    serde_json::to_value(&request.preview).expect("preview serializes")
                );
                assert_eq!(
                    response.message["ttl_seconds"],
                    json!(APPROVAL_TOKEN_TTL_SECONDS)
                );
                let (digest, approval_token) = approval_fields(&response);
                assert!(!digest.is_empty());
                assert!(digest.starts_with("sha256:"));
                assert_eq!(digest.len(), "sha256:".len() + 64);
                assert!(!approval_token.is_empty());
                let instruction = response.message["instruction"]
                    .as_str()
                    .expect("approval response includes instruction");
                assert!(instruction.contains(&digest));
                assert!(instruction.contains(&approval_token));
                assert!(instruction.contains("approval_token"));
            }
            other => panic!("expected approval required, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fallback_token_approves_same_digest_once() {
        let gate = DefaultWriteApprovalGate::new();
        let request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;

        let decision = gate
            .approve(
                ApprovalChannel::FallbackOnly,
                request,
                Some(approval_token.as_str()),
            )
            .await
            .expect("approval check succeeds");

        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn fallback_token_reuse_is_rejected() {
        let gate = DefaultWriteApprovalGate::new();
        let request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        assert_eq!(
            gate.approve(
                ApprovalChannel::FallbackOnly,
                request.clone(),
                Some(approval_token.as_str()),
            )
            .await
            .expect("approval check succeeds"),
            ApprovalDecision::Approved
        );

        let decision = gate
            .approve(
                ApprovalChannel::FallbackOnly,
                request,
                Some(approval_token.as_str()),
            )
            .await
            .expect("approval check succeeds");

        match decision {
            ApprovalDecision::Response(response) => assert_eq!(response.status, "cancelled"),
            other => panic!("expected cancelled response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fallback_token_expires() {
        let gate = DefaultWriteApprovalGate::with_ttl(Duration::ZERO);
        let request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;

        let decision = gate
            .approve(
                ApprovalChannel::FallbackOnly,
                request,
                Some(approval_token.as_str()),
            )
            .await
            .expect("approval check succeeds");

        match decision {
            ApprovalDecision::Response(response) => assert_eq!(response.status, "cancelled"),
            other => panic!("expected cancelled response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fallback_prunes_expired_unused_tokens() {
        let gate = DefaultWriteApprovalGate::with_ttl(Duration::ZERO);
        let request = sample_request();

        let (_digest, _approval_token) = require_approval(&gate, request.clone()).await;
        assert_eq!(gate.token_count(), 1);

        let (_digest, _approval_token) = require_approval(&gate, request).await;
        assert_eq!(gate.token_count(), 1);
    }

    #[tokio::test]
    async fn fallback_token_rejects_changed_digest() {
        let gate = DefaultWriteApprovalGate::new();
        let mut request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        request.params = json!({
            "files": ["//depot/main/a.txt", "//depot/main/c.txt"],
            "force": true
        });

        let decision = gate
            .approve(
                ApprovalChannel::FallbackOnly,
                request,
                Some(approval_token.as_str()),
            )
            .await
            .expect("approval check succeeds");

        match decision {
            ApprovalDecision::Response(response) => assert_eq!(response.status, "cancelled"),
            other => panic!("expected cancelled response, got {other:?}"),
        }
    }

    #[test]
    fn digest_omits_top_level_approval_token() {
        let gate = DefaultWriteApprovalGate::new();
        let mut redacted = sample_request();
        redacted.params = json!({
            "array": [{"approval_token": "nested-token", "path": "//depot/main/a.txt"}],
            "nested": {
                "keep": "stable"
            }
        });

        let mut with_secrets = redacted.clone();
        with_secrets.params = json!({
            "approval_token": "top-level-token",
            "array": [{
                "approval_token": "nested-token",
                "path": "//depot/main/a.txt"
            }],
            "nested": {
                "Authorization": "Bearer secret",
                "P4PASSWD": "p4-password",
                "P4TICKETS": "/tmp/tickets",
                "authorization": "basic secret",
                "keep": "stable",
                "password": "password",
                "ticket": "ticket"
            }
        });

        assert!(gate.digest(&with_secrets).starts_with("sha256:"));
        assert_ne!(gate.digest(&with_secrets), gate.digest(&redacted));

        redacted.params["nested"]["password"] = json!("password");
        redacted.params["nested"]["ticket"] = json!("ticket");
        redacted.params["nested"]["authorization"] = json!("basic secret");
        redacted.params["nested"]["Authorization"] = json!("Bearer secret");
        redacted.params["nested"]["P4PASSWD"] = json!("p4-password");
        redacted.params["nested"]["P4TICKETS"] = json!("/tmp/tickets");
        assert_eq!(gate.digest(&with_secrets), gate.digest(&redacted));

        redacted.params["nested"]["keep"] = json!("changed");
        assert_ne!(gate.digest(&with_secrets), gate.digest(&redacted));
    }

    #[test]
    fn digest_binds_arbitrary_body_fields_named_like_legacy_approval() {
        let gate = DefaultWriteApprovalGate::new();
        let mut first = sample_request();
        first.params = json!({
            "body": {
                "confirmation": "first"
            }
        });
        let mut second = first.clone();
        second.params["body"]["confirmation"] = json!("second");

        assert_ne!(gate.digest(&first), gate.digest(&second));
    }

    #[test]
    fn digest_binds_secret_field_values_without_omitting_them() {
        let gate = DefaultWriteApprovalGate::new();
        let mut first = sample_request();
        first.params = json!({
            "body": {
                "password": "first"
            }
        });
        let mut second = first.clone();
        second.params["body"]["password"] = json!("second");

        assert_ne!(gate.digest(&first), gate.digest(&second));
    }
}
