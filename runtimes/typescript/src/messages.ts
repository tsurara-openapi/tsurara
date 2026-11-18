/**
 * Messages of event channels (`x-event-channels`): values a server pushes outside any
 * operation, such as the updates of a Mercure hub received through `EventSource`. The client does not
 * own the connection; `decodeMessage` turns the text of one message into a typed value, so it works
 * with `EventSource`, `parseEventStream` and WebSockets alike.
 */
import type { StandardSchema, ValidationIssue } from './validate.ts';

/** The description of a channel that generated code passes to `decodeMessage`. */
export type ChannelDef = {
  readonly v: 1;
  /** The channel's name in the definition. */
  readonly id: string;
  /** The topic template, relative to the deployment's topic prefix. */
  readonly channel: string;
  /** How the text of a message is read. */
  readonly parseAs: 'json' | 'text';
};

/** Checks a decoded message of `channel`; returns its issues, or `undefined` when it is valid. */
export type MessageValidator = (
  value: unknown,
  channel: string,
) => readonly ValidationIssue[] | undefined;

export type DecodeOptions = {
  /** Validates the decoded value, such as `validateChannels(channelSchemas)`. */
  readonly validate?: MessageValidator | undefined;
};

export type MessageResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly kind: 'parse'; readonly data: string; readonly cause: unknown }
  | {
      readonly ok: false;
      readonly kind: 'validation';
      readonly value: unknown;
      readonly issues: readonly ValidationIssue[];
    };

/** Decodes the text of one message of `def`'s channel. Never throws for bad input. */
export const decodeMessage = <T>(
  def: ChannelDef,
  data: string,
  options?: DecodeOptions,
): MessageResult<T> => {
  let value: unknown = data;
  if (def.parseAs === 'json') {
    try {
      value = JSON.parse(data);
    } catch (cause) {
      return { ok: false, kind: 'parse', data, cause };
    }
  }
  const issues = options?.validate?.(value, def.id);
  if (issues !== undefined && issues.length > 0) {
    return { ok: false, kind: 'validation', value, issues };
  }
  return { ok: true, value: value as T };
};

/** Schemas per channel name, such as the generated `channelSchemas`. */
export type ChannelSchemas = { readonly [channel: string]: StandardSchema };

/**
 * A validator from Standard Schema schemas per channel (Zod, Valibot, ArkType). Channels without a
 * schema are not checked. Decoding is synchronous, so asynchronous schemas throw.
 */
export const validateChannels =
  (schemas: ChannelSchemas): MessageValidator =>
  (value, channel) => {
    const schema = Object.hasOwn(schemas, channel) ? schemas[channel] : undefined;
    if (schema === undefined) return undefined;
    const result = schema['~standard'].validate(value);
    if (result instanceof Promise) {
      throw new TypeError(
        `tsurara: the schema of channel ${channel} is asynchronous; messages are decoded synchronously`,
      );
    }
    return result.issues;
  };
