// End to end: the generated functions of the shared package against a server on localhost.
import assert from 'node:assert/strict';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { after, before, describe, test } from 'node:test';
import {
  ApiError,
  createPet,
  getPet,
  isHttpError,
  listPets,
  type Pet,
  unwrap,
} from '@acme/openapi';
import { getPet as getPetFromSubpath } from '@acme/openapi/operations/getPet';
import { createApi } from '../src/api.ts';

const pets: Pet[] = [{ id: 1, name: 'Rex', tag: 'dog' }];
const seen: { method: string; url: string; requestId: string | undefined }[] = [];
let flaky = 0;

const handler = async (req: IncomingMessage, res: ServerResponse) => {
  seen.push({
    method: req.method ?? '',
    url: req.url ?? '',
    requestId: req.headers['x-request-id'] as string,
  });
  const send = (status: number, body?: unknown) => {
    res.writeHead(status, body === undefined ? {} : { 'content-type': 'application/json' });
    res.end(body === undefined ? undefined : JSON.stringify(body));
  };
  const url = new URL(req.url ?? '/', 'http://localhost');
  if (url.pathname === '/slow') return;
  if (req.method === 'GET' && url.pathname === '/pets') {
    if (url.searchParams.get('limit') === '503' && flaky++ === 0) return send(503);
    return send(200, pets.slice(0, Number(url.searchParams.get('limit') ?? pets.length)));
  }
  const m = /^\/pets\/(\d+)$/.exec(url.pathname);
  if (req.method === 'GET' && m) {
    const pet = pets.find((p) => p.id === Number(m[1]));
    return pet ? send(200, pet) : send(404, { title: 'Not Found' });
  }
  if (req.method === 'POST' && url.pathname === '/pets') {
    let text = '';
    for await (const chunk of req) text += chunk;
    const body = JSON.parse(text) as { name?: string };
    if (!body.name) return send(422, { title: 'Invalid', detail: 'name is required' });
    const pet = { id: pets.length + 1, name: body.name };
    pets.push(pet);
    return send(201, pet);
  }
  send(500, { title: 'unexpected' });
};

const server = createServer((req, res) => void handler(req, res));
let base = '';

before(async () => {
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
after(() => {
  server.closeAllConnections();
  server.close();
});

describe('generated functions against a server', () => {
  test('success, with the context in the headers', async () => {
    const api = createApi(base);
    const r = await listPets(api, { query: { limit: 1 }, context: { requestId: 'r1' } });
    assert.ok(r.ok);
    assert.equal(r.data[0]?.name, 'Rex');
    assert.equal(seen.at(-1)?.requestId, 'r1');
  });

  test('documented errors are typed, and unwrap throws ApiError', async () => {
    const api = createApi(base);
    const missing = await getPet(api, { path: { petId: 99 } });
    assert.ok(isHttpError(missing, 404));
    if (isHttpError(missing, 404)) assert.equal(missing.error.title, 'Not Found');
    const invalid = await createPet(api, { body: { name: '' } });
    assert.ok(isHttpError(invalid, 422));
    if (isHttpError(invalid, 422)) assert.equal(invalid.error.detail, 'name is required');
    await assert.rejects(async () => unwrap(await getPet(api, { path: { petId: 99 } })), ApiError);
    const created = unwrap(await createPet(api, { body: { name: 'Tom' } }));
    assert.equal(created.name, 'Tom');
  });

  test('subpath imports are the same functions', async () => {
    assert.equal(getPetFromSubpath, getPet);
  });

  test('retries a 503 once', async () => {
    const api = createApi(base);
    const r = await listPets(api, { query: { limit: 503 } });
    assert.ok(r.ok);
  });

  test('timeouts and aborts are results', async () => {
    const slow = { ...createApi(base, 50) };
    const timeout = await slow.request({
      v: 1,
      id: 'slow',
      method: 'GET',
      path: '/slow',
      responses: { 200: { parseAs: 'json' } },
    });
    assert.equal((timeout as { kind: string }).kind, 'timeout');
    const controller = new AbortController();
    const pending = getPet(createApi(`${base}/slow-prefix`, 5000), {
      path: { petId: 1 },
      signal: controller.signal,
    });
    controller.abort();
    assert.equal((await pending).kind, 'aborted');
  });
});
