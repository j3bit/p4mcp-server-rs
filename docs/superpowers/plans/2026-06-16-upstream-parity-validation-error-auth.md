# Upstream Parity Validation Error Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the root causes behind the three open PR review comments by aligning changelist write validation, P4 benign-error normalization, and Review API credential discovery with upstream `perforce/p4mcp-server`.

**Architecture:** Keep the Rust port's direct `p4` CLI backend and write approval gate. Move upstream-required validation before approval, normalize command-specific benign P4 failures at the server workflow boundary, and model Review API credentials as "matching ticket file entry or configured P4 password/ticket" instead of ticket-file-only auth.

**Tech Stack:** Rust 2024, `rmcp`, `serde_json`, `reqwest`, `wiremock`, `tempfile`, local `p4` CLI semantics, upstream baseline `perforce/p4mcp-server` `v2026.2.2955897` commit `a64efb07511b2a62db41aeed110ab96744c4076a`.

---

## Reference Context

Upstream source of truth:
- `/private/tmp/p4mcp-upstream-a64/p4mcp/handlers/changelist_handlers.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/changelist_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/file_services.py`
- `/private/tmp/p4mcp-upstream-a64/p4mcp/services/review_services.py`

Verified upstream behavior:
- `modify_changelists`: upstream rejects missing `description` for both `create` and `update` before calling services.
- `sync_files`: upstream converts P4Exception containing `File(s) up-to-date` into success with message `Workspace is already up-to-date`.
- Review API auth: upstream uses `p4.password` from the active P4Python connection. In the CLI port, the equivalent credential is a matching ticket file entry or a configured `P4PASSWD` value. `p4 help tickets` says `p4 tickets` only lists tickets granted by `p4 login`; `p4 help environment` lists `P4PASSWD` and `P4CONFIG` as P4 client credential/config sources.

Design constraints:
- Do not fetch or patch P4 forms before approval.
- Do not use approval preview placeholders as real execution input.
- Do not call `p4 set -q P4PASSWD`; it can print a secret to command output. Read process `P4PASSWD` and process-visible `P4CONFIG` files instead.
- Do not include secrets in error messages, approval previews, or test assertion failure text.
- Keep this PR as upstream parity plus parity bug fixes; do not add Rust-only user-facing extensions.

## File Structure

- Modify `src/server.rs`
  - Validate changelist `description` for `create` and `update` before approval.
  - Add a small command-specific helper for benign P4 command failures.
  - Use the helper for approved `modify_files.sync`.
  - Pass configured Review API password fallback into `ReviewApiConfig::from_p4`.
  - Add focused unit tests for changelist validation and sync benign-error normalization.

- Modify `src/tools/reviews.rs`
  - Change `ReviewApiConfig::from_p4` to accept `configured_password: Option<&str>`.
  - Change ticket lookup to return `Ok(None)` when no matching ticket exists, while preserving ambiguity errors.
  - Add pure helpers to read a `P4PASSWD` value from process env input or a `P4CONFIG` file tree without shelling out secrets.

- Modify `tests/review_client_tests.rs`
  - Update existing `ReviewApiConfig::from_p4` calls with the new fourth argument.
  - Add pure auth fallback tests for no-ticket, ambiguous-ticket, env precedence, and parent `P4CONFIG` lookup.

- Modify `tests/mcp_smoke_tests.rs`
  - Add one Review API smoke test proving the public server path uses `P4PASSWD` when `p4 tickets` returns no matching ticket.
  - Guard environment mutation with a local lock and restore original values.

---

### Task 1: Validate Changelist Descriptions Before Approval

**Files:**
- Modify: `src/server.rs`
- Test: `src/server.rs`

- [ ] **Step 1: Add failing server tests for missing changelist descriptions**

In `src/server.rs`, inside `#[cfg(test)] mod tests`, add these tests after `modify_changelists_move_files_after_approval_reopens_files` and before `modify_changelists_update_without_approval_does_not_fetch_form`:

