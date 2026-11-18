//! Request bodies with several media types: `body` is tied to `contentType`.

use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /items/{id}:
    put:
      operationId: putItem
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      requestBody:
        required: true
        content:
          application/json:
            schema: { type: object, required: [name], properties: { name: { type: string } } }
          multipart/form-data:
            schema: { type: object, required: [file], properties: { file: { type: string, format: binary } } }
      responses: { "204": { description: ok } }
  /items:
    patch:
      operationId: patchItem
      requestBody:
        content:
          application/json: { schema: { type: object, properties: { name: { type: string } } } }
          text/plain: { schema: { type: string } }
      responses: { "204": { description: ok } }
"##;

fn files() -> Vec<(String, String)> {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    out.files
        .into_iter()
        .map(|f| (format!("gen/{}", f.path), f.contents))
        .collect()
}

#[test]
fn data_is_a_union_per_media_type() {
    let files = files();
    let (_, put) = files
        .iter()
        .find(|(p, _)| p == "gen/operations/putItem.ts")
        .unwrap();
    assert!(
        put.contains(
            "export type PutItemData = {\n  path: { id: string };\n} & (\n  | { contentType?: 'application/json'; body: { name: string } }\n  | {\n    contentType: 'multipart/form-data';"
        ),
        "{put}"
    );
}

#[test]
fn body_is_tied_to_content_type() {
    let mut files = files();
    files.push((
        "consumer.ts".to_owned(),
        r#"import { createClient } from '@tsurara/client';
import { patchItem } from './gen/operations/patchItem.js';
import { putItem } from './gen/operations/putItem.js';

const client = createClient({ baseUrl: 'https://x.test' });
const file = new Blob(['x']);

void putItem(client, { path: { id: '1' }, body: { name: 'a' } });
void putItem(client, { path: { id: '1' }, contentType: 'application/json', body: { name: 'a' } });
void putItem(client, { path: { id: '1' }, contentType: 'multipart/form-data', body: { file } });
// @ts-expect-error multipart values need `contentType: 'multipart/form-data'`
void putItem(client, { path: { id: '1' }, body: { file } });
// @ts-expect-error the JSON value does not fit the multipart member
void putItem(client, { path: { id: '1' }, contentType: 'multipart/form-data', body: { name: 'a' } });

// An optional body can be omitted in every member.
void patchItem(client, {});
void patchItem(client, { contentType: 'text/plain' });
void patchItem(client, { contentType: 'text/plain', body: 'x' });
// @ts-expect-error a string body needs `contentType: 'text/plain'`
void patchItem(client, { body: 'x' });
"#
        .to_owned(),
    ));
    assert_tsc_passes(&files, &TsOptions::default());
}
