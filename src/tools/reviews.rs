use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::SslVerify;
use crate::error::{P4McpError, Result};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewQueryAction {
    List,
    Dashboard,
    Get,
    Transitions,
    FilesReadby,
    Files,
    Comments,
    Activity,
}

impl ReviewQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Dashboard => "dashboard",
            Self::Get => "get",
            Self::Transitions => "transitions",
            Self::FilesReadby => "files_readby",
            Self::Files => "files",
            Self::Comments => "comments",
            Self::Activity => "activity",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryReviewsParams {
    pub action: ReviewQueryAction,
    #[serde(default)]
    pub review_id: Option<u64>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default = "default_comments_fields")]
    pub comments_fields: Option<String>,
    #[serde(default)]
    pub up_voters: Option<Vec<String>>,
    #[serde(default)]
    pub from_version: Option<u64>,
    #[serde(default)]
    pub to_version: Option<u64>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub after_updated: Option<String>,
    #[serde(default)]
    pub result_order: Option<String>,
    #[serde(default)]
    pub projects: Option<Vec<String>>,
    #[serde(default)]
    pub state: Option<Vec<String>>,
    #[serde(default)]
    pub keywords: Option<String>,
    #[serde(default)]
    pub keywords_fields: Option<Vec<String>>,
    #[serde(default)]
    pub include_transitions: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewModifyAction {
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

impl ReviewModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::RefreshProjects => "refresh_projects",
            Self::Vote => "vote",
            Self::Transition => "transition",
            Self::AppendParticipants => "append_participants",
            Self::AddComment => "add_comment",
            Self::ReplyComment => "reply_comment",
            Self::AppendChange => "append_change",
            Self::ReplaceWithChange => "replace_with_change",
            Self::Join => "join",
            Self::ArchiveInactive => "archive_inactive",
            Self::MarkCommentRead => "mark_comment_read",
            Self::MarkCommentUnread => "mark_comment_unread",
            Self::MarkAllCommentsRead => "mark_all_comments_read",
            Self::MarkAllCommentsUnread => "mark_all_comments_unread",
            Self::UpdateAuthor => "update_author",
            Self::UpdateDescription => "update_description",
            Self::ReplaceParticipants => "replace_participants",
            Self::DeleteParticipants => "delete_participants",
            Self::Leave => "leave",
            Self::Obliterate => "obliterate",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ModifyReviewsParams {
    pub action: ReviewModifyAction,
    #[serde(default)]
    pub review_id: Option<u64>,
    #[serde(default)]
    pub change_id: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub reviewers: Option<Vec<String>>,
    #[serde(default)]
    pub required_reviewers: Option<Vec<String>>,
    #[serde(default)]
    pub reviewer_group_names: Option<Vec<String>>,
    #[serde(default)]
    pub reviewer_groups_required: Option<Vec<String>>,
    #[serde(default)]
    pub comment_file_path: Option<String>,
    #[serde(default)]
    pub comment_left_line: Option<u64>,
    #[serde(default)]
    pub comment_right_line: Option<u64>,
    #[serde(default)]
    pub comment_version: Option<u64>,
    #[serde(default)]
    pub vote_value: Option<String>,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub transition: Option<String>,
    #[serde(default)]
    pub jobs: Option<Vec<String>>,
    #[serde(default)]
    pub fix_status: Option<String>,
    #[serde(default)]
    pub cleanup: Option<bool>,
    #[serde(default)]
    pub participant_user_names: Option<Vec<String>>,
    #[serde(default)]
    pub participant_users_required: Option<Vec<String>>,
    #[serde(default)]
    pub participant_group_names: Option<Vec<String>>,
    #[serde(default)]
    pub participant_groups_required: Option<Vec<String>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub task_state: Option<String>,
    #[serde(default)]
    pub notify: Option<String>,
    #[serde(default)]
    pub comment_id: Option<u64>,
    #[serde(default)]
    pub not_updated_since: Option<String>,
    #[serde(default)]
    pub max_reviews: u16,
    #[serde(default)]
    pub new_author: Option<String>,
    #[serde(default)]
    pub new_description: Option<String>,
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

impl QueryReviewsParams {
    pub fn to_http(&self) -> Result<BuiltReviewRequest> {
        let id = || required_id(self.review_id, "review_id");
        let built = match self.action {
            ReviewQueryAction::List => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews".into(),
                query: review_list_query(self),
                body: json!({}),
            },
            ReviewQueryAction::Dashboard => BuiltReviewRequest {
                method: "GET".into(),
                path: "/reviews/dashboard".into(),
                query: vec![("max".into(), self.max_results.to_string())],
                body: json!({}),
            },
            ReviewQueryAction::Get => {
                let mut request = get(format!("/reviews/{}", id()?));
                add_repeated_query(&mut request.query, "fields[]", self.fields.as_deref());
                if self.include_transitions == Some(true) {
                    request.query.push(("transitions".into(), "true".into()));
                }
                request
            }
            ReviewQueryAction::Transitions => get(format!("/reviews/{}/transitions", id()?)),
            ReviewQueryAction::FilesReadby => get(format!("/reviews/{}/files/readby", id()?)),
            ReviewQueryAction::Files => {
                let mut request = get(format!("/reviews/{}/files", id()?));
                if let Some(from) = self.from_version {
                    request.query.push(("from".into(), from.to_string()));
                }
                if let Some(to) = self.to_version {
                    request.query.push(("to".into(), to.to_string()));
                }
                request
            }
            ReviewQueryAction::Comments => get(format!("/reviews/{}/comments", id()?)),
            ReviewQueryAction::Activity => {
                let mut request = get(format!("/reviews/{}/activity", id()?));
                request
                    .query
                    .push(("max".into(), self.max_results.to_string()));
                request
            }
        };
        Ok(built)
    }
}

impl ModifyReviewsParams {
    pub fn to_http(&self, username: Option<&str>) -> Result<BuiltReviewRequest> {
        let review_id = || required_id(self.review_id, "review_id");
        let change_id = || required_id(self.change_id, "change_id");
        let built = match self.action {
            ReviewModifyAction::Create => {
                post("/reviews".into(), create_review_body(self, change_id()?))
            }
            ReviewModifyAction::RefreshProjects => post(
                format!("/reviews/{}/refreshProjects", review_id()?),
                json!({}),
            ),
            ReviewModifyAction::Vote => {
                post(format!("/reviews/{}/vote", review_id()?), vote_body(self)?)
            }
            ReviewModifyAction::Transition => post(
                format!("/reviews/{}/transitions", review_id()?),
                transition_body(self)?,
            ),
            ReviewModifyAction::AppendParticipants => post(
                format!("/reviews/{}/participants", review_id()?),
                participants_body(self),
            ),
            ReviewModifyAction::AddComment => {
                let mut request = post(
                    format!("/reviews/{}/comments", review_id()?),
                    comment_body(self, None)?,
                );
                push_optional(&mut request.query, "notify", self.notify.as_deref());
                request
            }
            ReviewModifyAction::ReplyComment => post(
                format!("/reviews/{}/comments", review_id()?),
                comment_body(self, self.comment_id)?,
            ),
            ReviewModifyAction::AppendChange => post(
                format!("/reviews/{}/appendchange", review_id()?),
                json!({"changeId": change_id()?}),
            ),
            ReviewModifyAction::ReplaceWithChange => post(
                format!("/reviews/{}/replacewithchange", review_id()?),
                json!({"changeId": change_id()?}),
            ),
            ReviewModifyAction::Join => post(
                format!("/reviews/{}/join", review_id()?),
                join_body(username),
            ),
            ReviewModifyAction::ArchiveInactive => {
                post("/reviews/archiveInactive".into(), archive_body(self)?)
            }
            ReviewModifyAction::MarkCommentRead => post(
                format!(
                    "/comments/{}/read",
                    required_id(self.comment_id, "comment_id")?
                ),
                json!({}),
            ),
            ReviewModifyAction::MarkCommentUnread => post(
                format!(
                    "/comments/{}/unread",
                    required_id(self.comment_id, "comment_id")?
                ),
                json!({}),
            ),
            ReviewModifyAction::MarkAllCommentsRead => post(
                format!("/reviews/{}/comments/read", review_id()?),
                json!({}),
            ),
            ReviewModifyAction::MarkAllCommentsUnread => post(
                format!("/reviews/{}/comments/unread", review_id()?),
                json!({}),
            ),
            ReviewModifyAction::UpdateAuthor => post_put(
                "PUT",
                format!("/reviews/{}/author", review_id()?),
                json!({"author": required_string(self.new_author.as_deref(), "new_author")?}),
            ),
            ReviewModifyAction::UpdateDescription => post_put(
                "PUT",
                format!("/reviews/{}/description", review_id()?),
                json!({"description": required_string(self.new_description.as_deref(), "new_description")?}),
            ),
            ReviewModifyAction::ReplaceParticipants => post_put(
                "PUT",
                format!("/reviews/{}/participants", review_id()?),
                participants_body(self),
            ),
            ReviewModifyAction::DeleteParticipants => delete(
                format!("/reviews/{}/participants", review_id()?),
                delete_participants_body(self),
            ),
            ReviewModifyAction::Leave => delete(
                format!("/reviews/{}/leave", review_id()?),
                join_body(username),
            ),
            ReviewModifyAction::Obliterate => {
                delete(format!("/reviews/{}", review_id()?), json!({}))
            }
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

fn post_put(method: &str, path: String, body: Value) -> BuiltReviewRequest {
    BuiltReviewRequest {
        method: method.into(),
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

fn default_comments_fields() -> Option<String> {
    Some("id,body,user,time".to_string())
}

fn required_id(value: Option<u64>, name: &str) -> Result<u64> {
    value.ok_or_else(|| P4McpError::InvalidInput {
        message: format!("{name} is required"),
    })
}

fn required_string(value: Option<&str>, name: &str) -> Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| P4McpError::InvalidInput {
            message: format!("{name} is required"),
        })
}

fn review_list_query(params: &QueryReviewsParams) -> Vec<(String, String)> {
    let mut query = vec![("max".into(), params.max_results.to_string())];
    push_optional(&mut query, "after", params.after.as_deref());
    push_optional(&mut query, "afterUpdated", params.after_updated.as_deref());
    push_optional(&mut query, "resultOrder", params.result_order.as_deref());
    push_optional(&mut query, "keywords", params.keywords.as_deref());
    add_repeated_query(&mut query, "project[]", params.projects.as_deref());
    add_repeated_query(&mut query, "state[]", params.state.as_deref());
    add_repeated_query(
        &mut query,
        "keywordsFields[]",
        params.keywords_fields.as_deref(),
    );
    add_repeated_query(&mut query, "fields[]", params.fields.as_deref());
    query
}

fn push_optional(query: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        query.push((name.to_string(), value.to_string()));
    }
}

fn add_repeated_query(query: &mut Vec<(String, String)>, name: &str, values: Option<&[String]>) {
    if let Some(values) = values {
        query.extend(values.iter().map(|value| (name.to_string(), value.clone())));
    }
}

fn create_review_body(params: &ModifyReviewsParams, change_id: u64) -> Value {
    let mut body = json!({"change": change_id});
    insert_string(&mut body, "description", params.description.as_deref());
    insert_array(&mut body, "reviewers", params.reviewers.as_deref());
    insert_array(
        &mut body,
        "requiredReviewers",
        params.required_reviewers.as_deref(),
    );
    if params.reviewer_group_names.is_some() || params.reviewer_groups_required.is_some() {
        body["reviewerGroups"] = reviewer_groups_body(params);
    }
    body
}

fn vote_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body = json!({"vote": required_string(params.vote_value.as_deref(), "vote_value")?});
    if let Some(version) = params.version {
        body["version"] = json!(version);
    }
    Ok(body)
}

fn transition_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body =
        json!({"transition": required_string(params.transition.as_deref(), "transition")?});
    if let Some(jobs) = &params.jobs {
        body["jobs"] = json!(jobs);
    }
    insert_string(&mut body, "fixStatus", params.fix_status.as_deref());
    if let Some(cleanup) = params.cleanup {
        body["cleanup"] = json!(cleanup);
    }
    Ok(body)
}

fn comment_body(params: &ModifyReviewsParams, parent_comment: Option<u64>) -> Result<Value> {
    let mut body = json!({"body": required_string(params.body.as_deref(), "body")?});
    let mut context = json!({});
    insert_string(&mut context, "file", params.comment_file_path.as_deref());
    if let Some(line) = params.comment_left_line {
        context["leftLine"] = json!(line);
    }
    if let Some(line) = params.comment_right_line {
        context["rightLine"] = json!(line);
    }
    if let Some(version) = params.comment_version {
        context["version"] = json!(version);
    }
    if let Some(comment) = parent_comment {
        context["comment"] = json!(comment);
    }
    if context.as_object().is_some_and(|object| !object.is_empty()) {
        body["context"] = context;
    }
    insert_string(&mut body, "taskState", params.task_state.as_deref());
    Ok(body)
}

fn archive_body(params: &ModifyReviewsParams) -> Result<Value> {
    let mut body = json!({
        "notUpdatedSince": required_string(params.not_updated_since.as_deref(), "not_updated_since")?,
        "description": params.description.as_deref().unwrap_or("Archiving inactive reviews"),
    });
    if params.max_reviews > 0 {
        body["max"] = json!(params.max_reviews);
    }
    Ok(body)
}

fn participants_body(params: &ModifyReviewsParams) -> Value {
    json!({
        "participants": {
            "users": participant_users(params),
            "groups": participant_groups(params)
        }
    })
}

fn delete_participants_body(params: &ModifyReviewsParams) -> Value {
    let mut participants = serde_json::Map::new();
    let users = empty_array_participants(
        params
            .participant_user_names
            .iter()
            .chain(params.participant_users_required.iter()),
    );
    if !users.is_empty() {
        participants.insert("users".to_string(), Value::Object(users));
    }

    let groups = empty_array_participants(
        params
            .participant_group_names
            .iter()
            .chain(params.participant_groups_required.iter()),
    );
    if !groups.is_empty() {
        participants.insert("groups".to_string(), Value::Object(groups));
    }

    let mut body = serde_json::Map::new();
    body.insert("participants".to_string(), Value::Object(participants));
    Value::Object(body)
}

fn empty_array_participants<'a>(
    groups: impl Iterator<Item = &'a Vec<String>>,
) -> serde_json::Map<String, Value> {
    let mut entries = serde_json::Map::new();
    for names in groups {
        for name in names {
            entries.insert(name.clone(), json!([]));
        }
    }
    entries
}

