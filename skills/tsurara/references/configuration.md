# Configuration

`tsurara.config.yaml` (or `.json`, `.toml`) at the project root; `docs/guide/configuration.md` in the tsurara repository (or the MCP resource `tsurara://docs/guide/configuration.md`) lists every key.

```yaml
input: ./openapi.yaml            # a file or a URL
targets:
  - name: web
    output: ./web/src/api
    typescript:
      importExtension: ts        # js (default), ts or none
      validators: zod            # also generate Zod schemas for validateResponses
      validatorConstraints: true # and check minLength, pattern, minimum, ... in them
      query: tanstack            # TanStack Query options, keys and filters (query/)
      mocks: msw                 # MSW handlers and mock factories (mocks/); or fetch
      infiniteQueries:           # pagination of reads, for infinite query options
        listPets: { param: query.cursor, next: /nextCursor }
  - name: worker
    output: ./worker/api
    language: python
naming:
  overrides:
    "#/components/schemas/Error": ApiError   # rename by JSON Pointer
typeMappings:
  - match: { type: string, format: uuid }
    typescript: { type: Uuid, import: { from: ./ids.ts, name: Uuid } }
hooks:
  afterWrite:
    - [biome, format, --write, "{output}"]
```

- `extends: ./base.yaml` merges another file; paths stay relative to the file they are written in.
- `${ENV}` in strings is replaced with environment variables (an unset one is an error).
- Hooks: `transformDocument`, `transformModel`, `extend` (add files next to the generated code, with the manifest of generated names; see `docs/guide/plugins.md`), `transformFiles`, `afterWrite`.
- `tsurara init --preset react-query` writes a configuration with query options, Zod schemas and MSW handlers for a React app, and prints the packages to install.
- Pagination can also be written on the operation in the definition: `x-tsurara-pagination: { param: query.cursor, next: /nextCursor }`.
- Python, Rust and Go generate mocks with `python.mocks`, `rust.mocks` and `go.mocks: true`.
- After changing the definition or the configuration, run `tsurara generate`, then `tsurara check` in CI.
