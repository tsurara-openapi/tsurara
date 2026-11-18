# pnpm monorepo example

One generated output shared by two apps (design.md chapter 9).

| Package | What it shows |
|---|---|
| `packages/openapi` (`@acme/openapi`) | The shared package: `tsurara.config.yaml`, the generated files in `src/generated` (`importExtension: ts`), and `exports` that point at `.ts` files: the barrel and the `operations/*` and `schemas/*` subpaths. It depends on `@tsurara/client` and re-exports it, so the workspace has one copy |
| `apps/app-1` | A browser app bundled with Vite. It imports from the barrel; `pnpm test` builds it and checks that only the operations it uses are in the bundle |
| `apps/web-next` | A Next.js app (App Router): a server component and a client component call the generated functions. `pnpm test` runs `next build`; no `transpilePackages` is needed |
| `apps/app-2` | A Node app run with type stripping (Node 22.18 or later), with its own context type, headers and `retry`. `pnpm test` runs end-to-end tests against a server on localhost: success, documented errors, `unwrap`, a retried 503, a timeout and an abort |

Every tsconfig that compiles code importing `@acme/openapi` has `allowImportingTsExtensions` (in `tsconfig.base.json`), because the generated files import each other with `.ts` extensions.

```sh
# From the repository root: the generator and the runtime this example links to.
cargo build --release -p tsurara-cli
pnpm install && pnpm --filter @tsurara/client build
# The example.
pnpm --dir examples/pnpm-monorepo install
pnpm --dir examples/pnpm-monorepo --filter @acme/openapi check   # tsurara check: generated files are current
pnpm --dir examples/pnpm-monorepo typecheck
pnpm --dir examples/pnpm-monorepo test
```

After changing `packages/openapi/openapi.yaml`, run `pnpm --dir examples/pnpm-monorepo generate`.
