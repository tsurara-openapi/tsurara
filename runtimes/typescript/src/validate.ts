/**
 * Response validation through the Standard Schema interface
 * (https://standardschema.dev), which Zod (3.24 and later), Valibot and ArkType implement. The client
 * depends on none of them.
 */
import type { OperationInfo } from './def.ts';

/** A path segment of an issue, as Standard Schema gives it. */
export type ValidationPathSegment = PropertyKey | { readonly key: PropertyKey };

export type ValidationIssue = {
  readonly message: string;
  readonly path?: readonly ValidationPathSegment[] | undefined;
};

type StandardResult =
  | { readonly value: unknown; readonly issues?: undefined }
  | { readonly issues: readonly ValidationIssue[] };

/** The part of the Standard Schema interface the client uses. */
export type StandardSchema = {
  readonly '~standard': {
    readonly version: 1;
    readonly vendor: string;
    readonly validate: (value: unknown) => StandardResult | Promise<StandardResult>;
  };
};

/** What a validator is told about the body it checks. */
export type ValidationInfo = {
  readonly operation: OperationInfo;
  readonly status: number;
  /** The media type of the response, without parameters. */
  readonly mediaType: string;
};

/**
 * Checks a response body parsed as JSON (success and documented error bodies). Returns the issues,
 * or `undefined` when the body is valid or there is nothing to check it with.
 */
export type Validator = (
  value: unknown,
  info: ValidationInfo,
) => readonly ValidationIssue[] | undefined | Promise<readonly ValidationIssue[] | undefined>;

/** Schemas per operation id, status (`200`, `2XX`, `default`) and media type. */
export type ResponseSchemas = {
  readonly [operationId: string]: {
    readonly [status: string]: { readonly [mediaType: string]: StandardSchema };
  };
};

/**
 * A validator from schemas per operation, such as the `responseSchemas` that tsurara generates with
 * `typescript.validators: zod`. A response without a schema is not checked.
 */
export const validateResponses =
  (schemas: ResponseSchemas): Validator =>
  async (value, info) => {
    const byStatus = schemas[info.operation.id];
    if (!byStatus) return undefined;
    const keys = [String(info.status), `${Math.floor(info.status / 100)}XX`, 'default'];
    const byType = keys.map((k) => byStatus[k]).find((x) => x !== undefined);
    const schema = byType?.[info.mediaType] ?? byType?.['application/json'];
    if (!schema) return undefined;
    const result = await schema['~standard'].validate(value);
    return result.issues;
  };
