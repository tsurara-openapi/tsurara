//! The TypeScript backend: types, OperationDefs and call functions for `@tsurara/client`.

pub mod emit;
mod example;
pub mod external;
pub mod lower;
mod mocks;
pub mod naming;
pub mod options;
mod query;
mod vendor;
pub mod zod;

pub use emit::{CLIENT_RANGE_PREFIX, HEADER, emit};

pub use options::TypeScriptOptions;
use tsurara_backend::{Backend, Emitted, Markers, Output, VersionMarker};
use tsurara_sem::{Model, Options};

/// The `@tsurara/client` version range that generated code requires.
pub const CLIENT_VERSION_RANGE: &str = "^0.1.0";

/// Version of the OperationDef format.
pub const OPERATION_DEF_VERSION: u32 = 1;

/// The TypeScript backend, configured with the `typescript` section of a target.
#[derive(Debug, Default, Clone)]
pub struct TypeScript {
    pub options: TypeScriptOptions,
}

impl Backend for TypeScript {
    fn id(&self) -> &'static str {
        "typescript"
    }

    fn markers(&self) -> Markers {
        Markers {
            generated: HEADER.to_owned(),
            version: Some(VersionMarker {
                file: "index.ts".to_owned(),
                prefix: CLIENT_RANGE_PREFIX.to_owned(),
            }),
            sidecars: Vec::new(),
        }
    }

    fn emit(&self, model: &Model, opts: &Options) -> Result<Emitted, tsurara_backend::Error> {
        let (files, warnings, manifest) = emit(model, opts, &self.options);
        Ok(Emitted {
            files,
            warnings,
            manifest: Some(manifest),
        })
    }

    fn example(&self, input: &tsurara_backend::ExampleInput) -> Option<tsurara_backend::Example> {
        example::example(input, &self.options)
    }

    fn options_json(&self) -> serde_json::Value {
        use options::{Layout, Mocks, Query, Validators};
        let o = &self.options;
        serde_json::json!({
            "importExtension": o.import_extension.suffix(),
            "layout": match o.layout {
                Layout::Split => "split",
                Layout::Tags => "tags",
                Layout::Single => "single",
            },
            "clientModule": o.client_module,
            "validators": o.validators.map(|Validators::Zod| "zod"),
            "query": o.query.map(|Query::Tanstack| "tanstack"),
            "mocks": o.mocks.map(|m| match m {
                Mocks::Msw => "msw",
                Mocks::Fetch => "fetch",
            }),
        })
    }
}

/// Builds the model from a loaded definition and generates TypeScript.
pub fn generate(
    set: &tsurara_loader::DocumentSet,
    opts: &Options,
    ts: &TypeScriptOptions,
) -> Result<Output, tsurara_backend::Error> {
    let backend = TypeScript {
        options: ts.clone(),
    };
    tsurara_backend::generate(&backend, set, opts)
}
