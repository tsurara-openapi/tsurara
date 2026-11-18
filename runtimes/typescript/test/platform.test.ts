// The client on platforms without some web globals, as in React Native: the globals are
// removed while each test runs.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef, reactNativeFile } from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

// Node's fetch implementation initializes itself on first use and needs the globals then.
await new Response('{}').json();
new Headers();
new FormData();
new Request('https://x');

const g = globalThis as Record<string, unknown>;
const saved = {
  ReadableStream: g.ReadableStream,
  DOMException: g.DOMException,
  any: AbortSignal.any,
};

/** Node's own Response needs ReadableStream; the mock server builds its responses with it. */
const respond = (status: number, body: unknown): Response => {
  g.ReadableStream = saved.ReadableStream;
  try {
    return json(status, body);
  } finally {
    delete g.ReadableStream;
  }
};

beforeEach(() => {
  delete g.ReadableStream;
  delete g.DOMException;
  (AbortSignal as { any?: unknown }).any = undefined;
});

afterEach(() => {
  g.ReadableStream = saved.ReadableStream;
  g.DOMException = saved.DOMException;
  AbortSignal.any = saved.any;
});

const postDef = {
  v: 1,
  id: 'addPet',
  method: 'POST',
  path: '/pet',
  body: { required: true, contents: { 'application/json': {} } },
  responses: { 200: { parseAs: 'json' } },
} as const satisfies OperationDef;

describe('without ReadableStream, AbortSignal.any and DOMException', () => {
  test('JSON requests succeed', async () => {
    const m = mockFetch(() => respond(200, { id: 1 }));
    const api = createClient({ baseUrl: 'https://x/v1?key=k', fetch: m.fetch });
    const r = await api.request<AnyResult>(postDef, { body: { name: 'a' } });
    assert.equal(r.kind, 'success', 'cause' in r ? String(r.cause) : '');
    const get = await api.request<AnyResult>(getPetDef, {
      path: { petId: 1 },
      query: { expand: ['a', 'b'] },
    });
    assert.equal(get.kind, 'success');
    assert.equal(m.calls[1]?.url, 'https://x/v1/pet/1?key=k&expand=a,b');
  });

  test('a timeout returns kind timeout', async () => {
    const api = createClient({ baseUrl: 'https://x', fetch: hangingFetch, timeout: 10 });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'timeout');
  });

  test('an abort with a signal returns kind aborted and never rejects', async () => {
    const api = createClient({ baseUrl: 'https://x', fetch: hangingFetch, timeout: 1000 });
    const controller = new AbortController();
    const pending = api.request<AnyResult>(getPetDef, {
      path: { petId: 1 },
      signal: controller.signal,
    });
    controller.abort();
    const r = await pending;
    assert.equal(r.kind, 'aborted');
  });

  test('React Native file objects are uploaded as file parts', async () => {
    const m = mockFetch(() => respond(200, {}));
    const uploadDef = {
      v: 1,
      id: 'upload',
      method: 'POST',
      path: '/upload',
      body: { required: true, contents: { 'multipart/form-data': {} } },
      responses: { 200: { parseAs: 'json' } },
    } as const satisfies OperationDef;
    const file = reactNativeFile({ uri: 'file:///a.png', name: 'a.png', type: 'image/png' });
    const original = FormData.prototype.append as (name: string, value: string) => void;
    const seen: unknown[] = [];
    FormData.prototype.append = function (this: FormData, name: string, value: unknown): void {
      seen.push(value);
      // Node's FormData only takes strings and Blobs; record what a React Native one would get.
      original.call(this, name, typeof value === 'object' ? 'file' : String(value));
    } as FormData['append'];
    try {
      const api = createClient({ baseUrl: 'https://x', fetch: m.fetch });
      const r = await api.request<AnyResult>(uploadDef, { body: { file, note: 'n' } });
      assert.equal(r.kind, 'success', 'cause' in r ? String(r.cause) : '');
    } finally {
      FormData.prototype.append = original as FormData['append'];
    }
    assert.deepEqual(seen[0], { uri: 'file:///a.png', name: 'a.png', type: 'image/png' });
    assert.equal(seen[1], 'n');
  });
});

test('a stream response without a body is a parse failure', async () => {
  const streamDef = {
    v: 1,
    id: 'events',
    method: 'GET',
    path: '/events',
    responses: { 200: { parseAs: 'stream' } },
  } as const satisfies OperationDef;
  const noBody = { status: 200, headers: new Headers(), body: null } as unknown as Response;
  const api = createClient({ baseUrl: 'https://x', fetch: async () => noBody });
  const r = await api.request<AnyResult>(streamDef);
  assert.equal(r.kind, 'parse');
  assert.match(String('cause' in r ? r.cause : ''), /stream/);
});
