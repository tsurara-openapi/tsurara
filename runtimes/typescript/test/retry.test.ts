import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  type AnyResult,
  type ClientOptions,
  createClient,
  type Middleware,
  type OperationDef,
  retry,
} from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

const postDef = {
  v: 1,
  id: 'createPet',
  method: 'POST',
  path: '/pet',
  body: { required: true, contents: { 'application/json': {} } },
  responses: { 200: { parseAs: 'json' } },
} as const satisfies OperationDef;

const putBinaryDef = {
  v: 1,
  id: 'upload',
  method: 'PUT',
  path: '/file',
  body: { required: true, contents: { 'application/octet-stream': {} } },
  responses: { 200: { parseAs: 'json' } },
} as const satisfies OperationDef;

/** Responds with the given statuses in order, then 200. */
const sequence = (...statuses: number[]) =>
  mockFetch(() => {
    const status = statuses.shift() ?? 200;
    return json(status, { status });
  });

const client = (options: Omit<ClientOptions, 'baseUrl'>) =>
  createClient({ baseUrl: 'https://x', ...options });

const noDelay = { delay: () => 0 };

describe('retry', () => {
  test('retries retryable statuses until success', async () => {
    const m = sequence(503, 502);
    const api = client({ fetch: m.fetch, middleware: [retry({ max: 2, ...noDelay })] });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'success');
    assert.equal(m.calls.length, 3);
  });

  test('returns the last response after max retries', async () => {
    const m = sequence(503, 503, 503, 503);
    const api = client({ fetch: m.fetch, middleware: [retry({ max: 2, ...noDelay })] });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'http');
    assert.equal(r.status, 503);
    assert.equal(m.calls.length, 3);
  });

  test('retries network failures', async () => {
    let n = 0;
    const api = client({
      fetch: async () => {
        n++;
        if (n === 1) throw new TypeError('fetch failed');
        return json(200, {});
      },
      middleware: [retry(noDelay)],
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'success');
    assert.equal(n, 2);
  });

  test('does not retry other statuses', async () => {
    const m = sequence(404);
    const api = client({ fetch: m.fetch, middleware: [retry(noDelay)] });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'http');
    assert.equal(m.calls.length, 1);
  });

  test('only retries idempotent methods by default', async () => {
    const m = sequence(503);
    const api = client({ fetch: m.fetch, middleware: [retry(noDelay)] });
    await api.request(postDef, { body: { name: 'a' } });
    assert.equal(m.calls.length, 1);

    const m2 = sequence(503);
    const api2 = client({
      fetch: m2.fetch,
      middleware: [retry({ ...noDelay, methods: ['POST'] })],
    });
    const r = await api2.request<AnyResult>(postDef, { body: { name: 'a' } });
    assert.equal(r.kind, 'success');
    assert.equal(m2.calls.length, 2);
    assert.equal(m2.calls[1]?.init.body, '{"name":"a"}');
  });

  test('does not retry stream bodies', async () => {
    const m = sequence(503);
    const api = client({ fetch: m.fetch, middleware: [retry(noDelay)] });
    const body = new ReadableStream<Uint8Array>({
      start: (c) => {
        c.enqueue(new Uint8Array([1]));
        c.close();
      },
    });
    const r = await api.request<AnyResult>(putBinaryDef, { body });
    assert.ok(r.kind === 'http');
    assert.equal(m.calls.length, 1);
  });

  test('rebuilds the request on every attempt', async () => {
    const m = sequence(503);
    const addHeader: Middleware = async (req, next) => {
      req.headers.append('x-attempt', '1');
      req.url.searchParams.append('n', '1');
      return next(req);
    };
    const api = client({ fetch: m.fetch, middleware: [retry(noDelay), addHeader] });
    await api.request(getPetDef, { path: { petId: 1 } });
    assert.equal(m.calls.length, 2);
    for (const call of m.calls) {
      assert.equal(new Headers(call.init.headers).get('x-attempt'), '1');
      assert.equal(new URL(call.url).searchParams.getAll('n').length, 1);
    }
  });

  test('does not retry after a timeout', async () => {
    let n = 0;
    const api = client({
      fetch: (input, init) => {
        n++;
        return hangingFetch(input, init);
      },
      timeout: 20,
      middleware: [retry(noDelay)],
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'timeout');
    assert.equal(n, 1);
  });

  test('waits between attempts', async () => {
    const m = sequence(503);
    const delays: number[] = [];
    const api = client({
      fetch: m.fetch,
      middleware: [
        retry({
          delay: (attempt) => {
            delays.push(attempt);
            return 5;
          },
        }),
      ],
    });
    const started = Date.now();
    await api.request(getPetDef, { path: { petId: 1 } });
    assert.deepEqual(delays, [1]);
    assert.ok(Date.now() - started >= 4);
  });
});

describe('retry delays', () => {
  /** Answers with `first` (status and headers), then 200, and records the waits. */
  const setup = (status: number, headers: Record<string, string>, options = {}) => {
    let calls = 0;
    const waits: number[] = [];
    const fetch = mockFetch(() => {
      calls++;
      return calls === 1
        ? new Response('{}', {
            status,
            headers: { 'content-type': 'application/json', ...headers },
          })
        : json(200, {});
    }).fetch;
    const api = client({
      fetch,
      middleware: [
        retry({
          sleep: async (ms) => {
            waits.push(ms);
          },
          ...options,
        }),
      ],
    });
    return { api, waits };
  };

  test('Retry-After in seconds is honored', async () => {
    const { api, waits } = setup(429, { 'retry-after': '2' });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'success');
    assert.deepEqual(waits, [2000]);
  });

  test('Retry-After as an HTTP date is honored', async () => {
    const at = new Date(Date.now() + 3000).toUTCString();
    const { api, waits } = setup(503, { 'retry-after': at });
    await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(waits.length, 1);
    assert.ok((waits[0] ?? 0) > 1000 && (waits[0] ?? 0) <= 3000, String(waits[0]));
  });

  test('Retry-After is capped by maxDelay', async () => {
    const { api, waits } = setup(429, { 'retry-after': '120' }, { maxDelay: 5000 });
    await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.deepEqual(waits, [5000]);
  });

  test('without Retry-After, the backoff has full jitter', async () => {
    for (const random of [0, 0.5, 0.999]) {
      const { api, waits } = setup(503, {}, { random: () => random, baseDelay: 100 });
      await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
      assert.equal(waits.length, 1);
      assert.ok((waits[0] ?? -1) >= 0 && (waits[0] ?? 101) <= 100, String(waits[0]));
      assert.equal(waits[0], Math.floor(random * 100));
    }
  });

  test('a wait beyond the timeout stops retrying', async () => {
    let calls = 0;
    const api = client({
      fetch: mockFetch(() => {
        calls++;
        return new Response('{}', {
          status: 429,
          headers: { 'retry-after': '10', 'content-type': 'application/json' },
        });
      }).fetch,
      timeout: 1000,
      middleware: [retry({ sleep: async () => {} })],
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'http' && r.status === 429);
    assert.equal(calls, 1);
  });
});
