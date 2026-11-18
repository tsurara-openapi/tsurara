//! The sources of the runtimes in `runtimes/`, embedded at build time so that a vendored copy
//! (`runtime: vendor`) always matches the generator. Each list holds `(path, contents)`
//! pairs relative to the runtime's source directory, without tests.

include!(concat!(env!("OUT_DIR"), "/runtimes.rs"));

/// The line each vendored file carries after the generated-file marker (as a comment of its
/// language): the runtimes are MIT licensed.
pub const NOTICE: &str = "A copy of the tsurara runtime (MIT License, Copyright (c) 2026 nazo; https://github.com/tsurara-openapi/tsurara).";
