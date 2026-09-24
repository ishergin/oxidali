use cucumber::then;

use crate::{DaliWorld, WEB_FIXTURE_APP_JS, WEB_FIXTURE_INDEX_HTML};

fn last_response(world: &DaliWorld) -> &crate::TestResponse {
    world.last_response.as_ref().expect("no HTTP response recorded")
}

fn web_fixture_bytes(fixture: &str) -> &'static [u8] {
    match fixture {
        "index.html.gz" => WEB_FIXTURE_INDEX_HTML,
        "app.js.gz" => WEB_FIXTURE_APP_JS,
        other => panic!("unknown web fixture: {other}"),
    }
}

// WEB-001 WEB-002 WEB-003 WEB-004
#[then(regex = r#"^the response content type should be "([^"]+)"$"#)]
async fn response_content_type_should_be(world: &mut DaliWorld, expected: String) {
    let response = last_response(world);
    assert_eq!(
        response.content_type, expected,
        "unexpected Content-Type (status {})",
        response.status
    );
}

// WEB-001 WEB-002
#[then(regex = r#"^the response content encoding should be "([^"]+)"$"#)]
async fn response_content_encoding_should_be(world: &mut DaliWorld, expected: String) {
    let response = last_response(world);
    assert_eq!(
        response.content_encoding, expected,
        "unexpected Content-Encoding (status {})",
        response.status
    );
}

// WEB-001 WEB-002 WEB-003
#[then(regex = r#"^the response body should equal the "([^"]+)" web fixture$"#)]
async fn response_body_should_equal_web_fixture(world: &mut DaliWorld, fixture: String) {
    let response = last_response(world);
    let expected = web_fixture_bytes(&fixture);
    assert_eq!(
        response.body, expected,
        "body does not match fixture {fixture} (status {})",
        response.status
    );
}
