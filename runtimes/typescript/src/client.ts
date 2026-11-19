import { authenticate } from './auth.ts';
import {
  OPERATION_DEF_VERSION,
  type OperationDef,
  type OperationInfo,
  type ParseAs,
  type ResponseDef,
  type SecurityUse,
} from './def.ts';
import { readEvents } from './events.ts';
import { readJsonItems } from './items.ts';
import { anySignal, isInstance, timeoutReason } from './platform.ts';
import type { QueryScope } from './query.ts';
import type { AnyResult, RawResult, TransportFailure } from './result.ts';
import {
  buildCookie,
  buildQuery,
  buildQueryString,
  encodeBody,
  expandPath,
  serializeHeaderParam,
} from './serialize.ts';
import type { Validator } from './validate.ts';

/**
 * Options of `fetch` that tsurara passes through, such as `cache`, `credentials` or Next.js's
 * `next`. The method, headers, body, signal and redirect handling are the client's.
 */
export type FetchOptions = Omit<RequestInit, 'method' | 'headers' | 'body' | 'signal' | 'redirect'>;

/** Header value. Headers with `undefined` are not sent. */
export type HeaderValues = Headers | { readonly [name: string]: unknown };

/** Options that can be overridden per call. */
export type RequestOptions<Ctx = unknown> = {
  signal?: AbortSignal | undefined;
  headers?: HeaderValues | undefined;
  timeout?: number | undefined;
  baseUrl?: string | undefined;
  context?: Ctx | undefined;
  /** Media type of the body. Defaults to the first media type in the definition. */
  contentType?: string | undefined;
  /** Validation for this call: another validator, or `false` to turn the client's off. */
  validate?: Validator | false | undefined;
  /** Options of `fetch` for this call, over those of the client. */
  fetchOptions?: FetchOptions | undefined;
};

/** Value received by `client.request`. The generated `Data` type combined with `RequestOptions`. */
export type RequestInput<Ctx = unknown> = RequestOptions<Ctx> & {
  path?: { readonly [name: string]: unknown };
  query?: { readonly [name: string]: unknown };
  cookies?: { readonly [name: string]: unknown };
  querystring?: unknown;
  body?: unknown;
};

/** Request received by middleware. Passing it to `next` rebuilds the fetch arguments on every attempt. */
export type MiddlewareRequest<Ctx = unknown> = {
  readonly operation: OperationInfo;
  readonly context: Ctx | undefined;
  url: URL;
  method: string;
  headers: Headers;
  body: BodyInit | null;
  /** Options of `fetch` (the client's with the call's over them). */
  fetchOptions: FetchOptions;
  readonly signal: AbortSignal;
  /** When the client's timeout ends (milliseconds since the epoch), if there is one. */
  readonly deadline?: number | undefined;
};

export type Next<Ctx = unknown> = (req: MiddlewareRequest<Ctx>) => Promise<Response>;

/** Function that intercepts requests and responses. Called from the outermost first. */
export type Middleware<Ctx = unknown> = (
  req: MiddlewareRequest<Ctx>,
  next: Next<Ctx>,
) => Promise<Response>;

/** Information passed to onResult and to header functions. */
export type RequestInfo<Ctx = unknown> = {
  readonly operation: OperationInfo;
  readonly context: Ctx | undefined;
};

/** Information passed to onUndocumentedStatus. */
export type UndocumentedStatusInfo<Ctx = unknown> = RequestInfo<Ctx> & {
  readonly status: number;
  readonly response: Response;
};

