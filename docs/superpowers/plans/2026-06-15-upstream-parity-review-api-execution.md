# Upstream Parity Review API Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `query_reviews` and approved `modify_reviews` execute the Swarm Review API instead of returning dry-run request metadata.

**Architecture:** Keep the Rust port aligned with the README baseline: `perforce/p4mcp-server` `v2026.2.2955897` at commit `a64efb07511b2a62db41aeed110ab96744c4076a`. The upstream Python server resolves Swarm configuration through P4, then executes `requests.get/post/put/delete`; the Rust port should mirror that with direct `p4` CLI calls to discover `P4.Swarm.URL`, current user, and ticket, then use `ReviewHttpClient` for HTTP execution. Preserve the approved Rust write approval gate: `modify_reviews` must build a side-effect-free approval preview first, and only after approval may it run P4 discovery commands or send HTTP writes.

**Tech Stack:** Rust 2024, `rmcp`, existing `P4Executor`, `reqwest`, `serde_json`, `wiremock`, `tokio`, direct `p4` CLI invocations.

---

## Review Thread

- PR: `https://github.com/j3bit/p4mcp-server-rs/pull/2`
- Thread: `PRRT_kwDOS5FH5M6JcdM0`
- Comment: `PRRC_kwDOS5FH5M7LSMRp`
- Reviewer summary: `query_reviews` and approved `modify_reviews` are advertised as supported, but only return constructed method/path/body metadata with status `dry_run`.
- Decision: true upstream-parity issue. Do not remove the review tools. Wire them to execute the built Review API requests.

## Upstream Behavior To Preserve

- `p4mcp/tools/review_tools.py` registers `query_reviews` when `"reviews"` is in `server.toolsets`.
- `query_reviews` calls `handle_with_logging(server, "query", "reviews", params, "query_reviews", ctx)`.
- `p4mcp/handlers/review_handlers.py` dispatches query actions to `ReviewServices.list_reviews`, `review_dashboard`, `get_review_info`, `get_review_files_readby`, `get_review_files`, `get_review_activity`, and `get_review_comments`.
- `modify_reviews` is not registered when upstream runs readonly, and otherwise calls `handle_modify_with_delete_gate(...)`.
- `p4mcp/services/review_services.py` resolves:
  - auth from the active P4 connection user and ticket
  - API base from `p4 property -l -n P4.Swarm.URL`
  - Swarm endpoint base as `<swarm_url>/api/v11`
- The Rust port keeps `modify_reviews` registered and blocks writes through `SafetyPolicy` plus write approval. This is an approved Rust-port safety extension and must remain.

## File Structure

- Modify `src/tools/reviews.rs`
  - Keep Review API request builders.
  - Add `ReviewApiConfig` and parsing helpers for P4 CLI discovery output.
  - Add `ReviewHttpClient::execute_approved` for server-side approved writes.
  - Keep `ReviewHttpClient::execute` as the read-only guarded entry point so direct client use cannot send write HTTP methods accidentally.
  - Add `ReviewHttpClient::new_with_ssl_verify` so existing `SslVerify` config applies to review API execution.

- Modify `src/server.rs`
  - Add `review_http_client_from_p4`.
  - Add `text_invocation`.
  - Change `query_reviews` to discover Review API config and execute GET requests.
  - Change `modify_reviews_inner` to discover Review API config and execute the write request only after `require_write_approval` returns approved.
  - Map Review API execution errors to MCP internal errors without exposing credentials.
  - Update unit tests for approved review writes.

- Modify `tests/review_client_tests.rs`
  - Add parser coverage for Swarm URL, current user, exact ticket match, single user fallback, and ambiguous ticket rejection.
  - Add `execute_approved` coverage for POST execution.

- Modify `tests/mcp_smoke_tests.rs`
  - Replace the dry-run query smoke test with a real Review API execution smoke test using `wiremock` and the injected P4 executor.

- No README change is required. README already documents the upstream baseline and the write approval gate delta.

## Constraints

- Do not reintroduce `confirmation`.
- Do not execute any P4 command or HTTP write before `modify_reviews` approval succeeds.
- Do not remove `query_reviews` or `modify_reviews`.
- Do not expand this fix into a full review parameter-schema redesign. The current Rust `ReviewRequest` body-based mapping remains the request shape for this PR.
- Do not put ticket values, Authorization headers, or base64 credentials in errors, logs, approval previews, or tool responses.
- Keep `query_reviews` from executing non-GET requests even though the shared `ReviewRequest` enum can build write requests.
- Keep `modify_reviews` from executing GET requests; callers should use `query_reviews` for read actions.

