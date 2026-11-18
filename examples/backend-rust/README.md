# An external backend in Rust

A backend outside tsurara (design.md 18.11) that depends on the published crates (`tsurara-sem`, `tsurara-lower`, `tsurara-naming`, `tsurara-backend`) instead of path dependencies: it deserializes the model it receives, lowers it with `tsurara_lower::lower`, allocates names with its own `NamingPolicy` through `tsurara_lower::allocate`, and writes Kotlin-like data classes.

```yaml
input: openapi.yaml
output: out
language: { command: [cargo, run, --quiet, --manifest-path, examples/backend-rust/Cargo.toml, --] }
```

`crates/tsurara-cli/tests/backend_kit.rs` builds it against the crates as `cargo package` makes them and checks that it generates what the same code generates in-process.
