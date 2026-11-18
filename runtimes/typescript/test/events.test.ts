import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  type AnyResult,
  createClient,
  type OperationDef,
  parseEventStream,
  type ServerSentEvent,
} from '../src/index.ts';
import { mockFetch } from './helpers.ts';

/** A stream that hands over `chunks` one by one, and records whether it was cancelled. */
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

const collect = async (chunks: (string | Uint8Array)[]): Promise<ServerSentEvent[]> => {
  const out: ServerSentEvent[] = [];
  for await (const e of parseEventStream(streamOf(chunks).body)) out.push(e);
  return out;
};

describe('parseEventStream', () => {
  test('fields, the default event name and several data lines', async () => {
    const events = await collect([
      'event: note\ndata: {"id":"1"}\nid: 7\nretry: 3000\n\n',
      'data: a\ndata: b\n\n',
      'data\n\n',
    ]);
    assert.deepEqual(events, [
      { event: 'note', data: '{"id":"1"}', id: '7', retry: 3000 },
      { event: 'message', data: 'a\nb' },
      { event: 'message', data: '' },
    ]);
  });

  test('comments, unknown fields and events without data are skipped', async () => {
    const events = await collect([
      ': heartbeat\n\n',
      'event: ping\n\n',
      'retry: 10\n\n',
      'foo: bar\ndata:x\nretry: soon\nid: a\0b\n\n',
    ]);
    assert.deepEqual(events, [{ event: 'message', data: 'x' }]);
  });

  test('only one leading space of a value is removed', async () => {
    assert.deepEqual(await collect(['data:  two\ndata:none\n\n']), [
      { event: 'message', data: ' two\nnone' },
    ]);
  });

  test('every line ending, also split across chunks', async () => {
    const expected = [
      { event: 'a', data: '1' },
      { event: 'b', data: '2' },
    ];
    assert.deepEqual(await collect(['event: a\r\ndata: 1\r\n\r\nevent: b\rdata: 2\r\r']), expected);
    // CR and LF of one line ending in different chunks are one line ending, not a blank line.
    assert.deepEqual(
      await collect(['event: a\r', '\ndata: 1\r', '\n\r', '\nevent: b\r', 'data: 2\r', '\r']),
      expected,
    );
    const whole = 'event: a\ndata: 1\n\nevent: b\ndata: 2\n\n';
    assert.deepEqual(await collect([...whole]), expected);
  });

  test('characters split across chunks, and a byte order mark', async () => {
    const bytes = new TextEncoder().encode('﻿data: 日本語\n\n');
    const chunks = [bytes.slice(0, 11), bytes.slice(11)];
    assert.deepEqual(await collect(chunks), [{ event: 'message', data: '日本語' }]);
  });

  test('an event that the stream ends in the middle of is dropped', async () => {
    assert.deepEqual(await collect(['data: 1\n\ndata: 2\n']), [{ event: 'message', data: '1' }]);
  });

  test('leaving the loop cancels the stream', async () => {
    const { body, state } = streamOf(['data: 1\n\n', 'data: 2\n\n', 'data: 3\n\n']);
    for await (const e of parseEventStream(body)) {
      assert.equal(e.data, '1');
      break;
    }
    assert.equal(state.cancelled, true);
  });
});

const def = (jsonEvents?: true | readonly string[]) =>
  ({
    v: 1,
    id: 'stream',
    method: 'GET',
    path: '/stream',
    responses: { 200: { parseAs: 'events', ...(jsonEvents && { jsonEvents }) } },
  }) as const satisfies OperationDef;

const request = async (chunks: string[], d: OperationDef) => {
  const m = mockFetch(
    () => new Response(streamOf(chunks).body, { headers: { 'content-type': 'text/event-stream' } }),
  );
  const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
  const r = await api.request<AnyResult>(d, {});
  assert.ok(r.ok, r.kind);
  return r.data as AsyncIterable<unknown>;
};

const all = async (events: AsyncIterable<unknown>): Promise<unknown[]> => {
  const out: unknown[] = [];
  for await (const e of events) out.push(e);
  return out;
};

describe("parseAs: 'events'", () => {
  const chunks = ['event: note\ndata: {"id":"1"}\nid: 7\n\n', 'event: text\ndata: {"id":"2"}\n\n'];

  test('jsonEvents: true parses the data of every event', async () => {
    assert.deepEqual(await all(await request(chunks, def(true))), [
      { event: 'note', data: { id: '1' }, id: '7' },
      { event: 'text', data: { id: '2' } },
    ]);
  });

  test('a list parses the data of those events only', async () => {
    assert.deepEqual(await all(await request(chunks, def(['note']))), [
      { event: 'note', data: { id: '1' }, id: '7' },
      { event: 'text', data: '{"id":"2"}' },
    ]);
  });

  test('without jsonEvents the data stays text', async () => {
    assert.deepEqual(await all(await request(['data: {"id":"1"}\n\n'], def())), [
      { event: 'message', data: '{"id":"1"}' },
    ]);
  });

  test('data that is not valid JSON throws while iterating', async () => {
    const events = await request(['event: note\ndata: {\n\n'], def(true));
    await assert.rejects(all(events), /event "note" is not valid JSON/);
  });

  test('a response without a body is a parse failure', async () => {
    const m = mockFetch(() => {
      const response = new Response(null, { status: 200 });
      Object.defineProperty(response, 'body', { value: null });
      return response;
    });
    const api = createClient({ baseUrl: 'https://x.test', fetch: m.fetch });
    const r = await api.request<AnyResult>(def(true), {});
    assert.equal(r.kind, 'parse');
  });

  test('aborting the request ends the iteration with the abort reason', async () => {
    const abort = new AbortController();
    const api = createClient({
      baseUrl: 'https://x.test',
      fetch: async (_input, init) => {
        const encoder = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(encoder.encode('data: 1\n\n'));
            init?.signal?.addEventListener('abort', () => controller.error(init.signal?.reason));
          },
        });
        return new Response(body);
      },
    });
    const r = await api.request<AnyResult>(def(), { signal: abort.signal });
    assert.ok(r.ok);
    const seen: unknown[] = [];
    await assert.rejects(async () => {
      for await (const e of r.data as AsyncIterable<unknown>) {
        seen.push(e);
        abort.abort(new Error('stop'));
      }
    }, /stop/);
    assert.equal(seen.length, 1);
  });
});
