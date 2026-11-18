import { isServer, type QueryClient } from '@tanstack/react-query';
import { makeQueryClient } from './api';

let browserClient: QueryClient | undefined;

/**
 * On the server, a new QueryClient per request, so that requests (and tenants) never share a cache.
 * In the browser, one for the page's life, so that a suspending render does not lose it.
 */
export const getQueryClient = (): QueryClient => {
  if (isServer) return makeQueryClient();
  browserClient ??= makeQueryClient();
  return browserClient;
};
