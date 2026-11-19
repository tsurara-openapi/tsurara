import type { BodyDef, EncodingDef, ParamDef, ParamStyle, ParamsDef } from './def.ts';
import { isInstance, isReactNativeFile } from './platform.ts';

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' &&
  v !== null &&
  !Array.isArray(v) &&
  !isInstance(v, 'Blob') &&
  !isReactNativeFile(v);

const toText = (v: unknown): string => {
  if (v === null) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean' || typeof v === 'bigint') return String(v);
  if (v instanceof Date) return v.toISOString();
  return JSON.stringify(v);
};

const encode = (v: string, allowReserved: boolean | undefined): string =>
  allowReserved
    ? encodeURI(v).replace(/[#?&=+]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)
    : encodeURIComponent(v);

const entries = (v: Record<string, unknown>): [string, unknown][] =>
  Object.entries(v).filter(([, x]) => x !== undefined);

/** Serializes the value of a parameter given with `content`. */
const contentValue = (value: unknown, contentType: string): string =>
  /json/i.test(contentType) ? JSON.stringify(value) : toText(value);

/** Expands one path parameter (simple, label, matrix). */
export const serializePathParam = (name: string, value: unknown, def: ParamDef = {}): string => {
  const style: ParamStyle = def.style ?? 'simple';
  const explode = def.explode ?? false;
  const enc = (s: string) => encode(s, def.allowReserved);
  if (def.contentType) return enc(contentValue(value, def.contentType));
  const prefix = style === 'label' ? '.' : style === 'matrix' ? ';' : '';
  if (Array.isArray(value)) {
    const items = value.map((v) => enc(toText(v)));
    if (style === 'matrix') {
      return explode ? items.map((i) => `;${name}=${i}`).join('') : `;${name}=${items.join(',')}`;
    }
    const sep = explode && style === 'label' ? '.' : ',';
    return prefix + items.join(sep);
  }
  if (isObject(value)) {
    const kv = entries(value);
    if (explode) {
      const sep = style === 'label' ? '.' : style === 'matrix' ? ';' : ',';
      return prefix + kv.map(([k, v]) => `${enc(k)}=${enc(toText(v))}`).join(sep);
    }
    const flat = kv.flatMap(([k, v]) => [enc(k), enc(toText(v))]).join(',');
    return style === 'matrix' ? `;${name}=${flat}` : prefix + flat;
  }
  const text = enc(toText(value));
  return style === 'matrix' ? `;${name}=${text}` : prefix + text;
};

/** A segment that URL parsing would remove or merge: empty, `.` or `..` (also percent-encoded). */
const isDotOrEmpty = (segment: string): boolean => {
  const s = segment.replace(/%2e/gi, '.');
  return s === '' || s === '.' || s === '..';
};

/**
 * Embeds path parameters into the path template. Throws a `TypeError` when the template does not
 * start with `/` (appended to a base URL, it could change the host or port), when a parameter is
 * missing, or when its value would make a whole segment empty, `.` or `..`: URL parsing removes dot
 * segments, so such a value would call another endpoint (spec/README.md).
 */
export const expandPath = (
  template: string,
  values: Record<string, unknown> | undefined,
  defs: ParamsDef['path'],
): string => {
  if (!template.startsWith('/')) {
    throw new TypeError(`tsurara: the path ${JSON.stringify(template)} does not start with /`);
  }
  return template
    .split('/')
    .map((segment) => {
      if (!segment.includes('{')) return segment;
      const expanded = segment.replace(/\{([^}]+)\}/g, (_, name: string) => {
        const value = values?.[name];
        if (value === undefined)
          throw new TypeError(`tsurara: the path parameter ${name} is missing`);
        return serializePathParam(name, value, defs?.[name]);
      });
      if (expanded.split('/').some(isDotOrEmpty)) {
        throw new TypeError(
          `tsurara: a path parameter value makes the segment ${JSON.stringify(expanded)}, which would change the path`,
        );
      }
      return expanded;
    })
    .join('/');
};

/** Turns one query parameter into a list of encoded `key=value` strings. */
export const serializeQueryParam = (name: string, value: unknown, def: ParamDef = {}): string[] => {
  if (value === undefined) return [];
  const style: ParamStyle = def.style ?? 'form';
  const explode = def.explode ?? style === 'form';
  const enc = (s: string) => encode(s, def.allowReserved);
  const key = encodeURIComponent(name);
  if (def.contentType) return [`${key}=${enc(contentValue(value, def.contentType))}`];
  if (style === 'deepObject' && isObject(value)) {
    const out: string[] = [];
    const walk = (prefix: string, v: unknown) => {
      if (isObject(v)) {
        for (const [k, x] of entries(v)) walk(`${prefix}[${k}]`, x);
      } else if (Array.isArray(v)) {
        for (const x of v) out.push(`${encodeURIComponent(prefix)}=${enc(toText(x))}`);
      } else {
        out.push(`${encodeURIComponent(prefix)}=${enc(toText(v))}`);
      }
    };
    walk(name, value);
    return out;
  }
  if (Array.isArray(value)) {
    const items = value.map((v) => enc(toText(v)));
    if (explode) return items.map((i) => `${key}=${i}`);
    const sep = style === 'spaceDelimited' ? '%20' : style === 'pipeDelimited' ? '|' : ',';
    return [`${key}=${items.join(sep)}`];
  }
  if (isObject(value)) {
    const kv = entries(value);
    if (explode) return kv.map(([k, v]) => `${encodeURIComponent(k)}=${enc(toText(v))}`);
    const sep = style === 'spaceDelimited' ? '%20' : style === 'pipeDelimited' ? '|' : ',';
    return [`${key}=${kv.flatMap(([k, v]) => [enc(k), enc(toText(v))]).join(sep)}`];
  }
  return [`${key}=${enc(toText(value))}`];
};

/**
 * Builds the query string (without the leading `?`). Parameters whose value is `null` are left out,
 * like missing ones: RFC 6570, which the parameter styles come from, treats `null` as undefined.
 */
export const buildQuery = (
  values: Record<string, unknown> | undefined,
  defs: ParamsDef['query'],
): string => {
  if (!values) return '';
  return entries(values)
    .filter(([, v]) => v !== null)
    .flatMap(([name, v]) => serializeQueryParam(name, v, defs?.[name]))
    .join('&');
};

/** `in: querystring` from 3.2. Serializes the whole query string as one value. */
export const buildQueryString = (value: unknown, contentType: string): string => {
  if (value === undefined) return '';
  if (/x-www-form-urlencoded/i.test(contentType) && isObject(value)) {
    return entries(value)
      .filter(([, v]) => v !== null)
      .flatMap(([k, v]) => serializeQueryParam(k, v, { style: 'form', explode: true }))
      .join('&');
  }
  return encodeURIComponent(contentValue(value, contentType));
};

/** Value of a header parameter (simple). */
export const serializeHeaderParam = (value: unknown, def: ParamDef = {}): string => {
  if (def.contentType) return contentValue(value, def.contentType);
  const explode = def.explode ?? false;
  if (Array.isArray(value)) return value.map(toText).join(',');
  if (isObject(value)) {
    const kv = entries(value);
    return explode
      ? kv.map(([k, v]) => `${k}=${toText(v)}`).join(',')
      : kv.flatMap(([k, v]) => [k, toText(v)]).join(',');
  }
  return toText(value);
};

/** Value of the Cookie header (form). */
export const buildCookie = (
  values: Record<string, unknown> | undefined,
  defs: ParamsDef['cookies'],
): string => {
  if (!values) return '';
  const parts: string[] = [];
  for (const [name, v] of entries(values)) {
    const def = defs?.[name] ?? {};
    const explode = def.explode ?? true;
    if (def.contentType) {
      parts.push(`${name}=${encodeURIComponent(contentValue(v, def.contentType))}`);
    } else if (Array.isArray(v)) {
      if (explode) for (const x of v) parts.push(`${name}=${encodeURIComponent(toText(x))}`);
      else parts.push(`${name}=${v.map((x) => encodeURIComponent(toText(x))).join(',')}`);
    } else if (isObject(v)) {
      if (explode) {
        for (const [k, x] of entries(v)) parts.push(`${k}=${encodeURIComponent(toText(x))}`);
      } else {
        const flat = entries(v).flatMap(([k, x]) => [k, encodeURIComponent(toText(x))]);
        parts.push(`${name}=${flat.join(',')}`);
      }
    } else {
      parts.push(`${name}=${encodeURIComponent(toText(v))}`);
    }
  }
  return parts.join('; ');
};

export type EncodedBody = {
  body: BodyInit | null;
  /** Content-Type to set. `null` for multipart, which is left to fetch. */
  contentType: string | null;
};

const mediaTypeOf = (contentType: string): string =>
  (contentType.split(';')[0] ?? '').trim().toLowerCase();

const isWildcard = (mt: string): boolean => mt.includes('*');

/** Values that are sent as raw bytes. */
const isBinary = (v: unknown): v is Blob | ArrayBuffer | ArrayBufferView | ReadableStream =>
  isInstance(v, 'Blob') ||
  v instanceof ArrayBuffer ||
  ArrayBuffer.isView(v) ||
  isInstance(v, 'ReadableStream');

/** Appends one multipart value to the FormData. */
const appendPart = (form: FormData, name: string, value: unknown, enc: EncodingDef | undefined) => {
  if (value === undefined) return;
  if (isInstance(value, 'Blob')) {
    form.append(name, value as Blob);
  } else if (isReactNativeFile(value)) {
    // React Native's FormData uploads `{ uri, name, type }` as a file part.
    form.append(name, value as unknown as Blob);
  } else if (enc?.contentType && (isObject(value) || Array.isArray(value))) {
    form.append(name, new Blob([JSON.stringify(value)], { type: enc.contentType }));
  } else if (isObject(value)) {
    // A part with a Content-Type would have to be a Blob, which FormData sends as a file (with a
    // file name); servers such as PHP then take it for an upload. Without an `encoding` that asks
    // for a media type, an object is sent as a field with its JSON text.
    form.append(name, JSON.stringify(value));
  } else if (enc?.contentType && /json/i.test(enc.contentType)) {
    form.append(name, new Blob([JSON.stringify(value)], { type: enc.contentType }));
  } else {
    form.append(name, toText(value));
  }
};

/** Builds the request body. */
export const encodeBody = (
  value: unknown,
  def: BodyDef,
  requestedType: string | undefined,
): EncodedBody => {
  const contentType = requestedType ?? Object.keys(def.contents)[0] ?? 'application/json';
  const mt = mediaTypeOf(contentType);
  const media = def.contents[contentType] ?? def.contents[mt];
  const encoding = media?.encoding ?? {};
  if (mt.startsWith('multipart/')) {
    if (isInstance(value, 'FormData')) return { body: value as FormData, contentType: null };
    const form = new FormData();
    if (isObject(value)) {
      for (const [name, v] of entries(value)) {
        if (Array.isArray(v) && !encoding[name]?.contentType) {
          for (const item of v) appendPart(form, name, item, encoding[name]);
        } else {
          appendPart(form, name, v, encoding[name]);
        }
      }
    }
    return { body: form, contentType: null };
  }
  if (mt === 'application/x-www-form-urlencoded') {
    if (isInstance(value, 'URLSearchParams'))
      return { body: value as URLSearchParams, contentType };
    const parts = isObject(value)
      ? entries(value).flatMap(([name, v]) => {
          const e = encoding[name];
          return serializeQueryParam(name, v, {
            style: e?.style ?? 'form',
            explode: e?.explode ?? true,
            ...(e?.contentType ? { contentType: e.contentType } : {}),
          });
        })
      : [];
    return { body: parts.join('&').replace(/%20/g, '+'), contentType };
  }
  if (mt.startsWith('text/')) {
    return { body: toText(value), contentType };
  }
  if (isWildcard(mt)) {
    // A wildcard body: binary values are sent as is, with a Blob's type as the Content-Type;
    // anything else is sent as JSON (design.md 8.5).
    if (isBinary(value)) {
      const type = isInstance(value, 'Blob') && (value as Blob).type ? (value as Blob).type : null;
      return { body: value as BodyInit, contentType: type };
    }
    return { body: JSON.stringify(value), contentType: 'application/json' };
  }
  if (mt === 'application/json' || mt.endsWith('+json')) {
    if (isBinary(value) || isInstance(value, 'FormData')) {
      // JSON.stringify would silently turn these into `{}`.
      throw new TypeError(
        `${Object.prototype.toString.call(value)} cannot be sent as a ${mt} body`,
      );
    }
    return { body: JSON.stringify(value), contentType };
  }
  // Binary.
  const body = value as BodyInit;
  return { body, contentType };
};
