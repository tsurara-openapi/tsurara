import assert from 'node:assert/strict';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { after, before, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';

const listen = async (
  handler: (req: IncomingMessage, res: ServerResponse) => void,
): Promise<{ server: Server; origin: string }> => {
  const server = createServer(handler);
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { server, origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}` };
};

const def = {
  v: 1,
  id: 'getPet',
  method: 'GET',
  path: '/pets/{id}',
  params: { path: { id: {} } },
  responses: { 200: { parseAs: 'json' } },
} as const satisfies OperationDef;

const otherHeaders: (string | undefined)[] = [];
let api: ReturnType<typeof createClient>;
let servers: Server[] = [];

before(async () => {
  const other = await listen((req, res) => {
    otherHeaders.push(req.headers['x-api-key'] as string | undefined);
    res.writeHead(200, { 'content-type': 'application/json' }).end('{"id":9}');
  });
  const main = await listen((req, res) => {
    if (req.url === '/pets/1') res.writeHead(302, { location: '/pets/2' }).end();
    else if (req.url === '/pets/2')
      res
        .writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ key: req.headers['x-api-key'] }));
    else res.writeHead(307, { location: `${other.origin}/pets/9` }).end();
  });
  servers = [other.server, main.server];
  api = createClient({ baseUrl: main.origin, headers: { 'X-Api-Key': 'secret' } });
});

after(() => {
  for (const s of servers) s.close();
});

test('a redirect within the origin is followed with the headers', async () => {
  const r = await api.request<AnyResult>(def, { path: { id: 1 } });
  assert.equal(r.kind, 'success');
  assert.deepEqual(r.ok && r.data, { key: 'secret' });
});

test('a redirect to another origin is not followed: its response is the result', async () => {
  const r = await api.request<AnyResult>(def, { path: { id: 3 } });
  assert.ok(r.kind === 'http', r.kind);
  assert.equal(r.status, 307);
  assert.deepEqual(otherHeaders, []);
});