```rust
    #[tokio::test]
    async fn modify_changelists_create_without_description_rejects_before_approval() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let params = modify_changelists_params(ChangelistModifyAction::Create);

        let err = match server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("create without description should be rejected"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("description is required for create"));
        assert!(executor.invocations().is_empty());
        assert!(approval_gate.calls().is_empty());
    }

    #[tokio::test]
    async fn modify_changelists_update_without_description_rejects_before_approval() {
        let executor = Arc::new(FakeExecutor::success(P4CommandOutput {
            records: Vec::new(),
            text: json!({}),
        }));
        let approval_gate = Arc::new(FakeApprovalGate::approval_required());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate.clone(),
        );
        let mut params = modify_changelists_params(ChangelistModifyAction::Update);
        params.changelist_id = Some("123".to_string());

        let err = match server
            .modify_changelists_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("update without description should be rejected"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::invalid_params("", None).code);
        assert!(err.message.contains("description is required for update"));
        assert!(executor.invocations().is_empty());
        assert!(approval_gate.calls().is_empty());
    }
```

- [ ] **Step 2: Run the failing tests**

Run:

```bash
rtk cargo test description_rejects_before_approval
```

Expected: FAIL. The create case currently reaches approval with an empty form, and the update case currently reaches approval with the preview placeholder.

- [ ] **Step 3: Add pre-approval description validation**

In `src/server.rs`, replace the beginning of `modify_changelists_inner` from:

```rust
        let action = params.action.as_str().to_string();
        let changelist_id = match params.action {
            ChangelistModifyAction::Create => Some("new".to_string()),
            ChangelistModifyAction::Update
            | ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => Some(required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &action,
            )?),
        };
        let stdin = match params.action {
            ChangelistModifyAction::Create => Some(change_form(
                params.description.as_deref().unwrap_or_default(),
                &[],
            )),
            ChangelistModifyAction::Update => {
                Some("Description:\n\tapproval preview placeholder\n".to_string())
            }
            ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => None,
        };
```

with:

```rust
        let action = params.action.as_str().to_string();
        let changelist_id = match params.action {
            ChangelistModifyAction::Create => Some("new".to_string()),
            ChangelistModifyAction::Update
            | ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => Some(required_option(
                params.changelist_id.as_deref(),
                "changelist_id",
                &action,
            )?),
        };
        let description = match params.action {
            ChangelistModifyAction::Create | ChangelistModifyAction::Update => Some(
                required_option(params.description.as_deref(), "description", &action)?,
            ),
            ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => None,
        };
        let stdin = match params.action {
            ChangelistModifyAction::Create => Some(change_form(
                description
                    .as_deref()
                    .expect("create changelist description was required"),
                &[],
            )),
            ChangelistModifyAction::Update => {
                Some("Description:\n\tapproval preview placeholder\n".to_string())
            }
            ChangelistModifyAction::Submit
            | ChangelistModifyAction::Delete
            | ChangelistModifyAction::MoveFiles => None,
        };
```

Then replace the approved update patch block from:

```rust
            let patched = patch_change_description_form(
                existing,
                params.description.as_deref().unwrap_or_default(),
            )
            .map_err(to_mcp_error)?;
```

with:

```rust
            let patched = patch_change_description_form(
                existing,
                description
                    .as_deref()
                    .expect("update changelist description was required"),
            )
            .map_err(to_mcp_error)?;
```

- [ ] **Step 4: Run focused changelist tests**

Run:

```bash
rtk cargo test modify_changelists_update
rtk cargo test description_rejects_before_approval
```

Expected: PASS.

