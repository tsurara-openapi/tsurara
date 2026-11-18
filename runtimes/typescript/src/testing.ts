/**
 * `@tsurara/client/testing`: a `fetch` that answers from handlers routed by operation,
 * for tests without a server or a mocking library. Routing follows spec/README.md (Mock transports),
 * shared with the runtimes of the other languages. A separate entry point, so production bundles do
 * not include it.
 */
import type { OperationDef } from './def.ts';

/** What a handler receives. */
export type MockRequest = {
  readonly method: string;
  readonly url: URL;
  /** Path parameters, percent-decoded. */
  readonly params: Readonly<Record<string, string>>;
  readonly query: URLSearchParams;
  readonly headers: Headers;
  /** The body as text, or `undefined` without one. */
  readonly body: string | undefined;
  /** The raw request, for bodies that are not text (form data, bytes). */
  readonly request: Request;
};

/** What a handler returns: a status with a JSON value, text or bytes. */
export type MockResponse = {
  readonly status: number;
  readonly headers?: Readonly<Record<string, string>>;
  /** Encoded as JSON with `Content-Type: application/json`. */
  readonly json?: unknown;
  readonly text?: string;
  readonly bytes?: Uint8Array;
  /** The Content-Type of `text` or `bytes`. */
  readonly contentType?: string;
};

export type MockHandler = (request: MockRequest) => MockResponse | Promise<MockResponse>;

/** A route by OperationDef, or by method and path template. */
export type MockRoute =
  | { readonly operation: OperationDef; readonly handler: MockHandler }
  | { readonly method: string; readonly path: string; readonly handler: MockHandler };

export type MockOptions = {
  /** Removed from request paths before routing (the path of the client's base URL). */
  readonly basePath?: string;
  /** `error` (default): an unmatched request fails the transport. `record`: it is answered with 501. */
  readonly unmatched?: 'error' | 'record';
};

export type MockCall = {
  readonly method: string;
  readonly url: string;
  /** The OperationDef of the route, when the route was given one. */
  readonly route: OperationDef | undefined;
  readonly params: Readonly<Record<string, string>>;
  readonly body: string | undefined;
};

type Compiled = {
  readonly method: string;
  readonly segments: readonly (string | RegExp)[];
  readonly names: readonly (readonly string[])[];
  readonly literals: number;
  readonly route: MockRoute;
};

const escapeRegExp = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const compile = (route: MockRoute): Compiled => {
  const method = ('operation' in route ? route.operation.method : route.method).toUpperCase();
  const path = 'operation' in route ? route.operation.path : route.path;
  const segments: (string | RegExp)[] = [];
  const names: string[][] = [];
  let literals = 0;
  for (const segment of path.split('/')) {
    if (!segment.includes('{')) {
      segments.push(segment);
      names.push([]);
      literals += 1;
      continue;
    }
    const found: string[] = [];
    // Greedy groups: the last match wins inside a segment (`{name}.{ext}`).
    const source = segment.replace(/\{([^}]+)\}|([^{]+)/g, (_, name: string | undefined, text) => {
      if (name !== undefined) {
        found.push(name);
        return '(.+)';
      }
      return escapeRegExp(text as string);
    });
    segments.push(new RegExp(`^${source}$`));
    names.push(found);
  }
  return { method, segments, names, literals, route };
};

/** Routes requests to handlers (spec/README.md); exported for the conformance vectors. */
export const createRouter = (
  routes: readonly MockRoute[],
  options: MockOptions = {},
): {
  match: (
    method: string,
    path: string,
  ) => { index: number; route: MockRoute; params: Record<string, string> } | undefined;
} => {
  const compiled = routes.map(compile);
  const base = options.basePath ?? '';
  return {
    match: (method, rawPath) => {
      const path = rawPath.split('?')[0] ?? '';
      if (!path.startsWith(base)) return undefined;
      const rest = path.slice(base.length);
      const parts = rest.split('/');
      let best: { index: number; route: MockRoute; params: Record<string, string> } | undefined;
      let bestLiterals = -1;
      compiled.forEach((c, index) => {
        if (c.method !== method.toUpperCase() || c.segments.length !== parts.length) return;
        const params: Record<string, string> = {};
        for (let i = 0; i < parts.length; i++) {
          const segment = c.segments[i];
          const part = parts[i] ?? '';
          if (segment === undefined) return;
          if (typeof segment === 'string') {
            if (segment !== part) return;
            continue;
          }
          const m = segment.exec(part);
          if (!m) return;
          for (const [j, name] of (c.names[i] ?? []).entries()) {
            params[name] = decodeURIComponent(m[j + 1] ?? '');
          }
        }
        if (c.literals > bestLiterals) {
          best = { index, route: c.route, params };
          bestLiterals = c.literals;
        }
      });
      return best;
    },
  };
};

const respond = (r: MockResponse): Response => {
  const headers = new Headers(r.headers);
  let body: BodyInit | null = null;
  if (r.json !== undefined) {
    body = JSON.stringify(r.json);
    if (!headers.has('content-type')) headers.set('content-type', 'application/json');
  } else if (r.text !== undefined) {
    body = r.text;
  } else if (r.bytes !== undefined) {
    body = new Blob([r.bytes as Uint8Array<ArrayBuffer>]);
  }
  if (r.contentType !== undefined && !headers.has('content-type')) {
    headers.set('content-type', r.contentType);
  }
  return new Response(body, { status: r.status, headers });
};

/**
 * A `fetch` for `createClient({ fetch })` that answers from `routes`, with the calls it received and
 * the unmatched requests (with `unmatched: 'record'`).
 */
export const mockFetch = (
  routes: readonly MockRoute[],
  options: MockOptions = {},
): {
  fetch: (input: URL | Request | string, init?: RequestInit) => Promise<Response>;
  calls: MockCall[];
  unmatched: MockCall[];
} => {
  const router = createRouter(routes, options);
  const calls: MockCall[] = [];
  const unmatched: MockCall[] = [];
  const fetch = async (input: URL | Request | string, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    const body = request.body === null ? undefined : await request.clone().text();
    const m = router.match(request.method, url.pathname);
    const call: MockCall = {
      method: request.method,
      url: request.url,
      route: m && 'operation' in m.route ? m.route.operation : undefined,
      params: m?.params ?? {},
      body,
    };
    if (m === undefined) {
      if (options.unmatched === 'record') {
        unmatched.push(call);
        return new Response(null, { status: 501 });
      }
      throw new TypeError(`tsurara mock: no route for ${request.method} ${url.pathname}`);
    }
    calls.push(call);
    return respond(
      await m.route.handler({
        method: request.method,
        url,
        params: m.params,
        query: url.searchParams,
        headers: request.headers,
        body,
        request,
      }),
    );
  };
  return { fetch, calls, unmatched };
};
