//! TanStack Query options and query keys (`typescript.query: tanstack`, design.md 9.4).
//!
//! One file per operation under `query/`, which the types, the functions and the barrel never import,
//! so apps without TanStack Query do not load it. Reads (`GET`, `HEAD`, `QUERY`) get a query key
//! factory and query options; other methods get mutation options. Operations whose success is a
//! stream get neither. The types come from `@tanstack/query-core`, which every adapter re-exports, so
//! the objects work with `useQuery`, `useSuspenseQuery`, `prefetchQuery`, `useMutation` and their Vue,
//! Solid and Svelte equivalents.

use crate::HEADER;
use crate::options::TypeScriptOptions;
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_sem::model::Model;
use tsurara_sem::operation::{BodyKind, Operation, Pagination, ParamIn};
use tsurara_sem::print::{property_key, string_literal};
use tsurara_sem::{Warning, WarningKind};

/// The query files, and per operation id the helpers each declares, by manifest role.
pub type Helpers = Vec<(String, Vec<(&'static str, String, String)>)>;

/// `operation_files[i]` is the file of `model.operations[i]`.
pub fn emit(
    model: &Model,
    operation_files: &[String],
    opts: &TypeScriptOptions,
    warnings: &mut Vec<Warning>,
) -> (Vec<OutputFile>, Helpers) {
    let mut helpers = Vec::new();
    // Configured pagination that names no operation.
    if let Some(first) = model.operations.first() {
        for id in opts.infinite_queries.keys() {
            if !model.operations.iter().any(|op| op.id == *id) {
                warnings.push(Warning {
                    kind: WarningKind::InvalidDefinition,
                    loc: tsurara_loader::Loc::root(first.loc.doc.clone()),
                    message: format!(
                        "`typescript.infiniteQueries` names `{id}`, which is not an operation id"
                    ),
                    language: None,
                });
            }
        }
    }
    let files = model
        .operations
        .iter()
        .zip(operation_files)
        .filter(|(op, _)| !streams(op))
        .map(|(op, file)| {
            let pagination = pagination(op, opts, warnings);
            let path = format!("query/{}.ts", op.name);
            let function = &op.name;
            let pascal = pascal_of(function);
            let mut roles: Vec<(&'static str, String)> = if is_query(op) {
                vec![
                    ("queryKey", format!("{function}QueryKey")),
                    ("queryOptions", format!("{function}QueryOptions")),
                    ("queryFilter", format!("{function}QueryFilter")),
                    ("setQueriesData", format!("set{pascal}QueriesData")),
                    ("getQueriesData", format!("get{pascal}QueriesData")),
                ]
            } else {
                vec![("mutationOptions", format!("{function}MutationOptions"))]
            };
            if pagination.is_some() {
                roles.push(("infiniteQueryKey", format!("{function}InfiniteQueryKey")));
                roles.push((
                    "infiniteQueryOptions",
                    format!("{function}InfiniteQueryOptions"),
                ));
            }
            helpers.push((
                op.id.clone(),
                roles
                    .into_iter()
                    .map(|(role, name)| (role, name, path.clone()))
                    .collect(),
            ));
            OutputFile {
                contents: operation_file(op, file, opts, pagination.as_ref()),
                path,
            }
        })
        .collect();
    (files, helpers)
}

fn pascal_of(function: &str) -> String {
    let mut c = function.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect::<String>())
        .unwrap_or_default()
}

/// The pagination of a read: the configuration's, else the definition's (`x-tsurara-pagination`,
/// already checked by the model). A configured one is checked here.
fn pagination(
    op: &Operation,
    opts: &TypeScriptOptions,
    warnings: &mut Vec<Warning>,
) -> Option<Pagination> {
    let (p, configured) = match opts.infinite_queries.get(&op.id) {
        Some(p) => (p.clone(), true),
        None => (op.pagination.clone()?, false),
    };
    let warn = |warnings: &mut Vec<Warning>, why: String| {
        let source = if configured {
            format!("`typescript.infiniteQueries.{}`", op.id)
        } else {
            "`x-tsurara-pagination`".to_owned()
        };
        warnings.push(Warning {
            kind: WarningKind::InvalidDefinition,
            loc: op.loc.clone(),
            message: format!(
                "{source} of `{}` ({} {}): {why}; it gets no infinite query options",
                op.id, op.method, op.path
            ),
            language: None,
        });
    };
    if !is_query(op) {
        warn(
            warnings,
            "only reads (GET, HEAD, QUERY) can be infinite queries".into(),
        );
        return None;
    }
    if configured {
        if let Err(e) = p.check(&op.params) {
            warn(warnings, e);
            return None;
        }
    }
    let (_, name) = p.param.split_once('.')?;
    let required = op.params.iter().any(|q| q.name == name && q.required);
    if required && p.initial.is_none() {
        warn(
            warnings,
            format!(
                "`{}` is required, so `initial` must give the first page",
                p.param
            ),
        );
        return None;
    }
    Some(p)
}

/// A JSON Pointer into a value of `root` as optional chaining: `/meta/next` is `root?.['meta']?.['next']`.
fn pointer_access(root: &str, pointer: &str) -> String {
    let mut out = root.to_owned();
    for token in pointer.split('/').skip(1) {
        let token = token.replace("~1", "/").replace("~0", "~");
        if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
            write!(out, "?.[{token}]").unwrap();
        } else {
            write!(out, "?.[{}]", string_literal(&token)).unwrap();
        }
    }
    out
}