- [ ] **Step 5: Commit changelist validation fix**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: require changelist descriptions before approval"
```

Expected: commit succeeds with only `src/server.rs` staged.

---

### Task 2: Normalize Up-To-Date Sync as Success

**Files:**
- Modify: `src/server.rs`
- Test: `src/server.rs`

- [ ] **Step 1: Add queued executor failure support for server tests**

In `src/server.rs`, inside `impl QueuedExecutor`, add this method after `fn success`:

```rust
        fn results(outputs: Vec<crate::error::Result<P4CommandOutput>>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                invocations: Mutex::new(Vec::new()),
            }
        }
```

- [ ] **Step 2: Add failing tests for approved sync benign and non-benign P4 errors**

In `src/server.rs`, inside `#[cfg(test)] mod tests`, add these tests after `modify_files_after_approval_calls_executor_once`:

```rust
    #[tokio::test]
    async fn modify_files_sync_up_to_date_after_approval_returns_success() {
        let executor = Arc::new(QueuedExecutor::results(vec![Err(P4McpError::P4Command {
            message: "p4 exited with failure; status: exit status: 1; stdout: ; stderr: File(s) up-to-date\n"
                .to_string(),
        })]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor.clone(),
            approval_gate,
        );
        let params = modify_files_params(Some("approved-token"));

        let response = server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
            .expect("up-to-date sync should be successful");

        assert_eq!(response.0.status, "success");
        assert_eq!(response.0.action, "sync");
        assert_eq!(response.0.message, json!("Workspace is already up-to-date"));
        let invocations = executor.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].args, ["sync", "-f", "//depot/main/file.txt"]);
    }

    #[tokio::test]
    async fn modify_files_sync_other_p4_error_remains_internal_error() {
        let executor = Arc::new(QueuedExecutor::results(vec![Err(P4McpError::P4Command {
            message: "p4 exited with failure; stderr: no such file(s)".to_string(),
        })]));
        let approval_gate = Arc::new(FakeApprovalGate::approved());
        let server = P4McpServer::with_executor_and_approval(
            test_config(false),
            executor,
            approval_gate,
        );
        let params = modify_files_params(Some("approved-token"));

        let err = match server
            .modify_files_inner(params, ApprovalChannel::FallbackOnly)
            .await
        {
            Ok(_) => panic!("non-benign sync errors should still fail"),
            Err(err) => err,
        };

        assert_eq!(err.code, ErrorData::internal_error("", None).code);
        assert!(err.message.contains("p4 command failed"));
        assert!(err.message.contains("no such file"));
    }
```

- [ ] **Step 3: Run the failing sync tests**

Run:

```bash
rtk cargo test modify_files_sync_
```

Expected: FAIL. The up-to-date sync currently returns an internal error.

- [ ] **Step 4: Add a command-specific benign-error helper**

In `src/server.rs`, add this method immediately after `call_p4_tool`:

```rust
    async fn call_p4_tool_with_benign_success(
        &self,
        action: &str,
        invocation: P4Invocation,
        benign_message: &str,
        success_message: Value,
    ) -> McpResult<Json<ToolResponse>> {
        match self.executor.run(invocation, P4Env::new()).await {
            Ok(output) => Ok(Json(ToolResponse::success(action, output_message(output)))),
            Err(error) if error.to_string().contains(benign_message) => {
                Ok(Json(ToolResponse::success(action, success_message)))
            }
            Err(error) => Err(to_mcp_error(error)),
        }
    }
```

- [ ] **Step 5: Use the helper only for approved `modify_files.sync`**

In `src/server.rs`, replace the tail of `modify_files_inner`:

```rust
        self.call_p4_tool(action, invocation).await
```

with:

```rust
        if params.action == FileModifyAction::Sync {
            return self
                .call_p4_tool_with_benign_success(
                    action,
                    invocation,
                    "File(s) up-to-date",
                    json!("Workspace is already up-to-date"),
                )
                .await;
        }
        self.call_p4_tool(action, invocation).await
```

- [ ] **Step 6: Run focused sync tests**

Run:

```bash
rtk cargo test modify_files_sync
```

Expected: PASS. The missing-file validation test must still reject before approval.

