import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  type AnyResult,
  createClient,
  type Middleware,
  type OperationDef,
  type RequestInfo,
  type UndocumentedStatusInfo,
} from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

describe('middleware', () => {
  test('headers added by middleware end up in the request', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const seen: unknown[] = [];
    const tracing: Middleware = async (req, next) => {
      seen.push(req.operation);
      req.headers.set('traceparent', `00-${req.operation.id}`);
      return next(req);
    };
    const api = createClient({ baseUrl: 'https://x', fetch: m.fetch, middleware: [tracing] });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'success');
    assert.equal(new Headers(m.calls[0]?.init.headers).get('traceparent'), '00-getPetById');
    assert.deepEqual(seen, [
      { id: 'getPetById', method: 'GET', path: '/pet/{petId}', tags: ['pet'] },
    ]);
  });

  test('middleware runs from the outermost first', async () => {
    const m = mockFetch(() => json(200, {}));
    const order: string[] = [];
    const named =
      (name: string): Middleware =>
      async (req, next) => {
        order.push(`${name}:in`);
        const res = await next(req);
        order.push(`${name}:out`);
        return res;
      };
    const api = createClient({
      baseUrl: 'https://x',
      fetch: m.fetch,
      middleware: [named('a'), named('b')],
    });
    await api.request(getPetDef, { path: { petId: 1 } });
    assert.deepEqual(order, ['a:in', 'b:in', 'b:out', 'a:out']);
  });

  test('middleware can replace the response', async () => {
    const m = mockFetch(() => json(500, {}));
    const fallback: Middleware = async () => json(200, { id: 2 });
    const api = createClient({ baseUrl: 'https://x', fetch: m.fetch, middleware: [fallback] });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.ok(r.kind === 'success');
    assert.deepEqual(r.data, { id: 2 });
    assert.equal(m.calls.length, 0);
  });

  test('exceptions thrown by middleware become kind unexpected', async () => {
    const boom = new Error('boom');
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, {})).fetch,
      middleware: [
        async () => {
          throw boom;
        },
      ],
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'unexpected');
    assert.ok(!r.ok && 'cause' in r && r.cause === boom);
  });
});

describe('onResult', () => {
  test('receives timeouts with the operation and context', async () => {
    const got: [AnyResult, RequestInfo<{ trace: string }>][] = [];
    const api = createClient<{ trace: string }>({
      baseUrl: 'https://x',
      fetch: hangingFetch,
      timeout: 10,
      onResult: (result, info) => {
        got.push([result, info]);
      },
    });
    const r = await api.request<AnyResult>(getPetDef, {
      path: { petId: 1 },
      context: { trace: 't1' },
    });
    assert.equal(r.kind, 'timeout');
    assert.equal(got.length, 1);
    assert.equal(got[0]?.[0], r);
    assert.equal(got[0]?.[1].operation.id, 'getPetById');
    assert.deepEqual(got[0]?.[1].context, { trace: 't1' });
  });

  test('exceptions from onResult do not change the result', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, { id: 1 })).fetch,
      onResult: () => {
        throw new Error('ignored');
      },
    });
    const r = await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(r.kind, 'success');
  });
});

describe('onUndocumentedStatus', () => {
  const def: OperationDef = {
    ...getPetDef,
    responses: { 200: { parseAs: 'json' }, 404: { parseAs: 'json' } },
  };

  test('is called for statuses the definition does not document', async () => {
    const got: UndocumentedStatusInfo<{ trace: string }>[] = [];
    let status = 201;
    const api = createClient<{ trace: string }>({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(status, { id: 1 })).fetch,
      onUndocumentedStatus: (info) => {
        got.push(info);
      },
    });
    const created = await api.request<AnyResult>(def, {
      path: { petId: 1 },
      context: { trace: 't' },
    });
    assert.equal(created.kind, 'http');
    status = 500;
    await api.request<AnyResult>(def, { path: { petId: 1 } });
    for (const documented of [200, 404]) {
      status = documented;
      await api.request<AnyResult>(def, { path: { petId: 1 } });
    }
    assert.deepEqual(
      got.map((i) => [i.operation.id, i.status]),
      [
        ['getPetById', 201],
        ['getPetById', 500],
      ],
    );
    assert.deepEqual(got[0]?.context, { trace: 't' });
    assert.ok(created.kind === 'http');
    assert.equal(got[0]?.response, created.response);
  });

  test('is not called for statuses covered by default', async () => {
    let calls = 0;
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(500, {})).fetch,
      onUndocumentedStatus: () => {
        calls++;
      },
    });
    await api.request<AnyResult>(getPetDef, { path: { petId: 1 } });
    assert.equal(calls, 0);
  });

  test('exceptions from the hook do not change the result', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(201, { id: 1 })).fetch,
      onUndocumentedStatus: () => {
        throw new Error('ignored');
      },
    });
    const r = await api.request<AnyResult>(def, { path: { petId: 1 } });
    assert.equal(r.kind, 'http');
  });
});

describe('optional options', () => {
  test('accept undefined, which means not set', async () => {
    const dev = false as boolean;
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, { id: 1 })).fetch,
      headers: undefined,
      middleware: undefined,
      onResult: dev ? () => {} : undefined,
      onUndocumentedStatus: dev ? () => {} : undefined,
      timeout: undefined,
      validate: undefined,
    });
    const r = await api.request<AnyResult>(getPetDef, {
      path: { petId: 1 },
      signal: undefined,
      headers: undefined,
      timeout: undefined,
      baseUrl: undefined,
      context: undefined,
      contentType: undefined,
    });
    assert.equal(r.kind, 'success');
  });
});
