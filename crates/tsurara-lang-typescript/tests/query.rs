//! TanStack Query options and query keys (`typescript.query: tanstack`).

use std::path::Path;
use tsurara_lang_typescript::options::Query;
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes};

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    DocumentSet::load(&url, &f as &dyn Fetcher).unwrap()
}

fn files(name: &str, opts: &TypeScriptOptions) -> Vec<(String, String)> {
    generate(&load(name), &Options::default(), opts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn tanstack() -> TypeScriptOptions {
    TypeScriptOptions {
        query: Some(Query::Tanstack),
        ..TypeScriptOptions::default()
    }
}

#[test]
fn without_the_option_nothing_mentions_tanstack() {
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        for (path, contents) in files(name, &TypeScriptOptions::default()) {
            assert!(
                !contents.contains("@tanstack") && !path.starts_with("query/"),
                "{name} {path}"
            );
        }
    }
}

#[test]
fn queries_for_reads_and_mutations_for_writes() {
    let files = files("v3.0-petstore.yaml", &tanstack());
    let get = |p: &str| {
        &files
            .iter()
            .find(|(path, _)| path == p)
            .unwrap_or_else(|| panic!("{p}"))
            .1
    };
    let show = get("query/showPetById.ts");
    assert!(
        show.contains("export function showPetByIdQueryKey("),
        "{show}"
    );
    assert!(
        show.contains("export function showPetByIdQueryOptions<Ctx>("),
        "{show}"
    );
    assert!(show.contains("['showPetById', data]"), "{show}");
    let list = get("query/listPets.ts");
    assert!(list.contains("data: ListPetsData = {}"), "{list}");
    let create = get("query/createPets.ts");
    assert!(
        create.contains("export function createPetsMutationOptions<Ctx>("),
        "{create}"
    );
    assert!(!create.contains("QueryKey"), "{create}");
    // The types and functions never import the query files.
    for (path, contents) in &files {
        if !path.starts_with("query/") && path != "index.ts" {
            assert!(!contents.contains("@tanstack"), "{path}");
        }
    }
    insta::assert_snapshot!(format!("{show}\n{create}"));
}

/// The type test of the issue: data, error and keys are typed for `useQuery`, `useSuspenseQuery`,
/// `useMutation`, `setQueryData` and `getQueryData`, with TypeScript 5.8 and 7.
const USAGE: &str = r#"import { QueryClient, useMutation, useQuery, useSuspenseQuery } from '@tanstack/react-query';
import { type ApiError, createClient } from '@tsurara/client';
import type { CreatePetsResult } from './operations/createPets.js';
import type { ShowPetByIdResult } from './operations/showPetById.js';
import { createPetsMutationOptions } from './query/createPets.js';
import { listPetsQueryKey, listPetsQueryOptions } from './query/listPets.js';
import { showPetByIdQueryKey, showPetByIdQueryOptions } from './query/showPetById.js';
import type { Pet } from './schemas/Pet.js';
import type { Pets } from './schemas/Pets.js';

const api = createClient({ baseUrl: 'https://petstore.example.com' });
const qc = new QueryClient();

export function usePet(petId: string): Pet | undefined {
  const q = useQuery(showPetByIdQueryOptions(api, { path: { petId } }));
  const error: ApiError<ShowPetByIdResult> | null = q.error;
  void error;
  return q.data;
}

export function usePets(): Pets {
  return useSuspenseQuery(listPetsQueryOptions(api)).data;
}

export function useCreate(pet: Pet): void {
  const m = useMutation(createPetsMutationOptions(api));
  const error: ApiError<CreatePetsResult> | null = m.error;
  void error;
  m.mutate({ body: pet });
}

export function rename(petId: string, name: string): Pet | undefined {
  const key = showPetByIdQueryKey({ path: { petId } });
  qc.setQueryData(key, (old) => {
    const pet: Pet | undefined = old;
    return pet && { ...pet, name };
  });
  void qc.invalidateQueries({ queryKey: ['showPetById'] });
  void qc.invalidateQueries({ queryKey: listPetsQueryKey() });
  const cached = qc.getQueryData(key);
  return cached;
}

// @ts-expect-error The data of showPetById is a Pet.
export const wrong: number | undefined = qc.getQueryData<Pet>(showPetByIdQueryKey({ path: { petId: '1' } }));
"#;

#[test]
fn query_options_type_check_with_react_query() {
    let mut files = files("v3.0-petstore.yaml", &tanstack());
    files.push(("usage.ts".into(), USAGE.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

#[test]
fn every_definition_type_checks_with_query_options() {
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        assert_tsc_passes(&files(name, &tanstack()), &TsOptions::default());
    }
}

/// Everyday TanStack Query code written with the generated options and no annotations:
/// `select`, `initialData`, `skipToken`, `useQueries`, the `QueryClient` methods a router loader uses,
/// and the context `onMutate` returns. Each check is an exact type equality; the `@ts-expect-error`
/// lines prove the types are not `any`.
const INFERENCE: &str = r#"import {
  QueryClient,
  skipToken,
  useMutation,
  useQueries,
  useQuery,
  useSuspenseQuery,
} from '@tanstack/react-query';
import { createClient } from '@tsurara/client';
import { createPetsMutationOptions } from './query/createPets.js';
import { showPetByIdQueryKey, showPetByIdQueryOptions } from './query/showPetById.js';
import type { Pet } from './schemas/Pet.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};

const api = createClient({ baseUrl: 'https://petstore.example.com' });
const qc = new QueryClient();
const pet: Pet = { id: 1, name: 'a' };
const options = showPetByIdQueryOptions(api, { path: { petId: '1' } });

export function select(): void {
  const q = useQuery({ ...options, select: (p) => p.name });
  expect<Equal<typeof q.data, string | undefined>>();
  // @ts-expect-error `select` decides the data.
  expect<Equal<typeof q.data, Pet | undefined>>();
}

export function initialData(): void {
  const q = useQuery({ ...options, initialData: pet });
  expect<Equal<typeof q.data, Pet>>();
  const plain = useQuery(options);
  // @ts-expect-error Without initial data, the data may be undefined.
  expect<Equal<typeof plain.data, Pet>>();
}

export function skipped(petId: string | undefined): void {
  const q = useQuery(showPetByIdQueryOptions(api, petId === undefined ? skipToken : { path: { petId } }));
  expect<Equal<typeof q.data, Pet | undefined>>();
  // @ts-expect-error A suspense query cannot be skipped.
  useSuspenseQuery(showPetByIdQueryOptions(api, petId === undefined ? skipToken : { path: { petId } }));
}

export function parallel(ids: string[]): void {
  const qs = useQueries({ queries: ids.map((petId) => showPetByIdQueryOptions(api, { path: { petId } })) });
  expect<Equal<(typeof qs)[number]['data'], Pet | undefined>>();
  const [one] = useQueries({ queries: [options] });
  expect<Equal<typeof one.data, Pet | undefined>>();
}

export async function loader(): Promise<void> {
  await qc.prefetchQuery(options);
  const ensured = await qc.ensureQueryData(options);
  expect<Equal<typeof ensured, Pet>>();
  const fetched = await qc.fetchQuery(options);
  expect<Equal<typeof fetched, Pet>>();
  const suspended = useSuspenseQuery(options);
  expect<Equal<typeof suspended.data, Pet>>();
  const cached = qc.getQueryData(options.queryKey);
  expect<Equal<typeof cached, Pet | undefined>>();
}

export function optimistic(): void {
  useMutation({
    ...createPetsMutationOptions(api),
    onMutate: async () => ({ previous: qc.getQueryData(showPetByIdQueryKey({ path: { petId: '1' } })) }),
    onError: (_error, _variables, context) => {
      expect<Equal<typeof context, { previous: Pet | undefined } | undefined>>();
      // @ts-expect-error The context is what `onMutate` returns.
      expect<Equal<typeof context, unknown>>();
    },
  });
}
"#;

#[test]
fn query_options_infer_like_hand_written_ones() {
    let mut files = files("v3.0-petstore.yaml", &tanstack());
    files.push(("inference.ts".into(), INFERENCE.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

/// The same options with the Vue adapter, which re-exports the core types.
const VUE: &str = r#"import { QueryClient, skipToken, useMutation, useQueries, useQuery } from '@tanstack/vue-query';
import { createClient } from '@tsurara/client';
import { createPetsMutationOptions } from './query/createPets.js';
import { showPetByIdQueryOptions } from './query/showPetById.js';
import type { Pet } from './schemas/Pet.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};

const api = createClient({ baseUrl: 'https://petstore.example.com' });
const qc = new QueryClient();
const options = showPetByIdQueryOptions(api, { path: { petId: '1' } });

export async function vue(petId: string | undefined): Promise<void> {
  const q = useQuery(options);
  expect<Equal<typeof q.data.value, Pet | undefined>>();
  // @ts-expect-error The data is a Pet.
  expect<Equal<typeof q.data.value, number | undefined>>();
  const named = useQuery({ ...options, select: (p) => p.name });
  expect<Equal<typeof named.data.value, string | undefined>>();
  const skipped = useQuery(showPetByIdQueryOptions(api, petId === undefined ? skipToken : { path: { petId } }));
  expect<Equal<typeof skipped.data.value, Pet | undefined>>();
  useQueries({ queries: [options] });
  const fetched = await qc.fetchQuery(options);
  expect<Equal<typeof fetched, Pet>>();
  const m = useMutation(createPetsMutationOptions(api));
  m.mutate({ body: { id: 1, name: 'a' } });
}
"#;

#[test]
fn query_options_type_check_with_vue_query() {
    let mut files = files("v3.0-petstore.yaml", &tanstack());
    files.push(("vue.ts".into(), VUE.into()));
    // The declarations of `@tanstack/vue-query` 5.107 fail `exactOptionalPropertyTypes` themselves
    // (`QueryClient.isMutating`); the generated files and `vue.ts` are still checked with it.
    let options = TsOptions {
        skip_lib_check: true,
        ..TsOptions::default()
    };
    assert_tsc_passes(&files, &options);
}

/// Scoped keys and typed filters: the scope a client carries goes first in its keys, the
/// key factories take it as an argument, and the filters match every query of an operation.
const SCOPES: &str = r#"import { QueryClient, useMutation, useQuery } from '@tanstack/react-query';
import { createClient, queryScopeFilter } from '@tsurara/client';
import { createPetsMutationOptions } from './query/createPets.js';
import { listPetsQueryFilter, listPetsQueryKey } from './query/listPets.js';
import {
  showPetByIdQueryFilter,
  showPetByIdQueryKey,
  showPetByIdQueryOptions,
} from './query/showPetById.js';
import type { Pet } from './schemas/Pet.js';
import type { Pets } from './schemas/Pets.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};

const tenant = { tenant: 'a', api: 'pets' };
const api = createClient({ baseUrl: 'https://petstore.example.com', queryScope: tenant });
const qc = new QueryClient();

export async function edited(pet: Pet): Promise<void> {
  const q = useQuery(showPetByIdQueryOptions(api, { path: { petId: '1' } }));
  expect<Equal<typeof q.data, Pet | undefined>>();
  useMutation({
    ...createPetsMutationOptions(api),
    onSuccess: async () => {
      qc.setQueryData(showPetByIdQueryKey({ path: { petId: String(pet.id) } }, tenant), pet);
      await qc.invalidateQueries(listPetsQueryFilter(tenant));
    },
  });
  // `setQueriesData` does not infer the data from a filter (TanStack 5.107); name it.
  qc.setQueriesData<Pet>(showPetByIdQueryFilter(tenant), (old) => {
    expect<Equal<typeof old, Pet | undefined>>();
    return old;
  });
  const list = qc.getQueryData(listPetsQueryKey({}, tenant));
  expect<Equal<typeof list, Pets | undefined>>();
  // @ts-expect-error The data of listPets is Pets.
  expect<Equal<typeof list, Pet | undefined>>();
  // Unscoped keys and filters are those of a client without `queryScope`.
  qc.removeQueries(showPetByIdQueryFilter());
  void showPetByIdQueryKey({ path: { petId: '1' } });
  // Logout or a tenant switch: every query of the scope.
  qc.removeQueries(queryScopeFilter(tenant));
}
"#;

#[test]
fn scoped_keys_and_filters_type_check() {
    let mut files = files("v3.0-petstore.yaml", &tanstack());
    files.push(("scopes.ts".into(), SCOPES.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

/// Failures in components: `isHttpError` narrows the error of `useQuery` and `useMutation`
/// to the documented body of a status, and `queryRetry` is accepted as TanStack's `retry`.
const ERRORS_DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets/{petId}:
    get:
      operationId: getPet
      parameters: [{ name: petId, in: path, required: true, schema: { type: integer } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "401": { description: no, content: { application/json: { schema: { $ref: "#/components/schemas/Problem" } } } }
    patch:
      operationId: updatePet
      parameters: [{ name: petId, in: path, required: true, schema: { type: integer } }]
      requestBody: { required: true, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "422": { description: bad, content: { application/json: { schema: { $ref: "#/components/schemas/FieldErrors" } } } }
components:
  schemas:
    Pet: { type: object, required: [id, name], properties: { id: { type: integer }, name: { type: string } } }
    Problem: { type: object, required: [title], properties: { title: { type: string } } }
    FieldErrors: { type: object, required: [fields], properties: { fields: { type: object, additionalProperties: { type: string } } } }
"##;

const ERRORS: &str = r#"import { QueryCache, QueryClient, useMutation, useQuery } from '@tanstack/react-query';
import { ApiError, createClient, isHttpError, queryRetry } from '@tsurara/client';
import { getPetQueryOptions } from './query/getPet.js';
import { updatePetMutationOptions } from './query/updatePet.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};

const api = createClient({ baseUrl: 'https://petstore.example.com' });
export const qc: QueryClient = new QueryClient({
  defaultOptions: { queries: { retry: queryRetry() } },
  queryCache: new QueryCache({
    onError: (error) => {
      if (error instanceof ApiError && error.result.kind === 'http' && !error.result.documented) {
        console.warn('undocumented status', error.status, error.operationId);
      }
    },
  }),
});

export function form(): void {
  const m = useMutation(updatePetMutationOptions(api));
  if (m.error && isHttpError(m.error.result, 422)) {
    expect<Equal<typeof m.error.result.error.fields, { [key: string]: string }>>();
  }
  const q = useQuery(getPetQueryOptions(api, { path: { petId: 1 } }));
  if (q.error && isHttpError(q.error.result, 401)) {
    expect<Equal<typeof q.error.result.error.title, string>>();
  }
  // @ts-expect-error 422 is not a documented status of getPet.
  if (q.error && isHttpError(q.error.result, 422)) void 0;
}
"#;

#[test]
fn failures_narrow_in_components() {
    let set =
        DocumentSet::from_value(tsurara_loader::parse_document(ERRORS_DOC.as_bytes()).unwrap());
    let mut files: Vec<(String, String)> = generate(&set, &Options::default(), &tanstack())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    files.push(("errors.ts".into(), ERRORS.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

/// Infinite queries: pagination from `x-tsurara-pagination` (cursor) and from the
/// configuration (page numbers, both directions), and one without `next`, for which the app gives
/// `getNextPageParam`.
const PAGED_DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - { name: cursor, in: query, schema: { type: string } }
        - { name: limit, in: query, schema: { type: integer } }
      x-tsurara-pagination: { param: query.cursor, next: /nextCursor }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [items]
                properties:
                  items: { type: array, items: { $ref: "#/components/schemas/Pet" } }
                  nextCursor: { type: string }
  /orders:
    get:
      operationId: listOrders
      parameters:
        - { name: page, in: query, schema: { type: integer } }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [items, meta]
                properties:
                  items: { type: array, items: { type: string } }
                  meta:
                    type: object
                    properties:
                      next: { type: [integer, "null"] }
                      prev: { type: [integer, "null"] }
  /events:
    get:
      operationId: listEvents
      parameters:
        - { name: offset, in: query, schema: { type: integer } }
      x-tsurara-pagination: { param: query.offset, initial: 0 }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: array, items: { type: string } }
  /pets/{petId}:
    get:
      operationId: getPet
      parameters: [{ name: petId, in: path, required: true, schema: { type: string } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
components:
  schemas:
    Pet: { type: object, required: [id, name], properties: { id: { type: integer }, name: { type: string } } }
"##;

fn paged() -> TypeScriptOptions {
    let mut opts = tanstack();
    opts.infinite_queries.insert(
        "listOrders".into(),
        tsurara_sem::operation::Pagination {
            param: "query.page".into(),
            initial: Some(serde_json::json!(1)),
            next: Some("/meta/next".into()),
            previous: Some("/meta/prev".into()),
        },
    );
    opts
}

fn paged_files(opts: &TypeScriptOptions) -> tsurara_backend::Output {
    let set =
        DocumentSet::from_value(tsurara_loader::parse_document(PAGED_DOC.as_bytes()).unwrap());
    generate(&set, &Options::default(), opts).unwrap()
}

const INFINITE: &str = r#"import {
  type InfiniteData,
  QueryClient,
  useInfiniteQuery,
  useSuspenseInfiniteQuery,
} from '@tanstack/react-query';
import { useInfiniteQuery as useVueInfiniteQuery } from '@tanstack/vue-query';
import { createClient, type SuccessData } from '@tsurara/client';
import type { ListPetsResult } from './operations/listPets.js';
import { listEventsInfiniteQueryOptions } from './query/listEvents.js';
import { listOrdersInfiniteQueryOptions } from './query/listOrders.js';
import {
  listPetsInfiniteQueryKey,
  listPetsInfiniteQueryOptions,
  listPetsQueryFilter,
  listPetsQueryKey,
} from './query/listPets.js';
import type { Pet } from './schemas/Pet.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};

const api = createClient({ baseUrl: 'https://petstore.example.com' });
const qc = new QueryClient();

export async function lists(): Promise<void> {
  const pets = useInfiniteQuery(listPetsInfiniteQueryOptions(api, { query: { limit: 20 } }));
  expect<Equal<typeof pets.data, InfiniteData<SuccessData<ListPetsResult>, string | undefined> | undefined>>();
  const first = pets.data?.pages[0]?.items;
  expect<Equal<typeof first, Pet[] | undefined>>();
  void pets.fetchNextPage();
  const all = useInfiniteQuery({
    ...listPetsInfiniteQueryOptions(api),
    select: (data) => data.pages.flatMap((page) => page.items),
  });
  expect<Equal<typeof all.data, Pet[] | undefined>>();
  const suspended = useSuspenseInfiniteQuery(listPetsInfiniteQueryOptions(api));
  expect<Equal<typeof suspended.data.pageParams, (string | undefined)[]>>();

  const orders = useInfiniteQuery(listOrdersInfiniteQueryOptions(api));
  void orders.fetchPreviousPage();
  const page = orders.data?.pages[0]?.meta.next;
  expect<Equal<typeof page, number | null | undefined>>();

  // Without `next`, the app says where the next page is.
  useInfiniteQuery({
    ...listEventsInfiniteQueryOptions(api),
    getNextPageParam: (last, pages) => (last.length === 0 ? undefined : pages.length * 10),
  });
  // @ts-expect-error `getNextPageParam` is missing.
  useInfiniteQuery(listEventsInfiniteQueryOptions(api));

  await qc.prefetchInfiniteQuery(listPetsInfiniteQueryOptions(api));
  const cached = qc.getQueryData(listPetsInfiniteQueryKey());
  expect<Equal<typeof cached, InfiniteData<SuccessData<ListPetsResult>, string | undefined> | undefined>>();
  const plain = qc.getQueryData(listPetsQueryKey());
  expect<Equal<typeof plain, SuccessData<ListPetsResult> | undefined>>();
  await qc.invalidateQueries(listPetsQueryFilter());

  const vue = useVueInfiniteQuery(listPetsInfiniteQueryOptions(api));
  expect<Equal<typeof vue.data.value, InfiniteData<SuccessData<ListPetsResult>, string | undefined> | undefined>>();
}
"#;

#[test]
fn infinite_queries_type_check() {
    let mut files: Vec<(String, String)> = paged_files(&paged())
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    files.push(("infinite.ts".into(), INFINITE.into()));
    // `@tanstack/vue-query` fails `exactOptionalPropertyTypes` in its own declarations (see above).
    let options = TsOptions {
        skip_lib_check: true,
        ..TsOptions::default()
    };
    assert_tsc_passes(&files, &options);
}

#[test]
fn only_paginated_reads_get_infinite_options() {
    let out = paged_files(&paged());
    let get = |p: &str| {
        &out.files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("{p}"))
            .contents
    };
    for read in [
        "query/listPets.ts",
        "query/listOrders.ts",
        "query/listEvents.ts",
    ] {
        assert!(get(read).contains("InfiniteQueryOptions"), "{read}");
    }
    assert!(!get("query/getPet.ts").contains("InfiniteQueryOptions"));
    // Without the configuration, listOrders has no pagination.
    let plain = paged_files(&tanstack());
    let orders = &plain
        .files
        .iter()
        .find(|f| f.path == "query/listOrders.ts")
        .unwrap()
        .contents;
    assert!(!orders.contains("InfiniteQueryOptions"));
}

#[test]
fn configured_pagination_that_does_not_fit_warns() {
    let mut opts = paged();
    opts.infinite_queries.insert(
        "noSuchOperation".into(),
        tsurara_sem::operation::Pagination {
            param: "query.page".into(),
            initial: None,
            next: None,
            previous: None,
        },
    );
    opts.infinite_queries.insert(
        "getPet".into(),
        tsurara_sem::operation::Pagination {
            param: "query.page".into(),
            initial: None,
            next: None,
            previous: None,
        },
    );
    let out = paged_files(&opts);
    let messages: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("noSuchOperation")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("getPet") && m.contains("query.page")),
        "{messages:?}"
    );
}

#[test]
fn infinite_query_snapshot() {
    let out = paged_files(&paged());
    let get = |p: &str| {
        out.files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("{p}"))
            .contents
            .clone()
    };
    insta::assert_snapshot!(format!(
        "{}\n{}",
        get("query/listPets.ts"),
        get("query/listOrders.ts")
    ));
}

/// Patching every cached query of an operation with the data typed: TanStack's
/// `setQueriesData` and `getQueriesData` do not infer the data from a filter.
const QUERIES_DATA: &str = r#"import { QueryClient } from '@tanstack/react-query';
import { getShowPetByIdQueriesData, setShowPetByIdQueriesData } from './query/showPetById.js';
import type { Pet } from './schemas/Pet.js';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
const expect = <T extends true>(): void => {};
const qc = new QueryClient();

export function renamed(name: string): void {
  setShowPetByIdQueriesData(qc, (old) => {
    expect<Equal<typeof old, Pet | undefined>>();
    return old && { ...old, name };
  });
  setShowPetByIdQueriesData(qc, (old) => old, { tenant: 'a' });
  const all = getShowPetByIdQueriesData(qc);
  expect<Equal<(typeof all)[number][1], Pet | undefined>>();
  // @ts-expect-error The data of showPetById is a Pet.
  setShowPetByIdQueriesData(qc, () => 1);
}
"#;

#[test]
fn queries_data_is_typed() {
    let mut files = files("v3.0-petstore.yaml", &tanstack());
    files.push(("queries.ts".into(), QUERIES_DATA.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}