/// Whether a success response is read as a stream, which has no single value to cache.
fn streams(op: &Operation) -> bool {
    op.responses
        .iter()
        .filter(|r| r.status.is_success() == Some(true))
        .flat_map(|r| &r.contents)
        .any(|c| c.body_kind == BodyKind::Stream)
}

fn is_query(op: &Operation) -> bool {
    matches!(op.method.as_str(), "GET" | "HEAD" | "QUERY")
}

fn operation_file(
    op: &Operation,
    file: &str,
    opts: &TypeScriptOptions,
    pagination: Option<&Pagination>,
) -> String {
    let derived = |template: &str| {
        op.type_names
            .get(template)
            .unwrap_or_else(|| panic!("{} has no name for {template}", op.id))
            .clone()
    };
    let function = &op.name;
    let data = derived("{P}Data");
    let result = derived("{P}Result");
    let pascal = pascal_of(function);
    let module = format!(
        "../{}{}",
        file.strip_suffix(".ts").unwrap_or(file),
        opts.import_extension.suffix()
    );
    let id = string_literal(&op.id);
    let success = format!("SuccessData<{result}>");
    let error = format!("ApiError<{result}>");
    // Reads take a scope in their key and filter factories.
    let scope_types = if is_query(op) {
        "type QueryScope, type QueryScopeKey, "
    } else {
        ""
    };
    let mut code = format!(
        "{HEADER}\n\nimport type * as TanStack from '@tanstack/query-core';\nimport {{ type ApiError, type Client, {scope_types}type SuccessData, unwrap }} from '{}';\nimport {{ {function}, type {data}, type {result} }} from '{module}';\n\n",
        opts.client_module
    );
    // The data argument can be left out when the operation has no required input.
    let optional = if op.has_required_input() {
        String::new()
    } else {
        " = {}".to_owned()
    };
    if is_query(op) {
        let key = format!("{pascal}QueryKey");
        let tagged = format!("TanStack.DataTag<{key}, {success}, {error}>");
        let filter_tagged = format!(
            "TanStack.DataTag<readonly [{id}] | readonly [QueryScopeKey, {id}], {success}, {error}>"
        );
        // Overload signatures cannot have initializers: an optional argument is `data?` there.
        let overload_data = if optional.is_empty() {
            format!("data: {data}")
        } else {
            format!("data?: {data}")
        };
        write!(
            code,
            "/**\n * The query key of `{function}`: the operation id, then the request data, after `{{ scope }}` when the\n * client has a `queryScope`.\n */\nexport type {key} = readonly [{id}, {data}] | readonly [QueryScopeKey, {id}, {data}];\n\n/**\n * Options for `useQuery`, `useSuspenseQuery`, `useQueries`, `prefetchQuery` and the like. Every place a\n * key appears has the same tagged key type, so that the adapters (React, Vue, ...) infer one key type\n * and `getQueryData` and `setQueryData` know the data.\n */\nexport type {pascal}QueryOptions = TanStack.QueryObserverOptions<\n  {success},\n  {error},\n  {success},\n  {success},\n  {tagged}\n> & {{\n  queryKey: {tagged};\n  queryFn: (context: TanStack.QueryFunctionContext<{tagged}>) => Promise<{success}>;\n}};\n\n/** Query options of `{function}` that may be skipped: `queryFn` is `skipToken` until the data is known. */\nexport type {pascal}SkippableQueryOptions = Omit<{pascal}QueryOptions, 'queryFn'> & {{\n  queryFn: {pascal}QueryOptions['queryFn'] | TanStack.SkipToken;\n}};\n\n/** The query key of `{function}`, typed so that `getQueryData` and `setQueryData` know its data. */\nexport function {function}QueryKey(data: {data}{optional}, scope?: QueryScope): {tagged} {{\n  return (scope === undefined ? [{id}, data] : [{{ scope }}, {id}, data]) as unknown as {tagged};\n}}\n\n/**\n * A filter for every query of `{function}`, with the scope of the client or without one, for\n * `invalidateQueries`, `setQueriesData`, `cancelQueries` and the like.\n */\nexport function {function}QueryFilter(scope?: QueryScope): {{ queryKey: {filter_tagged} }} {{\n  return {{\n    queryKey: (scope === undefined ? [{id}] : [{{ scope }}, {id}]) as unknown as {filter_tagged},\n  }};\n}}\n\n/** The queries of `{function}` (not its infinite queries, whose data is pages): the key is the data's. */\nfunction {function}Queries(scope?: QueryScope): TanStack.QueryFilters {{\n  const length = scope === undefined ? 2 : 3;\n  return {{ ...{function}QueryFilter(scope), predicate: (query) => query.queryKey.length === length }};\n}}\n\n/**\n * Sets the data of every `{function}` query, of the scope or without one, with the data typed:\n * TanStack's `setQueriesData` does not infer it from a filter. Infinite queries are left alone.\n */\nexport function set{pascal}QueriesData(\n  queryClient: TanStack.QueryClient,\n  updater: TanStack.Updater<{success} | undefined, {success} | undefined>,\n  scope?: QueryScope,\n): Array<[TanStack.QueryKey, {success} | undefined]> {{\n  return queryClient.setQueriesData<{success}>({function}Queries(scope), updater);\n}}\n\n/** The data of every `{function}` query, of the scope or without one, typed. */\nexport function get{pascal}QueriesData(\n  queryClient: TanStack.QueryClient,\n  scope?: QueryScope,\n): Array<[TanStack.QueryKey, {success} | undefined]> {{\n  return queryClient.getQueriesData<{success}>({function}Queries(scope));\n}}\n\n/**\n * Query options that call `{function}` and throw `ApiError` for failures (`unwrap`). Pass `skipToken`\n * instead of the data to keep the query from running until the data is known (not with\n * `useSuspenseQuery`, which cannot be skipped).\n */\nexport function {function}QueryOptions<Ctx>(client: Client<Ctx>, {overload_data}): {pascal}QueryOptions;\nexport function {function}QueryOptions<Ctx>(\n  client: Client<Ctx>,\n  data: {data} | TanStack.SkipToken,\n): {pascal}SkippableQueryOptions;\nexport function {function}QueryOptions<Ctx>(\n  client: Client<Ctx>,\n  data: {data} | TanStack.SkipToken{optional},\n): {pascal}SkippableQueryOptions {{\n  if (typeof data === 'symbol') {{\n    // `skipToken` itself, told apart by type so that this file imports nothing at run time. The key\n    // keeps the operation id, so filters by operation still match.\n    return {{\n      queryKey: {function}QueryKey(data as unknown as {data}, client.options.queryScope),\n      queryFn: data as TanStack.SkipToken,\n    }};\n  }}\n  return {{\n    queryKey: {function}QueryKey(data, client.options.queryScope),\n    queryFn: async ({{ signal }}: TanStack.QueryFunctionContext<{tagged}>) =>\n      unwrap(await {function}(client, {{ ...data, signal }})),\n  }};\n}}\n"
        )
        .unwrap();
        if let Some(p) = pagination {
            infinite(
                &mut code,
                p,
                &InfiniteNames {
                    function,
                    pascal: &pascal,
                    data: &data,
                    result: &result,
                    id: &id,
                    optional: &optional,
                },
            );
        }
    } else {
        write!(
            code,
            "/** Options for `useMutation`: the variables are the request data of `{function}`. */\nexport type {pascal}MutationOptions = TanStack.MutationOptions<{success}, {error}, {data}>;\n\n/** Mutation options that call `{function}` and throw `ApiError` for failures (`unwrap`). */\nexport function {function}MutationOptions<Ctx>(client: Client<Ctx>): {pascal}MutationOptions {{\n  return {{\n    mutationKey:\n      client.options.queryScope === undefined ? [{id}] : [{{ scope: client.options.queryScope }}, {id}],\n    mutationFn: async (data: {data}) => unwrap(await {function}(client, data)),\n  }};\n}}\n"
        )
        .unwrap();
    }
    code
}

