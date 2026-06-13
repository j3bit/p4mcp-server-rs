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
    #[serde(default = "default_body")]
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
        if self.action == ReviewAction::Obliterate
            && self.body.get("confirmation").and_then(Value::as_str) != Some("PROCEED")
        {
            return Err(P4McpError::ConfirmationRequired);
        }

        let id = || {
            self.review_id.ok_or_else(|| P4McpError::InvalidInput {
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
            ReviewAction::RefreshProjects => {
                post(format!("/reviews/{}/refreshProjects", id()?), json!({}))
            }
            ReviewAction::Vote => post(format!("/reviews/{}/vote", id()?), self.body.clone()),
            ReviewAction::Transition => {
                post(format!("/reviews/{}/transitions", id()?), self.body.clone())
            }
            ReviewAction::AppendParticipants => post(
                format!("/reviews/{}/participants", id()?),
                self.body.clone(),
            ),
            ReviewAction::AddComment | ReviewAction::ReplyComment => {
                post(format!("/reviews/{}/comments", id()?), self.body.clone())
            }
            ReviewAction::AppendChange => post(
                format!("/reviews/{}/appendchange", id()?),
                self.body.clone(),
            ),
            ReviewAction::ReplaceWithChange => post(
                format!("/reviews/{}/replacewithchange", id()?),
                self.body.clone(),
            ),
            ReviewAction::Join => post(format!("/reviews/{}/join", id()?), self.body.clone()),
            ReviewAction::ArchiveInactive => {
                post("/reviews/archiveInactive".to_string(), self.body.clone())
            }
            ReviewAction::MarkCommentRead => post(format!("/comments/{}/read", id()?), json!({})),
            ReviewAction::MarkCommentUnread => {
                post(format!("/comments/{}/unread", id()?), json!({}))
            }
            ReviewAction::MarkAllCommentsRead => {
                post(format!("/reviews/{}/comments/read", id()?), json!({}))
            }
            ReviewAction::MarkAllCommentsUnread => {
                post(format!("/reviews/{}/comments/unread", id()?), json!({}))
            }
            ReviewAction::UpdateAuthor => {
                put(format!("/reviews/{}/author", id()?), self.body.clone())
            }
            ReviewAction::UpdateDescription => {
                put(format!("/reviews/{}/description", id()?), self.body.clone())
            }
            ReviewAction::ReplaceParticipants => put(
                format!("/reviews/{}/participants", id()?),
                self.body.clone(),
            ),
            ReviewAction::DeleteParticipants => delete(
                format!("/reviews/{}/participants", id()?),
                self.body.clone(),
            ),
            ReviewAction::Leave => delete(format!("/reviews/{}/leave", id()?), self.body.clone()),
            ReviewAction::Obliterate => delete(format!("/reviews/{}", id()?), json!({})),
        };
        Ok(built)
    }
}

fn get(path: String) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: "GET".into(),
        path,
        query: Vec::new(),
        body: json!({}),
    }
}

fn post(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: "POST".into(),
        path,
        query: Vec::new(),
        body,
    }
}

fn put(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: "PUT".into(),
        path,
        query: Vec::new(),
        body,
    }
}

fn delete(path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: "DELETE".into(),
        path,
        query: Vec::new(),
        body,
    }
}

fn default_max_results() -> u16 {
    10
}

fn default_body() -> Value {
    json!({})
}

#[derive(Clone)]
pub struct ReviewHttpClient {
    client: reqwest::Client,
    api_base: String,
    username: String,
    ticket: String,
}

impl ReviewHttpClient {
    pub fn new(
        api_base: String,
        username: String,
        ticket: String,
        accept_invalid_certs: bool,
    ) -> anyhow::Result<Self> {
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
