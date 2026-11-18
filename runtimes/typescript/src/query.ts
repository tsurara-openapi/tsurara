/**
 * Query key scopes: which callers may share cached results, for the TanStack Query options
 * tsurara generates (`typescript.query: tanstack`). Plain data, so the runtime does not depend on
 * TanStack Query.
 */

/**
 * A JSON value that names the callers that may share results: a tenant, an API, an environment. Never
 * a credential; it is part of every query key and is sent to the browser with dehydrated state.
 */
export type QueryScope =
  | string
  | number
  | boolean
  | null
  | readonly QueryScope[]
  | { readonly [key: string]: QueryScope };

/**
 * The first element of a scoped query key: `[{ scope }, operationId, data]`. The scope is `unknown`
 * here: a recursive type in the key makes adapters that map keys deeply (Vue's `MaybeRefDeep`) give
 * up. The functions that build keys take a `QueryScope`.
 */
export type QueryScopeKey = { readonly scope: unknown };

/**
 * A filter for every query (and mutation) of a scope, for `removeQueries`, `invalidateQueries` and
 * `isMutating` on logout or a tenant switch.
 */
export const queryScopeFilter = (
  scope: QueryScope,
): { queryKey: readonly [QueryScopeKey]; mutationKey: readonly [QueryScopeKey] } => ({
  queryKey: [{ scope }],
  mutationKey: [{ scope }],
});