/** Optional options accept `undefined`, which means not set, so they can be set conditionally. */
export type ClientOptions<Ctx = unknown> = {
  baseUrl: string;
  headers?:
    | HeaderValues
    | ((info: RequestInfo<Ctx>) => HeaderValues | undefined | Promise<HeaderValues | undefined>)
    | undefined;
  middleware?: readonly Middleware<Ctx>[] | undefined;
  /**
   * Returns the credential for a security scheme of the operation, or `undefined` if there is none.
   * The client uses the first alternative of the operation's `security` with every credential, and
   * places them by the scheme: API keys in their header, query or cookie, `bearer` and OAuth 2 tokens
   * as `Authorization: Bearer`, `basic` as `Authorization: Basic` (give `user:password`).
   */
  auth?:
    | ((
        scheme: SecurityUse,
        info: RequestInfo<Ctx>,
      ) => string | undefined | Promise<string | undefined>)
    | undefined;
  onResult?: ((result: AnyResult, info: RequestInfo<Ctx>) => void | Promise<void>) | undefined;
  /**
   * Called for a response whose status the definition does not document (a `kind: 'http'` Result with
   * `documented: false`, also for 2xx), so that drift between server and definition shows up, for
   * example in logs. Exceptions are ignored.
   */
  onUndocumentedStatus?: ((info: UndocumentedStatusInfo<Ctx>) => void | Promise<void>) | undefined;
  fetch?: ((input: URL | Request | string, init?: RequestInit) => Promise<Response>) | undefined;
  /** Options of `fetch` for every call, such as `cache` or `credentials`. */
  fetchOptions?: FetchOptions | undefined;
  timeout?: number | undefined;
  /**
   * Validates success and documented error bodies parsed as JSON; a body with issues becomes a
   * `kind: 'validation'` Result. `validateResponses(responseSchemas)` builds one from schemas.
   */
  validate?: Validator | undefined;
  /**
   * Which callers may share TanStack Query results (a tenant, an API, an environment; never a
   * credential). The generated query options put it first in their keys, `[{ scope }, operationId,
   * data]`, so that clients with different scopes on one `QueryClient` keep separate entries.
   */
  queryScope?: QueryScope | undefined;
};

export type Client<Ctx = unknown> = {
  readonly options: Readonly<ClientOptions<Ctx>>;
  /** Calls an operation. The returned Promise never rejects. */
  request<R = AnyResult>(def: OperationDef, input?: RequestInput<Ctx>): Promise<R>;
  /**
   * Calls an operation like `request`, but returns the response with its body unread, to read it
   * as the caller needs (`response.body` to stream it, `arrayBuffer()` to decode it). The timeout
   * covers the call until the response arrives; the signal still cancels the reading of the body.
   * `onResult` and `onUndocumentedStatus` are not called. The returned Promise never rejects.
   */
  requestRaw(def: OperationDef, input?: RequestInput<Ctx>): Promise<RawResult>;
};

/** A failure in fetch or further inside. Passes through middleware and becomes a Result on the outside. */
export class TransportError extends Error {
  readonly kind: 'network' | 'timeout' | 'aborted';
  constructor(kind: 'network' | 'timeout' | 'aborted', cause: unknown) {
    super(`tsurara: ${kind}`, { cause });
    this.name = 'TransportError';
    this.kind = kind;
  }
}

const operations = new WeakMap<object, OperationInfo>();

/** Information about the operation that returned a Result. Used by `unwrap`. */
export const operationOf = (result: object): OperationInfo | undefined => operations.get(result);

const info = (def: OperationDef): OperationInfo => ({
  id: def.id,
  method: def.method,
  path: def.path,
  tags: def.tags ?? [],
});

const applyHeaders = (target: Headers, values: HeaderValues | undefined) => {
  if (!values) return;
  if (values instanceof Headers) {
    values.forEach((v, k) => {
      target.set(k, v);
    });
    return;
  }
  for (const [k, v] of Object.entries(values)) {
    if (v !== undefined) target.set(k, typeof v === 'string' ? v : serializeHeaderParam(v));
  }
};

const mediaType = (header: string | null): string =>
  (header?.split(';')[0] ?? '').trim().toLowerCase();

const isJsonType = (mt: string): boolean => mt === 'application/json' || mt.endsWith('+json');

