//! Typed Server-Sent Events from `itemSchema` and `x-itemSchema` (design.md 8.5).

use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::operation::BodyKind;
use tsurara_sem::{Options, build_model};
use tsurara_testkit::{TsOptions, assert_tsc_passes};

/// A document with one event stream per shape of item schema. `{VERSION}` and `{KEY}` are replaced.
const DOC: &str = r##"
openapi: {VERSION}
info: { title: t, version: "1" }
paths:
  /stream:
    get:
      operationId: stream
      responses:
        "200":
          description: ok
          content:
            text/event-stream:
              schema: { type: string }
              {KEY}:
                type: object
                properties:
                  event: { type: string }
                  data: { type: string }
                  id: { type: string }
                  retry: { type: integer, minimum: 0 }
                required: [event, data]
                oneOf:
                  - properties:
                      event: { const: note }
                      data:
                        type: string
                        contentMediaType: application/json
                        contentSchema: { $ref: "#/components/schemas/Note" }
                  - properties:
                      event: { const: noteDeleted }
                      data:
                        type: string
                        contentMediaType: application/json
                        contentSchema:
                          type: object
                          required: [id]
                          properties: { id: { type: string } }
        "400":
          description: bad
          content:
            application/json: { schema: { $ref: "#/components/schemas/Note" } }
  /mixed:
    get:
      operationId: mixed
      responses:
        "200":
          description: ok
          content:
            text/event-stream:
              {KEY}:
                oneOf:
                  - $ref: "#/components/schemas/NoteEvent"
                  - properties:
                      event: { enum: [ping, pong] }
                      data: { type: string }
                  - properties:
                      data: { type: string, contentMediaType: application/json }
  /plain:
    get:
      operationId: plain
      responses:
        "200":
          description: ok
          content:
            text/event-stream:
              {KEY}:
                type: object
                required: [data, id]
                properties:
                  data:
                    type: string
                    contentMediaType: application/json
                    contentSchema: { $ref: "#/components/schemas/Note" }
  /raw:
    get:
      operationId: raw
      responses:
        "200":
          description: ok
          content:
            text/event-stream: { schema: { type: string } }
  /both:
    get:
      operationId: both
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { $ref: "#/components/schemas/Note" } }
            text/event-stream:
              {KEY}:
                properties:
                  data: { type: string }
components:
  schemas:
    Note:
      type: object
      required: [id]
      properties: { id: { type: string }, text: { type: string } }
    NoteEvent:
      type: object
      properties:
        event: { const: note }
        data:
          type: string
          contentMediaType: application/json
          contentSchema: { $ref: "#/components/schemas/Note" }
"##;

fn set(version: &str, key: &str) -> DocumentSet {
    let doc = DOC.replace("{VERSION}", version).replace("{KEY}", key);
    DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap())
}

fn files(version: &str, key: &str) -> Vec<(String, String)> {
    let out = generate(
        &set(version, key),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    assert!(out.warnings.len() <= 1, "{:?}", out.warnings);
    out.files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files.iter().find(|(p, _)| p == path).unwrap().1
}

#[test]
fn events_are_a_union_discriminated_by_event() {
    let files = files("3.1.1", "x-itemSchema");
    let stream = file(&files, "operations/stream.ts");
    insta::assert_snapshot!(stream);
    assert!(
        stream.contains("200: AsyncIterable<StreamEvent>;"),
        "{stream}"
    );
    assert!(
        stream.contains("200: { parseAs: 'events', jsonEvents: ['note', 'noteDeleted'] },"),
        "{stream}"
    );
}

#[test]
fn item_schema_of_3_2_and_the_extension_give_the_same_output() {
    assert_eq!(
        file(&files("3.2.0", "itemSchema"), "operations/stream.ts"),
        file(&files("3.1.1", "x-itemSchema"), "operations/stream.ts"),
    );
}

#[test]
fn only_events_with_json_data_are_listed_when_they_differ() {
    let files = files("3.1.1", "x-itemSchema");
    let mixed = file(&files, "operations/mixed.ts");
    assert!(
        mixed.contains(
            "export type MixedEvent =\n  | { event: 'note'; data: Note; id?: string; retry?: number }\n  | { event: 'ping' | 'pong'; data: string; id?: string; retry?: number }\n  | { event: string; data: string; id?: string; retry?: number };\n"
        ),
        "{mixed}"
    );
    assert!(
        mixed.contains("200: { parseAs: 'events', jsonEvents: ['note'] },"),
        "{mixed}"
    );
}

#[test]
fn json_data_without_an_event_name_among_text_events_warns() {
    let out = generate(
        &set("3.1.1", "x-itemSchema"),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    let warnings: Vec<String> = out.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("~1mixed") && warnings[0].contains("oneOf/2"),
        "{warnings:?}"
    );
}

#[test]
fn an_item_schema_without_alternatives_is_one_event() {
    let files = files("3.1.1", "x-itemSchema");
    let plain = file(&files, "operations/plain.ts");
    assert!(
        plain.contains(
            "export type PlainEvent = {\n  event: string;\n  data: Note;\n  id: string;\n  retry?: number;\n};\n"
        ),
        "{plain}"
    );
    assert!(
        plain.contains("200: { parseAs: 'events', jsonEvents: true },"),
        "{plain}"
    );
}

#[test]
fn a_stream_without_an_item_schema_stays_a_byte_stream() {
    let files = files("3.1.1", "x-itemSchema");
    let raw = file(&files, "operations/raw.ts");
    assert!(raw.contains("200: ReadableStream<Uint8Array>;"), "{raw}");
    assert!(raw.contains("200: { parseAs: 'stream' },"), "{raw}");
    assert!(!raw.contains("RawEvent"), "{raw}");
}

#[test]
fn events_next_to_another_media_type() {
    let files = files("3.1.1", "x-itemSchema");
    let both = file(&files, "operations/both.ts");
    assert!(
        both.contains("200: Note | AsyncIterable<BothEvent>;"),
        "{both}"
    );
    assert!(
        both.contains(
            "200: { parseAs: 'json', contentTypes: { 'application/json': 'json', 'text/event-stream': 'events' } },"
        ),
        "{both}"
    );
}

#[test]
fn the_body_kind_stays_stream_for_other_backends() {
    let model = build_model(&set("3.1.1", "x-itemSchema"), &Options::default()).unwrap();
    let op = model.operations.iter().find(|o| o.id == "stream").unwrap();
    let content = &op.responses[0].contents[0];
    assert_eq!(content.body_kind, BodyKind::Stream);
    assert!(content.events.is_some());
    // Bodies that are not event streams, and requests, have none.
    assert!(op.responses[1].contents[0].events.is_none());
}

#[test]
fn generated_events_pass_tsc() {
    let mut files = files("3.1.1", "x-itemSchema");
    files.push((
        "use.ts".to_owned(),
        r#"import type { Client } from '@tsurara/client';
import { type StreamEvent, stream } from './operations/stream.ts';

export const run = async (client: Client<unknown>): Promise<string[]> => {
  const out: string[] = [];
  const result = await stream(client);
  if (!result.ok) return out;
  for await (const e of result.data) {
    switch (e.event) {
      case 'note':
        out.push(e.data.text ?? e.data.id);
        break;
      case 'noteDeleted':
        out.push(e.data.id, e.id ?? '');
        break;
      default:
        e satisfies never;
    }
  }
  return out;
};

export const first: StreamEvent = { event: 'note', data: { id: '1' } };
"#
        .to_owned(),
    ));
    for (_, options) in TsOptions::matrix() {
        assert_tsc_passes(&files, &options);
    }
}
