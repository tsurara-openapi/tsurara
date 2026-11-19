use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_lang_typescript::options::{FunctionCase, NameCase};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::options::Filter;
use tsurara_sem::{Options, WarningKind, build_model};

/// The model with the names TypeScript assigns.
fn named(set: &DocumentSet, opts: &Options) -> tsurara_sem::Model {
    named_with(set, opts, &TypeScriptOptions::default())
}

fn named_with(set: &DocumentSet, opts: &Options, ts: &TypeScriptOptions) -> tsurara_sem::Model {
    tsurara_lang_typescript::lower::lower_model(&build_model(set, opts).unwrap(), opts, ts).0
}

fn set(yaml: &str) -> DocumentSet {
    DocumentSet::from_value(parse_document(yaml.as_bytes()).unwrap())
}

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets]
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { type: array, items: { $ref: "#/components/schemas/Pet" } } }
    post:
      operationId: listPets
      tags: [admin]
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { $ref: "#/components/schemas/pet" } }
  /pets/{id}:
    delete:
      operationId: delete
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      responses: { "204": { description: gone } }
    get:
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      responses: { "204": { description: ok } }
components:
  schemas:
    Pet:
      type: object
      properties:
        "@type": { type: string }
        class: { type: string }
        "x-y": { type: string }
        createdAt: { $ref: "#/components/schemas/Date" }
    pet: { type: string }
    Date: { type: string, format: date-time }
    pet_store: { type: string }
    Unused: { type: string }
"##;

#[test]
fn property_names_are_quoted_not_transformed() {
    let out = generate(
        &set(DOC),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    let pet = out
        .files
        .iter()
        .find(|f| f.path == "schemas/Pet.ts")
        .unwrap();
    assert!(
        pet.contents.contains("'@type'?: string;"),
        "{}",
        pet.contents
    );
    assert!(pet.contents.contains("class?: string;"));
    assert!(pet.contents.contains("'x-y'?: string;"));
    assert!(pet.contents.contains("createdAt?: DateModel;"));
}

#[test]
fn colliding_names() {
    let model = named(&set(DOC), &Options::default());
    let names: Vec<&str> = model.schemas.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Pet", "pet2", "DateModel", "pet_store", "Unused"]);
    let ops: Vec<&str> = model.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(ops, ["listPets", "listPets2", "deleteModel", "getPetsById"]);
    // The operation id stays the operationId.
    assert_eq!(model.operations[1].id, "listPets");
    // Rename warnings come from TypeScript's naming, in the order names are claimed.
    let opts = Options::default();
    let (_, warnings) = tsurara_lang_typescript::lower::lower_model(
        &build_model(&set(DOC), &opts).unwrap(),
        &opts,
        &TypeScriptOptions::default(),
    );
    let renamed: Vec<String> = warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Renamed)
        .map(|w| format!("#{}", w.loc.pointer))
        .collect();
    assert_eq!(
        renamed,
        [
            "#/components/schemas/pet",
            "#/components/schemas/Date",
            "#/paths/~1pets/post",
            "#/paths/~1pets~1{id}/delete",
        ]
    );
}

