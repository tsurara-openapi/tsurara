# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Language

Write all prose and comments in files in English (code comments, doc comments, docs, issues, commit messages). Test data that is intentionally non-ASCII (such as the Japanese identifiers in naming tests) stays as is.

## Project

tsurara generates TypeScript types and per-operation API call functions from OpenAPI (3.0/3.1/3.2; Swagger 2.0 is converted to 3.0 first). The generator is written in Rust and will ship as a single binary. Generated code depends on the runtime library `@tsurara/client`. The full design is in `docs/design.md`; the user documentation is `README.md` and `docs/guide/` (keep `docs/guide/configuration.md` in step with the config schema: a test in `tsurara-config` checks that it names every key). tsurara is being restructured to generate many languages (Rust, C#, Go, Python, PHP, Ruby, Kotlin, Java, Swift, GDScript, ...); the target architecture is chapter 18 of `docs/design.md`.

## Commands

Toolchain versions are pinned in `mise.toml` (Rust, Node, pnpm, Python, uv, Go). Run tools through mise (`mise exec -- cargo ...`): a `cargo` or `node` elsewhere on `PATH` may be another version. MSRV is 1.85 and is checked separately in CI, so do not use newer std APIs or language features.

Run `pnpm install` at the repo root first. The Rust tests that type-check generated code run `node_modules/.bin/tsc` (TypeScript 7.0.2) and the minimum supported TypeScript 5.8 (`node_modules/typescript58`), and are silently skipped without them (`TSURARA_REQUIRE_CHECKERS=typescript,python,rust,go` makes them fail instead, as in CI; the variable lists languages whose checkers must run, `python` needs pyright and Python 3.11 or later, `rust` needs cargo, and `go` needs Go from `mise.toml`). `tests/type_stripping.rs` runs generated code with Node type stripping and needs Node 22.18 or later (`TSURARA_REQUIRE_NODE=1` makes it mandatory).

Rust (what CI runs):

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p tsurara-lang-typescript --test cases     # one test target
cargo test -p tsurara-lang-typescript <test_name>      # filter by name
INSTA_UPDATE=always cargo test -p tsurara-lang-typescript   # rewrite insta snapshots, then review `git diff`
```

CI runs with `INSTA_UPDATE=no`, so changed snapshots must be committed.

Corpus tests (`crates/tsurara-cli/tests/corpus.rs`) generate from `corpus/edge-cases/*.yaml` and from the real-world definitions pinned in `corpus/sources.json`, and run tsc (the tsconfig matrix), pyright, `cargo clippy` and gofmt, go vet and go build on the output. `pnpm corpus:fetch` downloads the real-world definitions into `target/corpus`; they are checked only with `TSURARA_CORPUS=1` (about 20 minutes, most of the suite) or `TSURARA_REQUIRE_CORPUS=1` (as in CI, where missing ones fail). CI checks the edge cases on every pull request and the real-world definitions on pushes to main, on runs by hand and on pull requests with the label `corpus`. Run them when a change can affect the output for large definitions, or give its pull request that label.

The checkers of `tsurara-testkit` (tsc, pyright, cargo clippy, the Go tools) skip a check whose files, settings, runtime sources, lock files, `mise.toml` and testkit source are the same as in a check that passed before (`target/checker-cache`); `TSURARA_CHECKER_CACHE=0` turns that off. Add a definition to `corpus/edge-cases` for each bug found in real-world definitions.

Property tests (`crates/tsurara-cli/tests/properties.rs`) generate from random definitions and run tsc, pyright, cargo clippy and the Go tools on all of them at once; a failure is shrunk and saved in `target/proptest-failures`. `TSURARA_PROPTEST_CASES` and `TSURARA_PROPTEST_SEED` change the amount and the seed (CI: 48 cases, seed 2026; weekly: 2000).

After changing `README.md`, `docs/guide/`, `docs/languages/` or `skills/tsurara/`, regenerate `llms.txt` and `llms-full.txt` with `TSURARA_UPDATE_LLMS=1 cargo test -p tsurara-cli --test llms`. The code blocks of `skills/tsurara/references/*.md` are type-checked against the petstore output (`tests/skill.rs`).

After changing the config structs in `crates/tsurara-config/src/raw.rs`, regenerate the JSON Schema with `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-config` (the test fails when the file is out of date). After changing the serialized types of the model (`tsurara-sem`, `tsurara_loader::Loc`), the manifest or the protocol (`tsurara-backend`), regenerate `spec/sem-model.schema.json`, `spec/manifest.schema.json` and `spec/plugin-protocol.schema.json` with `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test spec_schemas`; they derive `schemars::JsonSchema` behind the `schema` feature.

TypeScript:

```
pnpm lint      # biome check .
pnpm format    # biome format --write .
pnpm build     # pnpm -r build
pnpm test      # pnpm -r test
pnpm --filter @tsurara/client test   # typecheck + node --test (runs .ts directly via type stripping)
```

## Architecture

Pipeline: input (file/URL) → `tsurara-loader` → `tsurara-sem` (model) → a backend (`tsurara-lang-typescript`) → `tsurara-cli` writes files. Plugins (`tsurara_backend::plugin::Plugin`, run by `tsurara_backend::generate_with_plugins`) hook in at the document, model, extend and files stages; every backend returns a manifest of the names it generated (`tsurara_backend::manifest`, design.md 11.1), which the extend stage receives. The target architecture for many languages is design.md chapter 18.

- `crates/tsurara-loader`: parses JSON/YAML, resolves `$ref` across files and URLs into a `DocumentSet`, and converts Swagger 2.0 (`normalize_swagger2`). HTTP fetching is behind the default `http` feature.
- `crates/tsurara-sem`: pure analysis with no I/O. `build_model` turns a `DocumentSet` into a `Model`.
  - `schema.rs` converts schemas into the type IR (`ir.rs`), `normalize.rs` normalizes it (allOf flattening and so on), and `operation.rs` builds the operations. The model holds no identifiers: `naming.rs` defines the name requests (`Model::names`, in claim order) that references (`Type::Named`) point to.
  - Approximate conversions are reported as `Warning`s (`warning.rs`) instead of failing.
  - `ir.rs` is the language-neutral type IR, used by `tsurara-lower` and every backend; its variants `Undefined`, `Ref`, `External` and `Builtin` belong to the TypeScript lowering only. `print.rs` prints TypeScript types (normalization uses it for a stable order) and `options.rs` holds the common options. Changes to the TypeScript output must be deliberate: review snapshot diffs. Compatibility of APIs and configuration keys is not required.
- `crates/tsurara-sample`: deterministic sample values (one JSON value per declaration, and per operation parameter, request body and response body) from the model, for mocks in every language (design.md 18.7.1).
- `crates/tsurara-naming`: the shared name allocator (`Names`: contexts, case-folding scopes, numbering, derived names that move aside when taken), the `NamingPolicy` trait, and casing helpers.
- `crates/tsurara-lower`: shared lowering of the model to `tir` (records, enums, tagged and untagged unions, aliases, and the types of each operation) for a language's `Capabilities`, with a validator (design.md 18.5), and TypeDefs (`typedef.rs`). TypeScript does not use it; Python does.
- `crates/tsurara-pretty`: the document printer every backend's printer uses (groups that are flat within a width limit or in the rest of the line, `flat_or`, nesting). `tsurara-sem/src/print.rs` builds TypeScript type layouts with it.
- `crates/tsurara-lang-python`: the Python backend (dataclasses, enums, TypeDefs, and call functions in `operations/` for the runtime `tsurara-client`), built on `tsurara-lower`, `tsurara-naming` and `tsurara-pretty`. Its tests run pyright (`node_modules/.bin/pyright`) and Python 3.11 from the runtime's virtual environment (`uv sync --project runtimes/python`); `docs/languages/python.md` describes the mapping.
- `crates/tsurara-lang-rust` (experimental): the Rust backend (serde structs and enums, `tagged_union!` from the runtime, a `mod.rs` module tree, and async call functions in `operations.rs` with an options struct per operation), built on `tsurara-lower`, `tsurara-naming` and `tsurara-pretty`. Its tests compile the output in a temporary crate with `cargo clippy -D warnings` and run decode tests (`tsurara_testkit::RustChecker`, target dir `target/rust-checker`); `docs/languages/rust.md` describes the mapping. Type modules import the types they use by name, never `use super::*` (a glob there made name resolution quadratic in the number of types).
- `crates/tsurara-lang-go` (experimental): the Go backend (structs with `encoding/json` tags, named types with constants, union wrappers whose methods call the runtime's helpers in `runtimes/go/types.go`, one package of `_gen.go` files printed as gofmt would, aligned with tabwriter rules, and one function per operation in `operations.rs`), built on `tsurara-lower` and `tsurara-naming`. Its tests write the output into a temporary module and require `gofmt -l` to print nothing, `go vet` and `go build` to pass, and run decode tests (`tsurara_testkit::GoChecker`); `docs/languages/go.md` describes the mapping.
- `crates/tsurara-runtimes`: the runtime sources of `runtimes/` (without tests), embedded by a build script for `runtime: vendor`; each backend's `vendor.rs` writes and adapts its copy (design.md 7.5).
- `crates/tsurara-backend`: the `Backend` trait, `Output`/`OutputFile`, `generate` and `generate_with_plugins`, the `Plugin` trait, and `operation_def` (OperationDefs as JSON for backends that print them as literals). It depends on no language crate.
- `crates/tsurara-lang-typescript`: the TypeScript backend. `naming.rs` holds the TypeScript policy and replays the model's name requests; `lower.rs` names and lowers the model (scalars, enums, unions, body types, mapped types) before `emit.rs` assigns output files and prints OperationDefs and functions. Its `tests/` hold the codegen tests (cases, snapshots, tsc checks).
- `crates/tsurara-config`: loads config files into one `Target` (input, output, codegen `Options`, hooks) per target. Each file is env-expanded, validated, and path-resolved on its own before `extends` merging, so paths stay relative to the file they are written in. `raw.rs` holds the serde structs that also generate `schema/tsurara.config.schema.json`.
- `crates/tsurara-cli`: the `tsurara` command. `main.rs` holds the registry of built-in backends. `tsurara generate` works (`write.rs` skips unchanged files and deletes stale files that start with the backend's generated-file marker, with their sidecars; `tsurara_backend::Markers`); `hooks.rs` runs the configuration hooks (`transformDocument`, `transformFiles` as plugins; `afterWrite` after writing); `tsurara check` and `tsurara init` are in `main.rs` (`client_version.rs` compares the runtime version); `inspect.rs` is `tsurara inspect` and `mcp.rs` the MCP server (`tsurara mcp`, which embeds `docs/guide/*` and `docs/languages/*`: a new guide must be added to its list), and `--format json` prints the reports of `src/output.rs` (a library target, `tsurara_cli`, whose schema is `spec/cli-output.schema.json`). Tests in `tests/generate.rs` run the real binary.
- `crates/tsurara-testkit` (unpublished): `assert_tsc_passes` writes generated files to a temp dir and runs `tsc --noEmit`, with `@tsurara/client` mapped to `runtimes/typescript/src`.
- `runtimes/typescript`: `@tsurara/client`. Generated code calls `request` with an `OperationDef`, and gets a `Result` back (`unwrap()` throws `ApiError`). Serialization, multipart included, lives here rather than in generated code. `retry.ts` is the bundled retry middleware, `unwrap.ts` holds `unwrap`, `ApiError`, and `bindClient`, `events.ts` reads Server-Sent Events (`parseAs: 'events'`, emitted for `text/event-stream` responses with `itemSchema` or `x-itemSchema`; design.md 8.5), and `items.ts` reads JSON Lines and JSON text sequences (`jsonLines`, `jsonSeq`). `test/conformance.test.ts` runs the vectors in `spec/conformance/`; `src/testing.ts` is the mock transport (`@tsurara/client/testing`), which the other runtimes mirror (`tsurara_client.testing`, the Rust `testing` feature, the Go package `tsuraratest`).
- `runtimes/typescript/hermes`: `@tsurara/hermes-check` (private), which bundles `@tsurara/client` with React Native's polyfills and Babel preset, compiles it with React Native's `hermesc` and runs it on the Hermes VM named by `TSURARA_HERMES` (the v0.13.0 CLI release; skipped without it unless `TSURARA_REQUIRE_HERMES=1`, as in CI).
- `runtimes/python`: `tsurara-client`, the Python runtime (httpx; `Client` and `AsyncClient`, Results, `unwrap`, the TypeDef codec in `_codec.py`, and the readers of events and JSON Lines in `_sequences.py`). `tests/test_conformance.py` runs the vectors in `spec/conformance/`. Run with `uv sync --project runtimes/python`, `uv run --directory runtimes/python pytest`, and `node_modules/.bin/pyright -p runtimes/python` (strict for `src`).
- `runtimes/rust`: `tsurara-client` for Rust (its own Cargo workspace; reqwest transport behind a `Transport` trait whose `send_streaming` the client reads every response with, `Result<Success, Failure>`, `Items` for responses read item by item in `sequences.rs`, `Nullable`, `tagged_union!`, and the `Input` methods generated functions encode their arguments with in `encode.rs`). `tests/conformance.rs` runs the vectors in `spec/conformance/`; run it with `cargo test --manifest-path runtimes/rust/Cargo.toml --all-features` (the `testing` feature has the mock transport).
- `runtimes/go`: the Go runtime, module `github.com/tsurara-openapi/tsurara/runtimes/go`, package `tsurara` (standard library only, Go 1.24; ordered JSON values in `value.go`, `Do` returning `(*Success[T], error)` with `*Failure[E]`, `DoItems` with `Items[T]` for responses read item by item in `sequences.go` over a `StreamTransport`, the `Input` methods generated functions call in `encode.go`). `conformance_test.go` runs the vectors in `spec/conformance/`; run it with `go -C runtimes/go test -race ./...` (Go comes from `mise.toml`).
- `spec/`: the runtime contract shared by all languages (design.md 18.9): `operation-def.schema.json`, the Result kinds and runtime capabilities in `README.md`, and request and response vectors in `conformance/`. `tsurara-lang-typescript/tests/operation_def_schema.rs` validates emitted OperationDefs against the schema.
- `packages/plugin`: `@tsurara/plugin`, the TypeScript plugin SDK (`definePlugin`, the runner of the session protocol in `tsurara_backend::session`, the `ts.file` writer, `runPlugin`). `src/generated/protocol.ts` is generated by tsurara from `spec/plugin-protocol.schema.json`: rewrite it with `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test plugin_sdk` after the schema changes. `crates/tsurara-cli/src/plugins.rs` runs plugin modules.
- `packages/plugin-python`, `packages/plugin-go`, `crates/tsurara-plugin`: the plugin SDKs for Python, Go and Rust. The Python and Go protocol types (`src/tsurara_plugin/protocol/`, `protocol/`) are generated by tsurara with the same `plugin_sdk` test as the TypeScript ones. `tests/plugin_sdks.rs` checks that the same plugin in every SDK writes the same files.
- `packages/tsurara`: the npm package `tsurara`, which runs the binary of the platform from its optional dependency `@tsurara/cli-<os>-<cpu>` (`lib/platforms.js` lists the platforms and Rust targets; `scripts/npm-packages.mjs` writes the packages of a release, and adds the optional dependencies, which the workspace cannot have before they are published).

Generated code must satisfy TypeScript's `erasableSyntaxOnly`, and `tsconfig.base.json` uses strict options (`exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`, `isolatedDeclarations`, `verbatimModuleSyntax`). Imports in TS sources use `.ts` extensions (`rewriteRelativeImportExtensions`).

## Test first

Write the tests before the implementation. For a new feature or a bug fix, first add tests that express the expected behavior, run them and confirm they fail for the expected reason, then implement until they pass. A test that passes before the implementation does not test the change; fix the test. Pure refactors that keep behavior unchanged rely on the existing tests and snapshots instead.

## CI

`.github/workflows/ci.yml` runs on every push to main and pull request; its `ci` job passes when every other job does (use it as the required check). On pull requests, the `changes` job runs only the jobs the changed files need (`scripts/ci-scope.mjs`: documentation runs the `rust` job, repository settings run none, and any file it does not name runs every job); give a new kind of file a group there only when no other job reads it. Actions are pinned by commit with the version in a comment (Dependabot updates them); downloads are checked against a sha256. `audit` runs cargo-deny (`deny.toml`: RustSec advisories, the allowed licenses, crates.io only) on both Cargo workspaces and `pnpm audit --prod`; `actionlint` lints the workflows (run it locally with `go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12`); `cross-platform` builds the CLI on macOS and Windows and generates and checks code there; `node scripts/release-version.mjs` checks that every published package has the same version.

`.github/workflows/release.yml` runs on a tag `v<version>`: it builds the binaries of `packages/tsurara/lib/platforms.js`, drafts a GitHub release with attested archives, and after an approval of the `release` environment publishes to npm, crates.io and PyPI with trusted publishing and tags the Go modules (`runtimes/go/v<version>`, `packages/plugin-go/v<version>`; `packages/plugin-go/go.mod` requires the runtime of the same version). Run it by hand to build and pack without publishing. `msrv` checks with Rust 1.85, so a dependency that needs a newer Rust must be held back (as `regress` is).

## Dependencies

A dependency version must have been released at least 5 days earlier, so that a compromised release is likely to be found and pulled before it reaches the repository: pnpm refuses newer ones (`minimumReleaseAge: 7200` in each `pnpm-workspace.yaml`), and Dependabot waits as long (`cooldown` in `.github/dependabot.yml`). When adding or bumping a dependency by hand (npm, crates, PyPI, Go modules), pick a version at least 5 days old.

## Licenses

tsurara is MIT licensed. Every published package directory (the crates except `tsurara-testkit`, `runtimes/*`, `packages/*`) has a copy of the root `LICENSE` (`crates/tsurara-cli/tests/licenses.rs` checks them). Files taken from other projects are listed in `THIRD_PARTY_NOTICES.md` with their licenses (`LICENSES/`); add an entry for every such file. Ship `node scripts/third-party-licenses.mjs` (the notices of the crates the binary links) with every binary release.

## Codegen tests

- `crates/tsurara-lang-typescript/tests/cases/*.yaml`: table-driven cases (a small schema and the expected TypeScript type), run by `tests/cases.rs`. Its header comment documents the case format. The rows of the mapping table in design.md 6.4 go in `table.yaml`. All case outputs are also passed through `tsc` together.
- Full-output checks are insta snapshots in `snapshots/` next to each test. Input fixtures are in `tests/fixtures/`.

## Repository hygiene

The repository is public. Keep it to the product: code, tests, user documentation, the design (`docs/design.md`) and the specification (`spec/`).

- Do not keep work logs in the repository: no issue files, task lists, evaluation or review notes, progress reports or plans. Track work outside the repository; describe what a change does in its commit message.
- Do not refer to issue numbers in code, comments, tests or documentation. Explain the reason itself instead.
- Do not name other projects that are not public, nor people or private details.
- Commit messages have no `Co-Authored-By` trailers.
