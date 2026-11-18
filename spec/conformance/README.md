# Conformance vectors

Data that every runtime runs through its request function. They are generated from the examples of the OpenAPI specification and the cases `@tsurara/client` was built against; `runtimes/typescript/test/conformance.test.ts` is the reference runner.

## Values that are not JSON

Inputs and expected values use tagged objects for values JSON cannot hold:

- `{ "$blob": { "type": "image/png", "text": "..." } }`: a binary value with a media type and its contents as text.
- `{ "$file": { "name": "a.txt", "type": "text/plain", "text": "..." } }`: a binary value with a file name.

A missing value (`undefined` in JavaScript, no value in other languages) is `null` in expected data.

## `requests.json`

Each vector calls the operation with an input against the base URL `https://api.test` and checks the request the runtime sends.

```json
{
  "name": "query form array without explode",
  "operation": { "v": 1, "id": "op", "method": "GET", "path": "/q", "params": { "query": { "color": { "explode": false } } }, "responses": { "204": { "parseAs": "none" } } },
  "input": { "query": { "color": ["blue", "black", "brown"] } },
  "expect": { "method": "GET", "url": "https://api.test/q?color=blue,black,brown", "body": null }
}
```

- `input` has the groups of the call: `path`, `query`, `headers`, `cookies`, `querystring`, `body` and `contentType`.
- `expect.headers` lists headers to check; `null` means the header must be absent.
- `expect.body` is `null` (no body), `{ "text": "..." }`, `{ "bytes": { "type", "text" } }`, or `{ "form": [[name, value], ...] }` for multipart, in order. A `$blob` part is compared by type and contents only, because runtimes name parts without a file name differently.
- When `expect.kind` is present, the runtime must not send a request and must return a result of that kind.
- `credentials` (optional) maps security scheme names to the credential the runtime's auth hook returns for them; a scheme without an entry has no credential.

## `typedefs.json`

`typeDefs` is a TypeDef document; each vector names a `type` and lists `decode` cases (`json` to the expected `value`, or `error: true`) and `encode` cases (`value` to the expected `json`). Values use the neutral form: records are objects with `$type` and member names, extra properties go in `$additional`, unlisted values are `{ "$unknown": value }`, and external values are `{ "$external": converter, "value": value }`. `bytes` has no vector, because binary values only appear as bodies and multipart parts, not inside JSON. `crates/tsurara-lower/tests/typedef.rs` checks the vectors with a small reference converter and checks that they use every construct of the format.

## `sequences.json`

Responses read item by item (`parseAs` `events`, `jsonLines` and `jsonSeq`, design.md 8.5), for runtimes that declare those body kinds. Each vector answers with `response` whose body arrives as `chunks` (text, one write each, in order), and checks the result: for `success`, the `status` and the `items` read from `data` in order; with `error: true`, reading the next item after them fails. Events are objects with `event`, `data`, and `id` and `retry` only when the event has them. For `http`, the `status`, `documented` and `error` as in `responses.json`.

## `responses.json`

Each vector calls the operation without input, answers with `response` (`status`, `headers`, `body` as `{ "text": "..." }` or `null`), and checks the result: `kind`, and for `success` the `status` and `data`, for `http` the `status`, `documented` and `error`.

## `mock-routing.json`

Routing of the mock transports (spec/README.md, Mock transports): each vector lists routes as method and path template, a base path and a request; `expect.route` is the index of the route that answers (`null`: none) and `expect.params` the decoded path parameters.
