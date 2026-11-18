//! Shared lowering (design.md 18.5): turns the model into `tir` for a language's capabilities, so that
//! backends of nominal languages share hoisting, `allOf` handling, union fallbacks, open enums,
//! presence, recursion and tuple handling. TypeScript keeps its own lowering (its IR is the structural
//! subset of `tir`, with every capability on).
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: `lower`, `validate`, `Capabilities`,
//! `tir` (whose JSON form external backends receive as `lowered`), `allocate` and `AllocatedNames`,
//! and `typedef`.

pub mod caps;
mod lower;
pub mod names;
pub mod tir;
pub mod typedef;
mod validate;

pub use caps::{Capabilities, Enums, Inheritance};
pub use lower::lower;
pub use names::{AllocatedNames, allocate};
pub use validate::validate;
