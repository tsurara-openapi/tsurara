import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';
import { getPetDef, json, mockFetch } from './helpers.ts';

/** `名前,値\n` in Shift_JIS. */
const SHIFT_JIS = new Uint8Array([150, 188, 145, 79, 44, 146, 108, 10]);

const csvDef = {
  v: 1,
  id: 'exportCsv',
  method: 'GET',
  path: '/export',
  responses: { 200: { parseAs: 'text' }, 404: { parseAs: 'text' } },
} as const satisfies OperationDef;

const answer = (body: BodyInit | null, init: ResponseInit) =>
  mockFetch(() => new Response(body, init)).fetch;

describe('text bodies', () => {
  test('are decoded with the charset of the Content-Type', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: answer(SHIFT_JIS, { headers: { 'content-type': 'text/csv; charset=Shift_JIS' } }),
    });
    const r = await api.request<AnyResult>(csvDef);
    assert.ok(r.ok);
    assert.equal(r.data, '名前,値\n');
  });

  test('of undocumented statuses are decoded with their charset too', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: answer(SHIFT_JIS, {
        status: 500,
        headers: { 'content-type': 'text/plain; charset="shift_jis"' },
      }),
    });
    const r = await api.request<AnyResult>(csvDef);
    assert.ok(r.kind === 'http');
    assert.equal(r.error, '名前,値\n');
  });

  test('are UTF-8 without a charset', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: answer('名前', { headers: { 'content-type': 'text/csv' } }),
    });
    const r = await api.request<AnyResult>(csvDef);
    assert.ok(r.ok);
    assert.equal(r.data, '名前');
  });

  test('with a charset the platform cannot decode are a parse failure', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: answer('x', { headers: { 'content-type': 'text/csv; charset=x-no-such-charset' } }),
    });
    const r = await api.request<AnyResult>(csvDef);
    assert.ok(r.kind === 'parse');
    assert.match(String((r.cause as Error).message), /x-no-such-charset/);
  });
});

describe('requestRaw', () => {
  test('returns the response with its body unread, after middleware and auth', async () => {
    const m = mockFetch(
      () => new Response(SHIFT_JIS, { headers: { 'content-type': 'text/csv; charset=Shift_JIS' } }),
    );
    const seen: string[] = [];
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: m.fetch,
      middleware: [
        async (req, next) => {
          seen.push(req.operation.id);
          return next(req);
        },
      ],
    });
    const r = await api.requestRaw(csvDef, { headers: { 'x-a': '1' } });
    assert.ok(r.kind === 'response');
    assert.equal(r.ok, true);
    assert.equal(r.status, 200);
    assert.equal(r.response.bodyUsed, false);
    assert.deepEqual(new Uint8Array(await r.response.arrayBuffer()), SHIFT_JIS);
    assert.deepEqual(seen, ['exportCsv']);
    assert.equal(new Headers(m.calls[0]?.init.headers).get('x-a'), '1');
  });

  test('tells documented and undocumented error statuses apart', async () => {
    const api = createClient({ baseUrl: 'https://x.test', fetch: answer('no', { status: 404 }) });
    const documented = await api.requestRaw(csvDef);
    assert.ok(documented.kind === 'response');
    assert.equal(documented.ok, false);
    assert.equal(documented.documented, true);
    const other = createClient({ baseUrl: 'https://x.test', fetch: answer('no', { status: 418 }) });
    const undocumented = await other.requestRaw(csvDef);
    assert.ok(undocumented.kind === 'response');
    assert.equal(undocumented.ok, false);
    assert.equal(undocumented.documented, false);
  });

  test('returns transport failures as results', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: async () => {
        throw new TypeError('fetch failed');
      },
    });
    const r = await api.requestRaw(csvDef);
    assert.equal(r.kind, 'network');
  });
});

describe('a body that fails while it is read', () => {
  /** A response whose body sends a chunk and then fails. */
  const broken = () =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode('{"id":'));
          controller.error(new TypeError('terminated'));
        },
      }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    );

  test('is a network failure that keeps the response', async () => {
    const api = createClient({ baseUrl: 'https://x.test', fetch: mockFetch(broken).fetch });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'network');
    assert.equal(r.response?.status, 200);
  });

  test('is a timeout that keeps the response when the timeout ends it', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      timeout: 20,
      fetch: async (_input, init) =>
        new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode('{"id":'));
              init?.signal?.addEventListener('abort', () => controller.error(init.signal?.reason));
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'timeout');
    assert.equal(r.response?.status, 200);
  });

  test('failures before a response have none', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: async () => {
        throw new TypeError('fetch failed');
      },
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'network');
    assert.equal(r.response, undefined);
  });
});

describe('fetchOptions', () => {
  test('are passed to fetch, per call over the client', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: m.fetch,
      fetchOptions: { cache: 'no-store', credentials: 'include' },
    });
    await api.request(getPetDef, { path: { petId: 1 } });
    await api.request(getPetDef, {
      path: { petId: 1 },
      fetchOptions: { cache: 'force-cache', keepalive: true },
    });
    const [first, second] = m.calls;
    assert.equal(first?.init.cache, 'no-store');
    assert.equal(first?.init.credentials, 'include');
    assert.equal(second?.init.cache, 'force-cache');
    assert.equal(second?.init.credentials, 'include');
    assert.equal(second?.init.keepalive, true);
  });

  test('do not replace what the client builds', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    await api.request(getPetDef, {
      path: { petId: 1 },
      fetchOptions: { method: 'DELETE', body: 'x' } as RequestInit,
    });
    assert.equal(m.calls[0]?.init.method, 'GET');
    assert.equal(m.calls[0]?.init.body, null);
  });

  test('reach middleware, which can change them', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: m.fetch,
      middleware: [
        async (req, next) => {
          assert.equal(req.fetchOptions.cache, 'no-store');
          req.fetchOptions = { ...req.fetchOptions, priority: 'high' };
          return next(req);
        },
      ],
    });
    await api.request(getPetDef, { path: { petId: 1 }, fetchOptions: { cache: 'no-store' } });
    assert.equal(m.calls[0]?.init.priority, 'high');
  });
});
