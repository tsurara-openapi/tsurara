//! The query keys at run time, with `@tanstack/query-core`: two clients with different
//! scopes keep separate cache entries for the same request, filters match every query of their
//! operation and scope and nothing else, keys without a scope are `[operationId, data]`, and keys hash
//! the same after a JSON round trip (dehydrate on the server, hydrate in the browser). Infinite
//! queries fetch their pages in order.
//!
//! Needs Node 22.18 or later; skipped otherwise unless `TSURARA_REQUIRE_NODE=1`.

use std::path::Path;
use std::process::Command;
use tsurara_lang_typescript::options::{ImportExtension, Query};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::{repo_root, tsc_path};

const RUN: &str = r#"import { QueryClient, hashKey } from '@tanstack/query-core';
import { createClient, queryScopeFilter } from '@tsurara/client';
import { getPetByIdQueryFilter, getPetByIdQueryKey, getPetByIdQueryOptions } from './gen/query/getPetById.ts';
import { getInventoryQueryOptions } from './gen/query/getInventory.ts';
import { addPetMutationOptions } from './gen/query/addPet.ts';

const fail = (message, value) => {
  console.error(message, JSON.stringify(value));
  process.exit(1);
};
const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// Each client answers with its own pet name.
const client = (name, queryScope) =>
  createClient({
    baseUrl: 'https://petstore.test',
    ...(queryScope === undefined ? {} : { queryScope }),
    fetch: async () =>
      new Response(JSON.stringify({ id: 1, name, photoUrls: [] }), {
        headers: { 'content-type': 'application/json' },
      }),
  });
const a = client('a', { tenant: 'a' });
const b = client('b', { tenant: 'b' });
const plain = client('plain', undefined);
const data = { path: { petId: 1 } };

// Without a scope, the keys are `[operationId, data]`.
const plainOptions = getPetByIdQueryOptions(plain, data);
if (!equal(plainOptions.queryKey, ['getPetById', data])) fail('plain key', plainOptions.queryKey);
if (!equal(getPetByIdQueryKey(data), ['getPetById', data])) fail('plain key factory', getPetByIdQueryKey(data));
if (!equal(addPetMutationOptions(plain).mutationKey, ['addPet'])) fail('plain mutation key', addPetMutationOptions(plain).mutationKey);

// Two scopes on one QueryClient keep separate entries.
const qc = new QueryClient();
const fromA = await qc.fetchQuery(getPetByIdQueryOptions(a, data));
const fromB = await qc.fetchQuery(getPetByIdQueryOptions(b, data));
await qc.fetchQuery(plainOptions);
await qc.fetchQuery(getInventoryQueryOptions(a));
if (fromA.name !== 'a' || fromB.name !== 'b') fail('scoped fetches', [fromA, fromB]);
if (qc.getQueryData(getPetByIdQueryKey(data, { tenant: 'a' }))?.name !== 'a') fail('key of a', qc.getQueryData(getPetByIdQueryKey(data, { tenant: 'a' })));
if (qc.getQueryData(getPetByIdQueryKey(data, { tenant: 'b' }))?.name !== 'b') fail('key of b', null);
if (!equal(getPetByIdQueryOptions(a, data).queryKey, getPetByIdQueryKey(data, { tenant: 'a' }))) fail('options key of a', getPetByIdQueryOptions(a, data).queryKey);
if (!equal(addPetMutationOptions(a).mutationKey, [{ scope: { tenant: 'a' } }, 'addPet'])) fail('scoped mutation key', addPetMutationOptions(a).mutationKey);

// Filters: an operation in a scope, an operation without a scope, and a whole scope.
const names = (filter) => qc.getQueryCache().findAll(filter).map((q) => q.queryKey);
const ofA = names(getPetByIdQueryFilter({ tenant: 'a' }));
if (ofA.length !== 1 || !equal(ofA[0], getPetByIdQueryKey(data, { tenant: 'a' }))) fail('operation filter in a scope', ofA);
const unscoped = names(getPetByIdQueryFilter());
if (unscoped.length !== 1 || !equal(unscoped[0], ['getPetById', data])) fail('operation filter without a scope', unscoped);
const scopeA = names(queryScopeFilter({ tenant: 'a' }));
if (scopeA.length !== 2) fail('scope filter', scopeA);

// Keys hash the same after a JSON round trip, and property order does not matter.
for (const key of [getPetByIdQueryKey(data, { tenant: 'a', api: 'x' }), getPetByIdQueryKey(data)]) {
  if (hashKey(key) !== hashKey(JSON.parse(JSON.stringify(key)))) fail('round trip', key);
}
if (hashKey(getPetByIdQueryKey(data, { api: 'x', tenant: 'a' })) !== hashKey(getPetByIdQueryKey(data, { tenant: 'a', api: 'x' }))) fail('order', null);
console.log('ok');
"#;

