/** Version of the OperationDef format. Must match the `v` emitted by the generator. */
export const OPERATION_DEF_VERSION = 1;

/**
 * How to parse a response body. `auto` decides from the actual Content-Type: JSON media types are
 * parsed as JSON, `text/*` is read as a string, and anything else as a `Blob` (wildcard media types).
 * `events` reads Server-Sent Events into an async iterable of `{ event, data, id?, retry? }`, and
 * `jsonLines` and `jsonSeq` read JSON Lines and JSON text sequences into an async iterable of values.
 */
export type ParseAs =
  | 'json'
  | 'text'
  | 'blob'
  | 'stream'
  | 'events'
  | 'jsonLines'
  | 'jsonSeq'
  | 'none'
  | 'auto';

/** OpenAPI parameter style. */
export type ParamStyle =
  | 'simple'
  | 'label'
  | 'matrix'
  | 'form'
  | 'spaceDelimited'
  | 'pipeDelimited'
  | 'deepObject';

export type ParamDef = {
  readonly style?: ParamStyle;
  readonly explode?: boolean;
  readonly allowReserved?: boolean;
  /** Parameter given with `content`. The value is serialized with this media type. */
  readonly contentType?: string;
  /** TypeDef of the value, for runtimes of other languages. Ignored here. */
  readonly type?: unknown;
};

export type ParamsDef = {
  readonly path?: { readonly [name: string]: ParamDef };
  readonly query?: { readonly [name: string]: ParamDef };
  readonly headers?: { readonly [name: string]: ParamDef };
  readonly cookies?: { readonly [name: string]: ParamDef };
  /** `in: querystring` from 3.2. */
  readonly querystring?: { readonly contentType: string };
};

export type EncodingDef = {
  readonly contentType?: string;
  readonly style?: ParamStyle;
  readonly explode?: boolean;
};

export type MediaDef = {
  readonly encoding?: { readonly [property: string]: EncodingDef };
  /** TypeDef of the body, for runtimes of other languages. Ignored here. */
  readonly type?: unknown;
};

export type BodyDef = {
  readonly required?: boolean;
  /** Definition per media type. The first key is the default media type. */
  readonly contents: { readonly [mediaType: string]: MediaDef };
};

export type ResponseDef = {
  readonly parseAs: ParseAs;
  /** Mapping used when there are several media types with different parse methods. */
  readonly contentTypes?: { readonly [mediaType: string]: ParseAs };
  /**
   * With `events`: the names of the events whose data is parsed as JSON, or `true` for every
   * event. The data of other events stays a string.
   */
  readonly jsonEvents?: true | readonly string[];
  /**
   * With `none`: a body that arrives with a Content-Type is read anyway, as `auto` reads it, for a
   * response the definition gives no content.
   */
  readonly readWithContentType?: boolean;
  /** TypeDefs of the body per media type, for runtimes of other languages. Ignored here. */
  readonly types?: { readonly [mediaType: string]: unknown };
};

/** A security scheme an authentication alternative uses (spec/operation-def.schema.json). */
export type SecurityUse = {
  /** The name in `components.securitySchemes`. */
  readonly scheme: string;
  readonly type: 'apiKey' | 'http' | 'oauth2' | 'openIdConnect' | 'mutualTLS';
  /** `apiKey`: where the key goes. */
  readonly in?: 'header' | 'query' | 'cookie';
  /** `apiKey`: the header, query or cookie name. */
  readonly name?: string;
  /** `http`: the Authorization scheme, such as `bearer` or `basic`. */
  readonly httpScheme?: string;
  readonly scopes?: readonly string[];
};

/** Runtime metadata of an operation. The generator emits one per operation. */
export type OperationDef = {
  readonly v: 1;
  readonly id: string;
  readonly method: string;
  /** Path template (`/pet/{petId}`). */
  readonly path: string;
  readonly tags?: readonly string[];
  readonly params?: ParamsDef;
  readonly body?: BodyDef;
  /** Definition per status (`200`, `2XX`, `default`). */
  readonly responses: { readonly [status: string]: ResponseDef };
  /** Authentication alternatives in order; an empty alternative means none. */
  readonly security?: readonly (readonly SecurityUse[])[];
};

/** Operation information available to middleware and onResult. */
export type OperationInfo = {
  readonly id: string;
  readonly method: string;
  readonly path: string;
  readonly tags: readonly string[];
};
