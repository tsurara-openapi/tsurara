# An external backend in JavaScript

`backend.mjs` is a backend written outside tsurara (design.md 18.11). Its `describe` declares Go's capabilities and a Go-like naming policy, so `generate` receives `lowered` (the model lowered to `tir`) and `names` (allocated with that policy) besides the model. It writes one Go-like text file per declaration.

```yaml
input: openapi.yaml
output: out
language: { command: [node, backend.mjs] }
```

`examples/backend-rust` does the same in Rust with the published crates instead of the protocol.

It is also the template for a backend in its own repository ([the guide](../../docs/guide/backends.md)): `check.mjs` is its checker, which fails when two files declare the same type or a type refers to an undeclared one.

```sh
tsurara test-backend --backend 'node backend.mjs' --check 'node check.mjs'
EXAMPLE_NAMING_BUG=1 tsurara test-backend --backend 'node backend.mjs' --check 'node check.mjs'   # fails on names.yaml
```