fn node() -> bool {
    let required = std::env::var_os("TSURARA_REQUIRE_NODE").is_some();
    let version = Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let parsed = version.as_deref().and_then(|v| {
        let mut parts = v.trim_start_matches('v').split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    match parsed {
        Some(v) if v >= (22, 18) => true,
        _ if required => panic!("Node 22.18 or later is required, found {version:?}"),
        _ => {
            eprintln!("skipping: Node 22.18 or later is required, found {version:?}");
            false
        }
    }
}

fn petstore() -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/petstore3.json");
    let url = url::Url::parse("https://example.com/petstore3.json").unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    DocumentSet::load(&url, &f as &dyn Fetcher).unwrap()
}

#[test]
fn scoped_keys_and_filters_at_run_time() {
    if !node() {
        return;
    }
    assert_eq!(run_project(&petstore(), RUN), "ok");
}

/// Generates `set` with query options and `.ts` imports into `gen/`, next to `@tsurara/client` built
/// into `node_modules` (Node does not strip types there) and `@tanstack/query-core`, and runs
/// `script` with Node. Returns its output; fails on a non-zero exit.
fn run_project(set: &DocumentSet, script: &str) -> String {
    let tsc = tsc_path().expect("tsc is installed");
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("node_modules/@tsurara/client");
    let built = Command::new(tsc)
        .arg("-p")
        .arg(repo_root().join("runtimes/typescript/tsconfig.build.json"))
        .arg("--outDir")
        .arg(client.join("dist"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stdout)
    );
    std::fs::copy(
        repo_root().join("runtimes/typescript/package.json"),
        client.join("package.json"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("node_modules/@tanstack")).unwrap();
    std::os::unix::fs::symlink(
        std::fs::canonicalize(repo_root().join("node_modules/@tanstack/query-core")).unwrap(),
        dir.path().join("node_modules/@tanstack/query-core"),
    )
    .unwrap();
    let opts = TypeScriptOptions {
        import_extension: ImportExtension::Ts,
        query: Some(Query::Tanstack),
        ..TypeScriptOptions::default()
    };
    for f in generate(set, &Options::default(), &opts).unwrap().files {
        let full = dir.path().join("gen").join(&f.path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, f.contents).unwrap();
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), script).unwrap();
    let out = Command::new("node")
        .arg("run.ts")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Infinite queries at run time: three pages are fetched in order with the cursor of the
/// page before, and the last one ends the query.
const PAGES: &str = r#"import { QueryClient } from '@tanstack/query-core';
import { createClient } from '@tsurara/client';
import {
  getListPetsQueriesData,
  listPetsInfiniteQueryKey,
  listPetsInfiniteQueryOptions,
  listPetsQueryKey,
  listPetsQueryOptions,
  setListPetsQueriesData,
} from './gen/query/listPets.ts';

const pages = { '': { items: [1], nextCursor: 'b' }, b: { items: [2], nextCursor: 'c' }, c: { items: [3] } };
const urls = [];
const api = createClient({
  baseUrl: 'https://pets.test',
  fetch: async (input) => {
    const url = new URL(String(input));
    urls.push(url.search);
    return new Response(JSON.stringify(pages[url.searchParams.get('cursor') ?? '']), {
      headers: { 'content-type': 'application/json' },
    });
  },
});
const qc = new QueryClient();
const options = listPetsInfiniteQueryOptions(api, { query: { limit: 1 } });
const data = await qc.fetchInfiniteQuery({ ...options, pages: 3 });
const fail = (message, value) => {
  console.error(message, JSON.stringify(value));
  process.exit(1);
};
if (JSON.stringify(data.pages.map((p) => p.items)) !== '[[1],[2],[3]]') fail('pages', data.pages);
if (JSON.stringify(data.pageParams) !== JSON.stringify([undefined, 'b', 'c'])) fail('page params', data.pageParams);
if (JSON.stringify(urls) !== JSON.stringify(['?limit=1', '?limit=1&cursor=b', '?limit=1&cursor=c'])) fail('requests', urls);
if (options.getNextPageParam(data.pages[2]) !== undefined) fail('the last page ends the query', data);
if (options.getNextPageParam(data.pages[0]) !== 'b') fail('the next cursor', data.pages[0]);
if (JSON.stringify(qc.getQueryData(listPetsInfiniteQueryKey({ query: { limit: 1 } }))) !== JSON.stringify(data)) fail('key', null);
if (qc.getQueryData(listPetsQueryKey({ query: { limit: 1 } })) !== undefined) fail('the plain key is another entry', null);
// Patching every listPets query changes the query entries, not the infinite ones.
await qc.fetchQuery(listPetsQueryOptions(api, { query: { limit: 1 } }));
setListPetsQueriesData(qc, (old) => old && { ...old, items: [] });
if (JSON.stringify(qc.getQueryData(listPetsQueryKey({ query: { limit: 1 } }))?.items) !== '[]') fail('patched', null);
if (qc.getQueryData(listPetsInfiniteQueryKey({ query: { limit: 1 } }))?.pages.length !== 3) fail('infinite untouched', null);
if (getListPetsQueriesData(qc).length !== 1) fail('one query entry', getListPetsQueriesData(qc));
console.log('ok');
"#;

const PAGED: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - { name: limit, in: query, schema: { type: integer } }
        - { name: cursor, in: query, schema: { type: string } }
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
                  items: { type: array, items: { type: integer } }
                  nextCursor: { type: string }
"##;

#[test]
fn infinite_queries_at_run_time() {
    if !node() {
        return;
    }
    let set = DocumentSet::from_value(tsurara_loader::parse_document(PAGED.as_bytes()).unwrap());
    assert_eq!(run_project(&set, PAGES), "ok");
}
