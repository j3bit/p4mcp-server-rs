use std::fs;

use p4mcp_server_rs::tools::reviews::{
    ModifyReviewsParams, QueryReviewsParams, ReviewHttpClient, ReviewModifyAction,
    ReviewQueryAction, configured_p4_password,
};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn list_reviews_request(max_results: u16) -> QueryReviewsParams {
    QueryReviewsParams {
        action: ReviewQueryAction::List,
        review_id: None,
        fields: None,
        comments_fields: Some("id,body,user,time".to_string()),
        up_voters: None,
        from_version: None,
        to_version: None,
        max_results,
        after: None,
        after_updated: None,
        result_order: None,
        projects: None,
        state: None,
        keywords: None,
        keywords_fields: None,
        include_transitions: None,
    }
}

fn modify_review_request(action: ReviewModifyAction) -> ModifyReviewsParams {
    ModifyReviewsParams {
        action,
        review_id: Some(123),
        change_id: None,
        description: None,
        reviewers: None,
        required_reviewers: None,
        reviewer_group_names: None,
        reviewer_groups_required: None,
        comment_file_path: None,
        comment_left_line: None,
        comment_right_line: None,
        comment_version: None,
        vote_value: None,
        version: None,
        transition: None,
        jobs: None,
        fix_status: None,
        cleanup: None,
        participant_user_names: None,
        participant_users_required: None,
        participant_group_names: None,
        participant_groups_required: None,
        body: None,
        task_state: None,
        notify: None,
        comment_id: None,
        not_updated_since: None,
        max_reviews: 0,
        new_author: None,
        new_description: None,
        approval_token: None,
    }
}

fn vote_review_request(approval_token: Option<&str>) -> ModifyReviewsParams {
    ModifyReviewsParams {
        vote_value: Some("up".to_string()),
        version: Some(2),
        approval_token: approval_token.map(str::to_string),
        ..modify_review_request(ReviewModifyAction::Vote)
    }
}

#[test]
fn add_comment_includes_notify_query_param_when_set() {
    let request = ModifyReviewsParams {
        body: Some("Looks good.".to_string()),
        notify: Some("delayed".to_string()),
        ..modify_review_request(ReviewModifyAction::AddComment)
    };

    let built = request.to_http(Some("alice")).unwrap();

    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/reviews/123/comments");
    assert_eq!(
        built.query,
        vec![("notify".to_string(), "delayed".to_string())]
    );
    assert_eq!(built.body, serde_json::json!({"body": "Looks good."}));
}

#[test]
fn delete_participants_uses_empty_array_dynamic_participant_keys() {
    let request = ModifyReviewsParams {
        participant_user_names: Some(vec!["bob".to_string()]),
        participant_users_required: Some(vec!["carol".to_string()]),
        participant_group_names: Some(vec!["dev-team".to_string()]),
        participant_groups_required: Some(vec!["ops".to_string()]),
        ..modify_review_request(ReviewModifyAction::DeleteParticipants)
    };

    let built = request.to_http(Some("alice")).unwrap();

    assert_eq!(built.method, "DELETE");
    assert_eq!(built.path, "/reviews/123/participants");
    assert_eq!(
        built.body,
        serde_json::json!({
            "participants": {
                "users": {
                    "bob": [],
                    "carol": []
                },
                "groups": {
                    "dev-team": [],
                    "ops": []
                }
            }
        })
    );
}

#[test]
fn leave_uses_username_participant_body() {
    let request = modify_review_request(ReviewModifyAction::Leave);

    let built = request.to_http(Some("alice")).unwrap();

    assert_eq!(built.method, "DELETE");
    assert_eq!(built.path, "/reviews/123/leave");
    assert_eq!(
        built.body,
        serde_json::json!({
            "participants": {
                "users": {
                    "alice": []
                }
            }
        })
    );
}

#[test]
fn list_reviews_builds_v11_reviews_path() {
    let request = list_reviews_request(25);
    let built = request.to_http().unwrap();
    assert_eq!(built.method, "GET");
    assert_eq!(built.path, "/reviews");
    assert_eq!(built.query, vec![("max".to_string(), "25".to_string())]);
}

#[test]
fn vote_review_builds_post_payload() {
    let request = vote_review_request(None);
    let built = request.to_http(Some("user")).unwrap();
    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/reviews/123/vote");
    assert_eq!(built.body["vote"], "up");
}

