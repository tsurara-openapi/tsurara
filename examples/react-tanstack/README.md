# React and TanStack Query example

A Next.js app (App Router) over the card API in `openapi.yaml`: a search list read page by page, a detail page with an edit form, two tenants, MSW tests and server rendering, with nothing but generated code and `@tsurara/client` between the components and the API. [docs/guide/react.md](../../docs/guide/react.md) walks through it.

| Path | What it shows |
|---|---|
| `tsurara.config.yaml` | The configuration `tsurara init --preset react-query` writes |
| `src/api.ts` | A client per session (`auth`, `queryScope`) and the `QueryClient` (`queryRetry()`, undocumented statuses reported) |
| `src/cards.ts` | Infinite list options, the edit with an optimistic update, rollback and cache updates, typed 422 field errors, dropping a tenant's cache |
| `src/components/` | The list (search, infinite scrolling, prefetch on hover) and the editor |
| `app/` | Server prefetching with `HydrationBoundary`, a `QueryClient` per request |
| `test/` | Vitest with the generated MSW handlers: the logic, the components (happy-dom) and server rendering |

```sh
# From the repository root: the generator and the runtime this example links to.
cargo build --release -p tsurara-cli
pnpm install && pnpm --filter @tsurara/client build
# The example.
pnpm --dir examples/react-tanstack install
pnpm --dir examples/react-tanstack check   # tsurara check: generated files are current
pnpm --dir examples/react-tanstack test
pnpm --dir examples/react-tanstack build   # next build, which also writes the route types
pnpm --dir examples/react-tanstack typecheck
```

After changing `openapi.yaml`, run `pnpm --dir examples/react-tanstack generate`.