#[test]
fn files_do_not_collide_on_case_insensitive_file_systems() {
    let out = generate(
        &set(DOC),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    let mut lower: Vec<String> = out.files.iter().map(|f| f.path.to_lowercase()).collect();
    let n = lower.len();
    lower.sort();
    lower.dedup();
    assert_eq!(lower.len(), n);
}

#[test]
fn names_are_stable() {
    let a = named(&set(DOC), &Options::default());
    let b = named(&set(DOC), &Options::default());
    let names = |m: &tsurara_sem::Model| -> Vec<String> {
        m.schemas
            .iter()
            .map(|d| d.name.clone())
            .chain(m.operations.iter().map(|o| o.name.clone()))
            .collect()
    };
    assert_eq!(names(&a), names(&b));
}

#[test]
fn overrides_fix_names() {
    let mut opts = Options::default();
    opts.name_overrides
        .insert("#/components/schemas/Date".into(), "Timestamp".into());
    opts.name_overrides
        .insert("#/paths/~1pets/post".into(), "createPet".into());
    let model = named(&set(DOC), &opts);
    assert!(model.schemas.iter().any(|d| d.name == "Timestamp"));
    assert!(model.operations.iter().any(|o| o.name == "createPet"));
}

#[test]
fn pascal_case_is_opt_in() {
    let opts = TypeScriptOptions {
        schema_name_case: NameCase::Pascal,
        ..TypeScriptOptions::default()
    };
    let model = named_with(&set(DOC), &Options::default(), &opts);
    let names: Vec<&str> = model.schemas.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Pet", "Pet2", "DateModel", "PetStore", "Unused"]);
}

#[test]
fn filter_drops_unreferenced_schemas() {
    let opts = Options {
        filter: Filter {
            tags: vec!["pets".into()],
            ..Filter::default()
        },
        ..Options::default()
    };
    let out = generate(&set(DOC), &opts, &TypeScriptOptions::default()).unwrap();
    let paths: Vec<&str> = out.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "client.ts",
            "index.ts",
            "operations/listPets.ts",
            "schemas/DateModel.ts",
            "schemas/Pet.ts"
        ]
    );
    let by_id = Options {
        filter: Filter {
            operation_ids: vec!["delete".into()],
            ..Filter::default()
        },
        ..Options::default()
    };
    let out = generate(&set(DOC), &by_id, &TypeScriptOptions::default()).unwrap();
    assert!(
        out.files
            .iter()
            .any(|f| f.path == "operations/deleteModel.ts")
    );
    assert!(!out.files.iter().any(|f| f.path.starts_with("schemas/")));
}

fn operation_files(filter: Filter) -> Vec<String> {
    let opts = Options {
        filter,
        ..Options::default()
    };
    generate(&set(DOC), &opts, &TypeScriptOptions::default())
        .unwrap()
        .files
        .into_iter()
        .filter_map(|f| f.path.strip_prefix("operations/").map(str::to_owned))
        .collect()
}

#[test]
fn filter_excludes_by_tag_and_operation_id() {
    // Everything but the excluded tag; schemas only that operation used are dropped.
    let by_tag = Filter {
        exclude_tags: vec!["admin".into()],
        ..Filter::default()
    };
    assert_eq!(
        operation_files(by_tag.clone()),
        ["deleteModel.ts", "getPetsById.ts", "listPets.ts"]
    );
    let opts = Options {
        filter: by_tag,
        ..Options::default()
    };
    let out = generate(&set(DOC), &opts, &TypeScriptOptions::default()).unwrap();
    let schemas: Vec<&str> = out
        .files
        .iter()
        .filter_map(|f| f.path.strip_prefix("schemas/"))
        .collect();
    assert_eq!(schemas, ["DateModel.ts", "Pet.ts"]);

    let by_id = Filter {
        exclude_operation_ids: vec!["delete".into()],
        ..Filter::default()
    };
    assert_eq!(
        operation_files(by_id),
        ["getPetsById.ts", "listPets.ts", "listPets2.ts"]
    );

    // An exclusion wins over an inclusion.
    let both = Filter {
        operation_ids: vec!["listPets".into(), "delete".into()],
        exclude_tags: vec!["admin".into()],
        ..Filter::default()
    };
    assert_eq!(operation_files(both), ["deleteModel.ts", "listPets.ts"]);
}

