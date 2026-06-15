use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::SslVerify;
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
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltReviewRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Value,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReviewApiConfig {
    pub api_base: String,
    pub username: String,
    pub ticket: String,
}

#[derive(Clone, PartialEq, Eq)]
struct TicketEntry {
    server: String,
    user: String,
    ticket: String,
}

impl ReviewApiConfig {
    pub fn from_p4(
        info_records: &[Value],
        property_records: &[Value],
        tickets_stdout: &str,
    ) -> Result<Self> {
        let username = first_non_empty_field(info_records, &["userName", "User", "user"])
            .ok_or_else(|| P4McpError::P4Command {
                message: "failed to determine current P4 user".to_string(),
            })?;
        let server = first_non_empty_field(info_records, &["serverAddress", "serverUri"]);
        let swarm_url = first_non_empty_field(property_records, &["value"]).ok_or_else(|| {
            P4McpError::P4Command {
                message: "Swarm URL not configured on the server".to_string(),
            }
        })?;
        let ticket = ticket_for_user(tickets_stdout, &username, server.as_deref())?;

        Ok(Self {
            api_base: format!("{}/api/v11", swarm_url.trim_end_matches('/')),
            username,
            ticket,
        })
    }
}

fn first_non_empty_field(records: &[Value], fields: &[&str]) -> Option<String> {
    records.iter().find_map(|record| {
        fields.iter().find_map(|field| {
            record
                .get(*field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    })
}

fn ticket_for_user(stdout: &str, username: &str, server: Option<&str>) -> Result<String> {
    let entries: Vec<TicketEntry> = stdout
        .lines()
        .filter_map(parse_ticket_line)
        .filter(|entry| entry.user == username)
        .collect();

    if let Some(server) = server {
        let exact_matches: Vec<&TicketEntry> = entries
            .iter()
            .filter(|entry| entry.server == server)
            .collect();
        match exact_matches.as_slice() {
            [entry] => return Ok(entry.ticket.clone()),
            [_, ..] => {
                return Err(P4McpError::P4Command {
                    message: format!(
                        "multiple P4 tickets found for user {username}; configure a matching P4PORT or run p4 login for the active server"
                    ),
                });
            }
            [] => {}
        }
    }

    match entries.as_slice() {
        [entry] => Ok(entry.ticket.clone()),
        [] => Err(P4McpError::P4Command {
            message: format!("No P4 ticket found for user {username}. Please run p4 login first."),
        }),
        _ => Err(P4McpError::P4Command {
            message: format!(
                "multiple P4 tickets found for user {username}; configure a matching P4PORT or run p4 login for the active server"
            ),
        }),
    }
}

fn parse_ticket_line(line: &str) -> Option<TicketEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (server, rest) = line.split_once(" (")?;
    let (user, ticket_and_suffix) = rest.split_once(") ")?;
    let ticket = ticket_and_suffix.split_whitespace().next()?;
    if server.is_empty() || user.is_empty() || ticket.is_empty() {
        return None;
    }

    Some(TicketEntry {
        server: server.to_string(),
        user: user.to_string(),
        ticket: ticket.to_string(),
    })
}

impl ReviewRequest {
    pub fn to_http(&self, _api_base: &str) -> Result<BuiltReviewRequest> {
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
        let ssl_verify = if accept_invalid_certs {
            SslVerify::Disabled
        } else {
            SslVerify::Enabled
        };
        Self::new_with_ssl_verify(api_base, username, ticket, &ssl_verify)
    }

    pub fn new_with_ssl_verify(
        api_base: String,
        username: String,
        ticket: String,
        ssl_verify: &SslVerify,
    ) -> anyhow::Result<Self> {
        let mut builder = reqwest::Client::builder();
        match ssl_verify {
            SslVerify::Enabled => {}
            SslVerify::Disabled => {
                builder = builder.danger_accept_invalid_certs(true);
            }
            SslVerify::CaBundle(path) => {
                builder = builder.add_root_certificate(load_ca_bundle(path)?);
            }
        }

        Ok(Self {
            client: builder.build()?,
            api_base: api_base.trim_end_matches('/').to_string(),
            username,
            ticket,
        })
    }

    pub async fn execute(&self, request: &ReviewRequest) -> anyhow::Result<Value> {
        let built = request.to_http(&self.api_base)?;
        if built.method != "GET" {
            anyhow::bail!(
                "review API {} request requires MCP write approval before execution",
                built.method
            );
        }
        self.send(built).await
    }

    pub async fn execute_approved(&self, request: &ReviewRequest) -> anyhow::Result<Value> {
        let built = request.to_http(&self.api_base)?;
        self.send(built).await
    }

    async fn send(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
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
            anyhow::bail!("review API returned HTTP {status}");
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text })))
    }
}

fn load_ca_bundle(path: &PathBuf) -> anyhow::Result<reqwest::Certificate> {
    let pem = std::fs::read(path)?;
    Ok(reqwest::Certificate::from_pem(&pem)?)
}
