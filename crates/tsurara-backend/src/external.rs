//! The protocol of backends written as external programs (design.md 18.11). The CLI runs the
//! program with `describe`, then with `generate`; both exchange JSON.

use crate::{Manifest, Markers, OutputFile};
use tsurara_lower::tir::Module;
use tsurara_lower::{AllocatedNames, Capabilities};
use tsurara_naming::data::DataPolicy;
use tsurara_sem::{Model, Options, Warning};

/// The newest version of this protocol. Version 2 adds the manifest to `generate`'s answer.
pub const PROTOCOL: u32 = 2;

/// The versions the CLI still speaks. A program answers `describe` with one of them, and `generate`
/// receives the same.
pub const PROTOCOLS: &[u32] = &[1, 2];

/// What `describe` prints.
#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Describe {
    pub protocol: u32,
    /// The model versions (`semVersion`) the program accepts.
    pub sem_versions: Vec<u32>,
    pub markers: Markers,
    /// The shared lowering the program wants: `generate` then receives `lowered`, the
    /// model lowered to `tir` for these capabilities. Optional, in protocols 1 and 2.
    #[serde(default)]
    pub lowering: Option<Lowering>,
    /// The program's naming policy as data (needs `lowering`): `generate` then receives
    /// `names`, allocated for `lowered` with this policy.
    #[serde(default)]
    pub naming: Option<DataPolicy>,
}

/// What a program asks of the shared lowering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Lowering {
    /// The constructs the program's language has (`tsurara_lower::Capabilities`).
    pub capabilities: Capabilities,
}

/// What `generate` receives on stdin.
#[derive(Debug, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest<'a> {
    pub protocol: u32,
    pub sem_version: u32,
    pub model: &'a Model,
    pub common: &'a Options,
    /// The `options` of the target's `language` entry.
    pub options: &'a serde_json::Value,
    /// The model lowered for the capabilities of `describe`'s `lowering`; absent without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lowered: Option<&'a Module>,
    /// The names allocated for `lowered` with `describe`'s `naming`; absent without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<&'a AllocatedNames>,
}

/// What `generate` prints.
#[derive(Debug, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GenerateResponse {
    pub files: Vec<OutputFile>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// Protocol 2: what the program generated, by name. Optional.
    #[serde(default)]
    pub manifest: Option<Manifest>,
}
