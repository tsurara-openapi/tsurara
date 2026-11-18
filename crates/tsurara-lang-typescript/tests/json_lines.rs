//! Typed JSON Lines and JSON text sequences from `itemSchema` and `x-itemSchema` (design.md 8.5).

use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::operation::BodyKind;
use tsurara_sem::{Options, build_model};
use tsurara_testkit::{TsOptions, assert_tsc_passes};

const DOC: &str = r##"
openapi: 3.2.0
info: { title: t, version: "1" }
paths:
  /lines:
    get:
      operationId: lines
      responses:
        "200":
          description: ok
          content:
            application/jsonl:
              itemSchema: { $ref: "#/components/schemas/Item" }
  /ndjson:
    get:
      operationId: ndjson
      responses:
        "200":
          description: ok
          content:
            application/x-ndjson:
              x-itemSchema: { type: object, required: [n], properties: { n: { type: integer } } }
  /seq:
    get:
      operationId: seq
      responses:
        "200":
          description: ok
          content:
            application/json-seq:
              itemSchema: { $ref: "#/components/schemas/Item" }
  /raw:
    get:
      operationId: raw
      responses:
        "200":
          description: ok
          content:
            application/jsonl: { schema: { type: string } }
  /both:
    get:
      operationId: both
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { type: array, items: { $ref: "#/components/schemas/Item" } } }
            application/jsonl:
              itemSchema: { $ref: "#/components/schemas/Item" }
components:
  schemas:
    Item:
      type: object
      required: [id]
      properties: { id: { type: string }, name: { type: string } }
"##;

fn set() -> DocumentSet {
    DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap())
}

fn files() -> Vec<(String, String)> {
    let out = generate(&set(), &Options::default(), &TypeScriptOptions::default()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files.iter().find(|(p, _)| p == path).unwrap().1
}

#[test]
fn json_lines_are_an_async_iterable_of_items() {
    let files = files();
    let lines = file(&files, "operations/lines.ts");
    assert!(lines.contains("export type LinesItem = Item;\n"), "{lines}");
    assert!(lines.contains("200: AsyncIterable<LinesItem>;"), "{lines}");
    assert!(lines.contains("200: { parseAs: 'jsonLines' },"), "{lines}");
    let ndjson = file(&files, "operations/ndjson.ts");
    assert!(
        ndjson.contains("export type NdjsonItem = {\n  n: number;\n};\n")
            && ndjson.contains("200: AsyncIterable<NdjsonItem>;"),
        "{ndjson}"
    );
    assert!(
        ndjson.contains("200: { parseAs: 'jsonLines' },"),
        "{ndjson}"
    );
}

#[test]
fn json_text_sequences_are_read_by_record_separator() {
    let files = files();
    let seq = file(&files, "operations/seq.ts");
    assert!(seq.contains("200: AsyncIterable<SeqItem>;"), "{seq}");
    assert!(seq.contains("200: { parseAs: 'jsonSeq' },"), "{seq}");
}

#[test]
fn a_stream_without_an_item_schema_stays_bytes() {
    let files = files();
    let raw = file(&files, "operations/raw.ts");
    assert!(raw.contains("200: ReadableStream<Uint8Array>;"), "{raw}");
    assert!(raw.contains("200: { parseAs: 'stream' },"), "{raw}");
}

#[test]
fn items_next_to_json() {
    let files = files();
    let both = file(&files, "operations/both.ts");
    assert!(
        both.contains("200: Item[] | AsyncIterable<BothItem>;"),
        "{both}"
    );
    assert!(
        both.contains(
            "200: { parseAs: 'json', contentTypes: { 'application/json': 'json', 'application/jsonl': 'jsonLines' } },"
        ),
        "{both}"
    );
}

#[test]
fn the_model_keeps_the_stream_body_kind_with_the_item_type() {
    let model = build_model(&set(), &Options::default()).unwrap();
    let op = model.operations.iter().find(|o| o.id == "lines").unwrap();
    let content = &op.responses[0].contents[0];
    assert_eq!(content.body_kind, BodyKind::Stream);
    assert!(content.items.is_some());
    let raw = model.operations.iter().find(|o| o.id == "raw").unwrap();
    assert!(raw.responses[0].contents[0].items.is_none());
}

#[test]
fn generated_items_pass_tsc() {
    let mut files = files();
    files.push((
        "use.ts".to_owned(),
        r#"import type { Client } from '@tsurara/client';
import { lines } from './operations/lines.ts';

export const run = async (client: Client<unknown>): Promise<string[]> => {
  const out: string[] = [];
  const result = await lines(client);
  if (!result.ok) return out;
  for await (const item of result.data) out.push(item.name ?? item.id);
  return out;
};
"#
        .to_owned(),
    ));
    for (_, options) in TsOptions::matrix() {
        assert_tsc_passes(&files, &options);
    }
}
