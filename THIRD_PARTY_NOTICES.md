# Third-party notices

tsurara is MIT licensed ([LICENSE](LICENSE)). These files in the repository come from other projects, under their own licenses; they are used by tests and checks and are not part of the published packages.

## OpenAPI examples (CC BY 4.0)

From [OAI/learn.openapis.org](https://github.com/OAI/learn.openapis.org) (commit `5f26927b00d5d3d10c1a1a6bbe2d9b41b4a726e7`), © the OpenAPI Initiative, licensed under the [Creative Commons Attribution 4.0 International License](LICENSES/CC-BY-4.0.txt). Unchanged; the snapshots generated from them in `crates/*/tests/snapshots` are adaptations.

| File | Source |
|---|---|
| `crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml` | `examples/v3.0/petstore.yaml` |
| `crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore-expanded.yaml` | `examples/v3.0/petstore-expanded.yaml` |
| `crates/tsurara-lang-typescript/tests/fixtures/v3.1-non-oauth-scopes.yaml` | `examples/v3.1/non-oauth-scopes.yaml` |
| `crates/tsurara-lang-typescript/tests/fixtures/v3.1-webhook-example.yaml` | `examples/v3.1/webhook-example.yaml` |

## Swagger Petstore (Apache License 2.0)

The Swagger Petstore definitions of [swagger-api/swagger-petstore](https://github.com/swagger-api/swagger-petstore) (OpenAPI 3.0) and of the Swagger 2.0 petstore, licensed under the [Apache License 2.0](LICENSES/Apache-2.0.txt), as their `info.license` states:

- `crates/tsurara-lang-typescript/tests/fixtures/petstore3.json`
- `crates/tsurara-loader/tests/fixtures/petstore-swagger2.json`

## OpenAPI Specification (Apache License 2.0)

Some request and response vectors in `spec/conformance/` follow the examples of the [OpenAPI Specification](https://github.com/OAI/OpenAPI-Specification) (parameter serialization by `style` and `explode`), © the OpenAPI Initiative, licensed under the [Apache License 2.0](LICENSES/Apache-2.0.txt).

## React Native (MIT)

`runtimes/typescript/hermes/rn/URL.js`, `URLSearchParams.js` and `FormData.js` are from [React Native](https://github.com/facebook/react-native) 0.88.1, Copyright (c) Meta Platforms, Inc. and affiliates, licensed under the MIT License ([runtimes/typescript/hermes/rn/LICENSE](runtimes/typescript/hermes/rn/LICENSE)), with the Flow types removed. They are used only by the private Hermes check.

## Dependencies of the `tsurara` binary

The binary links Rust crates under permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0, ...). `node scripts/third-party-licenses.mjs` writes their notices and license texts, to ship with every binary release.