- [ ] **Step 7: Commit sync normalization fix**

Run:

```bash
rtk git add src/server.rs
rtk git commit -m "fix: treat up-to-date sync as success"
```

Expected: commit succeeds with only `src/server.rs` staged.

---

### Task 3: Model Review API Credentials Beyond Ticket Files

**Files:**
- Modify: `src/tools/reviews.rs`
- Test: `tests/review_client_tests.rs`

- [ ] **Step 1: Add failing Review API credential tests**

In `tests/review_client_tests.rs`, replace the current import:

```rust
use p4mcp_server_rs::tools::reviews::{
    ModifyReviewsParams, QueryReviewsParams, ReviewHttpClient, ReviewModifyAction,
    ReviewQueryAction,
};
```

with:

```rust
use std::fs;

use p4mcp_server_rs::tools::reviews::{
    ModifyReviewsParams, QueryReviewsParams, ReviewHttpClient, ReviewModifyAction,
    ReviewQueryAction, configured_p4_password,
};
```

Then add these tests after `review_api_config_uses_single_user_ticket_when_server_address_is_absent`:

```rust
#[test]
fn review_api_config_uses_configured_password_when_ticket_is_missing() {
    let config = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "",
        Some("password-or-ticket"),
    )
    .unwrap();

    assert_eq!(config.api_base, "https://swarm.example.com/api/v11");
    assert_eq!(config.username, "alice");
    assert_eq!(config.ticket, "password-or-ticket");
}

#[test]
fn review_api_config_does_not_use_password_fallback_for_ambiguous_tickets() {
    let error = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "perforce:1666 (alice) ticket-123\nother:1666 (alice) other-ticket\n",
        Some("secret-password"),
    )
    .err()
    .unwrap()
    .to_string();

    assert!(error.contains("multiple P4 tickets found for user alice"));
    assert!(!error.contains("ticket-123"));
    assert!(!error.contains("other-ticket"));
    assert!(!error.contains("secret-password"));
}

#[test]
fn configured_p4_password_prefers_env_password_over_config_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".p4config"), "P4PASSWD=config-password\n").unwrap();

    let password = configured_p4_password(Some("env-password"), Some(".p4config"), dir.path());

    assert_eq!(password.as_deref(), Some("env-password"));
}

#[test]
fn configured_p4_password_reads_parent_p4config_file() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child").join("workspace");
    fs::create_dir_all(&child).unwrap();
    fs::write(
        dir.path().join(".p4config"),
        "\
# comment
P4PORT=perforce:1666
P4PASSWD = config-password
",
    )
    .unwrap();

    let password = configured_p4_password(None, Some(".p4config"), &child);

    assert_eq!(password.as_deref(), Some("config-password"));
}
```

- [ ] **Step 2: Update existing `from_p4` tests to pass the new argument**

In `tests/review_client_tests.rs`, update every existing `ReviewApiConfig::from_p4(...)` call that does not need fallback by adding `None` as the fourth argument.

Example replacement:

```rust
    let config = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com/"
        })],
        "perforce:1666 (alice) ticket-123\nother:1666 (alice) wrong-ticket\n",
        None,
    )
    .unwrap();
```

After editing, run:

```bash
rtk rg -n "ReviewApiConfig::from_p4\\(" tests/review_client_tests.rs
```

Expected: every call has four arguments.

- [ ] **Step 3: Run the failing Review API credential tests**

Run:

```bash
rtk cargo test review_api_config --test review_client_tests
rtk cargo test configured_p4_password --test review_client_tests
```

Expected: FAIL because `from_p4` only accepts three arguments and `configured_p4_password` does not exist.

- [ ] **Step 4: Change Review API config lookup semantics**

In `src/tools/reviews.rs`, replace the first line:

```rust
use std::path::PathBuf;
```

with:

```rust
use std::{
    fs,
    path::{Path, PathBuf},
};
```

Replace `ReviewApiConfig::from_p4` with:

