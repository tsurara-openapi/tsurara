//! Loads OpenAPI definitions and resolves `$ref`.
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: `DocumentSet`, `Loc`, `parse_document`,
//! `normalize_swagger2` and the loading functions; the rest is internal.

mod docset;
mod fetch;
mod location;
mod parse;
pub mod pointer;
mod swagger2;
mod yaml;

pub use docset::{DocumentSet, LoadError, Loc, RefError};
pub use fetch::{DefaultFetcher, FetchError, Fetcher, HttpOptions, MemoryFetcher};
pub use location::{LocationError, resolve_input};
pub use parse::{ParseError, parse_document};
pub use swagger2::{SpecVersion, VersionError, detect_version, normalize_swagger2};
pub use yaml::YamlError;
