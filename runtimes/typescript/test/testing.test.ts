// The mock transport of `@tsurara/client/testing`: routing by operation, handlers,
// recorded calls and unmatched requests, and the routing vectors of spec/conformance.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';
import { createRouter, mockFetch } from '../src/testing.ts';

type Vector = {
  name: string;
  basePath: string;
  routes: { method: string; path: string }[];
  request: { method: string; path: string };
  expect: { route: number | null; params?: Record<string, string> };
};

const vectors = (
  JSON.parse(
    readFileSync(new URL('../../../spec/conformance/mock-routing.json', import.meta.url), 'utf8'),
  ) as { vectors: Vector[] }
).vectors;

for (const v of vectors) {
  test(`routing: ${v.name}`, () => {
    const router = createRouter(
      v.routes.map((r) => ({ method: r.method, path: r.path, handler: () => ({ status: 204 }) })),
      { basePath: v.basePath },
    );
    const m = router.match(v.request.method, v.request.path);
    if (v.expect.route === null) {
      assert.equal(m, undefined);
    } else {
      assert.equal(m?.index, v.expect.route);
      assert.deepEqual(m?.params, v.expect.params);
    }
  });
}

const getPet = {
  v: 1,
  id: 'getPet',
  method: 'GET',
  path: '/pets/{petId}',
  params: { path: { petId: {} } },
  responses: { 200: { parseAs: 'json' }, 404: { parseAs: 'json' } },
} as const satisfies OperationDef;

const addPet = {
  v: 1,
  id: 'addPet',
  method: 'POST',
  path: '/pets',
  body: { required: true, contents: { 'application/json': {} } },
  responses: { 201: { parseAs: 'json' } },
} as const satisfies OperationDef;

test('a call through the mock gets the handler response as a typed Result', async () => {
  const mock = mockFetch(
    [
      {
        operation: getPet,
        handler: ({ params }) =>
          params.petId === '1'
            ? { status: 200, json: { id: 1, name: 'Pochi' } }
            : { status: 404, json: { message: 'no pet' } },
      },
      {
        operation: addPet,
        handler: ({ body }) => ({
          status: 201,
          json: { id: 2, ...(JSON.parse(body ?? '{}') as object) },
        }),
      },
    ],
    { basePath: '/v1' },
  );
  const api = createClient({ baseUrl: 'https://api.test/v1', fetch: mock.fetch });
  const ok = await api.request<AnyResult>(getPet, { path: { petId: 1 } });
  assert.equal(ok.kind, 'success');
  assert.deepEqual(ok.ok && ok.data, { id: 1, name: 'Pochi' });
  const missing = await api.request<AnyResult>(getPet, { path: { petId: 9 } });
  assert.equal(missing.kind, 'http');
  assert.equal(missing.kind === 'http' && missing.documented, true);
  assert.deepEqual(missing.kind === 'http' && missing.error, { message: 'no pet' });
  const added = await api.request<AnyResult>(addPet, { body: { name: 'Tama' } });
  assert.deepEqual(added.ok && added.data, { id: 2, name: 'Tama' });
  assert.equal(mock.calls.length, 3);
  assert.equal(mock.calls[0]?.route?.id, 'getPet');
  assert.deepEqual(mock.calls[0]?.params, { petId: '1' });
  assert.equal(mock.calls[2]?.method, 'POST');
});

test('an unmatched request is a transport failure that names it', async () => {
  const mock = mockFetch([{ operation: getPet, handler: () => ({ status: 200, json: {} }) }]);
  const api = createClient({ baseUrl: 'https://api.test', fetch: mock.fetch });
  const r = await api.request<AnyResult>(addPet, { body: {} });
  assert.equal(r.kind, 'network');
  assert.match(String(r.kind === 'network' && r.cause), /POST \/pets/);
});

test('unmatched requests can be recorded and answered with 501', async () => {
  const mock = mockFetch([], { unmatched: 'record' });
  const api = createClient({ baseUrl: 'https://api.test', fetch: mock.fetch });
  const r = await api.request<AnyResult>(getPet, { path: { petId: 1 } });
  assert.equal(r.kind === 'http' && r.status, 501);
  assert.equal(mock.unmatched.length, 1);
  assert.equal(mock.unmatched[0]?.method, 'GET');
});

test('a handler that throws is a transport failure', async () => {
  const mock = mockFetch([
    {
      method: 'GET',
      path: '/pets/{petId}',
      handler: () => {
        throw new Error('connection reset');
      },
    },
  ]);
  const api = createClient({ baseUrl: 'https://api.test', fetch: mock.fetch });
  const r = await api.request<AnyResult>(getPet, { path: { petId: 1 } });
  assert.equal(r.kind, 'network');
});

test('text, bytes and headers in responses', async () => {
  const mock = mockFetch([
    {
      operation: getPet,
      handler: () => ({
        status: 200,
        headers: { 'x-trace': 't' },
        text: '{"id":1}',
        contentType: 'application/json',
      }),
    },
  ]);
  const api = createClient({ baseUrl: 'https://api.test', fetch: mock.fetch });
  const r = await api.request<AnyResult>(getPet, { path: { petId: 1 } });
  assert.deepEqual(r.ok && r.data, { id: 1 });
  assert.equal(r.ok && r.response.headers.get('x-trace'), 't');
});