```rust
impl ReviewApiConfig {
    pub fn from_p4(
        info_records: &[Value],
        property_records: &[Value],
        tickets_stdout: &str,
        configured_password: Option<&str>,
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
        let ticket = ticket_for_user(tickets_stdout, &username, server.as_deref())?
            .or_else(|| configured_password.and_then(non_blank_value))
            .ok_or_else(|| P4McpError::P4Command {
                message: format!(
                    "No P4 ticket or configured P4PASSWD found for user {username}. Please run p4 login first or configure P4PASSWD."
                ),
            })?;

        Ok(Self {
            api_base: format!("{}/api/v11", swarm_url.trim_end_matches('/')),
            username,
            ticket,
        })
    }
}
```

Replace `ticket_for_user` with:

```rust
fn ticket_for_user(stdout: &str, username: &str, server: Option<&str>) -> Result<Option<String>> {
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
            [entry] => return Ok(Some(entry.ticket.clone())),
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
        [entry] => Ok(Some(entry.ticket.clone())),
        [] => Ok(None),
        _ => Err(P4McpError::P4Command {
            message: format!(
                "multiple P4 tickets found for user {username}; configure a matching P4PORT or run p4 login for the active server"
            ),
        }),
    }
}
```

Add these helpers immediately after `parse_ticket_line`:

```rust
pub fn configured_p4_password(
    env_password: Option<&str>,
    p4config_name: Option<&str>,
    cwd: &Path,
) -> Option<String> {
    env_password
        .and_then(non_blank_value)
        .or_else(|| {
            let config_name = p4config_name.and_then(non_blank_value)?;
            let config_path = find_p4_config(cwd, &config_name)?;
            let content = fs::read_to_string(config_path).ok()?;
            p4_config_value(&content, "P4PASSWD")
        })
}

fn non_blank_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn find_p4_config(cwd: &Path, config_name: &str) -> Option<PathBuf> {
    cwd.ancestors()
        .map(|ancestor| ancestor.join(config_name))
        .find(|candidate| candidate.is_file())
}

fn p4_config_value(content: &str, key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let (name, value) = trimmed.split_once('=')?;
        if name.trim() == key {
            non_blank_value(value)
        } else {
            None
        }
    })
}
```

- [ ] **Step 5: Run Review API credential tests**

Run:

```bash
rtk cargo test review_api_config --test review_client_tests
rtk cargo test configured_p4_password --test review_client_tests
```

Expected: PASS.

- [ ] **Step 6: Commit Review API config semantics**

Run:

```bash
rtk git add src/tools/reviews.rs tests/review_client_tests.rs
rtk git commit -m "fix: support configured P4 password for reviews"
```

Expected: commit succeeds with only `src/tools/reviews.rs` and `tests/review_client_tests.rs` staged.

---

### Task 4: Wire Review Credential Fallback Through Server Execution

**Files:**
- Modify: `src/server.rs`
- Modify: `tests/mcp_smoke_tests.rs`

- [ ] **Step 1: Add an env guard for Review API smoke tests**

In `tests/mcp_smoke_tests.rs`, replace the top import:

```rust
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
};
```

with:

```rust
use std::{
    collections::VecDeque,
    env,
    ffi::OsString,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex, MutexGuard},
};
```

Then add this helper after the imports and before `#[tokio::test]` functions:

```rust
static REVIEW_AUTH_ENV_LOCK: Mutex<()> = Mutex::new(());

struct ReviewAuthEnvGuard {
    _lock: MutexGuard<'static, ()>,
    p4passwd: Option<OsString>,
    p4config: Option<OsString>,
}

impl ReviewAuthEnvGuard {
    fn new(p4passwd: &str) -> Self {
        let lock = REVIEW_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_p4passwd = env::var_os("P4PASSWD");
        let previous_p4config = env::var_os("P4CONFIG");
        unsafe {
            env::set_var("P4PASSWD", p4passwd);
            env::remove_var("P4CONFIG");
        }
        Self {
            _lock: lock,
            p4passwd: previous_p4passwd,
            p4config: previous_p4config,
        }
    }
}

impl Drop for ReviewAuthEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.p4passwd {
                Some(value) => env::set_var("P4PASSWD", value),
                None => env::remove_var("P4PASSWD"),
            }
            match &self.p4config {
                Some(value) => env::set_var("P4CONFIG", value),
                None => env::remove_var("P4CONFIG"),
            }
        }
    }
}
```

