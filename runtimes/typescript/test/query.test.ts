import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { CancelledError, QueryClient } from '@tanstack/query-core';
import {
  type AnyResult,
  ApiError,
  type Client,
  createClient,
  queryRetry,
  retry,
  unwrap,
} from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

// Query options as the generated factories build them (`typescript.query: tanstack`).
const getPet = (api: Client) => ({
  queryKey: ['getPetById', { path: { petId: 1 } }],
  queryFn: async ({ signal }: { signal: AbortSignal }) =>
    unwrap(await api.request<AnyResult>(getPetDef, { path: { petId: 1 }, signal })),
});

const failing = (respond: () => Response | Promise<Response>) => {
  const mock = mockFetch(respond);
  return { api: createClient({ baseUrl: 'https://x', fetch: mock.fetch }), calls: mock.calls };
};

const queryClient = () =>
  new QueryClient({ defaultOptions: { queries: { retry: queryRetry(), retryDelay: 0 } } });

const failure = (result: Record<string, unknown>) =>
  new ApiError({ ok: false, ...result } as unknown as Extract<AnyResult, { ok: false }>);

describe('queryRetry', () => {
  test('retries what the retry middleware retries, and nothing else', () => {
    const decide = queryRetry();
    const retried = [
      failure({ kind: 'network', cause: new TypeError('offline') }),
      ...[408, 429, 500, 502, 503, 504].map((status) =>
        failure({ kind: 'http', status, documented: false, error: undefined }),
      ),
    ];
    const kept = [
      failure({ kind: 'timeout', cause: undefined }),
      failure({ kind: 'aborted', cause: undefined }),
      failure({ kind: 'parse', status: 200, cause: undefined }),
      failure({ kind: 'validation', status: 200, issues: [] }),
      failure({ kind: 'unexpected', cause: new Error('hook') }),
      ...[400, 401, 403, 404, 409, 422, 501].map((status) =>
        failure({ kind: 'http', status, documented: true, error: {} }),
      ),
    ];
    for (const e of retried) assert.equal(decide(0, e), true, `${e.kind} ${e.status}`);
    for (const e of kept) assert.equal(decide(0, e), false, `${e.kind} ${e.status}`);
    // Errors that are not from a call (a bug in a queryFn) are not retried either.
    assert.equal(decide(0, new TypeError('bug')), false);
  });

  test('stops after max retries, 2 by default like the middleware', () => {
    const e = failure({ kind: 'network', cause: undefined });
    assert.deepEqual(
      [0, 1, 2].map((n) => queryRetry()(n, e)),
      [true, true, false],
    );
    assert.deepEqual(
      [0, 1].map((n) => queryRetry({ max: 1 })(n, e)),
      [true, false],
    );
    assert.equal(queryRetry({ statuses: [418] })(0, failure({ kind: 'http', status: 418 })), true);
  });

  test('a server that keeps answering 503 is called max + 1 times', async () => {
    const { api, calls } = failing(() => json(503, {}));
    const error = await queryClient()
      .fetchQuery(getPet(api))
      .catch((e: unknown) => e);
    assert.ok(error instanceof ApiError && error.status === 503);
    assert.equal(calls.length, 3);
  });

  test('a documented 404 is not retried', async () => {
    const { api, calls } = failing(() => json(404, { title: 'gone' }));
    await queryClient()
      .fetchQuery(getPet(api))
      .catch(() => undefined);
    assert.equal(calls.length, 1);
  });

  test('a network failure is retried', async () => {
    const { api, calls } = failing(() => {
      throw new TypeError('offline');
    });
    await queryClient()
      .fetchQuery(getPet(api))
      .catch(() => undefined);
    assert.equal(calls.length, 3);
  });

  test('with the middleware instead, retry: false keeps the attempts to its own', async () => {
    const mock = mockFetch(() => json(503, {}));
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mock.fetch,
      middleware: [retry({ baseDelay: 0 })],
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    await qc.fetchQuery(getPet(api)).catch(() => undefined);
    assert.equal(mock.calls.length, 3);
  });
});

describe('both retrying layers', () => {
  const both = (respond: () => Response | Promise<Response>) => {
    const mock = mockFetch(respond);
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mock.fetch,
      middleware: [retry({ baseDelay: 0 })],
    });
    return { api, calls: mock.calls };
  };

  test('a 503 server is called as often as one layer allows', async () => {
    const { api, calls } = both(() => json(503, {}));
    await queryClient()
      .fetchQuery(getPet(api))
      .catch(() => undefined);
    assert.equal(calls.length, 3);
  });

  test('a network failure is sent as often as one layer allows', async () => {
    const { api, calls } = both(() => {
      throw new TypeError('offline');
    });
    await queryClient()
      .fetchQuery(getPet(api))
      .catch(() => undefined);
    assert.equal(calls.length, 3);
  });

  test('a failure the middleware did not retry is still retried by the query', async () => {
    // POST is not retried by the middleware, so the query retries it.
    const mock = mockFetch(() => json(503, {}));
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mock.fetch,
      middleware: [retry({ baseDelay: 0, methods: [] })],
    });
    await queryClient()
      .fetchQuery(getPet(api))
      .catch(() => undefined);
    assert.equal(mock.calls.length, 3);
  });
});

describe('cancelling a query', () => {
  test('ends it without an error state', async () => {
    const api = createClient({ baseUrl: 'https://x', fetch: hangingFetch });
    const qc = queryClient();
    const options = getPet(api);
    const pending = qc.fetchQuery(options).catch((e: unknown) => e);
    await new Promise((resolve) => setTimeout(resolve, 5));
    await qc.cancelQueries({ queryKey: options.queryKey });
    assert.ok((await pending) instanceof CancelledError);
    const state = qc.getQueryState(options.queryKey);
    assert.equal(state?.error, null);
    assert.equal(state?.fetchStatus, 'idle');
  });
});
