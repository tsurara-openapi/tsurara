# Runtime contract

Generated code in every language is thin: it describes each operation and passes the description to a runtime library, which builds the request and reads the response (design.md 8.5, 18.9). This directory defines what every runtime implements, independently of TypeScript.

| Part | Where |
|---|---|
| OperationDef format | `operation-def.schema.json` (JSON Schema, version 1) |
| Result kinds | Below |
| Serialization and parsing | design.md 8.5, made precise by the vectors in `conformance/`. Query parameters (and members of an `in: querystring` object) whose value is `null` are left out. A multipart object without an `encoding` Content-Type is a field with its JSON text. Text bodies are decoded with the `charset` of their Content-Type, UTF-8 without one (Go keeps the bytes of a `string` as they arrive, since its standard library has no other charsets). A request whose path template does not start with `/`, whose path parameter is missing, or whose expanded value makes a whole segment empty, `.` or `..`, is not sent and returns `unexpected` |
| Runtime capabilities | Below |
| TypeDef format | `type-def.schema.json` (version 1), for runtimes that convert values with TypeDefs |
| Conformance vectors | `conformance/` ([format](conformance/README.md)) |

## OperationDef

A backend may emit the OperationDef as a literal of its language. Converted to JSON, it must validate against `operation-def.schema.json`. `v` is the format version; it changes only when the format changes incompatibly, and every runtime checks it.

## TypeDefs

Runtimes of languages without annotation-driven serialization (Python with dataclasses, PHP, Ruby, GDScript) convert values with TypeDefs (design.md 18.7): `tsurara_lower::typedef::type_defs` builds them from the lowered module with the backend's names. OperationDefs refer to them with optional `type` (parameters and request body media types) and `types` (response bodies per media type); other runtimes ignore these fields.

TypeDefs belong to the types layer of a runtime (design.md 18.9), which has no HTTP client: the TypeDef vectors of `conformance/typedefs.json` must pass with that layer alone (in Python, `runtimes/python/tests/test_typedefs.py` runs without httpx).

Decoding (JSON to values) applies renames, decodes nested records, keeps unlisted values of open enums and open tagged unions as `{ "$unknown": value }`, coerces integers that arrive as floats with no fraction, and passes external types to converters the user registers by name. Encoding is the inverse: a missing optional field is omitted, `null` is written only for an explicit `null`, and unknown values are written back as received.

## Result kinds