struct InfiniteNames<'a> {
    function: &'a str,
    pascal: &'a str,
    data: &'a str,
    result: &'a str,
    id: &'a str,
    optional: &'a str,
}

/// Infinite query options. The options type has the same wide key everywhere a key
/// appears (`DataTag<QueryKey, InfiniteData<…>>`): Vue's `useInfiniteQuery` maps keys deeply and
/// rejects a union of tuples of different lengths. The key factory keeps the precise key.
fn infinite(code: &mut String, p: &Pagination, n: &InfiniteNames) {
    let InfiniteNames {
        function,
        pascal,
        data,
        result,
        id,
        optional,
    } = *n;
    let (location, name) = p.param.split_once('.').expect("checked");
    let group = ParamIn::parse(location).expect("checked").group();
    let field = property_key(name);
    let success = format!("SuccessData<{result}>");
    let error = format!("ApiError<{result}>");
    let page = format!("{pascal}PageParam");
    let infinite_data = format!("TanStack.InfiniteData<{success}, {page}>");
    let key = format!("{pascal}InfiniteQueryKey");
    let precise = format!("TanStack.DataTag<{key}, {infinite_data}, {error}>");
    let wide = format!("TanStack.DataTag<TanStack.QueryKey, {infinite_data}, {error}>");
    let initial = match &p.initial {
        Some(v) => serde_json::to_string(v).expect("JSON values serialize"),
        None => "undefined".to_owned(),
    };
    let mut page_fns = String::new();
    let mut page_types = String::new();
    if let Some(next) = &p.next {
        writeln!(
            page_types,
            "  getNextPageParam: (lastPage: {success}) => {page} | null | undefined;"
        )
        .unwrap();
        writeln!(
            page_fns,
            "    getNextPageParam: (lastPage: {success}) => {} ?? undefined,",
            pointer_access("lastPage", next)
        )
        .unwrap();
    }
    if let Some(previous) = &p.previous {
        writeln!(
            page_types,
            "  getPreviousPageParam: (firstPage: {success}) => {page} | null | undefined;"
        )
        .unwrap();
        writeln!(
            page_fns,
            "    getPreviousPageParam: (firstPage: {success}) => {} ?? undefined,",
            pointer_access("firstPage", previous)
        )
        .unwrap();
    }
    let observer = format!(
        "TanStack.InfiniteQueryObserverOptions<\n  {success},\n  {error},\n  {infinite_data},\n  {wide},\n  {page}\n>"
    );
    let (base, app_gives) = if p.next.is_some() {
        (observer, String::new())
    } else {
        (
            format!("Omit<{observer}, 'getNextPageParam'>"),
            " Spread them and give `getNextPageParam`: the definition does not say where the next page is.".to_owned(),
        )
    };
    write!(
        code,
        "\n/** The page parameter of `{function}`: the type of `{param}`. */\nexport type {page} = NonNullable<{data}[{group_lit}]>[{field_lit}];\n\n/**\n * The infinite query key of `{function}`: the query key with `{{ infinite: true }}` after the data, so\n * it never equals the query key and the operation's filter matches both.\n */\nexport type {key} =\n  | readonly [{id}, {data}, {{ readonly infinite: true }}]\n  | readonly [QueryScopeKey, {id}, {data}, {{ readonly infinite: true }}];\n\n/** Options for `useInfiniteQuery`, `useSuspenseInfiniteQuery`, `prefetchInfiniteQuery` and the like.{app_gives} */\nexport type {pascal}InfiniteQueryOptions = {base} & {{\n  queryKey: {wide};\n  queryFn: (context: TanStack.QueryFunctionContext<{wide}, {page}>) => Promise<{success}>;\n  initialPageParam: {page};\n{page_types}}};\n\n/** The infinite query key of `{function}`, typed so that `getQueryData` knows its pages. */\nexport function {function}InfiniteQueryKey(data: {data}{optional}, scope?: QueryScope): {precise} {{\n  return (\n    scope === undefined ? [{id}, data, {{ infinite: true }}] : [{{ scope }}, {id}, data, {{ infinite: true }}]\n  ) as unknown as {precise};\n}}\n\n/**\n * Infinite query options that call `{function}` with `{param}` set to the page parameter, starting at\n * `{initial}`, and throw `ApiError` for failures (`unwrap`).\n */\nexport function {function}InfiniteQueryOptions<Ctx>(\n  client: Client<Ctx>,\n  data: {data}{optional},\n): {pascal}InfiniteQueryOptions {{\n  return {{\n    queryKey: {function}InfiniteQueryKey(data, client.options.queryScope),\n    queryFn: async ({{ pageParam, signal }}: TanStack.QueryFunctionContext<{wide}, {page}>) =>\n      unwrap(\n        await {function}(client, {{\n          ...data,\n          {group}: {{ ...data.{group}, ...(pageParam === undefined ? {{}} : {{ {field}: pageParam }}) }},\n          signal,\n        }}),\n      ),\n    initialPageParam: {initial},\n{page_fns}  }};\n}}\n",
        param = p.param,
        group_lit = string_literal(group),
        field_lit = string_literal(name),
    )
    .unwrap();
}
