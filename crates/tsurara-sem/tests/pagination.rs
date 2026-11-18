//! `x-tsurara-pagination` on a read: which request field carries the page, its first
//! value, and where the next and previous values are in a response. Read into `Operation::pagination`;
//! an invalid one is an `invalid-definition` warning and is left out.

use tsurara_loader::DocumentSet;
use tsurara_sem::operation::Pagination;
use tsurara_sem::warning::WarningKind;
use tsurara_sem::{Options, build_model};

fn model(pagination: serde_json::Value) -> tsurara_sem::model::Model {
    let doc = serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {
            "/pets": {
                "get": {
                    "operationId": "listPets",
                    "parameters": [{ "name": "cursor", "in": "query", "schema": { "type": "string" } }],
                    "x-tsurara-pagination": pagination,
                    "responses": { "200": { "description": "ok" } },
                },
            },
        },
    });
    build_model(&DocumentSet::from_value(doc), &Options::default()).unwrap()
}

fn invalid(m: &tsurara_sem::model::Model) -> usize {
    m.warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidDefinition)
        .count()
}

#[test]
fn pagination_is_read() {
    let m = model(
        serde_json::json!({ "param": "query.cursor", "next": "/nextCursor", "previous": "/prev" }),
    );
    assert_eq!(
        m.operations[0].pagination,
        Some(Pagination {
            param: "query.cursor".into(),
            initial: None,
            next: Some("/nextCursor".into()),
            previous: Some("/prev".into()),
        })
    );
    assert_eq!(invalid(&m), 0, "{:?}", m.warnings);
    let m = model(serde_json::json!({ "param": "query.cursor", "initial": "first" }));
    let p = m.operations[0].pagination.as_ref().unwrap();
    assert_eq!(p.initial, Some(serde_json::json!("first")));
    assert_eq!(p.next, None);
}

#[test]
fn invalid_pagination_is_left_out() {
    for bad in [
        serde_json::json!({ "param": "query.page" }),
        serde_json::json!({ "param": "cursor" }),
        serde_json::json!({ "param": "body.cursor" }),
        serde_json::json!({ "param": "query.cursor", "next": "nextCursor" }),
        serde_json::json!({ "next": "/nextCursor" }),
        serde_json::json!("query.cursor"),
    ] {
        let m = model(bad.clone());
        assert_eq!(m.operations[0].pagination, None, "{bad}");
        assert_eq!(invalid(&m), 1, "{bad}: {:?}", m.warnings);
    }
}