- [ ] **Step 2: Add a failing server smoke test for `P4PASSWD` fallback**

In `tests/mcp_smoke_tests.rs`, add this test after `query_reviews_executes_review_api_request`:

```rust
#[tokio::test(flavor = "current_thread")]
async fn query_reviews_uses_p4passwd_when_tickets_are_empty() {
    let _env = ReviewAuthEnvGuard::new("password-123");
    let swarm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "5"))
        .and(header("authorization", "Basic YWxpY2U6cGFzc3dvcmQtMTIz"))
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
                "stdout": "",
                "stderr": ""
            }),
        },
    ]));
    let server = P4McpServer::with_executor(test_config(), executor.clone());

    let response = server
        .query_reviews(Parameters(QueryReviewsParams {
            max_results: 5,
            ..query_reviews_params(ReviewQueryAction::List)
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

- [ ] **Step 3: Run the failing server smoke test**

Run:

```bash
rtk cargo test query_reviews_uses_p4passwd_when_tickets_are_empty --test mcp_smoke_tests
```

Expected: FAIL because `review_http_client_from_p4` does not pass `P4PASSWD` into `ReviewApiConfig::from_p4`.

- [ ] **Step 4: Pass configured password into Review API config**

In `src/server.rs`, replace the top import:

```rust
use std::{path::Path, sync::Arc};
```

with:

```rust
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
```

In the `tools::reviews` import block, replace:

```rust
        reviews::{
            BuiltReviewRequest, ModifyReviewsParams, QueryReviewsParams, ReviewApiConfig,
            ReviewHttpClient,
        },
```

with:

```rust
        reviews::{
            BuiltReviewRequest, ModifyReviewsParams, QueryReviewsParams, ReviewApiConfig,
            ReviewHttpClient, configured_p4_password,
        },
```

Then replace this code in `review_http_client_from_p4`:

```rust
        let api_config =
            ReviewApiConfig::from_p4(&info.records, &swarm_property.records, tickets_stdout)
                .map_err(to_mcp_error)?;
```

with:

```rust
        let p4passwd = std::env::var("P4PASSWD").ok();
        let p4config = std::env::var("P4CONFIG").ok();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let configured_password =
            configured_p4_password(p4passwd.as_deref(), p4config.as_deref(), &cwd);
        let api_config = ReviewApiConfig::from_p4(
            &info.records,
            &swarm_property.records,
            tickets_stdout,
            configured_password.as_deref(),
        )
        .map_err(to_mcp_error)?;
```

- [ ] **Step 5: Run Review API smoke tests**

Run:

```bash
rtk cargo test query_reviews_ --test mcp_smoke_tests
```

Expected: PASS. Existing ticket-file auth must still work, and empty tickets plus `P4PASSWD` must work.

- [ ] **Step 6: Commit server Review API auth wiring**

Run:

```bash
rtk git add src/server.rs tests/mcp_smoke_tests.rs
rtk git commit -m "fix: wire review auth password fallback"
```

Expected: commit succeeds with only `src/server.rs` and `tests/mcp_smoke_tests.rs` staged.

---

### Task 5: Full Verification And PR Review Follow-Up

**Files:**
- No source edits expected unless verification exposes a defect.
- GitHub review replies use the existing PR #2 inline threads.

- [ ] **Step 1: Run formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS. If it fails, run `rtk cargo fmt`, then rerun `rtk cargo fmt --check`.

- [ ] **Step 2: Run focused test suites**

Run:

```bash
rtk cargo test modify_changelists_ --test tool_mapping_tests
rtk cargo test modify_changelists_
rtk cargo test modify_files_sync
rtk cargo test review_api_config --test review_client_tests
rtk cargo test configured_p4_password --test review_client_tests
rtk cargo test query_reviews_ --test mcp_smoke_tests
```

Expected: PASS.

- [ ] **Step 3: Run clippy**

Run:

```bash
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Run full test suite**

