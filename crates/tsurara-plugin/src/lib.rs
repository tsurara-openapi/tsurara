//! Write tsurara plugins in Rust. A plugin is a binary that calls [`run`] with its stages;
//! tsurara runs it once per run (for example `runtime: [cargo, run, --quiet, --manifest-path]` with
//! the plugin's `Cargo.toml` as the module) and calls the stages it declares over the session protocol
//! (`tsurara_backend::session`, design.md 11.2). The inputs use the real types of the model and the
//! manifest. Embedders of tsurara as a library implement `tsurara_backend::plugin::Plugin` instead.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
pub use tsurara_backend::OutputFile;
pub use tsurara_backend::manifest::Manifest;
use tsurara_backend::session::{SESSION_PROTOCOL, SessionRequest, Stage};
pub use tsurara_sem::Model;

/// What the extend stage receives, owned.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtendInput {
    pub language: String,
    pub model: Model,
    pub manifest: Option<Manifest>,
    /// The common options of the target (`Options` in its JSON form).
    pub common: Value,
    /// The options of the language (for TypeScript the import extension, layout and client module).
    pub options: Value,
    /// The first line every added file must start with.
    pub marker: String,
}

type Stage1<I, O> = Box<dyn Fn(I) -> Result<O, String>>;

/// A plugin: a name and any of the stages. `transform_document` and `transform_model` work on JSON
/// values, so that nothing a newer tsurara adds is lost.
#[derive(Default)]
pub struct Plugin {
    pub name: String,
    pub transform_document: Option<Stage1<Value, Value>>,
    pub transform_model: Option<Stage1<Value, Value>>,
    pub extend: Option<Stage1<ExtendInput, Vec<OutputFile>>>,
    pub transform_files: Option<Stage1<Vec<OutputFile>, Vec<OutputFile>>>,
}

impl Plugin {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    fn stages(&self) -> Vec<Stage> {
        let mut out = Vec::new();
        if self.transform_document.is_some() {
            out.push(Stage::TransformDocument);
        }
        if self.transform_model.is_some() {
            out.push(Stage::TransformModel);
        }
        if self.extend.is_some() {
            out.push(Stage::Extend);
        }
        if self.transform_files.is_some() {
            out.push(Stage::TransformFiles);
        }
        out
    }

    fn call(&self, stage: Stage, input: Value) -> Result<Value, String> {
        let missing = || format!("the plugin has no {} stage", stage.as_str());
        match stage {
            Stage::TransformDocument => {
                (self.transform_document.as_ref().ok_or_else(missing)?)(input)
            }
            Stage::TransformModel => (self.transform_model.as_ref().ok_or_else(missing)?)(input),
            Stage::Extend => {
                let f = self.extend.as_ref().ok_or_else(missing)?;
                let input = serde_json::from_value(input).map_err(|e| e.to_string())?;
                Ok(serde_json::to_value(f(input)?).expect("files serialize"))
            }
            Stage::TransformFiles => {
                let f = self.transform_files.as_ref().ok_or_else(missing)?;
                let input = serde_json::from_value(input).map_err(|e| e.to_string())?;
                Ok(serde_json::to_value(f(input)?).expect("files serialize"))
            }
        }
    }
}

/// Serves the plugin on stdin and stdout until stdin closes.
pub fn run(plugin: Plugin) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    if let Err(e) = serve(&plugin, stdin.lock(), &mut stdout) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Serves the plugin on `input` and `output` until `input` ends.
pub fn serve(plugin: &Plugin, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: SessionRequest = serde_json::from_str(&line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let answer = match request {
            SessionRequest::Hello { .. } => json!({
                "type": "hello",
                "protocol": SESSION_PROTOCOL,
                "name": plugin.name,
                "stages": plugin.stages(),
            }),
            SessionRequest::Call { stage, input } => match plugin.call(stage, input) {
                Ok(out) => json!({ "type": "result", "output": out }),
                Err(message) => json!({ "type": "error", "message": message }),
            },
        };
        writeln!(output, "{answer}")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_hello_and_calls() {
        let mut plugin = Plugin::new("files");
        plugin.transform_files = Some(Box::new(|mut files| {
            files.push(OutputFile {
                path: "a.txt".into(),
                contents: "x".into(),
            });
            Ok(files)
        }));
        let input = "{\"type\":\"hello\",\"protocol\":1}\n{\"type\":\"call\",\"stage\":\"transformFiles\",\"input\":[]}\n{\"type\":\"call\",\"stage\":\"extend\",\"input\":{}}\n";
        let mut out = Vec::new();
        serve(&plugin, input.as_bytes(), &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["stages"], json!(["transformFiles"]));
        assert_eq!(lines[1]["output"][0]["path"], "a.txt");
        assert_eq!(lines[2]["type"], "error");
    }
}