A request never fails with an exception or a rejected promise for the outcomes below; it returns a result whose kind is one of these. Each language chooses how to represent them (a discriminated union in TypeScript, an enum in Rust and Swift, a sealed hierarchy in Kotlin, Java and C#, `(T, error)` with typed errors in Go, a result object in GDScript, Python, PHP and Ruby). Runtimes may also offer an `unwrap` that raises for every kind but `success`.

| Kind | Success | When | Carries |
|---|---|---|---|
| `success` | yes | A success status of the definition was returned. A `default` response counts as success only when no other success status is defined | status, data, the raw response |
| `http` | no | An error status, or a status not in the definition. `documented` tells which; undocumented bodies are read as JSON if possible, otherwise as text | status, error, documented, the raw response |
| `network` | no | The transport failed (DNS, connection, CORS, and so on), also while the body was read | cause; a runtime may also carry the raw response when the failure ended the reading of its body (`@tsurara/client` does) |
| `timeout` | no | The client's timeout was exceeded, over all attempts | cause, and the raw response as for `network` |
| `aborted` | no | The caller cancelled | cause, and the raw response as for `network` |
| `parse` | no | The body could not be read as the definition says, including a Content-Type that does not match | cause, the raw response, content type |
| `unexpected` | no | User code (middleware, hooks) failed, or the input cannot be sent (for example a binary value as a JSON body) | cause |
| `validation` | no | Reserved; not returned yet | — |

The contract does not assume promises: a runtime may be synchronous, asynchronous, or both.

How many times a request was sent is not part of a result. `@tsurara/client` keeps it next to the results its `retry()` middleware gave up on, for its TanStack Query `retry` option (`queryRetry()`); runtimes without a query cache that retries on its own do not need it.

## Authentication

`security` in an OperationDef lists the authentication alternatives in order; each lists the schemes it needs together, and an empty alternative means none. A runtime asks a hook for the credential of each scheme (by its `scheme` name, with the operation) and uses the first alternative for which every scheme has one; without one, nothing is sent. Credentials are placed by scheme: `apiKey` in the header, query parameter or cookie it names; `http` with `bearer` (and `oauth2`, `openIdConnect`) as `Authorization: Bearer <credential>`; `http` with `basic` as `Authorization: Basic <base64 of the UTF-8 credential>`, where the credential is `user:password`; other `http` schemes as `Authorization: <scheme> <credential>`. `mutualTLS` is for the transport. The request vectors with `credentials` check this.

## Redirects

The default transport of a runtime follows a redirect only within the origin (scheme, host and port) of the request, at most 10 times. A redirect to another origin is not followed: its response is the result (an `http` failure with the 3xx status), so that credentials and other headers never reach another host. A runtime may follow fewer redirects (the Python runtime follows none, as httpx does by default). A transport the application provides decides for itself.

## Runtime capabilities

A runtime declares what it supports:

- body kinds (`json`, `text`, `blob`, `stream`, `none`, `auto`, and optionally `events`, `jsonLines` and `jsonSeq`),
- parameter styles (`simple`, `label`, `matrix`, `form`, `spaceDelimited`, `pipeDelimited`, `deepObject`),
- request media types beyond JSON (`multipart/form-data`, `application/x-www-form-urlencoded`, binary).

A backend compares each operation with the capabilities of its runtime. For an operation the runtime cannot perform, it emits the types but not the call function, and warns. `@tsurara/client` supports everything listed.

`events`, `jsonLines` and `jsonSeq` are the optional body kinds. `jsonLines` reads one JSON text per line (blank lines skipped) and `jsonSeq` one per record that starts with U+001E (RFC 7464; empty records skipped); a backend emits them for `application/jsonl`, `application/x-ndjson` and `application/json-seq` responses with an item schema, and `stream` for a runtime that does not declare them.

For `events`: a `text/event-stream` response whose media type has an item schema (`itemSchema` in OpenAPI 3.2, or `x-itemSchema`) is read as events, and `jsonEvents` names the events whose data is JSON. A backend whose runtime does not declare it emits `stream` for the same response, so every runtime can still perform the operation. Every runtime in this repository declares them: `@tsurara/client`, the Python and Rust `tsurara-client`, and the Go runtime. A runtime that declares them passes the vectors in `conformance/sequences.json`. Events are parsed as the HTML standard describes; an event without `data` is not handed over, and `id` and `retry` are those of the event itself.

A runtime passes every vector in `conformance/` for the features it declares before its backend emits call functions.

## Versions

`v` of the OperationDef and `v` of TypeDefs (`tsurara_lower::typedef::TYPE_DEF_VERSION`) are shared by all runtimes. The version range of the runtime library that generated code requires is per runtime; each backend records it in the generated files through its version marker (`tsurara_backend::Markers`), and `tsurara check` compares it with the installed library.

## Mock transports

Every runtime has a testing module with a mock transport: it answers at the transport seam (the `fetch` of the TypeScript client, an httpx transport, the Rust `Transport`, the Go `StreamTransport`), so tests need neither a server nor a third-party mocking library.

- A route is a method and a path template (usually taken from an OperationDef) with a handler.
- Routing: the query string is ignored; the router's base path (default empty) is removed from the request path, which must start with it. The rest is split on `/` and must have as many segments as the template. A literal segment must be equal; a segment with `{name}` parts matches when its literal parts match, each parameter taking at least one character (the last match wins inside a segment: `{name}.{ext}` on `report.final.pdf` gives `report.final` and `pdf`); parameters are percent-decoded after matching, so an encoded `/` stays in one parameter. The method is compared without case. Among matching routes, the one with the most literal segments wins, then the earliest. `conformance/mock-routing.json` holds the vectors.
- The handler receives the method, the URL, the decoded path parameters, the query pairs, the headers and the raw body, and returns a status, headers and a body (text, bytes, or a JSON value the mock encodes with `Content-Type: application/json`). A handler that fails (throws, raises, returns an error) makes the transport fail, which the client reports as a transport failure (`network`).
- A request no route matches fails the transport with a message naming the method and path, unless the mock records unmatched requests, then it answers `501`.
- The mock records every call (method, URL, matched route, parameters, body) for assertions.

## Event channels

A channel of `x-event-channels` (design.md 8.8) is described to a runtime by a channel definition: `{ v: 1, id, channel, parseAs }`, where `id` is the channel's name, `channel` the topic template and `parseAs` `json` or `text`. A runtime decodes the text of one message: `json` parses it (a failure is the `parse` kind, with the text), `text` keeps it; then an optional validator returns issues (the `validation` kind, with the value). Decoding never performs I/O, so it works with any transport. Only the TypeScript runtime implements it so far (`decodeMessage`, `validateChannels`).

## Model, manifest and plugin protocol

Programs outside tsurara (plugins, hooks, external backends) receive JSON whose shapes are defined here, generated from the Rust types:

| Schema | What | Version field |
|---|---|---|
| `sem-model.schema.json` | The semantic model (design.md 18.4.6), as `transformModel`, `extend` and external backends receive it | `semVersion` |
| `manifest.schema.json` | What a backend generated, by name (design.md 11.1) | `manifestVersion` |
| `plugin-protocol.schema.json` | The messages: `Describe`, `GenerateRequest` and `GenerateResponse` of external backends (18.11), `ExtendInput` of the `extend` hook, and `Files` of `extend` and `transformFiles` | `protocol` (external backends) |

Each version field changes only when its form changes incompatibly; the schemas change with every form change. `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test spec_schemas` rewrites them, and the test fails when a committed file differs from the Rust types. Every change is recorded in [CHANGELOG.md](CHANGELOG.md) with the fingerprint the test prints; the compatibility rules are in design.md chapter 14.
