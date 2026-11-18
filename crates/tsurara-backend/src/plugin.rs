//! Plugins (design.md chapter 11). A plugin hooks into one or more stages; each stage has a fixed input,
//! and plugins of the same stage run in the order they are given.

use crate::{Backend, Error, Manifest, Output, OutputFile, generate_from_model};
use serde_json::Value;
use tsurara_sem::{Model, Options, build_model};

/// A failure reported by a plugin. `generate_with_plugins` adds the plugin's name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PluginError {
    pub message: String,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait Plugin {
    /// Shown in errors.
    fn name(&self) -> &str;

    /// Document stage: rewrites the root definition (after Swagger 2.0 conversion).
    fn transform_document(&self, _root: &mut Value) -> Result<(), PluginError> {
        Ok(())
    }

    /// Model stage. Unstable: the model becomes the language-neutral semantic model, so
    /// plugins using this stage will need changes.
    fn transform_model(&self, _model: &mut Model) -> Result<(), PluginError> {
        Ok(())
    }

    /// Extend stage: returns files to add next to the backend's output. Each must start with the
    /// backend's generated-file marker (`ExtendInput::marker`), so that stale-file deletion and
    /// `tsurara check` cover it, and must not have the path of a file that is already generated.
    fn extend(&self, _input: &ExtendInput) -> Result<Vec<OutputFile>, PluginError> {
        Ok(Vec::new())
    }

    /// Files stage: rewrites the output files.
    fn transform_files(&self, _files: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        Ok(())
    }
}

/// What the extend stage receives.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ExtendInput<'a> {
    /// The backend's `id`.
    pub language: &'a str,
    /// The model the backend generated from (after model plugins).
    pub model: &'a Model,
    /// What the backend generated, by name; `None` for external backends that return none.
    pub manifest: Option<&'a Manifest>,
    pub common: &'a Options,
    /// The options of the language (`Backend::options_json`).
    pub options: &'a Value,
    /// The first line every added file must start with (`Markers::generated`).
    pub marker: &'a str,
}

/// Generates with plugins: document plugins, then the model, model plugins, the files, extend
/// plugins and files plugins. Plugins of each stage run in the order given.
pub fn generate_with_plugins(
    backend: &dyn Backend,
    mut set: tsurara_loader::DocumentSet,
    opts: &Options,
    plugins: &[&dyn Plugin],
) -> Result<Output, Error> {
    let failed = |p: &&dyn Plugin| {
        let name = p.name().to_owned();
        move |source| Error::Plugin { name, source }
    };
    if !plugins.is_empty() {
        let mut root = set.root().clone();
        for p in plugins {
            p.transform_document(&mut root).map_err(failed(p))?;
        }
        set.replace_root(root);
    }
    let mut model = build_model(&set, opts)?;
    for p in plugins {
        p.transform_model(&mut model).map_err(failed(p))?;
    }
    let mut output = generate_from_model(backend, &model, opts)?;
    let marker = backend.markers().generated;
    let language_options = backend.options_json();
    let mut added = Vec::new();
    for p in plugins {
        let input = ExtendInput {
            language: backend.id(),
            model: &model,
            manifest: output.manifest.as_ref(),
            common: opts,
            options: &language_options,
            marker: &marker,
        };
        for file in p.extend(&input).map_err(failed(p))? {
            let error = |message: String| failed(p)(PluginError::new(message));
            if !file.contents.starts_with(&marker) {
                return Err(error(format!(
                    "`{}` does not start with the generated-file marker `{marker}`",
                    file.path
                )));
            }
            if output
                .files
                .iter()
                .chain(&added)
                .any(|f: &OutputFile| f.path == file.path)
            {
                return Err(error(format!("`{}` is already generated", file.path)));
            }
            added.push(file);
        }
    }
    output.files.extend(added);
    for p in plugins {
        p.transform_files(&mut output.files).map_err(failed(p))?;
    }
    output.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(output)
}