---

### Task 1: Add Review API Discovery Parsing

**Files:**
- Modify: `src/tools/reviews.rs`
- Test: `tests/review_client_tests.rs`

- [ ] **Step 1: Add failing parser tests**

Append these tests to `tests/review_client_tests.rs` after `missing_body_deserializes_to_empty_object`:

```rust
#[test]
fn review_api_config_uses_swarm_property_and_matching_ticket() {
    let config = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com/"
        })],
        "perforce:1666 (alice) ticket-123\nother:1666 (alice) wrong-ticket\n",
    )
    .unwrap();

    assert_eq!(config.api_base, "https://swarm.example.com/api/v11");
    assert_eq!(config.username, "alice");
    assert_eq!(config.ticket, "ticket-123");
}

#[test]
fn review_api_config_uses_single_user_ticket_when_server_address_is_absent() {
    let config = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "perforce:1666 (alice) ticket-123\nother:1666 (bob) other-ticket\n",
    )
    .unwrap();

    assert_eq!(config.api_base, "https://swarm.example.com/api/v11");
    assert_eq!(config.username, "alice");
    assert_eq!(config.ticket, "ticket-123");
}

#[test]
fn review_api_config_rejects_ambiguous_user_tickets() {
    let error = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "perforce:1666 (alice) ticket-123\nother:1666 (alice) other-ticket\n",
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("multiple P4 tickets found for user alice"));
    assert!(!error.contains("ticket-123"));
    assert!(!error.contains("other-ticket"));
}

#[test]
fn review_api_config_requires_swarm_url_property() {
    let error = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[],
        "perforce:1666 (alice) ticket-123\n",
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("Swarm URL not configured"));
}
```

- [ ] **Step 2: Run parser tests and verify failure**

Run:

```bash
rtk cargo test --test review_client_tests review_api_config -- --nocapture
```

Expected: FAIL because `ReviewApiConfig` does not exist.

- [ ] **Step 3: Add `ReviewApiConfig` and parsers**

In `src/tools/reviews.rs`, add this code after `BuiltReviewRequest`:

```rust
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
        let swarm_url = first_non_empty_field(property_records, &["value"])
            .ok_or_else(|| P4McpError::P4Command {
                message: "Swarm URL not configured on the server".to_string(),
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
        if let Some(entry) = entries.iter().find(|entry| entry.server == server) {
            return Ok(entry.ticket.clone());
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
```

- [ ] **Step 4: Run parser tests and verify they pass**

Run:

```bash
rtk cargo test --test review_client_tests review_api_config -- --nocapture
```

Expected: PASS for the four `review_api_config_*` tests.

- [ ] **Step 5: Commit parser work**

Run:

```bash
rtk git add src/tools/reviews.rs tests/review_client_tests.rs
rtk git commit -m "fix: parse review api config from p4"
```

Expected: commit succeeds.

---

### Task 2: Allow Approved Review Writes In The HTTP Client

**Files:**
- Modify: `src/tools/reviews.rs`
- Test: `tests/review_client_tests.rs`

- [ ] **Step 1: Add failing HTTP client write test**

In `tests/review_client_tests.rs`, add `body_json` to the matcher imports:

```rust
use wiremock::matchers::{body_json, header, method, path, query_param};
```

Append this test after `execute_vote_rejects_write_without_approval`:

```rust
#[tokio::test]
async fn execute_approved_vote_sends_post_with_body_and_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v11/reviews/123/vote"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .and(body_json(serde_json::json!({"vote": "up", "version": 2})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "vote": "recorded"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = ReviewHttpClient::new(
        format!("{}/api/v11", server.uri()),
        "user".into(),
        "ticket".into(),
        false,
    )
    .unwrap();
    let request = ReviewRequest {
        action: ReviewAction::Vote,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"vote": "up", "version": 2}),
        approval_token: Some("approved-token".to_string()),
    };

    let result = client.execute_approved(&request).await.unwrap();

    assert_eq!(result, serde_json::json!({ "vote": "recorded" }));
}
```

