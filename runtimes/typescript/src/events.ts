/** One Server-Sent Event, as `parseEventStream` reads it. */
export type ServerSentEvent = {
  /** The event name; `message` when the event has none. */
  event: string;
  /** The data lines, joined with newlines. */
  data: string;
  /** The `id` field of this event, if it has one. */
  id?: string;
  /** The `retry` field of this event (milliseconds), if it has one. */
  retry?: number;
};

/**
 * Reads a `text/event-stream` body as events, parsed as the HTML standard describes: lines end
 * with CRLF, LF or CR, comments and unknown fields are skipped, and an event is complete at a
 * blank line. As in `EventSource`, events without a `data` field are not handed over, and an
 * event the stream ends in the middle of is dropped. `id` and `retry` are those of the event
 * itself; reconnecting is up to the caller.
 *
 * The stream can be read once. Leaving the loop cancels it, which closes the connection.
 */
export async function* parseEventStream(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<ServerSentEvent, void, undefined> {
  const reader = body.getReader();
  // Removes a leading byte order mark.
  const decoder = new TextDecoder();
  let buffer = '';
  let event = '';
  let data: string[] = [];
  let id: string | undefined;
  let retry: number | undefined;

  /** Takes one complete line of the buffer, or the event that a blank line completes. */
  const line = (text: string): ServerSentEvent | undefined => {
    if (text === '') {
      const complete: ServerSentEvent | undefined =
        data.length > 0
          ? {
              event: event === '' ? 'message' : event,
              data: data.join('\n'),
              ...(id !== undefined && { id }),
              ...(retry !== undefined && { retry }),
            }
          : undefined;
      event = '';
      data = [];
      id = undefined;
      retry = undefined;
      return complete;
    }
    const colon = text.indexOf(':');
    if (colon === 0) return undefined;
    const field = colon < 0 ? text : text.slice(0, colon);
    let value = colon < 0 ? '' : text.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'event') event = value;
    else if (field === 'data') data.push(value);
    else if (field === 'id' && !value.includes('\0')) id = value;
    else if (field === 'retry' && /^\d+$/.test(value)) retry = Number(value);
    return undefined;
  };

  try {
    for (;;) {
      const { done, value } = await reader.read();
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      // A CR at the end may be the first half of a CRLF, so it waits for what follows.
      const held = !done && buffer.endsWith('\r') ? '\r' : '';
      const lines = buffer.slice(0, buffer.length - held.length).split(/\r\n|\r|\n/);
      buffer = (lines.pop() ?? '') + held;
      for (const text of lines) {
        const complete = line(text);
        if (complete) yield complete;
      }
      if (done) return;
    }
  } finally {
    // Closes the connection when the caller leaves the loop early; a stream that already ended
    // or failed has nothing to cancel.
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

/**
 * The events of a response with `parseAs: 'events'`: those named in `json` (all for `true`) have
 * their data parsed as JSON.
 */
export async function* readEvents(
  body: ReadableStream<Uint8Array>,
  json: true | readonly string[] | undefined,
): AsyncGenerator<unknown, void, undefined> {
  for await (const e of parseEventStream(body)) {
    if (json === true || json?.includes(e.event)) {
      let data: unknown;
      try {
        data = JSON.parse(e.data);
      } catch (cause) {
        throw new Error(`the data of the event "${e.event}" is not valid JSON`, { cause });
      }
      yield { ...e, data };
    } else {
      yield e;
    }
  }
}
