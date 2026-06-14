use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use rmcp::{Peer, RoleServer};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const APPROVAL_TOKEN_TTL_SECONDS: u64 = 300;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApprovalPreview {
    pub summary: String,
    pub tool: String,
    pub action: String,
    pub targets: Vec<String>,
    pub changelist: Option<String>,
    pub workspace: Option<String>,
    pub stream: Option<String>,
    pub review: Option<String>,
    pub command: Vec<String>,
    pub request: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HttpPreview {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApprovalRequest {
    pub preview: ApprovalPreview,
    pub http: Option<HttpPreview>,
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    ApprovalRequired {
        preview: ApprovalPreview,
        digest: String,
        approval_token: String,
        ttl_seconds: u64,
        instruction: String,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub enum ApprovalChannel {
    Elicitation(Peer<RoleServer>),
    FallbackOnly,
}

#[async_trait]
pub trait WriteApprovalGate: Send + Sync {
    async fn check(&self, request: ApprovalRequest, channel: ApprovalChannel) -> ApprovalDecision;
}

pub struct DefaultWriteApprovalGate {
    ttl: Duration,
    tokens: Mutex<HashMap<String, ApprovalTokenRecord>>,
}

struct ApprovalTokenRecord {
    digest: String,
    expires_at: Instant,
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
        let value = serde_json::to_value(request).expect("approval request serializes to JSON");
        let mut canonical = String::new();
        write_canonical_json(&value, &mut canonical);

        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hex_encode(&hasher.finalize())
    }

    fn check_fallback(&self, request: ApprovalRequest) -> ApprovalDecision {
        let digest = self.digest(&request);

        if let Some(token) = request.approval_token.as_deref() {
            return self.consume_token(token, &digest);
        }

        self.require_approval(request.preview, digest)
    }

    fn require_approval(&self, preview: ApprovalPreview, digest: String) -> ApprovalDecision {
        let approval_token = match generate_token() {
            Ok(token) => token,
            Err(error) => {
                return ApprovalDecision::Rejected {
                    reason: format!("failed to generate approval token: {error}"),
                };
            }
        };

        let expires_at = Instant::now() + self.ttl;
        self.tokens
            .lock()
            .expect("approval token store mutex poisoned")
            .insert(
                approval_token.clone(),
                ApprovalTokenRecord {
                    digest: digest.clone(),
                    expires_at,
                },
            );

        ApprovalDecision::ApprovalRequired {
            preview,
            digest: digest.clone(),
            approval_token: approval_token.clone(),
            ttl_seconds: self.ttl.as_secs(),
            instruction: format!(
                "Approval required. Re-run the same request with approval_token \"{approval_token}\" within {} seconds to approve digest {digest}.",
                self.ttl.as_secs()
            ),
        }
    }

    fn consume_token(&self, token: &str, digest: &str) -> ApprovalDecision {
        let record = self
            .tokens
            .lock()
            .expect("approval token store mutex poisoned")
            .remove(token);

        let Some(record) = record else {
            return ApprovalDecision::Rejected {
                reason: "approval token is unknown or already used".to_string(),
            };
        };

        if Instant::now() >= record.expires_at {
            return ApprovalDecision::Rejected {
                reason: "approval token expired".to_string(),
            };
        }

        if record.digest != digest {
            return ApprovalDecision::Rejected {
                reason: "approval token does not match request digest".to_string(),
            };
        }

        ApprovalDecision::Approved
    }
}

#[async_trait]
impl WriteApprovalGate for DefaultWriteApprovalGate {
    async fn check(&self, request: ApprovalRequest, channel: ApprovalChannel) -> ApprovalDecision {
        match channel {
            ApprovalChannel::Elicitation(_) | ApprovalChannel::FallbackOnly => {
                self.check_fallback(request)
            }
        }
    }
}

fn generate_token() -> Result<String, getrandom::Error> {
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
            let mut keys = values
                .keys()
                .filter(|key| !is_redacted_field(key))
                .collect::<Vec<_>>();
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

fn is_redacted_field(field: &str) -> bool {
    matches!(
        field,
        "approval_token"
            | "confirmation"
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

    use serde_json::json;

    use super::*;

    fn sample_request() -> ApprovalRequest {
        ApprovalRequest {
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
                command: vec![
                    "p4".to_string(),
                    "sync".to_string(),
                    "//depot/main/a.txt".to_string(),
                    "//depot/main/b.txt".to_string(),
                ],
                request: json!({
                    "files": ["//depot/main/a.txt", "//depot/main/b.txt"],
                    "force": true
                }),
            },
            http: Some(HttpPreview {
                method: "POST".to_string(),
                path: "/mcp/tools/modify_files".to_string(),
            }),
            approval_token: None,
        }
    }

    async fn require_approval(
        gate: &DefaultWriteApprovalGate,
        request: ApprovalRequest,
    ) -> (String, String) {
        match gate.check(request, ApprovalChannel::FallbackOnly).await {
            ApprovalDecision::ApprovalRequired {
                digest,
                approval_token,
                ..
            } => (digest, approval_token),
            other => panic!("expected approval required, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fallback_without_token_returns_approval_required() {
        let gate = DefaultWriteApprovalGate::new();
        let request = sample_request();

        let decision = gate
            .check(request.clone(), ApprovalChannel::FallbackOnly)
            .await;

        match decision {
            ApprovalDecision::ApprovalRequired {
                preview,
                digest,
                approval_token,
                ttl_seconds,
                instruction,
            } => {
                assert_eq!(preview, request.preview);
                assert_eq!(ttl_seconds, APPROVAL_TOKEN_TTL_SECONDS);
                assert!(!digest.is_empty());
                assert!(!approval_token.is_empty());
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
        let mut request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        request.approval_token = Some(approval_token);

        let decision = gate.check(request, ApprovalChannel::FallbackOnly).await;

        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn fallback_token_reuse_is_rejected() {
        let gate = DefaultWriteApprovalGate::new();
        let mut request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        request.approval_token = Some(approval_token.clone());
        assert_eq!(
            gate.check(request.clone(), ApprovalChannel::FallbackOnly)
                .await,
            ApprovalDecision::Approved
        );

        request.approval_token = Some(approval_token);
        let decision = gate.check(request, ApprovalChannel::FallbackOnly).await;

        assert!(matches!(decision, ApprovalDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn fallback_token_expires() {
        let gate = DefaultWriteApprovalGate::with_ttl(Duration::ZERO);
        let mut request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        request.approval_token = Some(approval_token);

        let decision = gate.check(request, ApprovalChannel::FallbackOnly).await;

        assert!(matches!(decision, ApprovalDecision::Rejected { .. }));
    }

    #[tokio::test]
    async fn fallback_token_rejects_changed_digest() {
        let gate = DefaultWriteApprovalGate::new();
        let mut request = sample_request();
        let (_digest, approval_token) = require_approval(&gate, request.clone()).await;
        request.approval_token = Some(approval_token);
        request.preview.request = json!({
            "files": ["//depot/main/a.txt", "//depot/main/c.txt"],
            "force": true
        });

        let decision = gate.check(request, ApprovalChannel::FallbackOnly).await;

        assert!(matches!(decision, ApprovalDecision::Rejected { .. }));
    }

    #[test]
    fn digest_omits_approval_and_secret_fields() {
        let gate = DefaultWriteApprovalGate::new();
        let mut redacted = sample_request();
        redacted.preview.request = json!({
            "array": [{"path": "//depot/main/a.txt"}],
            "nested": {
                "keep": "stable"
            }
        });

        let mut with_secrets = redacted.clone();
        with_secrets.approval_token = Some("approval-token".to_string());
        with_secrets.preview.request = json!({
            "array": [{
                "approval_token": "nested-token",
                "path": "//depot/main/a.txt"
            }],
            "nested": {
                "Authorization": "Bearer secret",
                "P4PASSWD": "p4-password",
                "P4TICKETS": "/tmp/tickets",
                "authorization": "basic secret",
                "confirmation": true,
                "keep": "stable",
                "password": "password",
                "ticket": "ticket"
            }
        });

        assert_eq!(gate.digest(&with_secrets), gate.digest(&redacted));

        redacted.preview.request["nested"]["keep"] = json!("changed");
        assert_ne!(gate.digest(&with_secrets), gate.digest(&redacted));
    }
}
