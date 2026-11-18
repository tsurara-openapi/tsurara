import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

describe('request', () => {
  test('builds the URL, query, headers and cookies', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const api = createClient({
      baseUrl: 'https://api.example.com/v1/',
      fetch: m.fetch,
      headers: async () => ({ Authorization: 'Bearer t' }),
    });
    const def = {
      ...getPetDef,
      params: {
        ...getPetDef.params,
        headers: { 'X-Ids': {} },
        cookies: { session: {} },
      },
    } as const satisfies OperationDef;
    const r = await api.request<AnyResult>(def, {
      path: { petId: 7 },
      query: { expand: ['a', 'b'] },
      headers: { 'X-Ids': [1, 2], 'X-Skip': undefined },
      cookies: { session: 'abc' },
    });
    assert.equal(r.kind, 'success');
    const call = m.calls[0];
    assert.ok(call);
    assert.equal(call.url, 'https://api.example.com/v1/pet/7?expand=a,b');
    const headers = new Headers(call.init.headers);
    assert.equal(headers.get('authorization'), 'Bearer t');
    assert.equal(headers.get('x-ids'), '1,2');
    assert.equal(headers.get('x-skip'), null);
    assert.equal(headers.get('cookie'), 'session=abc');
    assert.equal(call.init.method, 'GET');
  });

  test('sends a JSON body', async () => {
    const m = mockFetch(() => new Response(null, { status: 204 }));
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    const def = {
      v: 1,
      id: 'addPet',
      method: 'POST',
      path: '/pet',
      body: { required: true, contents: { 'application/json': {} } },
      responses: { 204: { parseAs: 'none' } },
    } as const satisfies OperationDef;
    const r = await api.request<AnyResult>(def, { body: { name: 'x' } });
    assert.deepEqual(
      { kind: r.kind, data: r.ok ? r.data : null },
      { kind: 'success', data: undefined },
    );
    const call = m.calls[0];
    assert.ok(call);
    assert.equal(call.init.body, '{"name":"x"}');
    assert.equal(new Headers(call.init.headers).get('content-type'), 'application/json');
  });
});

