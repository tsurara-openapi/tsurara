import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { type AnyResult, createClient, type OperationDef } from '../src/index.ts';

/** A response body that hands over `chunks` one by one, and records whether it was cancelled. */
const streamOf = (chunks: (string | Uint8Array)[]) => {
  const state = { cancelled: false };
  const encoder = new TextEncoder();
  let i = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(controller) {
      const chunk = chunks[i++];
      if (chunk === undefined) controller.close();
      else controller.enqueue(typeof chunk === 'string' ? encoder.encode(chunk) : chunk);
    },
    cancel() {
      state.cancelled = true;
    },
  });
  return { body, state };
};

const def = (parseAs: 'jsonLines' | 'jsonSeq') =>
  ({
    v: 1,
    id: 'items',
    method: 'GET',
    path: '/items',
    responses: { 200: { parseAs } },
  }) as const satisfies OperationDef;

const request = async (parseAs: 'jsonLines' | 'jsonSeq', body: ReadableStream<Uint8Array>) => {
  const api = createClient({ baseUrl: 'https://x.test', fetch: async () => new Response(body) });
  const r = await api.request<AnyResult>(def(parseAs), {});
  assert.ok(r.ok, r.kind);
  return r.data as AsyncIterable<unknown>;
};

const all = async (
  parseAs: 'jsonLines' | 'jsonSeq',
  chunks: (string | Uint8Array)[],
): Promise<unknown[]> => {
  const out: unknown[] = [];
  for await (const item of await request(parseAs, streamOf(chunks).body)) out.push(item);
  return out;
};

describe("parseAs: 'jsonLines'", () => {
  test('one JSON value per line, also split across chunks', async () => {
    assert.deepEqual(await all('jsonLines', ['{"a":1}\n{"a"', ':2}\r\n', '3\n']), [
      { a: 1 },
      { a: 2 },
      3,
    ]);
  });

  test('blank lines are skipped and the last line needs no line ending', async () => {
    assert.deepEqual(await all('jsonLines', ['\n{"a":1}\n\n  \n', '"end"']), [{ a: 1 }, 'end']);
  });

  test('characters split across chunks', async () => {
    const bytes = new TextEncoder().encode('"日本語"\n');
    assert.deepEqual(await all('jsonLines', [bytes.slice(0, 3), bytes.slice(3)]), ['日本語']);
  });

  test('a line that is not valid JSON throws while iterating', async () => {
    await assert.rejects(all('jsonLines', ['{"a":1}\n{\n']), /line 2 is not valid JSON/);
  });

  test('leaving the loop cancels the stream', async () => {
    const { body, state } = streamOf(['1\n', '2\n', '3\n']);
    for await (const item of await request('jsonLines', body)) {
      assert.equal(item, 1);
      break;
    }
    assert.equal(state.cancelled, true);
  });
});

describe("parseAs: 'jsonSeq'", () => {
  test('records start with RS and may span lines', async () => {
    assert.deepEqual(await all('jsonSeq', ['\x1e{"a":\n1}\n\x1e', '[1,', '2]\n', '\x1e"x"\n']), [
      { a: 1 },
      [1, 2],
      'x',
    ]);
  });

  test('a record separator alone in a chunk, and text before the first one', async () => {
    assert.deepEqual(await all('jsonSeq', ['junk', '\x1e', '1\n\x1e2\n']), [1, 2]);
  });

  test('empty records are skipped', async () => {
    assert.deepEqual(await all('jsonSeq', ['\x1e\n\x1e1\n\x1e\x1e2\n']), [1, 2]);
  });

  test('a record that is not valid JSON throws while iterating', async () => {
    await assert.rejects(all('jsonSeq', ['\x1e1\n\x1e{\n']), /record 2 is not valid JSON/);
  });
});
