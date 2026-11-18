// Messages of event channels: decoding the text of one message, with optional validation.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import * as v from 'valibot';
import { type ChannelDef, decodeMessage, validateChannels } from '../src/index.ts';

type BoardMessage =
  | { type: 'card.added'; seq: number; card: { id: string } }
  | { type: 'phase.changed'; seq: number; phase: string };

const boardDef = {
  v: 1,
  id: 'board',
  channel: 'board/{boardId}',
  parseAs: 'json',
} as const satisfies ChannelDef;

test('a valid message decodes to its value', () => {
  const r = decodeMessage<BoardMessage>(
    boardDef,
    '{"type":"phase.changed","seq":3,"phase":"vote"}',
  );
  assert.equal(r.ok, true);
  if (r.ok && r.value.type === 'phase.changed') {
    assert.equal(r.value.phase, 'vote');
  }
});

test('text that is not JSON is a parse failure with the text', () => {
  const r = decodeMessage<BoardMessage>(boardDef, '{oops');
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.kind, 'parse');
  assert.equal(r.ok === false && r.kind === 'parse' && r.data, '{oops');
});

test('a validator reports issues as a validation failure', () => {
  const schema = v.object({ type: v.string(), seq: v.number() });
  const validate = validateChannels({ board: schema });
  const ok = decodeMessage<BoardMessage>(boardDef, '{"type":"x","seq":1}', { validate });
  assert.equal(ok.ok, true);
  const bad = decodeMessage<BoardMessage>(boardDef, '{"type":"x","seq":"1"}', { validate });
  assert.equal(bad.ok, false);
  assert.equal(bad.ok === false && bad.kind, 'validation');
  assert.ok(bad.ok === false && bad.kind === 'validation' && bad.issues.length > 0);
  // A channel without a schema is not checked.
  const other = decodeMessage({ ...boardDef, id: 'other' }, '"anything"', { validate });
  assert.equal(other.ok, true);
});

test('asynchronous schemas are rejected, since decoding is synchronous', () => {
  const schema = v.pipeAsync(
    v.string(),
    v.checkAsync(async () => true),
  );
  const validate = validateChannels({ board: schema });
  assert.throws(() => decodeMessage(boardDef, '"x"', { validate }), /synchronous/);
});
