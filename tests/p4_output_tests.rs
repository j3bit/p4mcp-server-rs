use p4mcp_server_rs::p4::output::{parse_json_lines, text_response};
use serde_json::json;

#[test]
fn parses_line_delimited_json_records() {
    let input = r#"{"code":"stat","data":"one"}
{"code":"info","data":"two"}
"#;
    let parsed = parse_json_lines(input).unwrap();
    assert_eq!(
        parsed,
        vec![
            json!({"code": "stat", "data": "one"}),
            json!({"code": "info", "data": "two"})
        ]
    );
}

#[test]
fn ignores_blank_json_lines() {
    let input = "\n{\"code\":\"info\",\"data\":\"ok\"}\n\n";
    let parsed = parse_json_lines(input).unwrap();
    assert_eq!(parsed, vec![json!({"code": "info", "data": "ok"})]);
}

#[test]
fn reports_bad_json_line_number() {
    let err = parse_json_lines("{\"ok\": true}\nnot-json")
        .unwrap_err()
        .to_string();
    assert!(err.contains("line 2"));
}

#[test]
fn text_response_keeps_stdout_and_stderr() {
    let response = text_response("file content\n", "warning\n");
    assert_eq!(response["stdout"], "file content\n");
    assert_eq!(response["stderr"], "warning\n");
}
