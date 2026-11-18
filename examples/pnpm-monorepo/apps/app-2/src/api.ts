// A Node app run directly with type stripping (Node 22.18 or later): it imports the generated `.ts`
// files of the shared package as they are.
import { type Client, createClient, retry } from '@acme/openapi';

export type Context = { requestId: string };

export const createApi = (baseUrl: string, timeout = 1000): Client<Context> =>
  createClient<Context>({
    baseUrl,
    timeout,
    headers: (info) => ({ 'x-request-id': info.context?.requestId }),
    middleware: [retry({ max: 1, baseDelay: 10 })],
  });