fn participant_users(params: &ModifyReviewsParams) -> Value {
    let mut users = serde_json::Map::new();
    if let Some(names) = &params.participant_user_names {
        for name in names {
            users.insert(name.clone(), json!({"required": "no"}));
        }
    }
    if let Some(names) = &params.participant_users_required {
        for name in names {
            users.insert(name.clone(), json!({"required": "yes"}));
        }
    }
    Value::Object(users)
}

fn participant_groups(params: &ModifyReviewsParams) -> Value {
    let mut groups = serde_json::Map::new();
    if let Some(names) = &params.participant_group_names {
        for name in names {
            groups.insert(name.clone(), json!({"required": "none"}));
        }
    }
    if let Some(names) = &params.participant_groups_required {
        for name in names {
            groups.insert(name.clone(), json!({"required": "all"}));
        }
    }
    Value::Object(groups)
}

fn reviewer_groups_body(params: &ModifyReviewsParams) -> Value {
    let mut groups = Vec::new();
    if let Some(names) = &params.reviewer_group_names {
        groups.extend(
            names
                .iter()
                .map(|name| json!({"name": name, "required": "false"})),
        );
    }
    if let Some(names) = &params.reviewer_groups_required {
        groups.extend(
            names
                .iter()
                .map(|name| json!({"name": name, "required": "true"})),
        );
    }
    Value::Array(groups)
}

