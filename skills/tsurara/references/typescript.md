# TypeScript

Each operation is a function `operationId(client, options)` in `operations/`, with types `{Op}Data` (its input), `{Op}Result` (every outcome) and the schema types in `schemas/`. Create one client per base URL and pass it explicitly.

```ts
import { createClient, unwrap } from '@tsurara/client';
import { showPetById } from './operations/showPetById.js';
import type { Pet } from './schemas/Pet.js';

const api = createClient({ baseUrl: 'https://petstore.example.com/v1' });

// Results never throw: check `ok`, then `kind`.
export async function petName(petId: string): Promise<string | undefined> {
  const result = await showPetById(api, { path: { petId } });
  if (result.ok) {
    return result.data.name;
  }
  if (result.kind === 'http' && result.documented) {
    console.error(result.status, result.error.message); // the documented error body
  } else {
    console.error(result.kind); // network, timeout, aborted, parse, ...
  }
  return undefined;
}

// Where an exception is right (inside a TanStack Query queryFn, a script): `unwrap` returns the data
// or throws `ApiError` with the Result.
export async function loadPet(petId: string): Promise<Pet> {
  return unwrap(await showPetById(api, { path: { petId } }));
}
```

- Imports end in the configured extension (`.js` by default, `.ts` with `typescript.importExtension: ts`).
- `{ path, query, headers, cookies, body }` follow the definition; optional parameters can be left out.
- Request bodies of several media types take `contentType` next to `body`.
- `createClient` options: `headers`, `auth` (per security scheme), `timeout`, `middleware`, `onResult`, `validate` (with the generated Zod schemas, `typescript.validators: zod`), `queryScope` (TanStack Query keys).
- With TanStack Query or MSW, read [tanstack-query.md](tanstack-query.md): use the generated options and handlers, not a hand-written `queryFn`, key or handler.