- [ ] **Step 2: Run the new client test and verify failure**

Run:

```bash
rtk cargo test --test review_client_tests execute_approved_vote_sends_post_with_body_and_basic_auth -- --nocapture
```

Expected: FAIL because `ReviewHttpClient::execute_approved` does not exist.

- [ ] **Step 3: Add `execute_approved` and shared HTTP send path**

In `src/tools/reviews.rs`, add these imports near the existing imports:

```rust
use std::path::PathBuf;

use crate::config::SslVerify;
```

In `src/tools/reviews.rs`, replace the existing `impl ReviewHttpClient` block with this version:

```rust
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
            anyhow::bail!("review API returned HTTP {status}: {text}");
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text })))
    }
}

fn load_ca_bundle(path: &PathBuf) -> anyhow::Result<reqwest::Certificate> {
    let pem = std::fs::read(path)?;
    Ok(reqwest::Certificate::from_pem(&pem)?)
}
```

- [ ] **Step 4: Run review client tests**

Run:

```bash
rtk cargo test --test review_client_tests -- --nocapture
```

Expected: PASS. Existing `execute_vote_rejects_write_without_approval` must still pass, proving direct `execute` remains read-only guarded.

- [ ] **Step 5: Commit HTTP client work**

Run:

```bash
rtk git add src/tools/reviews.rs tests/review_client_tests.rs
rtk git commit -m "fix: execute approved review writes"
```

Expected: commit succeeds.

---

### Task 3: Execute `query_reviews`

**Files:**
- Modify: `src/server.rs`
- Test: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add failing query execution smoke test**

In `tests/mcp_smoke_tests.rs`, update the imports:

```rust
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
```

Replace the existing `query_reviews_returns_dry_run_request_metadata` test with:

```rust
#[tokio::test]
async fn query_reviews_executes_review_api_request() {
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(header("authorization", "Basic YWxpY2U6dGlja2V0LTEyMw=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reviews": [123]
        })))
        .expect(1)
        .mount(&swarm)
        .await;

    let executor = Arc::new(QueuedExecutor::success(vec![
        P4CommandOutput {
            records: vec![json!({
                "userName": "alice",
                "serverAddress": "perforce:1666"
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: vec![json!({
                "value": swarm.uri()
            })],
            text: json!({}),
        },
        P4CommandOutput {
            records: Vec::new(),
            text: json!({
                "stdout": "perforce:1666 (alice) ticket-123\n",
                "stderr": ""
            }),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_reviews(Parameters(ReviewRequest {
            action: ReviewAction::List,
            review_id: None,
            max_results: 5,
            body: json!({}),
            approval_token: None,
        }))
        .await
        .unwrap();

    assert_eq!(response.0.status, "success");
    assert_eq!(response.0.action, "list");
    assert_eq!(response.0.message, json!({ "reviews": [123] }));

    let invocations = executor.invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].args, ["info"]);
    assert_eq!(
        invocations[1].args,
        ["property", "-l", "-n", "P4.Swarm.URL"]
    );
    assert_eq!(invocations[2].args, ["tickets"]);
}
```

- [ ] **Step 2: Run query smoke test and verify failure**

Run:

```bash
rtk cargo test --test mcp_smoke_tests query_reviews_executes_review_api_request -- --nocapture
```

Expected: FAIL because `query_reviews` still returns `dry_run` and does not call the injected executor or Swarm mock.

- [ ] **Step 3: Import Review API types in server**

In `src/server.rs`, change the review import:

```rust
reviews::{BuiltReviewRequest, ReviewApiConfig, ReviewHttpClient, ReviewRequest},
```

- [ ] **Step 4: Add review client discovery helpers**

In `src/server.rs`, add this method inside `impl P4McpServer`, immediately after `call_p4_tool`:

```rust
    async fn review_http_client_from_p4(&self) -> McpResult<ReviewHttpClient> {
        let info = self
            .run_p4(json_invocation(vec!["info".to_string()], None))
            .await?;
        let swarm_property = self
            .run_p4(json_invocation(
                vec![
                    "property".to_string(),
                    "-l".to_string(),
                    "-n".to_string(),
                    "P4.Swarm.URL".to_string(),
                ],
                None,
            ))
            .await?;
        let tickets = self
            .run_p4(text_invocation(vec!["tickets".to_string()]))
            .await?;
        let tickets_stdout = tickets
            .text
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let api_config =
            ReviewApiConfig::from_p4(&info.records, &swarm_property.records, tickets_stdout)
                .map_err(to_mcp_error)?;

        ReviewHttpClient::new_with_ssl_verify(
            api_config.api_base,
            api_config.username,
            api_config.ticket,
            &self.config.ssl_verify,
        )
        .map_err(review_api_error)
    }
```

