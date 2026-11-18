/**
 * Reads a sequential JSON body item by item: one JSON text per line for JSON Lines
 * (`application/jsonl`, `application/x-ndjson`), or one per record starting with the record
 * separator U+001E for JSON text sequences (`application/json-seq`, RFC 7464). Blank lines and
 * empty records are skipped. A text that is not valid JSON throws from the iteration.
 *
 * The stream can be read once. Leaving the loop cancels it, which closes the connection.
 */
export async function* readJsonItems(
  body: ReadableStream<Uint8Array>,
  framing: 'lines' | 'seq',
): AsyncGenerator<unknown, void, undefined> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const separator = framing === 'lines' ? /\r\n|\r|\n/ : '\u001e';
  const what = framing === 'lines' ? 'line' : 'record';
  let buffer = '';
  let count = 0;
  /** Whether the text before the first separator was dropped (always, for lines). */
  let started = framing === 'lines';
  const parse = (text: string): { value: unknown } | undefined => {
    count++;
    if (text.trim() === '') return undefined;
    try {
      return { value: JSON.parse(text) };
    } catch (cause) {
      throw new Error(`${what} ${count} is not valid JSON`, { cause });
    }
  };
  try {
    for (;;) {
      const { done, value } = await reader.read();
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      // A CR at the end may be the first half of a CRLF, so it waits for what follows.
      const held = framing === 'lines' && !done && buffer.endsWith('\r') ? '\r' : '';
      const parts = buffer.slice(0, buffer.length - held.length).split(separator);
      buffer = (parts.pop() ?? '') + held;
      // Text before the first record separator is not a record.
      if (!started && parts.length > 0) {
        parts.shift();
        started = true;
      }
      for (const text of parts) {
        const item = parse(text);
        if (item) yield item.value;
      }
      if (done) {
        const item = started ? parse(buffer) : undefined;
        if (item) yield item.value;
        return;
      }
    }
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}
