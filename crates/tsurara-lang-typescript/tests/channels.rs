//! Typed event channels (`x-event-channels`): the payload type, the topic builder and the
//! decoder of each channel, and the Zod schema map.

use std::process::Command;
use tsurara_lang_typescript::options::{ImportExtension, Validators};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root, tsc_path};

/// The event channel contract, with one more channel and a schema named like a
/// derived name.
const DEFINITION: &str = r##"
openapi: 3.1.0
info: { title: boards, version: "1" }
paths: {}
x-event-channels:
  board:
    summary: Updates every participant of a board may see
    channel: 'board/{boardId}'
    parameters:
      boardId: { type: string }
    private: true
    content:
      application/json:
        schema: { $ref: '#/components/schemas/BoardEvent' }
  board-user:
    channel: 'board/{boardId}/user/{userId}'
    parameters:
      boardId: { type: string }
      userId: { type: integer }
    content:
      application/json:
        schema: { $ref: '#/components/schemas/CardAdded' }
components:
  schemas:
    BoardEvent:
      oneOf:
        - $ref: '#/components/schemas/CardAdded'
        - $ref: '#/components/schemas/PhaseChanged'
      discriminator:
        propertyName: type
        mapping:
          card.added: '#/components/schemas/CardAdded'
          phase.changed: '#/components/schemas/PhaseChanged'
    CardAdded:
      type: object
      required: [type, seq, card]
      properties:
        type: { const: card.added }
        seq: { type: integer }
        card: { $ref: '#/components/schemas/Card' }
    PhaseChanged:
      type: object
      required: [type, seq, phase]
      properties:
        type: { const: phase.changed }
        seq: { type: integer }
        phase: { type: string }
    Card:
      type: object
      required: [id, body]
      properties:
        id: { type: string, format: uuid }
        body: { type: string }
"##;

fn files(opts: &TypeScriptOptions) -> Vec<(String, String)> {
    let set = DocumentSet::from_value(parse_document(DEFINITION.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), opts).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("no {path}"))
        .1
}

#[test]
fn a_channel_has_a_message_type_a_topic_builder_and_a_decoder() {
    let files = files(&TypeScriptOptions::default());
    let board = file(&files, "channels/board.ts");
    assert!(
        board.contains("export type BoardMessage = BoardEvent;"),
        "{board}"
    );
    assert!(
        board.contains("export type BoardTopicParams = {\n  boardId: string;\n};"),
        "{board}"
    );
    assert!(
        board.contains("export function boardTopic(params: BoardTopicParams): string"),
        "{board}"
    );
    assert!(board.contains("export function decodeBoard("), "{board}");
    assert!(file(&files, "channels/boardUser.ts").contains("userId: number"));
    // Channels are not in the barrel, like the query and Zod files.
    assert!(!file(&files, "index.ts").contains("channels/"));
    insta::assert_snapshot!(board);
}

const USAGE: &str = r#"import { boardTopic, decodeBoard } from './channels/board.js';

export function apply(data: string): string {
  const r = decodeBoard(data);
  if (!r.ok) return r.kind;
  switch (r.value.type) {
    case 'card.added':
      return r.value.card.body;
    case 'phase.changed':
      return r.value.phase;
  }
}

export const topic: string = boardTopic({ boardId: 'a' });
// @ts-expect-error The parameter is required.
export const missing: string = boardTopic({});
"#;

#[test]
fn messages_narrow_by_type() {
    let mut files = files(&TypeScriptOptions::default());
    files.push(("usage.ts".into(), USAGE.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

#[test]
fn zod_schemas_per_channel() {
    let files = files(&TypeScriptOptions {
        validators: Some(Validators::Zod),
        ..TypeScriptOptions::default()
    });
    let schemas = file(&files, "zod/channels.ts");
    assert!(
        schemas.contains("export const channelSchemas: ChannelSchemas = {"),
        "{schemas}"
    );
    assert!(
        schemas.contains("board: z.lazy(() => BoardEventSchema),"),
        "{schemas}"
    );
    assert!(
        schemas.contains("'board-user': z.lazy(() => CardAddedSchema),"),
        "{schemas}"
    );
    assert_tsc_passes(&files, &TsOptions::default());
}

/// Runs the topic builder and the decoder with Node type stripping and the compiled client.
const RUN: &str = r#"import { decodeMessage, validateChannels } from '@tsurara/client';
import { boardTopic, decodeBoard } from './gen/channels/board.ts';
import { boardUserTopic } from './gen/channels/boardUser.ts';
import { channelSchemas } from './gen/zod/channels.ts';

const check = (cond: boolean, what: string): void => {
  if (!cond) throw new Error(what);
};
check(boardTopic({ boardId: 'a b' }) === 'board/a%20b', 'topic is encoded');
check(boardUserTopic({ boardId: 'b', userId: 7 }) === 'board/b/user/7', 'several parameters');
const ok = decodeBoard('{"type":"phase.changed","seq":1,"phase":"vote"}');
check(ok.ok && ok.value.type === 'phase.changed', 'decodes');
const parse = decodeBoard('{oops');
check(!parse.ok && parse.kind === 'parse', 'parse failure');
const validate = validateChannels(channelSchemas);
const invalid = decodeBoard('{"type":"phase.changed","seq":"1","phase":"vote"}', { validate });
check(!invalid.ok && invalid.kind === 'validation', 'validation failure');
void decodeMessage;
console.log('ok');
"#;

#[test]
fn topics_and_decoding_run_on_node() {
    let node_ok = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            let v = String::from_utf8_lossy(&o.stdout);
            let mut parts = v.trim().trim_start_matches('v').split('.');
            let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            (major, minor) >= (22, 18)
        });
    if !node_ok {
        assert!(
            std::env::var_os("TSURARA_REQUIRE_NODE").is_none(),
            "Node 22.18 or later is required"
        );
        eprintln!("skipping: Node 22.18 or later is required");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("node_modules/@tsurara/client");
    let built = Command::new(tsc_path().expect("tsc is installed"))
        .arg("-p")
        .arg(repo_root().join("runtimes/typescript/tsconfig.build.json"))
        .arg("--outDir")
        .arg(client.join("dist"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stdout)
    );
    std::fs::copy(
        repo_root().join("runtimes/typescript/package.json"),
        client.join("package.json"),
    )
    .unwrap();
    // Zod for the schema map.
    let zod = dir.path().join("node_modules/zod");
    std::os::unix::fs::symlink(repo_root().join("node_modules/zod"), &zod).unwrap();
    let opts = TypeScriptOptions {
        import_extension: ImportExtension::Ts,
        validators: Some(Validators::Zod),
        ..TypeScriptOptions::default()
    };
    for (path, contents) in files(&opts) {
        let full = dir.path().join("gen").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), RUN).unwrap();
    let out = Command::new("node")
        .arg("run.ts")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("ok"),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
