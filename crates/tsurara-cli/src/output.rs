//! What `generate`, `check` and `inspect` print with `--format json`. Every report is one JSON object on
//! stdout; `spec/cli-output.schema.json` is generated from these types.

use schemars::JsonSchema;
use serde::Serialize;

/// Any report of the command.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Report {
    Generate(GenerateReport),
    Check(CheckReport),
}

#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReport {
    /// Always `generate`.
    pub command: String,
    pub targets: Vec<GeneratedTarget>,
    /// Set when the command failed; the targets before the failure are listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedTarget {
    pub name: String,
    /// `typescript`, `python`, `rust`, `go` or `external`.
    pub language: String,
    /// The output directory.
    pub output: String,
    /// Paths relative to the output directory.
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub deleted: Vec<String>,
    pub warnings: Vec<WarningReport>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WarningReport {
    /// `unresolved-ref`, ... (`tsurara inspect warning <code>` explains it).
    pub code: String,
    /// The document, as a path relative to the current directory or a URL.
    pub document: String,
    /// JSON Pointer in the document.
    pub pointer: String,
    pub message: String,
    /// The backend that reported it, for approximations of one language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    /// Always `check`.
    pub command: String,
    pub targets: Vec<CheckedTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckedTarget {
    pub name: String,
    pub output: String,
    /// Whether the files on disk match and the runtime satisfies the generated code.
    pub up_to_date: bool,
    pub differences: Vec<DifferenceReport>,
    /// Problems with the installed runtime library, and notes when it could not be checked.
    pub runtime: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DifferenceReport {
    /// `changed`, `missing` or `stale`.
    pub kind: String,
    pub path: String,
}

/// The JSON Schema of `Report`.
pub fn json_schema() -> String {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Report>();
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}
