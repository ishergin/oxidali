use cucumber::then;
use serde_json::Value;

use crate::DaliWorld;

// OP-100 COMM-001 COMM-030
#[then(regex = r#"^the operations list should contain a key starting with "([^"]+)"$"#)]
async fn then_operations_list_has_key_with_prefix(world: &mut DaliWorld, prefix: String) {
    let response = world.last_response().expect("last response");
    let json: Value = serde_json::from_slice(&response.body).expect("response should be JSON");
    let keys = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    assert!(
        keys.iter()
            .filter_map(Value::as_str)
            .any(|key| key.starts_with(&prefix)),
        "no operation key starting with {prefix:?} in {json:?}"
    );
}

// POLICY-009
#[then(regex = r#"^the operations list should contain no key starting with "([^"]+)"$"#)]
async fn then_operations_list_lacks_key_with_prefix(world: &mut DaliWorld, prefix: String) {
    let response = world.last_response().expect("last response");
    let json: Value = serde_json::from_slice(&response.body).expect("response should be JSON");
    let keys = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    let found: Vec<&str> = keys
        .iter()
        .filter_map(Value::as_str)
        .filter(|key| key.starts_with(&prefix))
        .collect();
    assert!(found.is_empty(), "unexpected operations {found:?}");
}
