//! The `Plugin` trait (design.md 11).

use serde_json::Value;
use tsurara_backend::plugin::{ExtendInput, Plugin, PluginError};
use tsurara_backend::{OutputFile, generate_with_plugins};
use tsurara_lang_typescript::TypeScript;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::ir::External;
use tsurara_sem::{Model, Options};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Pet: { type: object, properties: { id: { type: string } } }
"##;

fn set() -> DocumentSet {
    DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap())
}

/// Appends `suffix` to the description of `Pet`, to observe the order of document plugins.
struct Describe(&'static str);

impl Plugin for Describe {
    fn name(&self) -> &str {
        "describe"
    }
    fn transform_document(&self, root: &mut Value) -> Result<(), PluginError> {
        let pet = &mut root["components"]["schemas"]["Pet"];
        let old = pet["description"].as_str().unwrap_or("").to_owned();
        pet["description"] = Value::String(format!("{old}{}", self.0));
        Ok(())
    }
}

struct RenameModel;

impl Plugin for RenameModel {
    fn name(&self) -> &str {
        "rename"
    }
    fn transform_model(&self, model: &mut Model) -> Result<(), PluginError> {
        model.title = Some("renamed".into());
        Ok(())
    }
}

struct AddFile;

impl Plugin for AddFile {
    fn name(&self) -> &str {
        "add-file"
    }
    fn transform_files(&self, files: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        files.push(OutputFile {
            path: "extra.ts".into(),
            contents: "export {};\n".into(),
        });
        Ok(())
    }
}

struct Fail;

impl Plugin for Fail {
    fn name(&self) -> &str {
        "fail"
    }
    fn transform_files(&self, _: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        Err(PluginError::new("boom"))
    }
}

fn pet(files: &[OutputFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path == "schemas/Pet.ts")
        .unwrap()
        .contents
}

#[test]
fn document_plugins_run_in_order() {
    let plugins: [&dyn Plugin; 2] = [&Describe("first "), &Describe("second")];
    let out = generate_with_plugins(&TypeScript::default(), set(), &Options::default(), &plugins)
        .unwrap();
    assert!(
        pet(&out.files).contains("first second"),
        "{}",
        pet(&out.files)
    );
}

#[test]
fn model_and_files_plugins_run() {
    let plugins: [&dyn Plugin; 2] = [&RenameModel, &AddFile];
    let out = generate_with_plugins(&TypeScript::default(), set(), &Options::default(), &plugins)
        .unwrap();
    assert!(out.files.iter().any(|f| f.path == "extra.ts"));
}

#[test]
fn plugin_errors_name_the_plugin() {
    let plugins: [&dyn Plugin; 1] = [&Fail];
    let err = generate_with_plugins(&TypeScript::default(), set(), &Options::default(), &plugins)
        .unwrap_err();
    assert_eq!(err.to_string(), "plugin `fail`: boom");
}

#[test]
fn overrides_replace_a_type_by_json_pointer() {
    let mut opts = Options::default();
    opts.type_overrides.insert(
        "#/components/schemas/Pet/properties/id".into(),
        [(
            "typescript".to_owned(),
            External {
                code: "`pet-${string}`".into(),
                import: None,
            },
        )]
        .into(),
    );
    let out = generate_with_plugins(&TypeScript::default(), set(), &opts, &[]).unwrap();
    assert!(
        pet(&out.files).contains("id?: `pet-${string}`"),
        "{}",
        pet(&out.files)
    );
}

const API: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets/{id}:
    get:
      operationId: getPet
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "404": { description: no, content: { application/json: { schema: { $ref: "#/components/schemas/Error" } } } }
components:
  schemas:
    Pet: { type: object, properties: { id: { type: string } } }
    Error: { type: object, properties: { message: { type: string } } }
"##;

fn api() -> DocumentSet {
    DocumentSet::from_value(parse_document(API.as_bytes()).unwrap())
}

/// Adds one file per operation that imports the call function and its Data type, as the manifest
/// names them.
struct Hooks;

impl Plugin for Hooks {
    fn name(&self) -> &str {
        "hooks"
    }
    fn extend(&self, input: &ExtendInput) -> Result<Vec<OutputFile>, PluginError> {
        let manifest = input
            .manifest
            .expect("the TypeScript backend returns a manifest");
        Ok(manifest
            .operations
            .iter()
            .filter_map(|op| {
                let function = op.function.as_ref()?;
                let data = op.types.get("data")?;
                Some(OutputFile {
                    path: format!("hooks/{}.ts", function.identifier),
                    contents: format!(
                        "{}\n\nimport {{ {} }} from '../{}';\nimport type {{ {} }} from '../{}';\n",
                        input.marker,
                        function.identifier,
                        function.file,
                        data.identifier,
                        data.file
                    ),
                })
            })
            .collect())
    }
}

/// Returns `files` from `extend`.
struct Extend(Vec<OutputFile>);

impl Plugin for Extend {
    fn name(&self) -> &str {
        "extend"
    }
    fn extend(&self, _: &ExtendInput) -> Result<Vec<OutputFile>, PluginError> {
        Ok(self.0.clone())
    }
}

/// Records the paths `transformFiles` receives.
struct SeeFiles(std::cell::RefCell<Vec<String>>);

impl Plugin for SeeFiles {
    fn name(&self) -> &str {
        "see"
    }
    fn transform_files(&self, files: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        *self.0.borrow_mut() = files.iter().map(|f| f.path.clone()).collect();
        Ok(())
    }
}

#[test]
fn the_output_has_a_manifest_of_types_and_operations() {
    let out =
        generate_with_plugins(&TypeScript::default(), api(), &Options::default(), &[]).unwrap();
    let manifest = out.manifest.expect("manifest");
    assert_eq!(
        manifest.manifest_version,
        tsurara_backend::manifest::MANIFEST_VERSION
    );
    assert_eq!(manifest.language, "typescript");
    let error = manifest
        .types
        .iter()
        .find(|t| t.export.identifier == "ErrorModel")
        .expect("Error is renamed to ErrorModel");
    assert_eq!(error.export.file, "schemas/ErrorModel.ts");
    let op = &manifest.operations[0];
    assert_eq!(op.operation_id, "getPet");
    let function = op.function.as_ref().unwrap();
    assert_eq!(
        (function.identifier.as_str(), function.file.as_str()),
        ("getPet", "operations/getPet.ts")
    );
    assert_eq!(op.operation_def.as_ref().unwrap().identifier, "getPetDef");
    assert_eq!(op.types["data"].identifier, "GetPetData");
    assert_eq!(op.types["result"].identifier, "GetPetResult");
}

#[test]
fn extend_adds_files_before_transform_files() {
    let see = SeeFiles(Default::default());
    let plugins: [&dyn Plugin; 2] = [&Hooks, &see];
    let out = generate_with_plugins(&TypeScript::default(), api(), &Options::default(), &plugins)
        .unwrap();
    let hook = out
        .files
        .iter()
        .find(|f| f.path == "hooks/getPet.ts")
        .unwrap();
    assert!(
        hook.contents.starts_with(tsurara_lang_typescript::HEADER),
        "{}",
        hook.contents
    );
    assert!(
        hook.contents
            .contains("import { getPet } from '../operations/getPet.ts';"),
        "{}",
        hook.contents
    );
    assert!(see.0.borrow().contains(&"hooks/getPet.ts".to_owned()));
}

#[test]
fn extend_files_must_carry_the_marker() {
    let plugins: [&dyn Plugin; 1] = [&Extend(vec![OutputFile {
        path: "extra.ts".into(),
        contents: "export {};\n".into(),
    }])];
    let err = generate_with_plugins(&TypeScript::default(), api(), &Options::default(), &plugins)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("`extra.ts` does not start with the generated-file marker"),
        "{err}"
    );
}

#[test]
fn extend_cannot_replace_a_generated_file() {
    let plugins: [&dyn Plugin; 1] = [&Extend(vec![OutputFile {
        path: "schemas/Pet.ts".into(),
        contents: format!("{}\n", tsurara_lang_typescript::HEADER),
    }])];
    let err = generate_with_plugins(&TypeScript::default(), api(), &Options::default(), &plugins)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("`schemas/Pet.ts` is already generated"),
        "{err}"
    );
}