Near `json_invocation`, add:

```rust
fn text_invocation(args: Vec<String>) -> P4Invocation {
    P4Invocation {
        args,
        stdin: None,
        mode: OutputMode::Text,
    }
}
```

Near `to_mcp_error`, add:

```rust
fn review_api_error(error: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("review API request failed: {error}"), None)
}
```

- [ ] **Step 5: Change `query_reviews` to execute GET requests**

In `src/server.rs`, replace the body of `query_reviews` with:

```rust
        self.policy()
            .check(Access::Read, Toolset::Reviews, "query_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        if built.method != "GET" {
            return Err(to_mcp_error(invalid_input(
                "query_reviews only supports read review actions",
            )));
        }
        let action = review_action_name(&params);
        let client = self.review_http_client_from_p4().await?;
        let message = client.execute(&params).await.map_err(review_api_error)?;
        Ok(Json(ToolResponse::success(&action, message)))
```

- [ ] **Step 6: Run query smoke test and verify it passes**

Run:

```bash
rtk cargo test --test mcp_smoke_tests query_reviews_executes_review_api_request -- --nocapture
```

Expected: PASS. The Swarm mock must receive one GET request, and the executor must record `info`, `property -l -n P4.Swarm.URL`, and `tickets`.

- [ ] **Step 7: Add query guard test**

Append this test after `query_reviews_executes_review_api_request` in `tests/mcp_smoke_tests.rs`:

```rust
#[tokio::test]
async fn query_reviews_rejects_write_actions_before_p4_discovery() {
    let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
        records: Vec::new(),
        text: json!({}),
    }));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let err = match server
        .query_reviews(Parameters(ReviewRequest {
            action: ReviewAction::Vote,
            review_id: Some(123),
            max_results: 10,
            body: json!({"vote": "up"}),
            approval_token: None,
        }))
        .await
    {
        Ok(_) => panic!("query_reviews should reject write review actions"),
        Err(err) => err,
    };

    assert_eq!(err.code, ErrorData::invalid_params("", None).code);
    assert!(err.message.contains("query_reviews only supports read review actions"));
    assert!(executor.invocations().is_empty());
}
```

- [ ] **Step 8: Run query review smoke tests**

Run:

```bash
rtk cargo test --test mcp_smoke_tests query_reviews -- --nocapture
```

Expected: PASS for `query_reviews_executes_review_api_request` and `query_reviews_rejects_write_actions_before_p4_discovery`.

- [ ] **Step 9: Commit query execution**

Run:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: execute review query api requests"
```

Expected: commit succeeds.

---

### Task 4: Execute Approved `modify_reviews`

**Files:**
- Modify: `src/server.rs`
- Test: `src/server.rs`

- [ ] **Step 1: Add test imports**

Inside the `#[cfg(test)] mod tests` in `src/server.rs`, update imports:

```rust
use std::{
    collections::VecDeque,
    io::Write,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
};

use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
```

Keep existing imports that are still used by the test module.

- [ ] **Step 2: Add queued test executor**

Inside `#[cfg(test)] mod tests`, add this executor before the existing `FakeExecutor`:

```rust
    struct QueuedExecutor {
        outputs: Mutex<VecDeque<crate::error::Result<P4CommandOutput>>>,
        invocations: Mutex<Vec<P4Invocation>>,
    }

    impl QueuedExecutor {
        fn success(outputs: Vec<P4CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
                invocations: Mutex::new(Vec::new()),
            }
        }

        fn invocations(&self) -> Vec<P4Invocation> {
            self.invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl P4Executor for QueuedExecutor {
        async fn run(
            &self,
            invocation: P4Invocation,
            _env: P4Env,
        ) -> crate::error::Result<P4CommandOutput> {
            self.invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(invocation);

            self.outputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .unwrap_or_else(|| {
                    Err(P4McpError::P4Command {
                        message: "queued executor exhausted".to_string(),
                    })
                })
        }
    }
```

