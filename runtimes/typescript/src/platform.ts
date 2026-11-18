/** Helpers for platforms other than browsers and Node. */

/** A file as React Native's `FormData` takes it: a local URI with a name and a media type. */
export type ReactNativeFile = {
  readonly uri: string;
  readonly name?: string;
  readonly type?: string;
};

/**
 * Passes a React Native file where a generated multipart part type expects `Blob | File`. The value
 * is sent unchanged: `FormData.append` in React Native uploads it as a file part.
 */
export const reactNativeFile = (file: ReactNativeFile): Blob => file as unknown as Blob;

/** Whether `v` is a React Native file object (recognized by a string `uri`). */
export const isReactNativeFile = (v: unknown): v is ReactNativeFile =>
  typeof v === 'object' && v !== null && typeof (v as { uri?: unknown }).uri === 'string';

/** `v instanceof C` for a global that may be missing on the platform. */
export const isInstance = (v: unknown, name: string): boolean => {
  const C = (globalThis as Record<string, unknown>)[name];
  return typeof C === 'function' && v instanceof (C as new (...args: never[]) => unknown);
};

/** The reason a timeout aborts with: a `DOMException` named `TimeoutError` where there is one. */
export const timeoutReason = (): Error =>
  typeof DOMException === 'undefined'
    ? Object.assign(new Error('timeout'), { name: 'TimeoutError' })
    : new DOMException('timeout', 'TimeoutError');

/** `AbortSignal.any`, or the same with listeners where it is missing. */
export const anySignal = (signals: AbortSignal[]): AbortSignal => {
  if (typeof AbortSignal.any === 'function') return AbortSignal.any(signals);
  const controller = new AbortController();
  for (const s of signals) {
    if (s.aborted) {
      controller.abort(s.reason);
      break;
    }
    s.addEventListener('abort', () => controller.abort(s.reason), { once: true });
  }
  return controller.signal;
};
