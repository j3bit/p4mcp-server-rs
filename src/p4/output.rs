use serde_json::{Value, json};

use crate::error::{P4McpError, Result};

pub fn parse_json_lines(stdout: &str) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for (idx, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|source| P4McpError::P4Json {
            line: idx + 1,
            source,
        })?;
        records.push(value);
    }
    Ok(records)
}

pub fn text_response(stdout: &str, stderr: &str) -> Value {
    json!({
        "stdout": stdout,
        "stderr": stderr,
    })
}
