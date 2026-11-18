import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  ApiError,
  bindClient,
  type Client,
  createClient,
  isHttpError,
  type RequestOptions,
  type TransportFailure,
  unwrap,
} from '../src/index.ts';
import { getPetDef, hangingFetch, json, mockFetch } from './helpers.ts';

/** Shaped like the generated `{P}Result` types. */
type GetPetResult =
  | { ok: true; kind: 'success'; status: 200; data: { id: number }; response: Response }
  | {
      ok: false;
      kind: 'http';
      documented: true;
      status: 404;
      error: { message: string };
      response: Response;
    }
  | {
      ok: false;
      kind: 'http';
      documented: false;
      status: number;
      error: unknown;
      response: Response;
    }
  | TransportFailure;

type GetPetData = { path: { petId: number } };

/** Shaped like generated call functions. */
function getPet<Ctx>(
  client: Client<Ctx>,
  options: GetPetData & RequestOptions<Ctx>,
): Promise<GetPetResult> {
  return client.request<GetPetResult>(getPetDef, options);
}

function listPets<Ctx>(client: Client<Ctx>, options?: RequestOptions<Ctx>): Promise<GetPetResult> {
  return client.request<GetPetResult>({ ...getPetDef, id: 'listPets', path: '/pet' }, options);
}

