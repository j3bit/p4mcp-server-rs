use p4mcp_server_rs::tools::reviews::{ReviewAction, ReviewHttpClient, ReviewRequest};
use schemars::JsonSchema;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn schema_has_property<T: JsonSchema>(name: &str) -> bool {
    let schema = schemars::schema_for!(T);
    let schema = serde_json::to_value(schema).unwrap();
    schema["properties"].as_object().unwrap().contains_key(name)
}

#[test]
fn list_reviews_builds_v11_reviews_path() {
    let request = ReviewRequest {
        action: ReviewAction::List,
        review_id: None,
        max_results: 25,
        body: serde_json::json!({}),
        approval_token: None,
    };
    let built = request
        .to_http("https://swarm.example.com/api/v11")
        .unwrap();
    assert_eq!(built.method, "GET");
    assert_eq!(built.path, "/reviews");
    assert_eq!(built.query, vec![("max".to_string(), "25".to_string())]);
}

#[test]
fn vote_review_builds_post_payload() {
    let request = ReviewRequest {
        action: ReviewAction::Vote,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"vote": "up", "version": 2}),
        approval_token: None,
    };
    let built = request
        .to_http("https://swarm.example.com/api/v11")
        .unwrap();
    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/reviews/123/vote");
    assert_eq!(built.body["vote"], "up");
}

#[test]
fn review_request_schema_exposes_approval_token() {
    assert!(schema_has_property::<ReviewRequest>("approval_token"));
}

#[test]
fn review_request_schema_omits_confirmation() {
    assert!(!schema_has_property::<ReviewRequest>("confirmation"));
}

#[test]
fn obliterate_review_builds_delete_without_body_confirmation() {
    let request = ReviewRequest {
        action: ReviewAction::Obliterate,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({}),
        approval_token: None,
    };
    let built = request
        .to_http("https://swarm.example.com/api/v11")
        .unwrap();
    assert_eq!(built.method, "DELETE");
    assert_eq!(built.path, "/reviews/123");
    assert_eq!(built.body, serde_json::json!({}));
}

#[test]
fn missing_body_deserializes_to_empty_object() {
    let request: ReviewRequest = serde_json::from_value(serde_json::json!({
        "action": "list"
    }))
    .unwrap();

    assert_eq!(request.body, serde_json::json!({}));
    assert_eq!(request.max_results, 10);
    assert_eq!(request.approval_token, None);
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
    let request = ReviewRequest {
        action: ReviewAction::List,
        review_id: None,
        max_results: 7,
        body: serde_json::json!({}),
        approval_token: None,
    };

    let result = client.execute(&request).await.unwrap();

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
    let request = ReviewRequest {
        action: ReviewAction::Vote,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"vote": "up", "version": 2}),
        approval_token: None,
    };

    let error = client.execute(&request).await.unwrap_err().to_string();

    assert!(error.contains("requires MCP write approval"));
    assert!(!error.contains("ticket"));
}

#[tokio::test]
async fn execute_non_success_returns_error_without_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v11/reviews"))
        .and(query_param("max", "10"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .respond_with(ResponseTemplate::new(500).set_body_string("server failed"))
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
        action: ReviewAction::List,
        review_id: None,
        max_results: 10,
        body: serde_json::json!({}),
        approval_token: None,
    };

    let error = client.execute(&request).await.unwrap_err().to_string();

    assert!(error.contains("HTTP 500"));
    assert!(!error.contains("ticket"));
    assert!(!error.contains("dXNlcjp0aWNrZXQ="));
}
