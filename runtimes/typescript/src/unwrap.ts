import { type Client, operationOf } from './client.ts';
import type { AnyResult } from './result.ts';

/** A failed Result. */
export type FailureResult = Exclude<AnyResult, { ok: true }>;

/** The failure members of a Result. */
export type FailureOf<R> = Exclude<R, { ok: true }>;

/** The `data` type of the success members of a Result. */
export type SuccessData<R> = Extract<R, { ok: true }> extends { data: infer D } ? D : never;

/**
 * Thrown by `unwrap` for a failed Result. The type parameter is the operation's Result type (such as
 * a generated `GetPetResult`): `ApiError<GetPetResult>` narrows `result` to that operation's failures,
 * so a documented error body has its schema type. The default is the generic failure.
 */
export class ApiError<R extends AnyResult = AnyResult> extends Error {
  readonly kind: FailureOf<R>['kind'];
  /** The HTTP status, when there was a response. */
  readonly status: number | undefined;
  readonly operationId: string | undefined;
  /** The original Result. */
  readonly result: FailureOf<R>;

  /** `operationId` defaults to the operation that returned `result`. */
  constructor(result: FailureOf<R>, operationId: string | undefined = operationOf(result)?.id) {
    const failure: FailureResult = result;
    const status = 'status' in failure ? failure.status : undefined;
    const cause = 'cause' in failure ? failure.cause : undefined;
    const what = failure.kind === 'http' ? `HTTP ${status}` : failure.kind;
    super(
      `tsurara: ${operationId ?? 'the operation'} failed with ${what}`,
      cause === undefined ? undefined : { cause },
    );
    this.name = 'ApiError';
    this.kind = result.kind;
    this.status = status;
    this.operationId = operationId;
    this.result = result;
  }
}

/**
 * Returns `data` of a successful Result, and throws an `ApiError` otherwise. A value that is not a
 * Result (such as the Promise of a call that was not awaited) throws a `TypeError`.
 */
export const unwrap = <R extends AnyResult>(result: R): SuccessData<R> => {
  const value: unknown = result;
  if (typeof (value as { then?: unknown } | null)?.then === 'function') {
    throw new TypeError(
      'tsurara: unwrap() received a Promise; await the call first: unwrap(await call(...))',
    );
  }
  if (
    typeof value !== 'object' ||
    value === null ||
    typeof (value as { ok?: unknown }).ok !== 'boolean' ||
    typeof (value as { kind?: unknown }).kind !== 'string'
  ) {
    throw new TypeError(
      `tsurara: unwrap() received a value that is not a Result: ${String(value)}`,
    );
  }
  if (result.ok) return result.data as SuccessData<R>;
  throw new ApiError<R>(result as FailureOf<R>);
};

type DocumentedHttp<R> = Extract<R, { kind: 'http'; documented: true }>;

/**
 * The statuses `isHttpError` accepts: the documented error statuses of a Result, and any number when
 * a `default` (or range) response covers the others.
 */
export type DocumentedErrorStatus<R> = DocumentedHttp<R> extends { status: infer S } ? S : never;

/**
 * The member `isHttpError(result, S)` narrows to: the member documented for `S`, or else the members
 * for `default` and ranges (`status: number`) with their status set to `S`.
 */
export type HttpErrorOf<R, S extends number> = [Extract<DocumentedHttp<R>, { status: S }>] extends [
  never,
]
  ? DocumentedHttp<R> extends infer M
    ? M extends { status: number }
      ? number extends M['status']
        ? M & { status: S }
        : never
      : never
    : never
  : Extract<DocumentedHttp<R>, { status: S }>;

/**
 * Whether `result` is the documented error of `status`, narrowing it so that `error` has the type of
 * that status (or of `default` for a status it covers). Checking `status` alone cannot narrow,
 * because the member for undocumented statuses has `status: number`; this also checks `documented`.
 */
export const isHttpError = <R extends AnyResult, const S extends DocumentedErrorStatus<R> & number>(
  result: R,
  status: S,
): result is R & HttpErrorOf<R, S> =>
  result.kind === 'http' && result.documented && result.status === status;

/**
 * A call function with the client argument bound. `Client` is invariant in `Ctx`, so a conditional type
 * cannot instantiate a generated `<Ctx>(client: Client<Ctx>, options) => …` function with the client's
 * `Ctx`. The options are read with `Ctx` left open, and `context` is typed again from the client.
 */
export type Bound<Ctx, F> = F extends (client: never, ...args: infer A) => infer R
  ? (...args: WithContext<A, Ctx>) => R
  : never;

type WithContext<A, Ctx> = A extends [options: infer O]
  ? [options: Rebind<O, Ctx>]
  : A extends [options?: infer O]
    ? [options?: Rebind<O, Ctx>]
    : A;

type Rebind<O, Ctx> = Omit<NonNullable<O>, 'context'> & { context?: Ctx };

/** Binds `client` as the first argument of generated call functions: `api.getPet(options)`. */
export const bindClient = <
  Ctx,
  const F extends { readonly [name: string]: (client: Client<Ctx>, ...args: never[]) => unknown },
>(
  client: Client<Ctx>,
  functions: F,
): { [K in keyof F]: Bound<Ctx, F[K]> } => {
  const bound: Record<string, unknown> = {};
  for (const [name, f] of Object.entries(functions)) {
    bound[name] = (...args: never[]) => f(client, ...args);
  }
  return bound as { [K in keyof F]: Bound<Ctx, F[K]> };
};
