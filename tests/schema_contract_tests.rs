use std::collections::BTreeSet;

use p4mcp_server_rs::tools::server::QueryServerParams;
use schemars::JsonSchema;

fn schema_properties<T: JsonSchema>() -> BTreeSet<String> {
    let schema = schemars::schema_for!(T);
    let schema = serde_json::to_value(schema).unwrap();
    schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn query_server_schema_matches_upstream_params_object() {
    let props = schema_properties::<QueryServerParams>();
    assert!(props.contains("action"));
    assert_eq!(props.len(), 1);
}
