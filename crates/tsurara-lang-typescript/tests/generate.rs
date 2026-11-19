//! Generates from whole definitions and checks snapshots, determinism, and tsc.

use std::path::Path;
use tsurara_backend::OutputFile;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_lang_typescript::options::{EnumKeyCase, EnumStyle, Layout};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes};
use url::Url;

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap();
    let url = Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), bytes);
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    normalize_swagger2(&mut set).unwrap();
    set
}

fn load_loader_fixture(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tsurara-loader/tests/fixtures")
        .join(name);
    let value = parse_document(&std::fs::read(path).unwrap()).unwrap();
    let mut set = DocumentSet::from_documents([(
        Url::parse("https://petstore.swagger.io/v2/swagger.json").unwrap(),
        value,
    )]);
    normalize_swagger2(&mut set).unwrap();
    set
}

fn render(files: &[OutputFile]) -> String {
    files
        .iter()
        .map(|f| format!("=== {}\n{}", f.path, f.contents))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ts_files(files: &[OutputFile]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|f| (format!("generated/{}", f.path), f.contents.clone()))
        .collect()
}

#[test]
fn kitchen_sink_snapshot() {
    let set = load("kitchen-sink.yaml");
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    insta::assert_snapshot!(render(&out.files));
    let warnings: Vec<String> = out.warnings.iter().map(ToString::to_string).collect();
    insta::assert_snapshot!("kitchen_sink_warnings", warnings.join("\n"));
}

#[test]
fn petstore_snapshot() {
    let set = load("petstore3.json");
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    insta::assert_snapshot!(render(&out.files));
}

#[test]
fn output_is_deterministic() {
    for name in [
        "kitchen-sink.yaml",
        "petstore3.json",
        "v3.0-petstore-expanded.yaml",
    ] {
        let a = generate(
            &load(name),
            &Options::default(),
            &TypeScriptOptions::default(),
        )
        .unwrap();
        let b = generate(
            &load(name),
            &Options::default(),
            &TypeScriptOptions::default(),
        )
        .unwrap();
        assert_eq!(a.files, b.files, "{name}");
    }
}

