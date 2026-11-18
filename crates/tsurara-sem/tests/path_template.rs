//! The `{name}` variables of a path template and the `in: path` parameters must agree.
//! A variable without a parameter becomes a required string parameter; a parameter that is not in
//! the template is left out. Both are `invalid-definition` warnings.

use tsurara_loader::DocumentSet;
use tsurara_sem::operation::ParamIn;
use tsurara_sem::warning::WarningKind;
use tsurara_sem::{Options, build_model};

fn model(doc: serde_json::Value) -> tsurara_sem::model::Model {
    build_model(&DocumentSet::from_value(doc), &Options::default()).unwrap()
}

fn path_params(op: &tsurara_sem::operation::Operation) -> Vec<(&str, bool)> {
    op.params
        .iter()
        .filter(|p| p.location == ParamIn::Path)
        .map(|p| (p.name.as_str(), p.required))
        .collect()
}

fn invalid(m: &tsurara_sem::model::Model) -> Vec<String> {
    m.warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidDefinition)
        .map(|w| w.message.clone())
        .collect()
}

/// A Huma v2 document whose POST has only a JSON body.
#[test]
fn a_variable_without_a_parameter_becomes_a_required_string() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/api/realtime/echo-channels/{id}/messages": {
                "post": {
                    "operationId": "echo-post",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object" } } } },
                    "responses": { "204": { "description": "done" } },
                },
            },
        },
    }));
    let op = &m.operations[0];
    assert_eq!(path_params(op), [("id", true)]);
    assert_eq!(op.params[0].ty, tsurara_sem::ir::Type::String);
    let w = invalid(&m);
    assert_eq!(w.len(), 1, "{:?}", m.warnings);
    assert!(
        w[0].contains("echo-post") && w[0].contains("`id`"),
        "{}",
        w[0]
    );
}

#[test]
fn a_parameter_not_in_the_template_is_left_out() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/pets/{petId}": {
                "parameters": [{ "name": "petId", "in": "path", "required": true, "schema": { "type": "integer" } }],
                "get": {
                    "parameters": [{ "name": "ownerId", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "204": { "description": "done" } },
                },
            },
        },
    }));
    let op = &m.operations[0];
    assert_eq!(path_params(op), [("petId", true)]);
    let w = invalid(&m);
    assert_eq!(w.len(), 1, "{:?}", m.warnings);
    // Without an operationId the operation is named by its method and path.
    assert!(
        w[0].contains("GET /pets/{petId}") && w[0].contains("`ownerId`"),
        "{}",
        w[0]
    );
}

#[test]
fn agreeing_templates_and_parameters_do_not_warn() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/a/{x}/b/{y}.json": {
                "parameters": [{ "name": "x", "in": "path", "required": true, "schema": { "type": "string" } }],
                "get": {
                    "parameters": [
                        { "name": "y", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "x", "in": "query", "schema": { "type": "string" } },
                    ],
                    "responses": { "204": { "description": "done" } },
                },
            },
        },
        "webhooks": {
            "{notAPath}": { "post": { "responses": { "204": { "description": "done" } } } },
        },
    }));
    assert_eq!(path_params(&m.operations[0]), [("x", true), ("y", true)]);
    assert!(invalid(&m).is_empty(), "{:?}", m.warnings);
}

#[test]
fn both_directions_on_one_operation_warn_twice() {
    let m = model(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/a/{x}/{y}": {
                "get": {
                    "operationId": "a",
                    "parameters": [{ "name": "z", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "204": { "description": "done" } },
                },
            },
        },
    }));
    assert_eq!(path_params(&m.operations[0]), [("x", true), ("y", true)]);
    let w = invalid(&m);
    assert_eq!(w.len(), 2, "{:?}", m.warnings);
    assert!(w[0].contains("`x`, `y`"), "{}", w[0]);
    assert!(w[1].contains("`z`"), "{}", w[1]);
}