#[test]
fn obliterate_review_builds_delete_without_body_confirmation() {
    let request = ModifyReviewsParams {
        action: ReviewModifyAction::Obliterate,
        review_id: Some(123),
        change_id: None,
        description: None,
        reviewers: None,
        required_reviewers: None,
        reviewer_group_names: None,
        reviewer_groups_required: None,
        comment_file_path: None,
        comment_left_line: None,
        comment_right_line: None,
        comment_version: None,
        vote_value: None,
        version: None,
        transition: None,
        jobs: None,
        fix_status: None,
        cleanup: None,
        participant_user_names: None,
        participant_users_required: None,
        participant_group_names: None,
        participant_groups_required: None,
        body: None,
        task_state: None,
        notify: None,
        comment_id: None,
        not_updated_since: None,
        max_reviews: 0,
        new_author: None,
        new_description: None,
        approval_token: None,
    };
    let built = request.to_http(Some("user")).unwrap();
    assert_eq!(built.method, "DELETE");
    assert_eq!(built.path, "/reviews/123");
    assert_eq!(built.body, serde_json::json!({}));
}

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
        None,
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
        None,
    )
    .unwrap();

    assert_eq!(config.api_base, "https://swarm.example.com/api/v11");
    assert_eq!(config.username, "alice");
    assert_eq!(config.ticket, "ticket-123");
}

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
fn review_api_config_uses_configured_password_when_ticket_is_for_other_server() {
    let config = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "other:1666 (alice) ticket-123\n",
        Some("password-or-ticket"),
    )
    .unwrap();

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
        None,
    )
    .err()
    .unwrap()
    .to_string();

    assert!(error.contains("multiple P4 tickets found for user alice"));
    assert!(!error.contains("ticket-123"));
    assert!(!error.contains("other-ticket"));
}

#[test]
fn review_api_config_rejects_duplicate_exact_server_tickets() {
    let error = p4mcp_server_rs::tools::reviews::ReviewApiConfig::from_p4(
        &[serde_json::json!({
            "userName": "alice",
            "serverAddress": "perforce:1666"
        })],
        &[serde_json::json!({
            "value": "https://swarm.example.com"
        })],
        "perforce:1666 (alice) ticket-123\nperforce:1666 (alice) other-ticket\n",
        None,
    )
    .err()
    .unwrap()
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
        None,
    )
    .err()
    .unwrap()
    .to_string();

    assert!(error.contains("Swarm URL not configured"));
}

#[tokio::test]
async fn execute_list_sends_get_with_query_and_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "7"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "reviews": [123]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = ReviewHttpClient::new(
        format!("{}/api/v11/", server.uri()),
        "user".into(),
        "ticket".into(),
        false,
    )
    .unwrap();
    let request = list_reviews_request(7);
    let built = request.to_http().unwrap();

    let result = client.execute(built).await.unwrap();

    assert_eq!(result, serde_json::json!({ "reviews": [123] }));
}

#[tokio::test]
async fn execute_vote_rejects_write_without_approval() {
    let client = ReviewHttpClient::new(
        "https://swarm.example.com/api/v11".into(),
        "user".into(),
        "ticket".into(),
        false,
    )
    .unwrap();
    let request = vote_review_request(None);
    let built = request.to_http(Some("user")).unwrap();

    let error = client.execute(built).await.unwrap_err().to_string();

    assert!(error.contains("requires MCP write approval"));
    assert!(!error.contains("ticket"));
}

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
    let request = vote_review_request(Some("approved-token"));
    let built = request.to_http(Some("user")).unwrap();

    let result = client.execute_approved(built).await.unwrap();

    assert_eq!(result, serde_json::json!({ "vote": "recorded" }));
}

#[tokio::test]
async fn execute_approved_add_comment_sends_post_with_query_body_and_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v11/reviews/123/comments"))
        .and(query_param("notify", "delayed"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .and(body_json(serde_json::json!({"body": "Looks good."})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "comment": "created"
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
    let request = ModifyReviewsParams {
        body: Some("Looks good.".to_string()),
        notify: Some("delayed".to_string()),
        ..modify_review_request(ReviewModifyAction::AddComment)
    };
    let built = request.to_http(Some("user")).unwrap();

    let result = client.execute_approved(built).await.unwrap();

    assert_eq!(result, serde_json::json!({ "comment": "created" }));
}

#[tokio::test]
async fn execute_non_success_returns_error_without_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "10"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string("server failed: Authorization: Basic dXNlcjp0aWNrZXQ= ticket"),
        )
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
    let request = list_reviews_request(10);
    let built = request.to_http().unwrap();

    let error = client.execute(built).await.unwrap_err().to_string();

    assert!(error.contains("HTTP 500"));
    assert!(!error.contains("Authorization: Basic"));
    assert!(!error.contains("ticket"));
    assert!(!error.contains("dXNlcjp0aWNrZXQ="));
}