describe('result kinds', () => {
  const run = async (
    fetch: (input: URL | Request | string, init?: RequestInit) => Promise<Response>,
    extra: { signal?: AbortSignal; timeout?: number } = {},
    def: OperationDef = getPetDef,
  ) => {
    const api = createClient({ baseUrl: 'https://x.test', fetch });
    return api.request<AnyResult>(def, { path: { petId: 1 }, ...extra });
  };

  test('success', async () => {
    const r = await run(mockFetch(() => json(200, { id: 1 })).fetch);
    assert.ok(r.ok && r.kind === 'success');
    assert.deepEqual(r.data, { id: 1 });
    assert.equal(r.status, 200);
  });

  test('documented http error', async () => {
    const r = await run(mockFetch(() => json(404, { message: 'nope' })).fetch);
    assert.ok(r.kind === 'http');
    assert.equal(r.documented, true);
    assert.equal(r.status, 404);
    assert.deepEqual(r.error, { message: 'nope' });
  });

  test('default covers other error statuses', async () => {
    const r = await run(mockFetch(() => json(500, { message: 'x' })).fetch);
    assert.ok(r.kind === 'http');
    assert.equal(r.documented, true);
    assert.equal(r.status, 500);
  });

  test('undocumented status', async () => {
    const def = { ...getPetDef, responses: { 200: { parseAs: 'json' } } } as const;
    const r = await run(mockFetch(() => new Response('oops', { status: 503 })).fetch, {}, def);
    assert.ok(r.kind === 'http');
    assert.equal(r.documented, false);
    assert.equal(r.error, 'oops');
  });

  test('undocumented success status when other successes are documented', async () => {
    const r = await run(mockFetch(() => json(201, {})).fetch);
    assert.ok(r.kind === 'http');
    assert.equal(r.documented, false);
    assert.equal(r.status, 201);
  });

  test('default is a success when no success status is documented', async () => {
    const def = {
      ...getPetDef,
      responses: { default: { parseAs: 'json' } },
    } as const satisfies OperationDef;
    const r = await run(mockFetch(() => json(201, { a: 1 })).fetch, {}, def);
    assert.ok(r.kind === 'success');
    assert.deepEqual(r.data, { a: 1 });
  });

  test('2XX ranges', async () => {
    const def = {
      ...getPetDef,
      responses: { '2XX': { parseAs: 'text' } },
    } as const satisfies OperationDef;
    const r = await run(
      mockFetch(
        () => new Response('ok', { status: 202, headers: { 'content-type': 'text/plain' } }),
      ).fetch,
      {},
      def,
    );
    assert.ok(r.kind === 'success');
    assert.equal(r.data, 'ok');
  });

  test('network', async () => {
    const r = await run(async () => {
      throw new TypeError('fetch failed');
    });
    assert.equal(r.kind, 'network');
    assert.ok(!r.ok && 'cause' in r && r.cause instanceof TypeError);
  });

  test('timeout', async () => {
    const r = await run(hangingFetch, { timeout: 20 });
    assert.equal(r.kind, 'timeout');
  });

  test('aborted', async () => {
    const controller = new AbortController();
    setTimeout(() => controller.abort(), 10);
    const r = await run(hangingFetch, { signal: controller.signal, timeout: 5_000 });
    assert.equal(r.kind, 'aborted');
  });

  test('already aborted', async () => {
    const r = await run(hangingFetch, { signal: AbortSignal.abort() });
    assert.equal(r.kind, 'aborted');
  });

  test('parse: invalid JSON', async () => {
    const r = await run(
      mockFetch(
        () => new Response('{', { status: 200, headers: { 'content-type': 'application/json' } }),
      ).fetch,
    );
    assert.ok(r.kind === 'parse');
    assert.equal(r.contentType, 'application/json');
  });

  test('parse: unexpected Content-Type', async () => {
    const r = await run(
      mockFetch(
        () => new Response('<html>', { status: 200, headers: { 'content-type': 'text/html' } }),
      ).fetch,
    );
    assert.equal(r.kind, 'parse');
  });

  test('unexpected: a throwing headers function', async () => {
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: mockFetch(() => json(200, {})).fetch,
      headers: () => {
        throw new Error('boom');
      },
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'unexpected');
  });

  test('unexpected: an unsupported OperationDef version', async () => {
    const def = { ...getPetDef, v: 2 } as unknown as OperationDef;
    const r = await run(mockFetch(() => json(200, {})).fetch, {}, def);
    assert.equal(r.kind, 'unexpected');
  });

  test('blob, text and stream', async () => {
    const def = {
      ...getPetDef,
      responses: {
        200: { parseAs: 'blob' },
        201: { parseAs: 'text' },
        202: { parseAs: 'stream' },
      },
    } as const satisfies OperationDef;
    const blob = await run(mockFetch(() => new Response('abc', { status: 200 })).fetch, {}, def);
    assert.ok(blob.ok && blob.data instanceof Blob);
    const text = await run(mockFetch(() => new Response('abc', { status: 201 })).fetch, {}, def);
    assert.ok(text.ok && text.data === 'abc');
    const stream = await run(mockFetch(() => new Response('abc', { status: 202 })).fetch, {}, def);
    assert.ok(stream.ok && stream.data instanceof ReadableStream);
  });

  test('contentTypes pick the parser by the response Content-Type', async () => {
    const def = {
      ...getPetDef,
      responses: {
        200: { parseAs: 'json', contentTypes: { 'application/json': 'json', 'text/csv': 'text' } },
      },
    } as const satisfies OperationDef;
    const r = await run(
      mockFetch(() => new Response('a,b', { status: 200, headers: { 'content-type': 'text/csv' } }))
        .fetch,
      {},
      def,
    );
    assert.ok(r.ok && r.data === 'a,b');
  });

  test('the promise never rejects', async () => {
    const throwing = [
      async () => {
        throw new Error('x');
      },
      async () => {
        throw 'string';
      },
      async () => null as unknown as Response,
    ];
    for (const f of throwing) {
      const r = await run(f);
      assert.equal(r.ok, false);
    }
  });
});
