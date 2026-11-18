import type { SecurityUse } from './def.ts';

/** Where the credentials of an operation go. */
export type Credentials = {
  readonly headers: [string, string][];
  readonly query: string[];
  readonly cookies: string[];
};

/** Base64 of the UTF-8 bytes of `text`. */
const base64 = (text: string): string => {
  let binary = '';
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary);
};

/**
 * The credentials for the first alternative of `security` whose every scheme gets one from `get`.
 * An empty alternative, or none that is complete, sends nothing (spec/README.md).
 */
export const authenticate = async (
  security: readonly (readonly SecurityUse[])[] | undefined,
  get: (scheme: SecurityUse) => string | undefined | Promise<string | undefined>,
): Promise<Credentials> => {
  const out: Credentials = { headers: [], query: [], cookies: [] };
  for (const alternative of security ?? []) {
    const values: [SecurityUse, string][] = [];
    for (const scheme of alternative) {
      // mutualTLS is handled by the transport.
      if (scheme.type === 'mutualTLS') continue;
      const value = await get(scheme);
      if (value === undefined) break;
      values.push([scheme, value]);
    }
    const needed = alternative.filter((s) => s.type !== 'mutualTLS').length;
    if (values.length < needed) continue;
    for (const [scheme, value] of values) place(out, scheme, value);
    return out;
  }
  return out;
};

const place = (out: Credentials, scheme: SecurityUse, value: string): void => {
  if (scheme.type === 'apiKey') {
    const name = scheme.name ?? scheme.scheme;
    if (scheme.in === 'query') {
      out.query.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);
    } else if (scheme.in === 'cookie') {
      out.cookies.push(`${name}=${encodeURIComponent(value)}`);
    } else {
      out.headers.push([name, value]);
    }
    return;
  }
  const httpScheme = (
    scheme.type === 'http' ? (scheme.httpScheme ?? 'bearer') : 'bearer'
  ).toLowerCase();
  if (httpScheme === 'basic') {
    out.headers.push(['authorization', `Basic ${base64(value)}`]);
  } else if (httpScheme === 'bearer') {
    out.headers.push(['authorization', `Bearer ${value}`]);
  } else {
    out.headers.push(['authorization', `${scheme.httpScheme} ${value}`]);
  }
};