fn insert_string(body: &mut Value, field: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        body[field] = json!(value);
    }
}

fn insert_array(body: &mut Value, field: &str, value: Option<&[String]>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        body[field] = json!(value);
    }
}

fn join_body(username: Option<&str>) -> Value {
    match username {
        Some(username) if !username.trim().is_empty() => {
            let mut users = serde_json::Map::new();
            users.insert(username.to_string(), json!([]));
            let mut participants = serde_json::Map::new();
            participants.insert("users".to_string(), Value::Object(users));
            let mut body = serde_json::Map::new();
            body.insert("participants".to_string(), Value::Object(participants));
            Value::Object(body)
        }
        _ => json!({}),
    }
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

    pub fn username(&self) -> &str {
        &self.username
    }

    pub async fn execute(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
        if built.method != "GET" {
            anyhow::bail!(
                "review API {} request requires MCP write approval before execution",
                built.method
            );
        }
        self.send(built).await
    }

    pub async fn execute_approved(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
        self.send(built).await
    }

    async fn send(&self, built: BuiltReviewRequest) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.api_base, built.path);
        let mut req = match built.method.as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url).json(&built.body),
            "PUT" => self.client.put(url).json(&built.body),
            "DELETE" => self.client.delete(url).json(&built.body),
            method => anyhow::bail!("unsupported review HTTP method: {method}"),
        };
        if !built.query.is_empty() {
            req = req.query(&built.query);
        }
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
