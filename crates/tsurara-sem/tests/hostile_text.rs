//! Keys of the definition that generated code prints into comments and literals (path keys, method
//! names, media types, channel names and templates) must not hold control characters.
//! Such a key is an `invalid-definition` warning, and what it names is left out.

use tsurara_loader::DocumentSet;
use tsurara_sem::warning::WarningKind;
use tsurara_sem::{Options, build_model};

fn model(doc: serde_json::Value) -> tsurara_sem::model::Model {
    let set = DocumentSet::from_value(doc);
    build_model(&set, &Options::default()).unwrap()
}

fn ok() -> serde_json::Value {
    serde_json::json!({ "200": { "description": "ok" } })
}

fn invalid(m: &tsurara_sem::model::Model) -> Vec<String> {
    m.warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidDefinition)
        .map(|w| format!("{} {}", w.loc, w.message))
        .collect()
}

#[test]
fn a_path_key_with_a_control_character_is_left_out() {
    for bad in ["/a\nb", "/a\rb", "/a\u{2028}b", "/a\u{0}b", "/a\u{85}b"] {
        let m = model(serde_json::json!({
            "openapi": "3.1.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                bad: { "get": { "operationId": "bad", "responses": ok() } },
                "/good": { "get": { "operationId": "good", "responses": ok() } },
            },
        }));
        let ids: Vec<&str> = m.operations.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["good"], "{bad:?}");
        assert_eq!(invalid(&m).len(), 1, "{bad:?}: {:?}", m.warnings);
    }
}

#[test]
fn a_method_with_a_control_character_is_left_out() {
    let m = model(serde_json::json!({
        "openapi": "3.2.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/a": {
                "additionalOperations": {
                    "PURGE\n": { "operationId": "bad", "responses": ok() },
                    "PURGE": { "operationId": "good", "responses": ok() },
                },
            },
        },
    }));
    let ids: Vec<&str> = m.operations.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["good"]);
    assert_eq!(invalid(&m).len(), 1, "{:?}", m.warnings);
}

#[test]
fn a_webhook_name_with_a_control_character_is_left_out() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "webhooks": {
            "bad\r": { "post": { "operationId": "bad", "responses": ok() } },
            "good": { "post": { "operationId": "good", "responses": ok() } },
        },
    }));
    let ids: Vec<&str> = m.webhooks.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["good"]);
    assert_eq!(invalid(&m).len(), 1, "{:?}", m.warnings);
}

#[test]
fn a_media_type_with_a_control_character_is_left_out() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/a": {
                "post": {
                    "operationId": "post",
                    "requestBody": {
                        "content": {
                            "application/json\nx": { "schema": { "type": "string" } },
                            "application/json": { "schema": { "type": "string" } },
                        },
                    },
                    "responses": {
                        "200": {
                            "description": "ok",
                            "content": {
                                "text/plain\u{2029}": { "schema": { "type": "string" } },
                                "text/plain": { "schema": { "type": "string" } },
                            },
                        },
                    },
                },
            },
        },
    }));
    let op = &m.operations[0];
    let request: Vec<&str> = op
        .body
        .as_ref()
        .unwrap()
        .contents
        .iter()
        .map(|c| c.media_type.as_str())
        .collect();
    assert_eq!(request, ["application/json"]);
    let response: Vec<&str> = op.responses[0]
        .contents
        .iter()
        .map(|c| c.media_type.as_str())
        .collect();
    assert_eq!(response, ["text/plain"]);
    assert_eq!(invalid(&m).len(), 2, "{:?}", m.warnings);
}

#[test]
fn a_channel_with_a_control_character_is_left_out() {
    let message = serde_json::json!({ "application/json": { "schema": { "type": "string" } } });
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {},
        "x-event-channels": {
            "bad\n": { "content": message },
            "badTemplate": { "channel": "boards/\r{id}", "content": message },
            "good": { "channel": "boards", "content": message },
        },
    }));
    let names: Vec<&str> = m.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["good"]);
    assert_eq!(invalid(&m).len(), 2, "{:?}", m.warnings);
}

#[test]
fn printable_text_is_kept() {
    // `*/`, quotes, backquotes and backslashes are escaped by the printers, not rejected.
    let path = "/a*/b\"\"\"c`d\\e$f";
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": { path: { "get": { "operationId": "a", "responses": ok() } } },
    }));
    assert_eq!(m.operations.len(), 1);
    assert!(invalid(&m).is_empty(), "{:?}", m.warnings);
}

/// A path key without a leading `/` would change the host or port a request goes to when it is
/// appended to a base URL without a path.
#[test]
fn a_path_key_without_a_leading_slash_is_left_out() {
    for bad in ["@evil.example/x", ".evil.example/x", ":8443/x", "x", ""] {
        let m = model(serde_json::json!({
            "openapi": "3.1.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                bad: { "get": { "operationId": "bad", "responses": ok() } },
                "/good": { "get": { "operationId": "good", "responses": ok() } },
            },
        }));
        let ids: Vec<&str> = m.operations.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["good"], "{bad:?}");
        assert_eq!(invalid(&m).len(), 1, "{bad:?}: {:?}", m.warnings);
    }
}

#[test]
fn webhook_names_need_no_leading_slash() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "webhooks": { "newPet": { "post": { "operationId": "newPet", "responses": ok() } } },
    }));
    assert_eq!(m.webhooks.len(), 1);
    assert!(invalid(&m).is_empty(), "{:?}", m.warnings);
}