/** Looks up the definition for a status: exact match, then a `2XX` range, then `default`. */
const findResponse = (
  def: OperationDef,
  status: number,
): { entry: ResponseDef; key: string } | undefined => {
  for (const key of [String(status), `${Math.floor(status / 100)}XX`, 'default']) {
    const entry = def.responses[key];
    if (entry) return { entry, key };
  }
  return undefined;
};

const hasSuccessEntry = (def: OperationDef): boolean =>
  Object.keys(def.responses).some((k) => {
    const range = /^([1-5])XX$/i.exec(k);
    if (range) return Number(range[1]) < 4;
    const n = Number(k);
    return Number.isInteger(n) && n < 400;
  });

const guessParseAs = (response: Response): ParseAs => {
  const mt = mediaType(response.headers.get('content-type'));
  if (response.status === 204 || response.status === 205 || response.status === 304) return 'none';
  if (isJsonType(mt)) return 'json';
  if (mt.startsWith('text/') || mt === '') return 'text';
  return 'blob';
};

class ParseFailure extends Error {}

/** The body could not be read to its end: the connection failed, or the signal ended it. */
class ReadFailure extends Error {
  readonly response: Response;
  constructor(cause: unknown, response: Response) {
    super('tsurara: the response body could not be read', { cause });
    this.response = response;
  }
}

/** The `charset` parameter of a Content-Type, in lower case. */
const charsetOf = (header: string | null): string | undefined => {
  const match = /;\s*charset\s*=\s*(?:"([^"]*)"|([^;\s]*))/i.exec(header ?? '');
  const charset = (match?.[1] ?? match?.[2])?.trim().toLowerCase();
  return charset === '' ? undefined : charset;
};

/** Reads a text body with the charset of its Content-Type (`response.text()` always reads UTF-8). */
const readText = async (response: Response): Promise<string> => {
  const charset = charsetOf(response.headers.get('content-type'));
  if (charset === undefined || charset === 'utf-8' || charset === 'utf8') return response.text();
  let decoder: TextDecoder;
  try {
    decoder = new TextDecoder(charset);
  } catch (e) {
    await response.body?.cancel();
    throw new ParseFailure(
      `the response has charset ${charset}, which this platform's TextDecoder cannot decode`,
      { cause: e },
    );
  }
  return decoder.decode(await response.arrayBuffer());
};

const readBody = async (
  response: Response,
  parseAs: ParseAs,
  entry?: ResponseDef,
): Promise<unknown> => {
  switch (parseAs) {
    case 'none':
      await response.body?.cancel();
      return undefined;
    case 'text':
      return readText(response);
    case 'blob':
      return response.blob();
    case 'stream':
      if (!response.body) {
        throw new ParseFailure(
          'the response has no body to stream; the fetch in use does not support streaming bodies (in React Native, use expo/fetch)',
        );
      }
      return response.body;
    case 'events':
      if (!response.body) {
        throw new ParseFailure(
          'the response has no body to read events from; the fetch in use does not support streaming bodies (in React Native, use expo/fetch)',
        );
      }
      return readEvents(response.body, entry?.jsonEvents);
    case 'jsonLines':
    case 'jsonSeq':
      if (!response.body) {
        throw new ParseFailure(
          'the response has no body to read items from; the fetch in use does not support streaming bodies (in React Native, use expo/fetch)',
        );
      }
      return readJsonItems(response.body, parseAs === 'jsonSeq' ? 'seq' : 'lines');
    case 'auto':
      return readBody(response, guessParseAs(response));
    case 'json': {
      const mt = mediaType(response.headers.get('content-type'));
      if (mt !== '' && !isJsonType(mt)) {
        await response.body?.cancel();
        throw new ParseFailure(`expected JSON but the response has Content-Type ${mt}`);
      }
      const text = await response.text();
      try {
        return JSON.parse(text);
      } catch (e) {
        throw new ParseFailure('the response body is not valid JSON', { cause: e });
      }
    }
  }
};

