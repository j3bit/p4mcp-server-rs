use p4mcp_server_rs::tools::reviews::{ReviewAction, ReviewHttpClient, ReviewRequest};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn list_reviews_builds_v11_reviews_path() {
    let request = ReviewRequest {
        action: ReviewAction::List,
        review_id: None,
        max_results: 25,
        body: serde_json::json!({}),
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
    };
    let built = request
        .to_http("https://swarm.example.com/api/v11")
        .unwrap();
    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/reviews/123/vote");
    assert_eq!(built.body["vote"], "up");
}

#[test]
fn obliterate_review_requires_confirmation() {
    let request = ReviewRequest {
        action: ReviewAction::Obliterate,
        review_id: Some(123),
        max_results: 10,
        body: serde_json::json!({"confirmation": "CANCEL"}),
    };
    assert!(
        request
            .to_http("https://swarm.example.com/api/v11")
            .is_err()
    );
}

#[test]
fn missing_body_deserializes_to_empty_object() {
    let request: ReviewRequest = serde_json::from_value(serde_json::json!({
        "action": "list"
    }))
    .unwrap();

    assert_eq!(request.body, serde_json::json!({}));
    assert_eq!(request.max_results, 10);
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
    };

    let result = client.execute(&request).await.unwrap();

    assert_eq!(result, serde_json::json!({ "reviews": [123] }));
}

#[tokio::test]
async fn execute_vote_sends_post_json_and_basic_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v11/reviews/123/vote"))
        .and(header("authorization", "Basic dXNlcjp0aWNrZXQ="))
        .and(body_json(serde_json::json!({
            "vote": "up",
            "version": 2
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true
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
    };

    let result = client.execute(&request).await.unwrap();

    assert_eq!(result, serde_json::json!({ "ok": true }));
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
    };

    let error = client.execute(&request).await.unwrap_err().to_string();

    assert!(error.contains("HTTP 500"));
    assert!(!error.contains("ticket"));
    assert!(!error.contains("dXNlcjp0aWNrZXQ="));
}
