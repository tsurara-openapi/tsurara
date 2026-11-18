//! Analysis of OpenAPI definitions into the model that backends generate code from. Does no I/O.
//!
//! The model is still TypeScript-shaped in places: `ir`, `print` and `options` move to the TypeScript
//! backend as the language-neutral model is introduced (design.md 18).
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: `Model` and the types it holds
//! (`model`, `ir`, `operation`, `naming`), `build_model`, `Options`, `Warning`; their JSON form is
//! `spec/sem-model.schema.json`. `print` and `normalize` are internal.

pub mod ir;
pub mod model;
pub mod naming;
pub mod normalize;
pub mod notation;
pub mod operation;
pub mod options;
pub mod print;
pub mod schema;
pub mod text;
pub mod warning;

pub use model::{Model, build_model};

/// Version of the model's JSON form, for plugins and external backends (design.md 18.4.6). It changes
/// when the form changes incompatibly.
pub const SEM_VERSION: u32 = 1;
pub use options::{Options, RuntimeSource};
pub use warning::{Warning, WarningKind};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Version(#[from] tsurara_loader::VersionError),
    #[error(
        "Swagger 2.0 documents must be converted with tsurara_loader::normalize_swagger2 first"
    )]
    Swagger2,
}
