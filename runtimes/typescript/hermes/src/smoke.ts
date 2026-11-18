// What a React Native app does with @tsurara/client, run on Hermes after the globals of
// `prelude.js`. The transport is a fetch that answers in-process, built on the polyfilled Response,
// because the VM has no network. Prints one line per check and a final `done` line that `run.mjs` reads.
import {
  type AnyResult,
  createClient,
  type OperationDef,
  reactNativeFile,
  unwrap,
} from '../../src/index.ts';

declare const print: (...args: unknown[]) => void;

type Call = { url: string; init: RequestInit };

const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });

const recording = (respond: (call: Call) => Response) => {
  const calls: Call[] = [];
  const fetch = async (input: URL | Request | string, init: RequestInit = {}) => {
    const call = { url: String(input), init };
    calls.push(call);
    return respond(call);
  };
  return { calls, fetch };
};

/** A fetch that never answers until aborted, like a request to a server that hangs. */
const hanging = async (_input: URL | Request | string, init?: RequestInit) =>
  new Promise<Response>((_, reject) => {
    init?.signal?.addEventListener('abort', () => reject(init.signal?.reason));
  });

const getPet = {
  v: 1,
  id: 'getPet',
  method: 'GET',
  path: '/pet/{petId}',
  params: { path: { petId: {} }, query: { tags: { style: 'form', explode: true } } },
  responses: { 200: { parseAs: 'json' }, 404: { parseAs: 'json' } },
  security: [[{ scheme: 'basic', type: 'http', httpScheme: 'basic' }]],
} as const satisfies OperationDef;

const addPet = {
  v: 1,
  id: 'addPet',
  method: 'POST',
  path: '/pet',
  body: { required: true, contents: { 'application/json': {} } },
  responses: { 201: { parseAs: 'json' } },
} as const satisfies OperationDef;

const upload = {
  v: 1,
  id: 'upload',
  method: 'POST',
  path: '/pet/{petId}/photo',
  params: { path: { petId: {} } },
  body: { required: true, contents: { 'multipart/form-data': {} } },
  responses: { 204: { parseAs: 'none' } },
} as const satisfies OperationDef;

const stream = {
  v: 1,
  id: 'stream',
  method: 'GET',
  path: '/events',
  responses: { 200: { parseAs: 'stream' } },
} as const satisfies OperationDef;

const checks: [string, () => Promise<void>][] = [];
const check = (name: string, run: () => Promise<void>): void => {
  checks.push([name, run]);
};
const expect = (cond: boolean, message: string): void => {
  if (!cond) throw new Error(message);
};
const describe = (r: AnyResult): string => `${r.kind}${'cause' in r ? `: ${String(r.cause)}` : ''}`;

check('a GET with path, query and basic auth', async () => {
  const m = recording(() => json(200, { id: 1, name: 'Pochi' }));
  const api = createClient({
    baseUrl: 'https://api.example.com/v1',
    fetch: m.fetch,
    auth: () => 'user:pässword',
  });
  const r = await api.request<AnyResult>(getPet, {
    path: { petId: 1 },
    query: { tags: ['a b', 'c'] },
  });
  expect(r.kind === 'success', describe(r));
  expect(r.ok && (r.data as { name: string }).name === 'Pochi', 'data');
  expect(
    m.calls[0]?.url === 'https://api.example.com/v1/pet/1?tags=a%20b&tags=c',
    `url ${m.calls[0]?.url}`,
  );
  const headers = new Headers(m.calls[0]?.init.headers);
  expect(
    headers.get('authorization') === 'Basic dXNlcjpww6Rzc3dvcmQ=',
    `auth ${headers.get('authorization')}`,
  );
});

check('a documented error status', async () => {
  const m = recording(() => json(404, { message: 'no pet' }));
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: m.fetch });
  const r = await api.request<AnyResult>(getPet, { path: { petId: 2 } });
  expect(r.kind === 'http' && r.status === 404, describe(r));
  let thrown: unknown;
  try {
    unwrap(r);
  } catch (e) {
    thrown = e;
  }
  expect(
    thrown instanceof Error && (thrown as { kind?: string }).kind === 'http',
    'unwrap throws ApiError',
  );
});

check('a POST with a JSON body', async () => {
  const m = recording(() => json(201, { id: 3 }));
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: m.fetch });
  const r = await api.request<AnyResult>(addPet, { body: { name: 'Tama' } });
  expect(r.kind === 'success', describe(r));
  expect(m.calls[0]?.init.body === '{"name":"Tama"}', `body ${String(m.calls[0]?.init.body)}`);
});

check('a multipart upload with reactNativeFile', async () => {
  const m = recording(() => new Response(null, { status: 204 }));
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: m.fetch });
  const photo = reactNativeFile({
    uri: 'file:///photo.jpg',
    name: 'photo.jpg',
    type: 'image/jpeg',
  });
  const r = await api.request<AnyResult>(upload, {
    path: { petId: 1 },
    body: { photo, note: 'n' },
  });
  expect(r.kind === 'success', describe(r));
  const body = m.calls[0]?.init.body as unknown as {
    getParts(): { fieldName: string; uri?: string; string?: string }[];
  };
  const parts = body.getParts();
  expect(parts[0]?.fieldName === 'photo' && parts[0]?.uri === 'file:///photo.jpg', 'file part');
  expect(parts[1]?.fieldName === 'note' && parts[1]?.string === 'n', 'text part');
  const headers = new Headers(m.calls[0]?.init.headers);
  expect(!headers.has('content-type'), 'the platform sets the multipart boundary');
});

check('a timeout', async () => {
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: hanging, timeout: 20 });
  const r = await api.request<AnyResult>(getPet, { path: { petId: 1 } });
  expect(r.kind === 'timeout', describe(r));
});

check('an abort', async () => {
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: hanging, timeout: 5000 });
  const controller = new AbortController();
  const pending = api.request<AnyResult>(getPet, { path: { petId: 1 }, signal: controller.signal });
  controller.abort();
  const r = await pending;
  expect(r.kind === 'aborted', describe(r));
});

check('a stream without a streaming fetch is a parse failure', async () => {
  const m = recording(() => new Response('data: 1\n\n', { status: 200 }));
  const api = createClient({ baseUrl: 'https://api.example.com', fetch: m.fetch });
  const r = await api.request<AnyResult>(stream, {});
  expect(r.kind === 'parse', describe(r));
});

const main = async (): Promise<void> => {
  let passed = 0;
  for (const [name, run] of checks) {
    try {
      await run();
      passed++;
      print(`ok - ${name}`);
    } catch (e) {
      print(`not ok - ${name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  print(`done ${passed}/${checks.length}`);
};

main().catch((e: unknown) => print(`not ok - crashed: ${String(e)}`));