Run:

```bash
rtk cargo test
```

Expected: PASS. If WireMock localhost binding fails under sandbox with `PermissionDenied`, rerun the same command with sandbox escalation.

- [ ] **Step 5: Confirm no generated P4 artifacts or unrelated files**

Run:

```bash
rtk git status --short --branch
rtk git ls-files --others --exclude-standard
```

Expected:
- Branch is `codex/add-gitignore`.
- Only intentional tracked changes exist, or the tree is clean after commits.
- No untracked `db.*`, `journal`, `monfile.mem`, or `server.locks/` files remain.

- [ ] **Step 6: Push the branch**

Run:

```bash
rtk git push
```

Expected: branch push succeeds.

- [ ] **Step 7: Reply to the three GitHub review threads**

Use `superpowers:receiving-code-review` plus `github:gh-address-comments`.

For the changelist description thread, reply in-thread:

```text
Fixed in the latest push.

This was a true upstream-parity validation gap. Upstream rejects missing `description` for both `modify_changelists.create` and `modify_changelists.update` before service execution. The Rust port now performs the same pre-approval validation, so a missing description cannot reach the write approval gate, cannot build a placeholder form, and cannot clear an existing changelist description after approval.
```

For the up-to-date sync thread, reply in-thread:

```text
Fixed in the latest push.

The Rust port now matches upstream `sync_files()` semantics for this command-specific benign P4 failure. After approval, `modify_files.sync` converts `File(s) up-to-date` into a successful `Workspace is already up-to-date` response while preserving normal errors for other P4 failures. The existing missing-file validation still rejects underspecified sync before approval.
```

For the Review API auth thread, reply in-thread:

```text
Fixed in the latest push.

The Review API config no longer treats `p4 tickets` as the only credential source. It still prefers an unambiguous matching ticket file entry, but when no matching ticket exists it falls back to a configured `P4PASSWD` value from the MCP process environment or process-visible `P4CONFIG` file. Ambiguous ticket file entries still fail instead of silently choosing a fallback, and tests assert that secrets are not included in error messages.
```

- [ ] **Step 8: Resolve the three GitHub review threads**

After replies are posted and the branch is pushed, resolve only these three threads:
- Preserve descriptions when update omits one.
- Treat up-to-date syncs as successful.
- Honor P4PASSWD for Review API auth.

Run the thread-aware fetch script afterward:

```bash
rtk python3 /Users/jeongsaebit/.codex/plugins/cache/openai-curated/github/c6ea566d/skills/gh-address-comments/scripts/fetch_comments.py
```

Expected: the three target threads report `isResolved: true`. Do not resolve unrelated new comments without triage.

---

## Self-Review

Spec coverage:
- Changelist validation root cause is covered by Task 1.
- P4 benign-error normalization root cause is covered by Task 2.
- Review API credential root cause is covered by Tasks 3 and 4.
- Verification and PR review handling are covered by Task 5.

Placeholder scan:
- No step relies on unspecified validation, unspecified tests, or deferred implementation.
- Every code-changing step includes concrete code or exact replacements.

Type consistency:
- `ReviewApiConfig::from_p4` is consistently updated from three arguments to four.
- `configured_p4_password` is public in `src/tools/reviews.rs` and imported by both tests and `src/server.rs`.
- `QueuedExecutor::results` uses the existing `crate::error::Result<P4CommandOutput>` type already stored in the test executor.
