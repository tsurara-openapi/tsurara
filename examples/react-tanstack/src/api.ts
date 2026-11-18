import { QueryCache, QueryClient } from '@tanstack/react-query';
import { ApiError, type Client, createClient, type QueryScope, queryRetry } from '@tsurara/client';

export const baseUrl = 'https://cards.example.com';

/** Who is signed in. The tenant decides which cached results may be shared. */
export type Session = { readonly tenant: string; readonly token: string };

/** The scope of a session's queries: part of every query key of its client. */
export const scopeOf = (session: Session): QueryScope => ({ tenant: session.tenant });

/** A client per session: its token, and its tenant as the query scope. */
export const createApi = (session: Session): Client =>
  createClient({
    baseUrl,
    queryScope: scopeOf(session),
    auth: () => session.token,
  });

/**
 * The QueryClient settings of the app: retries as the `retry()` middleware would do them (network
 * failures and 408, 429, 5xx), so 404 and 422 fail at once; and statuses the definition does not
 * document are reported.
 */
export const makeQueryClient = (
  report: (error: ApiError) => void = (error) =>
    console.warn('undocumented status', error.status, error.operationId),
): QueryClient =>
  new QueryClient({
    defaultOptions: { queries: { retry: queryRetry(), staleTime: 30_000 } },
    queryCache: new QueryCache({
      onError: (error) => {
        if (error instanceof ApiError && error.result.kind === 'http' && !error.result.documented)
          report(error);
      },
    }),
  });
