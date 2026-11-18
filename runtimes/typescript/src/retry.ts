import { type Middleware, type MiddlewareRequest, TransportError } from './client.ts';
import { isInstance } from './platform.ts';
import { ApiError } from './unwrap.ts';

export type RetryOptions = {
  /** Number of retries after the first attempt. Defaults to 2. */
  max?: number;
  /** Methods that are retried. Defaults to the idempotent methods GET, HEAD, OPTIONS, PUT, and DELETE. */
  methods?: readonly string[];
  /** Response statuses that are retried. Defaults to 408, 429, 500, 502, 503, and 504. */
  statuses?: readonly number[];
  /** First backoff in milliseconds, doubled on every retry. Defaults to 100. */
  baseDelay?: number;
  /** The longest wait in milliseconds, also for `Retry-After`. Defaults to 30 000. */
  maxDelay?: number;
  /**
   * Milliseconds to wait before retry number `attempt` (starting at 1). `response` is the response that
   * is being retried, or `undefined` after a network failure. Defaults to `Retry-After` when the
   * response has it, and otherwise to exponential backoff with full jitter (a random wait up to
   * `baseDelay` doubled per retry), both capped by `maxDelay`.
   */
  delay?: (attempt: number, response: Response | undefined) => number;
  /** Random numbers in [0, 1) for the jitter. For tests. */
  random?: () => number;
  /** Waits; rejects with the signal's reason when it is aborted. For tests. */
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
};

const DEFAULT_METHODS = ['GET', 'HEAD', 'OPTIONS', 'PUT', 'DELETE'];
/** The statuses `retry()` and `queryRetry()` retry by default. */
export const RETRY_STATUSES: readonly number[] = [408, 429, 500, 502, 503, 504];

/**
 * How many times `retry()` sent a request it gave up on, by the last response or by the cause of the
 * last network failure. `queryRetry()` reads it so that two retrying layers do not multiply.
 */
const attempts = new WeakMap<object, number>();

const record = (key: unknown, sent: number): void => {
  if (sent > 1 && typeof key === 'object' && key !== null) attempts.set(key, sent);
};

/** How many times the request of a failed call was sent: more than 1 when `retry()` retried it. */
export const sentTimes = (error: ApiError): number => {
  const result: { response?: unknown; cause?: unknown } = error.result;
  for (const key of [result.response, result.cause]) {
    if (typeof key === 'object' && key !== null) {
      const sent = attempts.get(key);
      if (sent !== undefined) return sent;
    }
  }
  return 1;
};

/** `Retry-After` in milliseconds: seconds or an HTTP date. */
export const retryAfter = (
  response: Response | undefined,
  now: number = Date.now(),
): number | undefined => {
  const value = response?.headers.get('retry-after')?.trim();
  if (!value) return undefined;
  if (/^\d+$/.test(value)) return Number(value) * 1000;
  const at = Date.parse(value);
  return Number.isNaN(at) ? undefined : Math.max(0, at - now);
};

/**
 * Middleware that retries network failures and retryable statuses. The request is rebuilt from the
 * original on every attempt, so changes made by inner middleware do not pile up. Stream bodies cannot
 * be sent twice and are never retried. The client's timeout covers all attempts together: a wait that
 * would end after it is not started, and the last response is returned. Nothing is retried after a
 * timeout or an abort.
 */
export const retry = <Ctx = unknown>(options: RetryOptions = {}): Middleware<Ctx> => {
  const max = options.max ?? 2;
  const methods = new Set((options.methods ?? DEFAULT_METHODS).map((m) => m.toUpperCase()));
  const statuses = new Set(options.statuses ?? RETRY_STATUSES);
  const baseDelay = options.baseDelay ?? 100;
  const maxDelay = options.maxDelay ?? 30_000;
  const random = options.random ?? Math.random;
  const sleep = options.sleep ?? wait;
  const delay =
    options.delay ??
    ((attempt: number, response: Response | undefined) => {
      const after = retryAfter(response);
      if (after !== undefined) return Math.min(after, maxDelay);
      return Math.floor(random() * Math.min(maxDelay, baseDelay * 2 ** (attempt - 1)));
    });
  const fits = (ms: number, deadline: number | undefined) =>
    deadline === undefined || Date.now() + ms < deadline;
  return async (req, next) => {
    if (!methods.has(req.method.toUpperCase()) || isInstance(req.body, 'ReadableStream')) {
      return next(req);
    }
    for (let attempt = 0; ; attempt++) {
      const copy: MiddlewareRequest<Ctx> = {
        ...req,
        url: new URL(req.url),
        headers: new Headers(req.headers),
      };
      let response: Response;
      try {
        response = await next(copy);
      } catch (e) {
        const retryable = e instanceof TransportError && e.kind === 'network';
        const giveUp = (): never => {
          if (e instanceof TransportError) record(e.cause, attempt + 1);
          throw e;
        };
        if (!retryable || attempt >= max || req.signal.aborted) giveUp();
        const ms = delay(attempt + 1, undefined);
        if (!fits(ms, req.deadline)) giveUp();
        await sleep(ms, req.signal);
        continue;
      }
      if (!statuses.has(response.status) || attempt >= max || req.signal.aborted) {
        if (statuses.has(response.status)) record(response, attempt + 1);
        return response;
      }
      const ms = delay(attempt + 1, response);
      if (!fits(ms, req.deadline)) {
        record(response, attempt + 1);
        return response;
      }
      await response.body?.cancel();
      await sleep(ms, req.signal);
    }
  };
};

/** Waits `ms` milliseconds, and throws the abort reason if the signal is aborted meanwhile. */
const wait = (ms: number, signal: AbortSignal): Promise<void> =>
  new Promise<void>((resolve, reject) => {
    const done = () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', done);
      if (signal.aborted) reject(signal.reason);
      else resolve();
    };
    const timer = setTimeout(done, Math.max(0, ms));
    signal.addEventListener('abort', done);
    if (signal.aborted) done();
  });

export type QueryRetryOptions = {
  /** Number of retries after the first attempt. Defaults to 2, as in `retry()`. */
  max?: number;
  /** Response statuses that are retried. Defaults to `RETRY_STATUSES`, as in `retry()`. */
  statuses?: readonly number[];
};

/**
 * The `retry` option for TanStack Query: retries what the `retry()` middleware retries (network
 * failures and its statuses) and nothing else, so documented errors such as 404 or 422, parse and
 * validation failures, timeouts and aborts fail at once. A failure the middleware already retried is
 * not retried again, so with both layers a request is sent at most as often as the middleware allows.
 */
export const queryRetry = (
  options: QueryRetryOptions = {},
): ((failureCount: number, error: unknown) => boolean) => {
  const max = options.max ?? 2;
  const statuses = new Set(options.statuses ?? RETRY_STATUSES);
  return (failureCount, error) => {
    if (failureCount >= max || !(error instanceof ApiError)) return false;
    // The `retry()` middleware already retried it: one layer is enough.
    if (sentTimes(error) > 1) return false;
    if (error.kind === 'network') return true;
    return error.kind === 'http' && error.status !== undefined && statuses.has(error.status);
  };
};
