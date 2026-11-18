//! The definition's constraints in the Zod schemas (`typescript.validatorConstraints: true`): lengths, bounds, `multipleOf`, `pattern`, item and property counts, and formats with a Zod
//! counterpart. Off by default, so that response validation does not become stricter.

use std::process::Command;
use tsurara_lang_typescript::options::{ImportExtension, Validators};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    C:
      type: object
      properties:
        n: { type: integer, minimum: 0 }
        m: { type: number, exclusiveMinimum: 0, maximum: 10, multipleOf: 0.5 }
        s: { type: string, minLength: 1, maxLength: 3, pattern: "^[a-z]+$" }
        list: { type: array, items: { type: string }, minItems: 1, maxItems: 2, uniqueItems: true }
        bag: { type: object, additionalProperties: { type: integer }, minProperties: 1, maxProperties: 2 }
        maybe: { type: [string, "null"], maxLength: 2 }
        uri: { type: string, format: uri }
        day: { type: string, format: date }
        name: { $ref: "#/components/schemas/Name" }
    Name: { type: string, minLength: 2 }
"##;

/// OpenAPI 3.0: `exclusiveMinimum` is a boolean next to `minimum`.
const DOC_30: &str = r##"
openapi: 3.0.3
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    D:
      type: object
      properties:
        x: { type: number, minimum: 0, exclusiveMinimum: true }
"##;

fn generate_with(doc: &str, opts: &TypeScriptOptions) -> tsurara_backend::Output {
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    generate(&set, &Options::default(), opts).unwrap()
}

fn constraints() -> TypeScriptOptions {
    TypeScriptOptions {
        validators: Some(Validators::Zod),
        validator_constraints: true,
        ..TypeScriptOptions::default()
    }
}

fn file<'a>(out: &'a tsurara_backend::Output, path: &str) -> &'a str {
    &out.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path}"))
        .contents
}

/// Each line is a schema, a JSON value and whether it must be accepted.
const CASES: &str = r#"
C	{"n":0}	true
C	{"n":-1}	false
C	{"m":10}	true
C	{"m":0}	false
C	{"m":10.5}	false
C	{"m":2.25}	false
C	{"s":"abc"}	true
C	{"s":""}	false
C	{"s":"abcd"}	false
C	{"s":"A"}	false
C	{"list":["a","b"]}	true
C	{"list":[]}	false
C	{"list":["a","b","c"]}	false
C	{"list":["a","a"]}	false
C	{"bag":{"a":1}}	true
C	{"bag":{}}	false
C	{"bag":{"a":1,"b":2,"c":3}}	false
C	{"maybe":null}	true
C	{"maybe":"ab"}	true
C	{"maybe":"abc"}	false
C	{"uri":"https://example.com/x"}	true
C	{"uri":"not a uri"}	false
C	{"day":"2026-11-17"}	true
C	{"day":"17/11/2026"}	false
C	{"name":"ab"}	true
C	{"name":"a"}	false
D	{"x":0.5}	true
D	{"x":0}	false
"#;

const RUN: &str = r#"import { readFileSync } from 'node:fs';
import { CSchema } from './gen/zod/C.ts';
import { DSchema } from './gen/zod/D.ts';

const schemas = { C: CSchema, D: DSchema };
const failures = [];
for (const line of readFileSync('cases.tsv', 'utf8').split('\n')) {
  if (line.trim() === '') continue;
  const [name, json, expected] = line.split('\t');
  const ok = schemas[name].safeParse(JSON.parse(json)).success;
  if (String(ok) !== expected) failures.push(`${name} ${json}: ${ok}`);
}
console.log(failures.length === 0 ? 'ok' : failures.join('\n'));
"#;

#[test]
fn constraints_accept_and_reject_the_boundaries() {
    if tsurara_testkit::tsc_path().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    std::os::unix::fs::symlink(
        std::fs::canonicalize(repo_root().join("node_modules/zod")).unwrap(),
        dir.path().join("node_modules/zod"),
    )
    .unwrap();
    let opts = TypeScriptOptions {
        import_extension: ImportExtension::Ts,
        ..constraints()
    };
    for doc in [DOC, DOC_30] {
        for f in generate_with(doc, &opts).files {
            if f.path.starts_with("zod/") || f.path.starts_with("schemas/") {
                let full = dir.path().join("gen").join(&f.path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, f.contents).unwrap();
            }
        }
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("cases.tsv"), CASES).unwrap();
    std::fs::write(dir.path().join("run.ts"), RUN).unwrap();
    let out = Command::new("node")
        .arg("run.ts")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}

#[test]
fn without_the_option_the_schemas_are_unchanged() {
    let plain = TypeScriptOptions {
        validators: Some(Validators::Zod),
        ..TypeScriptOptions::default()
    };
    let out = generate_with(DOC, &plain);
    let c = file(&out, "zod/C.ts");
    for check in [
        ".min(",
        ".max(",
        ".gt(",
        ".gte(",
        "multipleOf",
        "regex",
        "refine",
        "z.url",
        "z.iso.date",
    ] {
        assert!(!c.contains(check), "{check} in {c}");
    }
    // The types and their JSDoc are the same with and without the option.
    let with = generate_with(DOC, &constraints());
    assert_eq!(file(&out, "schemas/C.ts"), file(&with, "schemas/C.ts"));
}

#[test]
fn constrained_schemas_type_check() {
    let mut files: Vec<(String, String)> = generate_with(DOC, &constraints())
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    files.extend(
        generate_with(DOC_30, &constraints())
            .files
            .into_iter()
            .filter(|f| f.path.ends_with("/D.ts"))
            .map(|f| (f.path, f.contents)),
    );
    assert_tsc_passes(&files, &TsOptions::default());
}

#[test]
fn a_pattern_javascript_cannot_compile_is_left_out_with_a_warning() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    P:
      type: object
      properties:
        a: { type: string, pattern: "(?<x" }
        b: { type: string, pattern: "\\p{NotAProperty}" }
        c: { type: string, pattern: "^\\d+$" }
"##;
    let out = generate_with(doc, &constraints());
    let p = file(&out, "zod/P.ts");
    assert!(!p.contains("(?<x") && !p.contains("NotAProperty"), "{p}");
    assert!(p.contains(r"^\\d+$"), "{p}");
    let pattern_warnings = out
        .warnings
        .iter()
        .filter(|w| w.message.contains("pattern"))
        .count();
    assert_eq!(pattern_warnings, 2, "{:?}", out.warnings);
    let files: Vec<(String, String)> = out
        .files
        .iter()
        .map(|f| (f.path.clone(), f.contents.clone()))
        .collect();
    assert_tsc_passes(&files, &TsOptions::default());
}
