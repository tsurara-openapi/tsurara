# Changelog of the model, the manifest and the protocol

Every change to `sem-model.schema.json`, `manifest.schema.json` and `plugin-protocol.schema.json` gets an entry, newest first, with the fingerprint of each changed schema as `crates/tsurara-cli/tests/spec_schemas.rs` prints it. The rules for versions are in design.md chapter 14.

## 0.1.0: first version

- `semVersion` 1, `manifestVersion` 1, protocol 2 (protocol 1 is protocol 2 without `manifest` in `GenerateResponse`). Changes before the first release are not recorded.
- `sem-model.schema.json` 4ed358372b82800d
- `manifest.schema.json` 1c286604a2e18f33
- `plugin-protocol.schema.json` 6aa6e7bdb5a79447