/** Body for a status not in the definition. Returned in whatever form could be read, even if parsing fails. */
const readUndocumented = async (response: Response): Promise<unknown> => {
  const parseAs = guessParseAs(response);
  if (parseAs === 'json') {
    const text = await response.text();
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
  return readBody(response, parseAs);
};

const toResult = async (
  def: OperationDef,
  response: Response,
  validate: Validator | undefined,
  operation: OperationInfo,
): Promise<AnyResult> => {
  const status = response.status;
  const { found, success, documented } = classifyStatus(def, status);
  const contentType = response.headers.get('content-type');
  const mt = mediaType(contentType);
  /** Reads the body; a failure that is not about its contents keeps the response. */
  const read = async (body: () => Promise<unknown>): Promise<unknown> => {
    try {
      return await body();
    } catch (cause) {
      if (cause instanceof ParseFailure || cause instanceof SyntaxError) throw cause;
      throw new ReadFailure(cause, response);
    }
  };
  /** The body, and whether it was read as JSON. */
  const parse = async (entry: ResponseDef): Promise<[unknown, boolean]> => {
    const declared = entry.contentTypes?.[mt] ?? entry.parseAs;
    const parseAs =
      declared === 'none' && entry.readWithContentType && mt !== '' ? 'auto' : declared;
    const json = parseAs === 'json' || (parseAs === 'auto' && guessParseAs(response) === 'json');
    return [await read(() => readBody(response, parseAs, entry)), json];
  };
  /** The issues of a JSON body, if the client validates. */
  const check = async (value: unknown, json: boolean) =>
    json && validate ? validate(value, { operation, status, mediaType: mt }) : undefined;
  try {
    if (found && documented) {
      const [value, json] = await parse(found.entry);
      const issues = await check(value, json);
      if (issues && issues.length > 0) {
        return { ok: false, kind: 'validation', status, value, issues, response };
      }
      if (success) return { ok: true, kind: 'success', status, data: value, response };
      return { ok: false, kind: 'http', documented: true, status, error: value, response };
    }
    return {
      ok: false,
      kind: 'http',
      documented: false,
      status,
      error: await read(() => readUndocumented(response)),
      response,
    };
  } catch (cause) {
    if (cause instanceof ParseFailure || cause instanceof SyntaxError) {
      return { ok: false, kind: 'parse', cause, response, contentType };
    }
    throw cause;
  }
};

const failure = (
  kind: TransportFailure['kind'],
  cause: unknown,
  response?: Response,
): TransportFailure => {
  if (kind === 'parse' || kind === 'validation' || kind === 'unexpected') {
    return { ok: false, kind: 'unexpected', cause };
  }
  return response ? { ok: false, kind, cause, response } : { ok: false, kind, cause };
};

/** Whether a status is a success of the definition, and whether the definition documents it. */
const classifyStatus = (
  def: OperationDef,
  status: number,
): {
  found: { entry: ResponseDef; key: string } | undefined;
  success: boolean;
  documented: boolean;
} => {
  const found = findResponse(def, status);
  const isDefault = found?.key === 'default';
  const success = status < 400 && found !== undefined && (!isDefault || !hasSuccessEntry(def));
  return { found, success, documented: found !== undefined && (success || status >= 400) };
};

const REDIRECTS = new Set([301, 302, 303, 307, 308]);

/** Creates a client. Pass it as the first argument of generated functions. */
export const createClient = <Ctx = unknown>(options: ClientOptions<Ctx>): Client<Ctx> => {
  const fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  const middleware = options.middleware ?? [];

  // Redirects are followed here, within the origin of the request only: a redirect to another origin
  // is not followed and its response is the result, so that headers such as API keys never reach
  // another host. Browsers hide where a redirect goes (`redirect: 'manual'` gives an opaque
  // response), so there fetch follows them itself, under the browser's CORS rules.
  const manual = !(typeof window !== 'undefined' && typeof document !== 'undefined');
  const send: Next<Ctx> = async (req) => {
    const origin = req.url.origin;
    let url = req.url.href;
    let method = req.method;
    let body = req.body;
    const headers = new Headers(req.headers);
    for (let redirects = 0; ; redirects++) {
      const init: RequestInit & { duplex?: 'half' } = {
        ...req.fetchOptions,
        method,
        headers,
        body,
        signal: req.signal,
      };
      if (manual) init.redirect = 'manual';
      if (isInstance(body, 'ReadableStream')) init.duplex = 'half';
      let res: Response;
      try {
        res = await fetchImpl(url, init);
      } catch (cause) {
        throw new TransportError('network', cause);
      }
      const location = REDIRECTS.has(res.status) ? res.headers.get('location') : null;
      if (!manual || location === null) return res;
      const next = new URL(location, url);
      // A stream body cannot be sent again.
      const resend =
        !(res.status === 307 || res.status === 308) || !isInstance(body, 'ReadableStream');
      if (next.origin !== origin || !resend) return res;
      if (redirects >= 10)
        throw new TransportError('network', new Error('stopped after 10 redirects'));
      if (
        (res.status === 303 && method !== 'HEAD') ||
        ((res.status === 301 || res.status === 302) && method === 'POST')
      ) {
        method = 'GET';
        body = null;
        headers.delete('content-type');
      }
      await res.body?.cancel();
      url = next.href;
    }
  };

  /** Sends the request and reads the response into a Result, or with `raw` leaves its body unread. */
  async function run(def: OperationDef, input: RequestInput<Ctx>, raw: false): Promise<AnyResult>;
  async function run(def: OperationDef, input: RequestInput<Ctx>, raw: true): Promise<RawResult>;
  async function run(
    def: OperationDef,
    input: RequestInput<Ctx>,
    raw: boolean,
  ): Promise<AnyResult | RawResult> {
    const op = info(def);
    const reqInfo: RequestInfo<Ctx> = { operation: op, context: input.context };
    if (def.v !== OPERATION_DEF_VERSION) {
      return failure(
        'unexpected',
        new Error(
          `tsurara: OperationDef version ${String(def.v)} is not supported by this @tsurara/client (expected ${OPERATION_DEF_VERSION}); regenerate the code or update @tsurara/client`,
        ),
      );
    }
    const userSignal = input.signal;
    const timeoutMs = input.timeout ?? options.timeout;
    const deadline =
      timeoutMs !== undefined && Number.isFinite(timeoutMs) ? Date.now() + timeoutMs : undefined;
    const timeoutController = new AbortController();
    const timer =
      timeoutMs !== undefined && Number.isFinite(timeoutMs)
        ? setTimeout(() => timeoutController.abort(timeoutReason()), timeoutMs)
        : undefined;
    const signal = userSignal
      ? anySignal([userSignal, timeoutController.signal])
      : timeoutController.signal;
    const classify = (thrown: unknown): AnyResult => {
      // A failure while the body was read keeps the response; it is a network failure unless the
      // signal ended it.
      const response = thrown instanceof ReadFailure ? thrown.response : undefined;
      const cause = thrown instanceof ReadFailure ? thrown.cause : thrown;
      if (userSignal?.aborted) return failure('aborted', userSignal.reason ?? cause, response);
      if (timeoutController.signal.aborted) return failure('timeout', cause, response);
      if (cause instanceof TransportError) return failure(cause.kind, cause.cause, response);
      if (response) return failure('network', cause, response);
      return failure('unexpected', cause);
    };
    try {
      if (userSignal?.aborted) return failure('aborted', userSignal.reason);
      const base = input.baseUrl ?? options.baseUrl;
      const path = expandPath(def.path, input.path, def.params?.path);
      const credentials = await authenticate(def.security, (scheme) =>
        options.auth?.(scheme, reqInfo),
      );
      let query = [buildQuery(input.query, def.params?.query), ...credentials.query]
        .filter((s) => s !== '')
        .join('&');
      if (def.params?.querystring) {
        const qs = buildQueryString(input.querystring, def.params.querystring.contentType);
        query = [query, qs].filter((s) => s !== '').join('&');
      }
      // The URL is built as a string and parsed once: some platforms (React Native) have URL objects
      // whose setters are not implemented.
      let href = base.replace(/\/+$/, '');
      const hash = href.indexOf('#');
      if (hash >= 0) href = href.slice(0, hash);
      const at = href.indexOf('?');
      const baseQuery = at >= 0 ? href.slice(at + 1) : '';
      if (at >= 0) href = href.slice(0, at);
      const search = [baseQuery, query].filter((s) => s !== '').join('&');
      const url = new URL(href + path + (search === '' ? '' : `?${search}`));
      const headers = new Headers();
      const shared =
        typeof options.headers === 'function' ? await options.headers(reqInfo) : options.headers;
      applyHeaders(headers, shared);
      if (input.headers && !(input.headers instanceof Headers)) {
        const defs = def.params?.headers ?? {};
        for (const [k, v] of Object.entries(input.headers)) {
          if (v === undefined) continue;
          headers.set(k, typeof v === 'string' ? v : serializeHeaderParam(v, defs[k]));
        }
      } else {
        applyHeaders(headers, input.headers);
      }
      for (const [k, v] of credentials.headers) headers.set(k, v);
      const cookie = [buildCookie(input.cookies, def.params?.cookies), ...credentials.cookies]
        .filter((s) => s !== '')
        .join('; ');
      if (cookie !== '') {
        const existing = headers.get('cookie');
        headers.set('cookie', existing ? `${existing}; ${cookie}` : cookie);
      }
      let body: BodyInit | null = null;
      if (def.body && input.body !== undefined) {
        const encoded = encodeBody(input.body, def.body, input.contentType);
        body = encoded.body;
        if (encoded.contentType && !headers.has('content-type')) {
          headers.set('content-type', encoded.contentType);
        }
      }
      const req: MiddlewareRequest<Ctx> = {
        operation: op,
        context: input.context,
        url,
        method: def.method.toUpperCase(),
        headers,
        body,
        fetchOptions: { ...options.fetchOptions, ...input.fetchOptions },
        signal,
        deadline,
      };
      // Header and auth functions may have awaited; do not start a request that is already over.
      if (signal.aborted) return classify(signal.reason);
      const chain = middleware.reduceRight<Next<Ctx>>((next, mw) => (r) => mw(r, next), send);
      const response = await chain(req);
      if (raw) {
        const { success, documented } = classifyStatus(def, response.status);
        return { ok: success, kind: 'response', status: response.status, documented, response };
      }
      const validate = input.validate === false ? undefined : (input.validate ?? options.validate);
      return await toResult(def, response, validate, op);
    } catch (cause) {
      return classify(cause);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  return {
    options,
    requestRaw(def: OperationDef, input: RequestInput<Ctx> = {}): Promise<RawResult> {
      return run(def, input, true);
    },
    async request<R = AnyResult>(def: OperationDef, input: RequestInput<Ctx> = {}): Promise<R> {
      const result = await run(def, input, false);
      operations.set(result, info(def));
      if (options.onUndocumentedStatus && result.kind === 'http' && !result.documented) {
        try {
          await options.onUndocumentedStatus({
            operation: info(def),
            context: input.context,
            status: result.status,
            response: result.response,
          });
        } catch {
          // An observation hook, like onResult.
        }
      }
      if (options.onResult) {
        try {
          await options.onResult(result, { operation: info(def), context: input.context });
        } catch {
          // onResult is an observation hook, so its exceptions do not change the result.
        }
      }
      return result as R;
    },
  };
};
