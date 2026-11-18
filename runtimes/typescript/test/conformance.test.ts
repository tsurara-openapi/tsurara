// Runs the language-neutral conformance vectors in spec/conformance (design.md 18.9).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';
import { mockFetch } from './helpers.ts';

const load = (name: string): unknown[] =>
  JSON.parse(
    readFileSync(new URL(`../../../spec/conformance/${name}`, import.meta.url), 'utf8'),
  ) as unknown[];

type Tagged = {
  $blob?: { type: string; text: string };
  $file?: { name: string; type: string; text: string };
};

/** Turns tagged values (`$blob`, `$file`) into runtime values. */
const decode = (v: unknown): unknown => {
  if (Array.isArray(v)) return v.map(decode);
  if (v && typeof v === 'object') {
    const t = v as Tagged;
    if (t.$blob) return new Blob([t.$blob.text], { type: t.$blob.type });
    if (t.$file) return new File([t.$file.text], t.$file.name, { type: t.$file.type });
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, decode(x)]));
  }
  return v;
};

/** Turns runtime values into the neutral form of the vectors. `undefined` becomes `null`. */
const neutral = async (v: unknown): Promise<unknown> => {
  if (v === undefined) return null;
  if (v instanceof File) return { $file: { name: v.name, type: v.type, text: await v.text() } };
  if (v instanceof Blob) return { $blob: { type: v.type, text: await v.text() } };
  return v;
};

type RequestVector = {
  name: string;
  operation: OperationDef;
  input: Record<string, unknown>;
  credentials?: Record<string, string>;
  expect: {
    kind?: string;
    method?: string;
    url?: string;
    headers?: Record<string, string | null>;
    body?:
      | null
      | { text: string }
      | { bytes: { type: string; text: string } }
      | { form: [string, unknown][] };
  };
};

describe('request vectors', () => {
  for (const v of load('requests.json') as RequestVector[]) {
    test(v.name, async () => {
      const m = mockFetch(() => new Response(null, { status: 204 }));
      const api = createClient({
        baseUrl: 'https://api.test',
        fetch: m.fetch,
        auth: (scheme) => v.credentials?.[scheme.scheme],
      });
      const r = await api.request<AnyResult>(v.operation, decode(v.input) as never);
      if (v.expect.kind) {
        assert.equal(r.kind, v.expect.kind);
        assert.equal(m.calls.length, 0);
        return;
      }
      const call = m.calls[0];
      assert.ok(call, r.kind);
      assert.equal(call.url, v.expect.url);
      assert.equal(call.init.method, v.expect.method);
      const headers = new Headers(call.init.headers);
      for (const [k, want] of Object.entries(v.expect.headers ?? {})) {
        assert.equal(headers.get(k), want, k);
      }
      const body = call.init.body;
      const want = v.expect.body;
      if (want == null) {
        assert.ok(body == null, String(body));
      } else if ('text' in want) {
        assert.equal(body, want.text);
      } else if ('bytes' in want) {
        assert.deepEqual(await neutral(body), { $blob: want.bytes });
      } else {
        assert.ok(body instanceof FormData);
        // A part's file name is compared only when the vector expects one (`$file`): runtimes may
        // name parts without a file name differently (FormData uses `blob`).
        const parts = [...body.entries()];
        assert.equal(parts.length, want.form.length);
        for (const [i, [k, x]] of parts.entries()) {
          const [wantName, wantValue]: [string?, unknown?] = want.form[i] ?? [];
          assert.equal(k, wantName);
          const got = await neutral(x);
          const expected = wantValue as Tagged;
          if (expected?.$blob && got && typeof got === 'object' && '$file' in got) {
            const { type, text } = (got as Required<Tagged>).$file;
            assert.deepEqual({ $blob: { type, text } }, expected);
          } else {
            assert.deepEqual(got, wantValue);
          }
        }
      }
    });
  }
});

type ResponseVector = {
  name: string;
  operation: OperationDef;
  response: { status: number; headers: Record<string, string>; body: null | { text: string } };
  expect: { kind: string; status?: number; documented?: boolean; data?: unknown; error?: unknown };
};

describe('response vectors', () => {
  for (const v of load('responses.json') as ResponseVector[]) {
    test(v.name, async () => {
      const m = mockFetch(
        () =>
          new Response(v.response.body?.text ?? null, {
            status: v.response.status,
            headers: v.response.headers,
          }),
      );
      const api = createClient({ baseUrl: 'https://api.test', fetch: m.fetch });
      const r = await api.request<AnyResult>(v.operation, {});
      assert.equal(r.kind, v.expect.kind);
      if (r.kind === 'success') {
        assert.equal(r.status, v.expect.status);
        assert.deepEqual(await neutral(r.data), v.expect.data);
      } else if (r.kind === 'http') {
        assert.equal(r.status, v.expect.status);
        assert.equal(r.documented, v.expect.documented);
        assert.deepEqual(await neutral(r.error), v.expect.error);
      }
    });
  }
});

type SequenceVector = {
  name: string;
  operation: OperationDef;
  response: { status: number; headers: Record<string, string>; chunks: string[] };
  expect: {
    kind: string;
    status: number;
    items?: unknown[];
    error?: boolean;
    documented?: boolean;
  } & Record<string, unknown>;
};

describe('sequence vectors', () => {
  for (const v of load('sequences.json') as SequenceVector[]) {
    test(v.name, async () => {
      const encoder = new TextEncoder();
      const m = mockFetch(() => {
        let i = 0;
        const body = new ReadableStream<Uint8Array>({
          pull(controller) {
            const chunk = v.response.chunks[i++];
            if (chunk === undefined) controller.close();
            else controller.enqueue(encoder.encode(chunk));
          },
        });
        return new Response(body, { status: v.response.status, headers: v.response.headers });
      });
      const api = createClient({ baseUrl: 'https://api.test', fetch: m.fetch });
      const r = await api.request<AnyResult>(v.operation, {});
      assert.equal(r.kind, v.expect.kind);
      if (r.kind === 'success') {
        assert.equal(r.status, v.expect.status);
        const items: unknown[] = [];
        let failed = false;
        try {
          for await (const item of r.data as AsyncIterable<unknown>) items.push(item);
        } catch {
          failed = true;
        }
        assert.deepEqual(items, v.expect.items);
        assert.equal(failed, v.expect.error === true);
      } else if (r.kind === 'http') {
        assert.equal(r.status, v.expect.status);
        assert.equal(r.documented, v.expect.documented);
        assert.deepEqual(await neutral(r.error), v.expect.error);
      }
    });
  }
});