- [ ] **Step 3: Replace approved modify dry-run test**

Replace `modify_reviews_after_approval_returns_request_metadata` with:

```rust
    #[tokio::test]
    async fn modify_reviews_after_approval_executes_review_api_request() {
        let swarm = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v11/reviews/123/vote"))
            .and(header("authorization", "Basic YWxpY2U6dGlja2V0LTEyMw=="))
            .and(body_json(json!({"vote": "up", "version": 2})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "vote": "recorded"
            })))
            .expect(1)
            .mount(&swarm)
            .await;

        let executor = Arc::new(QueuedExecutor::success(vec![
            P4CommandOutput {
                records: vec![json!({
                    "userName": "alice",
                    "serverAddress": "perforce:1666"
                })],
                text: json!({}),
            },
            P4CommandOutput {
                records: vec![json!({
                    "value": swarm.uri()
                })],
                text: json!({}),
            },
            P4CommandOutput {
                records: Vec::new(),
                text: json!({
                    "stdout": "perforce:1666 (alice) ticket-123\n",
                    "stderr": ""
                }),
            },
        ]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let response = server
            .modify_reviews_inner(
                review_modify_params(Some("approved-token")),
                ApprovalChannel::FallbackOnly,
            )
            .await
            .expect("approved review write should execute");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "vote");
        assert_eq!(response.0.message, json!({"vote": "recorded"}));

        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 3);
        assert_eq!(invocations[0].args, ["info"]);
        assert_eq!(
            invocations[1].args,
            ["property", "-l", "-n", "P4.Swarm.URL"]
        );
        assert_eq!(invocations[2].args, ["tickets"]);

        let calls = approval_gate.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].approval_token.as_deref(), Some("approved-token"));
        assert_eq!(calls[0].request.params["approval_token"], json!(null));
    }
```

The existing `modify_reviews_without_approval_does_not_return_write_dry_run` must remain and must still assert that the executor receives no invocation before approval.

- [ ] **Step 4: Add modify GET guard test**

Add this test after `modify_reviews_approval_preview_uses_method_and_path`:

```rust
    #[tokio::test]
    async fn modify_reviews_rejects_read_actions_after_policy_before_approval() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );

        let err = match server
            .modify_reviews_inner(
                ReviewRequest {
                    action: ReviewAction::List,
                    review_id: None,
                    max_results: 10,
                    body: json!({}),
                    approval_token: Some("approved-token".to_string()),
                },
                ApprovalChannel::FallbackOnly,
            )
            .await
        {
            Ok(_) => panic!("modify_reviews should reject read review actions"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("modify_reviews only supports write review actions"));
        assert!(approval_gate.calls().is_empty());
        assert!(executor.invocations().is_empty());
    }
```

- [ ] **Step 5: Run modify review tests and verify failure**

Run:

```bash
rtk cargo test server::tests::modify_reviews -- --nocapture
```

Expected: FAIL because `modify_reviews_inner` still returns dry-run metadata after approval and does not execute the Review API request.

- [ ] **Step 6: Change `modify_reviews_inner` to execute after approval**

In `src/server.rs`, replace the body of `modify_reviews_inner` with:

```rust
        self.policy()
            .check(Access::Write, Toolset::Reviews, "modify_reviews")
            .map_err(to_mcp_error)?;
        let built = params.to_http("unused").map_err(to_mcp_error)?;
        if built.method == "GET" {
            return Err(to_mcp_error(invalid_input(
                "modify_reviews only supports write review actions",
            )));
        }
        let request = self.modify_reviews_approval_request(&params, &built);
        if let Some(response) = self
            .require_write_approval(channel, request, params.approval_token.as_deref())
            .await?
        {
            return Ok(response);
        }
        let action = review_action_name(&params);
        let client = self.review_http_client_from_p4().await?;
        let message = client
            .execute_approved(&params)
            .await
            .map_err(review_api_error)?;
        Ok(Json(ToolResponse::success(&action, message)))
```

- [ ] **Step 7: Run modify review tests and verify pass**

Run:

```bash
rtk cargo test server::tests::modify_reviews -- --nocapture
```

Expected: PASS. The unapproved write test must prove no P4 executor calls occur before approval. The approved write test must prove the Swarm mock receives one POST request.

