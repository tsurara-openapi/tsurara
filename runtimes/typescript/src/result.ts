import type { ValidationIssue } from './validate.ts';

/** Failures before HTTP, and exceptions from user code. Included in every operation's Result. */
export type TransportFailure =
  | { ok: false; kind: 'network'; cause: unknown }
  | { ok: false; kind: 'timeout'; cause: unknown }
  | { ok: false; kind: 'aborted'; cause: unknown }
  | {
      ok: false;
      kind: 'parse';
      cause: unknown;
      response: Response;
      contentType: string | null;
    }
  | { ok: false; kind: 'unexpected'; cause: unknown }
  | {
      /** The body does not match its schema (`validate`). */
      ok: false;
      kind: 'validation';
      status: number;
      /** The parsed body that failed validation. */
      value: unknown;
      issues: readonly ValidationIssue[];
      response: Response;
    };

export type SuccessResult = {
  ok: true;
  kind: 'success';
  status: number;
  data: unknown;
  response: Response;
};

export type HttpErrorResult = {
  ok: false;
  kind: 'http';
  documented: boolean;
  status: number;
  error: unknown;
  response: Response;
};

/** A type that every operation's Result is assignable to. Accepted by onResult and unwrap. */
export type AnyResult = SuccessResult | HttpErrorResult | TransportFailure;

export type ResultKind = AnyResult['kind'];
