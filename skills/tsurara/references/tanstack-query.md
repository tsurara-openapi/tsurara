# TanStack Query and mocks (TypeScript)

With `typescript.query: tanstack`, every operation also has options for TanStack Query 5 in `query/{function}.ts`, and with `typescript.mocks: msw` an MSW handler in `mocks/operations/{function}.ts`. Use them instead of writing a `queryFn`, a query key or a request handler by hand: `tsurara inspect operation <id>` (or `describe_operation`) lists them under "also generated". `tsurara init --preset react-query` turns both on; `docs/guide/react.md` walks through a complete app.

```ts
import {
  QueryClient,
  skipToken,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { createClient, isHttpError, queryRetry, queryScopeFilter } from '@tsurara/client';
import { createPetsMutationOptions } from './query/createPets.js';
import { listPetsQueryFilter } from './query/listPets.js';
import {
  setShowPetByIdQueriesData,
  showPetByIdQueryKey,
  showPetByIdQueryOptions,
} from './query/showPetById.js';

// One client per session; `queryScope` keeps the cache of tenants (or APIs) apart.
const scope = { tenant: 'acme' };
const api = createClient({ baseUrl: 'https://petstore.example.com/v1', queryScope: scope });

// Retries like the retry() middleware (network failures, 408, 429, 5xx), never 404 or 422.
const queryClient = new QueryClient({ defaultOptions: { queries: { retry: queryRetry() } } });

export function usePetName(petId: string | undefined): string | undefined {
  // `skipToken` while the input is not known; `select` decides the type of `data`.
  const pet = useQuery({
    ...showPetByIdQueryOptions(api, petId === undefined ? skipToken : { path: { petId } }),
    select: (p) => p.name,
  });
  if (pet.error && pet.error.kind === 'http' && pet.error.status === 404) return undefined;
  return pet.data;
}

function useCreatePet() {
  const qc = useQueryClient();
  return useMutation({
    ...createPetsMutationOptions(api),
    onSuccess: async () => {
      // Every listPets query of this tenant, by the generated filter: no hand-written key.
      await qc.invalidateQueries(listPetsQueryFilter(scope));
    },
    onError: (error) => {
      // The documented error body, typed, when the status is documented.
      if (isHttpError(error.result, 500)) console.error(error.result.error.message);
    },
  });
}

export function renamePet(petId: string, name: string): void {
  // One entry by its key, or every showPetById entry with the data typed.
  queryClient.setQueryData(showPetByIdQueryKey({ path: { petId } }, scope), (old) => old && { ...old, name });
  setShowPetByIdQueriesData(queryClient, (old) => old && { ...old, name }, scope);
}

export function signOut(): void {
  queryClient.removeQueries(queryScopeFilter(scope));
}

void useCreatePet;
```

- Keys are `[operationId, data]`, or `[{ scope }, operationId, data]` for a client with `queryScope`; build them with `{function}QueryKey(data, scope)`, never by hand.
- Paginated lists: write the pagination on the operation (`x-tsurara-pagination: { param: query.cursor, next: /nextCursor }`) or in `typescript.infiniteQueries`; the read then has `{function}InfiniteQueryOptions`, for `useInfiniteQuery`.
- The options throw `ApiError` (they `unwrap`); `error.result` is the Result, so branch with `isHttpError(error.result, status)` and `error.kind`.

Tests use the generated handlers; each takes a documented response, or a function of the request that returns one:

```ts
import { setupServer } from 'msw/node';
import { handlers } from './mocks/index.js';
import { showPetByIdHandler } from './mocks/operations/showPetById.js';
import { mockPet } from './mocks/schemas/Pet.js';

const server = setupServer(...handlers);
server.listen({ onUnhandledFrame: 'error' }); // MSW 3; `onUnhandledRequest` in MSW 2
server.use(showPetByIdHandler(({ params }) => ({ status: 200, body: mockPet({ id: Number(params.petId) }) })));
```