- [ ] **Step 8: Commit modify execution**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: execute approved review modify api requests"
```

Expected: commit succeeds.

---

### Task 5: Full Verification And PR Review Follow-Up

**Files:**
- Inspect: `src/tools/reviews.rs`
- Inspect: `src/server.rs`
- Inspect: `tests/review_client_tests.rs`
- Inspect: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS. If it fails, run `rtk cargo fmt`, inspect the diff, then rerun `rtk cargo fmt --check`.

- [ ] **Step 2: Clippy**

Run:

```bash
rtk cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Focused tests**

Run:

```bash
rtk cargo test --test review_client_tests -- --nocapture
rtk cargo test --test mcp_smoke_tests query_reviews -- --nocapture
rtk cargo test server::tests::modify_reviews -- --nocapture
```

Expected: all focused tests PASS.

- [ ] **Step 4: Full test suite**

Run:

```bash
rtk cargo test
```

Expected: PASS. If sandboxed execution fails only because `wiremock` cannot bind a local port, rerun the same command with elevated permissions and record that the sandbox failure was a local bind restriction.

- [ ] **Step 5: Inspect final diff**

Run:

```bash
rtk git diff --stat origin/main...HEAD
rtk git diff -- src/tools/reviews.rs src/server.rs tests/review_client_tests.rs tests/mcp_smoke_tests.rs
```

Expected:

- Review tools still registered.
- `query_reviews` no longer returns `ToolResponse::dry_run`.
- `modify_reviews_inner` no longer returns dry-run metadata after approval.
- `modify_reviews_inner` does not call `review_http_client_from_p4` until after approval.
- No response includes raw ticket values or Authorization headers.

- [ ] **Step 6: Push**

Run:

```bash
rtk git status --short --branch
rtk git push
```

Expected: branch pushes cleanly to the PR branch.

- [ ] **Step 7: Reply to the unresolved review thread**

Run this GitHub GraphQL mutation:

```bash
rtk gh api graphql \
  -f thread='PRRT_kwDOS5FH5M6JcdM0' \
  -f body=$'Fixed in the latest push.\n\n`query_reviews` now executes the built Swarm Review API GET request instead of returning dry-run metadata. The Rust port discovers the Review API base from `p4 property -l -n P4.Swarm.URL`, derives the active P4 user from `p4 info`, reads the matching ticket from `p4 tickets`, and sends the request through `ReviewHttpClient`.\n\nApproved `modify_reviews` now follows the same upstream execution path after the Rust write approval gate succeeds. The handler still builds the approval preview without P4 or HTTP side effects, rejects read actions on the modify path, and only performs P4 discovery plus POST/PUT/DELETE execution after approval. I added focused parser, HTTP client, query smoke, and approved-write tests.' \
  -f query='mutation($thread:ID!, $body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread, body:$body}){comment{id}}}'
```

Expected: GitHub returns a new review comment id under the existing thread.

- [ ] **Step 8: Resolve the review thread**

Run:

```bash
rtk gh api graphql \
  -f thread='PRRT_kwDOS5FH5M6JcdM0' \
  -f query='mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}'
```

Expected: response includes `"isResolved": true`.

- [ ] **Step 9: Confirm no unresolved review threads remain**

Run:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected: thread `PRRT_kwDOS5FH5M6JcdM0` has `isResolved: true`, or there are no unresolved actionable review threads.

---

## Self-Review

- Spec coverage: Task 1 covers P4 CLI discovery for the upstream Swarm URL/user/ticket flow. Task 2 covers the Review HTTP client execution path while keeping the direct client write guard. Task 3 fixes `query_reviews` dry-run behavior. Task 4 fixes approved `modify_reviews` dry-run behavior while preserving the write approval gate. Task 5 covers verification, push, GitHub reply, and resolve.
- Placeholder scan: The plan contains no `TBD`, no missing test bodies, no unnamed validation, and no unowned broad refactor instruction.
- Type consistency: The plan consistently uses the existing `ReviewRequest`, `ReviewAction`, `ReviewHttpClient`, `ReviewApiConfig`, `P4McpServer`, `P4Invocation`, `OutputMode::Text`, and `ToolResponse` names. `execute` remains read-only guarded; `execute_approved` is the only client path used after server-side approval.
