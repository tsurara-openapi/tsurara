//! Security schemes and the requirements of each operation.

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::operation::SecurityUse;
use tsurara_sem::{Options, WarningKind, build_model};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
security: [{ bearer: [] }]
paths:
  /inherits:
    get:
      operationId: inherits
      responses: { "204": { description: ok } }
  /own:
    get:
      operationId: own
      security: [{ api_key: [] }, { oauth: [read, write] }, { api_key: [], basic: [] }]
      responses: { "204": { description: ok } }
  /public:
    get:
      operationId: public
      security: []
      responses: { "204": { description: ok } }
  /unknown:
    get:
      operationId: unknown
      security: [{ nope: [] }, { api_key: [] }]
      responses: { "204": { description: ok } }
components:
  securitySchemes:
    api_key: { type: apiKey, in: header, name: X-Api-Key }
    bearer: { type: http, scheme: bearer, bearerFormat: JWT }
    basic: { type: http, scheme: basic }
    oauth:
      type: oauth2
      flows: { clientCredentials: { tokenUrl: "https://x/token", scopes: { read: r, write: w } } }
"##;

fn names(alternative: &[SecurityUse]) -> Vec<&str> {
    alternative.iter().map(|u| u.scheme.as_str()).collect()
}

#[test]
fn operations_resolve_their_requirements() {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    assert_eq!(
        model.security_schemes,
        ["api_key", "bearer", "basic", "oauth"]
    );
    let op = |id: &str| model.operations.iter().find(|o| o.id == id).unwrap();

    let inherits = &op("inherits").security;
    assert_eq!(inherits.len(), 1);
    assert_eq!(inherits[0][0].kind, "http");
    assert_eq!(inherits[0][0].http_scheme.as_deref(), Some("bearer"));

    let own = &op("own").security;
    assert_eq!(
        own.iter().map(|a| names(a)).collect::<Vec<_>>(),
        [vec!["api_key"], vec!["oauth"], vec!["api_key", "basic"]]
    );
    assert_eq!(own[0][0].location.as_deref(), Some("header"));
    assert_eq!(own[0][0].name.as_deref(), Some("X-Api-Key"));
    assert_eq!(own[1][0].scopes, ["read", "write"]);

    // `security: []` means no authentication: one empty alternative.
    assert_eq!(op("public").security, vec![Vec::<SecurityUse>::new()]);

    // An alternative with an unknown scheme cannot be satisfied: it is dropped with a warning.
    assert_eq!(
        op("unknown")
            .security
            .iter()
            .map(|a| names(a))
            .collect::<Vec<_>>(),
        [vec!["api_key"]]
    );
    assert!(
        model
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::InvalidDefinition && w.message.contains("nope"))
    );
}