#[test]
fn fixtures_pass_tsc_in_every_layout() {
    let fixtures = [
        "kitchen-sink.yaml",
        "petstore3.json",
        "v3.0-petstore-expanded.yaml",
        "v3.0-petstore.yaml",
        "v3.1-webhook-example.yaml",
        "v3.1-non-oauth-scopes.yaml",
    ];
    let mut files = Vec::new();
    for (i, name) in fixtures.iter().enumerate() {
        for (layout, lname) in [
            (Layout::Split, "split"),
            (Layout::Tags, "tags"),
            (Layout::Single, "single"),
        ] {
            let opts = TypeScriptOptions {
                layout,
                enum_style: if i % 2 == 0 {
                    EnumStyle::Union
                } else {
                    EnumStyle::Const
                },
                aggregate_maps: true,
                ..TypeScriptOptions::default()
            };
            let out = generate(&load(name), &Options::default(), &opts).unwrap();
            for f in out.files {
                files.push((format!("{name}/{lname}/{}", f.path), f.contents));
            }
        }
    }
    let swagger = generate(
        &load_loader_fixture("petstore-swagger2.json"),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    for f in swagger.files {
        files.push((format!("swagger2/{}", f.path), f.contents));
    }
    for (_, options) in TsOptions::matrix() {
        assert_tsc_passes(&files, &options);
    }
}

#[test]
fn import_extensions() {
    use tsurara_lang_typescript::TypeScriptOptions;
    use tsurara_lang_typescript::options::ImportExtension;
    let set = load("petstore3.json");
    for (ext, suffix) in [
        (ImportExtension::Js, ".js'"),
        (ImportExtension::Ts, ".ts'"),
        (ImportExtension::None, "/Pet'"),
    ] {
        let out = generate(
            &set,
            &Options::default(),
            &TypeScriptOptions {
                import_extension: ext,
                ..TypeScriptOptions::default()
            },
        )
        .unwrap();
        let op = out
            .files
            .iter()
            .find(|f| f.path == "operations/addPet.ts")
            .unwrap();
        assert!(
            op.contents.contains(&format!(
                "from '../schemas/Pet{}",
                suffix.trim_start_matches("/Pet")
            )),
            "{}",
            op.contents
        );
    }
    let _ = ts_files(&[]);
}

#[test]
fn enums_as_const_objects() {
    let doc = parse_document(
        br#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Status: { type: string, enum: [available, "in-stock", 1st] }
    Level: { type: integer, enum: [1, 2], nullable: true }
    Mixed: { enum: [a, { x: 1 }] }
"#,
    )
    .unwrap();
    let set = DocumentSet::from_value(doc);
    let out = generate(
        &set,
        &Options::default(),
        &TypeScriptOptions {
            enum_style: EnumStyle::Const,
            ..TypeScriptOptions::default()
        },
    )
    .unwrap();
    let pick = |p: &str| {
        out.files
            .iter()
            .find(|f| f.path == p)
            .unwrap()
            .contents
            .clone()
    };
    insta::assert_snapshot!(format!(
        "{}\n{}\n{}",
        pick("schemas/Status.ts"),
        pick("schemas/Level.ts"),
        pick("schemas/Mixed.ts")
    ));
    let index = pick("index.ts");
    assert!(index.contains("export * from './schemas/Status.js';"));
    assert!(index.contains("export type * from './schemas/Mixed.js';"));
}

#[test]
fn const_enum_keys_are_named_by_x_enum_varnames() {
    let doc = parse_document(
        br#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Color: { type: string, enum: [r, g], x-enum-varnames: [Red, Green] }
    Size: { type: integer, enum: [1, 2], x-enumNames: [Small, Large] }
"#,
    )
    .unwrap();
    let set = DocumentSet::from_value(doc);
    let out = generate(
        &set,
        &Options::default(),
        &TypeScriptOptions {
            enum_style: EnumStyle::Const,
            ..TypeScriptOptions::default()
        },
    )
    .unwrap();
    let pick = |p: &str| {
        out.files
            .iter()
            .find(|f| f.path == p)
            .unwrap()
            .contents
            .clone()
    };
    let color = pick("schemas/Color.ts");
    assert!(
        color.contains("export const Color = {\n  Red: 'r',\n  Green: 'g',\n} as const;"),
        "{color}"
    );
    let size = pick("schemas/Size.ts");
    assert!(
        size.contains("export const Size = {\n  Small: 1,\n  Large: 2,\n} as const;"),
        "{size}"
    );
}

#[test]
fn const_enum_keys_can_change_case() {
    let doc = parse_document(
        br#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    OrderStatus: { type: string, enum: [payment_pending, "in-stock", 1st] }
    Named: { type: string, enum: [x, y], x-enum-varnames: [PaymentPending, fullRefund] }
    Clash: { type: string, enum: [a-b, a_b] }
"#,
    )
    .unwrap();
    let set = DocumentSet::from_value(doc);
    let file = |case: EnumKeyCase, path: &str| {
        let out = generate(
            &set,
            &Options::default(),
            &TypeScriptOptions {
                enum_style: EnumStyle::Const,
                enum_key_case: case,
                ..TypeScriptOptions::default()
            },
        )
        .unwrap();
        out.files
            .iter()
            .find(|f| f.path == path)
            .unwrap()
            .contents
            .clone()
    };
    let status = file(EnumKeyCase::Constant, "schemas/OrderStatus.ts");
    assert!(
        status.contains(
            "  PAYMENT_PENDING: 'payment_pending',\n  IN_STOCK: 'in-stock',\n  '1ST': '1st',\n"
        ),
        "{status}"
    );
    let named = file(EnumKeyCase::Constant, "schemas/Named.ts");
    assert!(
        named.contains("  PAYMENT_PENDING: 'x',\n  FULL_REFUND: 'y',\n"),
        "{named}"
    );
    let pascal = file(EnumKeyCase::Pascal, "schemas/OrderStatus.ts");
    assert!(
        pascal.contains("  PaymentPending: 'payment_pending',\n  InStock: 'in-stock',\n"),
        "{pascal}"
    );
    let camel = file(EnumKeyCase::Camel, "schemas/Named.ts");
    assert!(
        camel.contains("  paymentPending: 'x',\n  fullRefund: 'y',\n"),
        "{camel}"
    );
    // Keys that become the same are numbered.
    let clash = file(EnumKeyCase::Constant, "schemas/Clash.ts");
    assert!(
        clash.contains("  A_B: 'a-b',\n  A_B_2: 'a_b',\n"),
        "{clash}"
    );
    let preserve = file(EnumKeyCase::Preserve, "schemas/OrderStatus.ts");
    assert!(
        preserve.contains("  payment_pending: 'payment_pending',\n"),
        "{preserve}"
    );
}
