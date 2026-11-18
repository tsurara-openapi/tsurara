// Response validation through Standard Schema. The client depends on no validation
// library: these tests use Valibot and a hand-written schema.
import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import * as v from 'valibot';
import {
  type AnyResult,
  ApiError,
  createClient,
  type OperationDef,
  type ResponseSchemas,
  type StandardSchema,
  unwrap,
  validateResponses,
} from '../src/index.ts';
import { json, mockFetch } from './helpers.ts';

const def = {
  v: 1,
  id: 'getPet',
  method: 'GET',
  path: '/pet',
  responses: {
    200: { parseAs: 'json', contentTypes: { 'application/json': 'json', 'text/plain': 'text' } },
    404: { parseAs: 'json' },
  },
} as const satisfies OperationDef;

const schemas: ResponseSchemas = {
  getPet: {
    200: { 'application/json': v.object({ id: v.number(), name: v.string() }) },
    404: { 'application/json': v.object({ title: v.string() }) },
  },
};

const respond = (status: number, body: unknown) =>
  createClient({
    baseUrl: 'https://x',
    fetch: mockFetch(() => json(status, body)).fetch,
    validate: validateResponses(schemas),
  });

describe('validate', () => {
  test('a valid response is a success', async () => {
    const r = await respond(200, { id: 1, name: 'Rex' }).request<AnyResult>(def);
    assert.equal(r.kind, 'success');
  });

  test('an invalid response is a validation failure with the issues and the value', async () => {
    const r = await respond(200, { id: 'one' }).request<AnyResult>(def);
    assert.ok(r.kind === 'validation', r.kind);
    assert.equal(r.status, 200);
    assert.deepEqual(r.value, { id: 'one' });
    assert.ok(r.issues.length >= 1);
    assert.ok(
      r.issues.some((i) => i.path?.some((p) => (typeof p === 'object' ? p.key : p) === 'id')),
    );
    assert.throws(
      () => unwrap(r),
      (e: unknown) => e instanceof ApiError && e.kind === 'validation',
    );
  });

  test('documented error bodies are validated too', async () => {
    const ok = await respond(404, { title: 'Not Found' }).request<AnyResult>(def);
    assert.equal(ok.kind, 'http');
    const bad = await respond(404, { message: 'x' }).request<AnyResult>(def);
    assert.equal(bad.kind, 'validation');
  });

  test('a call can turn validation off', async () => {
    const r = await respond(200, { id: 'one' }).request<AnyResult>(def, { validate: false });
    assert.equal(r.kind, 'success');
  });

  test('without validate, responses are not checked', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, { id: 'one' })).fetch,
    });
    assert.equal((await api.request<AnyResult>(def)).kind, 'success');
  });

  test('bodies that are not JSON are not validated', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => new Response('hi', { headers: { 'content-type': 'text/plain' } }))
        .fetch,
      validate: validateResponses(schemas),
    });
    assert.equal((await api.request<AnyResult>(def)).kind, 'success');
  });

  test('a hand-written Standard Schema works, also asynchronously', async () => {
    const positive: StandardSchema = {
      '~standard': {
        version: 1,
        vendor: 'test',
        validate: async (value) =>
          typeof value === 'number' && value > 0
            ? { value }
            : { issues: [{ message: 'not positive' }] },
      },
    };
    const api = (body: unknown) =>
      createClient({
        baseUrl: 'https://x',
        fetch: mockFetch(() => json(200, body)).fetch,
        validate: validateResponses({ getPet: { 200: { 'application/json': positive } } }),
      });
    assert.equal((await api(3).request<AnyResult>(def)).kind, 'success');
    const r = await api(-1).request<AnyResult>(def);
    assert.ok(r.kind === 'validation');
    assert.equal(r.issues[0]?.message, 'not positive');
  });
});
