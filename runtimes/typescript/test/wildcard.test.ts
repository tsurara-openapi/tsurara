import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';
import { mockFetch } from './helpers.ts';

const download = (parseAs: 'blob' | 'auto') =>
  ({
    v: 1,
    id: 'download',
    method: 'GET',
    path: '/file',
    responses: { 200: { parseAs } },
  }) as const satisfies OperationDef;

const upload = (mediaType: string) =>
  ({
    v: 1,
    id: 'upload',
    method: 'POST',
    path: '/file',
    body: { required: true, contents: { [mediaType]: {} } },
    responses: { 204: { parseAs: 'none' } },
  }) as const satisfies OperationDef;

const respond = (body: BodyInit, contentType: string) =>
  mockFetch(() => new Response(body, { status: 200, headers: { 'content-type': contentType } }));

describe('wildcard media types', () => {
  test('a wildcard response with a binary schema receives a Blob', async () => {
    const m = respond(new Uint8Array([37, 80, 68, 70]), 'application/pdf');
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    const r = await api.request<AnyResult>(download('blob'), {});
    assert.equal(r.kind, 'success');
    assert.ok(r.ok && r.data instanceof Blob);
  });

  test('auto parses by the actual Content-Type', async () => {
    const cases: [BodyInit, string, (v: unknown) => boolean][] = [
      ['{"a":1}', 'application/json', (v) => JSON.stringify(v) === '{"a":1}'],
      ['{"a":1}', 'application/problem+json', (v) => JSON.stringify(v) === '{"a":1}'],
      ['hello', 'text/plain; charset=utf-8', (v) => v === 'hello'],
      [new Uint8Array([1, 2]), 'application/pdf', (v) => v instanceof Blob],
    ];
    for (const [body, type, check] of cases) {
      const m = respond(body, type);
      const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
      const r = await api.request<AnyResult>(download('auto'), {});
      assert.ok(r.ok, `${type}: ${r.kind}`);
      assert.ok(check(r.data), type);
    }
  });

  test('auto reports invalid JSON as a parse failure', async () => {
    const m = respond('{', 'application/json');
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    const r = await api.request<AnyResult>(download('auto'), {});
    assert.equal(r.kind, 'parse');
  });

  test('a Blob sent to a wildcard body is sent as is with its type', async () => {
    for (const mediaType of ['*/*', 'application/*']) {
      const m = mockFetch(() => new Response(null, { status: 204 }));
      const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
      const blob = new Blob(['%PDF'], { type: 'application/pdf' });
      const r = await api.request<AnyResult>(upload(mediaType), { body: blob });
      assert.equal(r.kind, 'success');
      const call = m.calls[0];
      assert.ok(call);
      assert.equal(call.init.body, blob);
      assert.equal(new Headers(call.init.headers).get('content-type'), 'application/pdf');
    }
  });

  test('a value that is not binary sent to a wildcard body is sent as JSON', async () => {
    const m = mockFetch(() => new Response(null, { status: 204 }));
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    await api.request<AnyResult>(upload('application/*'), { body: { a: 1 } });
    const call = m.calls[0];
    assert.ok(call);
    assert.equal(call.init.body, '{"a":1}');
    assert.equal(new Headers(call.init.headers).get('content-type'), 'application/json');
  });

  test('binary values passed as a JSON body are unexpected, not silently sent as {}', async () => {
    const values: unknown[] = [
      new Blob(['x']),
      new ArrayBuffer(2),
      new Uint8Array(2),
      new ReadableStream(),
      new FormData(),
    ];
    for (const body of values) {
      const m = mockFetch(() => new Response(null, { status: 204 }));
      const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
      const r = await api.request<AnyResult>(upload('application/json'), { body });
      assert.equal(r.kind, 'unexpected', String(body));
      assert.equal(m.calls.length, 0);
    }
  });
});

describe('several body media types', () => {
  test('no body is sent when body is omitted, even with contentType', async () => {
    const m = mockFetch(() => new Response(null, { status: 204 }));
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    const def = {
      v: 1,
      id: 'patchItem',
      method: 'PATCH',
      path: '/item',
      body: { contents: { 'application/json': {}, 'text/plain': {} } },
      responses: { 204: { parseAs: 'none' } },
    } as const satisfies OperationDef;
    const r = await api.request<AnyResult>(def, { contentType: 'text/plain' });
    assert.equal(r.kind, 'success');
    const call = m.calls[0];
    assert.ok(call);
    assert.equal(call.init.body, null);
    assert.equal(new Headers(call.init.headers).get('content-type'), null);
  });
});
