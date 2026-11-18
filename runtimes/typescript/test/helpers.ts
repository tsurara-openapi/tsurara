import type { OperationDef } from '../src/index.ts';

export type Call = { url: string; init: RequestInit };

/** A fetch that records calls and returns a fixed response. */
export const mockFetch = (
  respond: (url: string, init: RequestInit) => Response | Promise<Response>,
): {
  fetch: (input: URL | Request | string, init?: RequestInit) => Promise<Response>;
  calls: Call[];
} => {
  const calls: Call[] = [];
  return {
    calls,
    fetch: async (input, init = {}) => {
      const url = String(input);
      calls.push({ url, init });
      return respond(url, init);
    },
  };
};

export const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });

/** A fetch that never responds until aborted. */
export const hangingFetch = async (_input: URL | Request | string, init?: RequestInit) =>
  new Promise<Response>((_, reject) => {
    init?.signal?.addEventListener('abort', () => reject(init.signal?.reason));
  });

export const getPetDef = {
  v: 1,
  id: 'getPetById',
  method: 'GET',
  path: '/pet/{petId}',
  tags: ['pet'],
  params: {
    path: { petId: {} },
    query: { expand: { style: 'form', explode: false } },
  },
  responses: {
    200: { parseAs: 'json' },
    404: { parseAs: 'json' },
    default: { parseAs: 'json' },
  },
} as const satisfies OperationDef;