#[test]
fn stale_header_and_client_range() {
    let out = generate(
        &set(DOC),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    for f in &out.files {
        assert!(
            f.contents.starts_with(tsurara_lang_typescript::HEADER),
            "{}",
            f.path
        );
        assert!(
            !f.contents.contains("tsurara 0."),
            "no generator version in output"
        );
    }
    let index = out.files.iter().find(|f| f.path == "index.ts").unwrap();
    assert!(index.contents.contains("// @tsurara/client ^0.1.0"));
}

const DERIVED: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      responses: { "204": { description: ok } }
components:
  schemas:
    ListPetsResult: { type: string }
    EXTRA
"##;

fn derived_doc(extra: &str) -> String {
    DERIVED.replace("EXTRA", extra)
}

#[test]
fn derived_names_that_collide_move_aside_with_a_warning() {
    // The schema keeps its name; only the colliding derived name gets `Operation` before its suffix
    // and the function and the other derived names stay (design.md 6.6).
    let out = generate(
        &set(&derived_doc("ListPetsOperationResult: { type: string }")),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    let all: String = out.files.iter().map(|f| f.contents.as_str()).collect();
    assert!(
        all.contains("export type ListPetsResult = string;"),
        "{all}"
    );
    assert!(
        all.contains("export type ListPetsOperationResult = string;"),
        "{all}"
    );
    assert!(
        all.contains("export type ListPetsOperationResult2 ="),
        "{all}"
    );
    assert!(all.contains("export type ListPetsData ="), "{all}");
    assert!(all.contains("export function listPets"), "{all}");
    assert!(all.contains("Promise<ListPetsOperationResult2>"), "{all}");
    let renamed: Vec<&str> = out
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Renamed)
        .map(|w| w.message.as_str())
        .collect();
    assert!(
        renamed
            .iter()
            .any(|m| m.contains("`ListPetsResult`") && m.contains("ListPetsOperationResult2")),
        "{renamed:?}"
    );
}

#[test]
fn derived_name_collisions_are_resolved_with_overrides() {
    let mut opts = Options::default();
    opts.name_overrides.insert(
        "#/components/schemas/ListPetsResult".to_owned(),
        "ListPetsOutcome".to_owned(),
    );
    let model = named(&set(&derived_doc("")), &opts);
    assert_eq!(model.operations[0].name, "listPets");
    // Adding a schema whose name collides with the function itself renames only the schema side.
    let model = named(&set(&derived_doc("ListPetsData2: { type: string }")), &opts);
    assert_eq!(model.operations[0].name, "listPets");
}

#[test]
fn derived_names_of_webhooks_move_aside_too() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
webhooks:
  newPet:
    post:
      responses: { "204": { description: ok } }
components:
  schemas:
    NewPetWebhookRequest: { type: string }
"##;
    let out = generate(
        &set(doc),
        &Options::default(),
        &TypeScriptOptions::default(),
    )
    .unwrap();
    let all: String = out.files.iter().map(|f| f.contents.as_str()).collect();
    assert!(
        all.contains("export type NewPetWebhookRequest = string;"),
        "{all}"
    );
    assert!(
        all.contains("export type NewPetOperationWebhookRequest ="),
        "{all}"
    );
}

#[test]
fn function_names_can_be_camel_case() {
    let doc = set(r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /a:
    get: { operationId: ListPets, responses: { "204": { description: ok } } }
    post: { operationId: create_pet, responses: { "204": { description: ok } } }
    put: { operationId: updatePet, responses: { "204": { description: ok } } }
"##);
    let functions = |case: FunctionCase| {
        let ts = TypeScriptOptions {
            function_name_case: case,
            ..TypeScriptOptions::default()
        };
        let out = generate(&doc, &Options::default(), &ts).unwrap();
        let mut paths: Vec<String> = out
            .files
            .iter()
            .filter_map(|f| f.path.strip_prefix("operations/")?.strip_suffix(".ts"))
            .map(str::to_owned)
            .collect();
        paths.sort();
        paths
    };
    assert_eq!(
        functions(FunctionCase::Preserve),
        ["ListPets", "create_pet", "updatePet"]
    );
    assert_eq!(
        functions(FunctionCase::Camel),
        ["createPet", "listPets", "updatePet"]
    );
}