describe('unwrap', () => {
  test('returns data on success', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, { id: 1 })).fetch,
    });
    const data: { id: number } = unwrap(await getPet(api, { path: { petId: 1 } }));
    assert.deepEqual(data, { id: 1 });
    // @ts-expect-error data is typed from the success member
    const wrong: string = unwrap(await getPet(api, { path: { petId: 1 } }));
    assert.ok(wrong);
  });

  test('throws ApiError for HTTP errors', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(404, { message: 'no' })).fetch,
    });
    const result = await getPet(api, { path: { petId: 1 } });
    const err = catchError(() => unwrap(result));
    assert.ok(err instanceof ApiError);
    assert.ok(err instanceof Error);
    assert.equal(err.name, 'ApiError');
    assert.equal(err.kind, 'http');
    assert.equal(err.status, 404);
    assert.equal(err.operationId, 'getPetById');
    assert.equal(err.result, result);
    assert.match(err.message, /getPetById/);
    assert.match(err.message, /404/);
    assert.ok(err.stack?.includes('unwrap.test.ts'));
  });

  test('throws ApiError with the cause for transport failures', async () => {
    const api = createClient({ baseUrl: 'https://x', fetch: hangingFetch, timeout: 10 });
    const result = await getPet(api, { path: { petId: 1 } });
    const err = catchError(() => unwrap(result));
    assert.ok(err instanceof ApiError);
    assert.equal(err.kind, 'timeout');
    assert.equal(err.status, undefined);
    assert.ok(result.kind === 'timeout');
    assert.equal(err.cause, result.cause);
    assert.match(err.message, /timeout/);
  });

  test('ApiError narrows result by the operation Result type', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(404, { message: 'no' })).fetch,
    });
    const result = await getPet(api, { path: { petId: 1 } });
    const err = catchError(() => unwrap(result)) as ApiError<GetPetResult>;
    assert.ok(err instanceof ApiError);
    if (err.result.kind === 'http' && err.result.documented) {
      const status: 404 = err.result.status;
      const message: string = err.result.error.message;
      assert.equal(status, 404);
      assert.equal(message, 'no');
    } else {
      assert.fail('a documented 404');
    }
    // @ts-expect-error success is not a failure member
    const success: ApiError<GetPetResult>['result']['kind'] = 'success';
    const untyped: ApiError = err;
    // The default stays the generic failure, so error is unknown.
    if (untyped.result.kind === 'http') {
      // @ts-expect-error unknown
      untyped.result.error.message;
    }
    assert.ok(success);
  });

  test('ApiError made directly names the operation of the result', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(404, { message: 'no' })).fetch,
    });
    const result = await getPet(api, { path: { petId: 1 } });
    const err = new ApiError(result.ok ? assert.fail() : result);
    assert.equal(err.operationId, 'getPetById');
    assert.match(err.message, /getPetById failed with HTTP 404/);
  });

  test('isHttpError narrows to the documented error of a status', async () => {
    let status = 404;
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(status, { message: 'no' })).fetch,
    });
    const result = await getPet(api, { path: { petId: 1 } });
    assert.ok(isHttpError(result, 404));
    if (isHttpError(result, 404)) {
      const message: string = result.error.message;
      assert.equal(message, 'no');
    }
    // A status the definition does not document is not narrowed to it.
    status = 418;
    const other = await getPet(api, { path: { petId: 1 } });
    assert.equal(isHttpError(other, 404), false);
    // @ts-expect-error 500 is not a documented status of the operation
    isHttpError(result, 500);
  });

  test('isHttpError narrows statuses covered by default to the default member', async () => {
    /** Shaped like a generated Result with a `default` (Problem) response. */
    type DeleteResult =
      | { ok: true; kind: 'success'; status: 204; data: undefined; response: Response }
      | {
          ok: false;
          kind: 'http';
          documented: true;
          status: 404;
          error: { notFound: string };
          response: Response;
        }
      | {
          ok: false;
          kind: 'http';
          documented: true;
          status: number;
          error: { title: string };
          response: Response;
        }
      | {
          ok: false;
          kind: 'http';
          documented: false;
          status: number;
          error: unknown;
          response: Response;
        }
      | TransportFailure;
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(500, { title: 'boom' })).fetch,
    });
    const def = {
      ...getPetDef,
      responses: {
        204: { parseAs: 'none' },
        404: { parseAs: 'json' },
        default: { parseAs: 'json' },
      },
    } as const;
    const result = await api.request<DeleteResult>(def, { path: { petId: 1 } });
    assert.equal(isHttpError(result, 500), true);
    if (isHttpError(result, 500)) {
      const title: string = result.error.title;
      const status: 500 = result.status;
      assert.equal(title, 'boom');
      assert.equal(status, 500);
    }
    if (isHttpError(result, 404)) {
      // A status documented on its own keeps its own member.
      const notFound: string = result.error.notFound;
      assert.fail(notFound);
    }
  });

  test('throws a TypeError for a Promise (a missing await)', async () => {
    const api = createClient({
      baseUrl: 'https://x',
      fetch: mockFetch(() => json(200, { id: 1 })).fetch,
    });
    const pending = getPet(api, { path: { petId: 1 } });
    // @ts-expect-error a Promise is not a Result
    const err = catchError(() => unwrap(pending));
    assert.ok(err instanceof TypeError);
    assert.match(err.message, /received a Promise/);
    assert.match(err.message, /await/);
    await pending;
  });

  test('throws a TypeError for values that are not Results', () => {
    for (const value of [undefined, null, 1, { data: 1 }]) {
      // @ts-expect-error not a Result
      const err = catchError(() => unwrap(value));
      assert.ok(err instanceof TypeError, String(value));
      assert.match(err.message, /not a Result/);
    }
  });
});

describe('bindClient', () => {
  test('binds the client as the first argument', async () => {
    const m = mockFetch(() => json(200, { id: 1 }));
    const client = createClient<{ trace: string }>({ baseUrl: 'https://x', fetch: m.fetch });
    const api = bindClient(client, { getPet, listPets });
    const r = await api.getPet({ path: { petId: 7 }, context: { trace: 't' } });
    assert.equal(r.kind, 'success');
    await api.listPets();
    assert.deepEqual(
      m.calls.map((c) => c.url),
      ['https://x/pet/7', 'https://x/pet'],
    );
    // @ts-expect-error required options stay required
    await api.getPet().catch(() => {});
    // @ts-expect-error the context type comes from the client
    await api.getPet({ path: { petId: 1 }, context: { trace: 1 } });
  });
});

const catchError = (f: () => unknown): unknown => {
  try {
    f();
  } catch (e) {
    return e;
  }
  assert.fail('expected an exception');
};
