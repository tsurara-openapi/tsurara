//! Mock factories and request handlers (`typescript.mocks: msw | fetch`).

use std::process::Command;
use tsurara_lang_typescript::options::{ImportExtension, Mocks};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root, tsc_path};

fn petstore() -> DocumentSet {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml");
    DocumentSet::from_value(parse_document(&std::fs::read(path).unwrap()).unwrap())
}

/// Statuses without `default`, so that undocumented statuses are type errors.
const STRICT: &str = r##"
openapi: 3.1.0
info: { title: strict, version: "1" }
paths:
  /pets/{petId}:
    get:
      operationId: getPet
      parameters: [{ name: petId, in: path, required: true, schema: { type: integer } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "404": { description: no, content: { application/json: { schema: { $ref: "#/components/schemas/Problem" } } } }
  /pets/{petId}/photo.{ext}:
    get:
      operationId: getPhoto
      parameters:
        - { name: petId, in: path, required: true, schema: { type: integer } }
        - { name: ext, in: path, required: true, schema: { type: string } }
      responses:
        "200": { description: ok, content: { image/png: { schema: { type: string, format: binary } } } }
  /pets:
    post:
      operationId: addPet
      requestBody: { content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
      responses: { "204": { description: ok } }
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id: { type: integer }
        name: { type: string, example: Pochi }
        owner: { $ref: "#/components/schemas/Owner" }
        parent: { $ref: "#/components/schemas/Pet" }
    Owner: { type: object, required: [name], properties: { name: { type: string } } }
    Problem: { type: object, required: [message], properties: { message: { type: string } } }
"##;

fn strict() -> DocumentSet {
    DocumentSet::from_value(parse_document(STRICT.as_bytes()).unwrap())
}

fn files(set: &DocumentSet, opts: &TypeScriptOptions) -> Vec<(String, String)> {
    generate(set, &Options::default(), opts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn with(mocks: Mocks) -> TypeScriptOptions {
    TypeScriptOptions {
        mocks: Some(mocks),
        ..TypeScriptOptions::default()
    }
}

#[test]
fn without_the_option_nothing_mentions_mocks() {
    for (path, contents) in files(&petstore(), &TypeScriptOptions::default()) {
        assert!(
            !contents.contains("msw") && !path.starts_with("mocks/"),
            "{path}"
        );
    }
}

#[test]
fn factories_and_handlers() {
    let files = files(&strict(), &with(Mocks::Msw));
    let get = |p: &str| {
        &files
            .iter()
            .find(|(path, _)| path == p)
            .unwrap_or_else(|| panic!("{p}"))
            .1
    };
    let pet = get("mocks/schemas/Pet.ts");
    assert!(
        pet.contains("export function mockPet(overrides?: Partial<Pet>): Pet {"),
        "{pet}"
    );
    // A named type that is its own sample is built by its factory; a cut cycle is written out.
    assert!(pet.contains("owner: mockOwner(),"), "{pet}");
    assert!(pet.contains("name: 'Pochi'"), "{pet}");
    let handler = get("mocks/operations/getPet.ts");
    assert!(
        handler.contains("export function getPetHandler("),
        "{handler}"
    );
    assert!(
        handler.contains("`${options.baseUrl ?? '*'}/pets/:petId`"),
        "{handler}"
    );
    let photo = get("mocks/operations/getPhoto.ts");
    assert!(
        photo.contains("RegExp"),
        "parameters inside a segment use a RegExp: {photo}"
    );
    assert!(get("mocks/index.ts").contains("export const handlers: HttpHandler[] = ["));
    // Nothing outside mocks/ imports them.
    for (path, contents) in &files {
        if !path.starts_with("mocks/") {
            assert!(
                !contents.contains("msw") && !contents.contains("mocks/"),
                "{path}"
            );
        }
    }
    insta::assert_snapshot!(format!("{pet}\n{handler}\n{}", get("mocks/index.ts")));
}

const TYPE_TEST: &str = r#"import { setupServer } from 'msw/node';
import { handlers } from './mocks/index.js';
import { getPetHandler } from './mocks/operations/getPet.js';
import { mockPet } from './mocks/schemas/Pet.js';

export const server: ReturnType<typeof setupServer> = setupServer(
  ...handlers,
  getPetHandler({ status: 404, body: { message: 'gone' } }),
  getPetHandler(({ params }) => ({ status: 200, body: mockPet({ id: Number(params.petId) }) })),
);

// @ts-expect-error 500 is not documented.
getPetHandler({ status: 500, body: { message: 'x' } });
// @ts-expect-error The body of 404 is a Problem.
getPetHandler({ status: 404, body: { code: 1 } });
// @ts-expect-error The name of a Pet is a string.
mockPet({ name: 1 });
"#;

#[test]
fn msw_handlers_type_check_and_reject_undocumented_responses() {
    let mut files = files(&strict(), &with(Mocks::Msw));
    files.push(("usage.ts".into(), TYPE_TEST.into()));
    assert_tsc_passes(&files, &TsOptions::default());
}

#[test]
fn fetch_routes_type_check() {
    let mut files = files(&strict(), &with(Mocks::Fetch));
    files.push((
        "usage.ts".into(),
        "import { mockFetch } from '@tsurara/client/testing';\nimport { routes } from './mocks/index.js';\nimport { getPetHandler } from './mocks/operations/getPet.js';\n\nexport const mock: ReturnType<typeof mockFetch> = mockFetch([getPetHandler({ status: 404, body: { message: 'gone' } }), ...routes]);\n".into(),
    ));
    assert_tsc_passes(&files, &TsOptions::default());
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        let path = repo_root()
            .join("crates/tsurara-lang-typescript/tests/fixtures")
            .join(name);
        let set = DocumentSet::from_value(parse_document(&std::fs::read(path).unwrap()).unwrap());
        for mocks in [Mocks::Msw, Mocks::Fetch] {
            assert_tsc_passes(&files_of(&set, mocks), &TsOptions::default());
        }
    }
}

fn files_of(set: &DocumentSet, mocks: Mocks) -> Vec<(String, String)> {
    files(set, &with(mocks))
}

/// The petstore call functions against `setupServer(...handlers)` and `mockFetch(routes)` on Node.
const RUN: &str = r#"import { createClient, isHttpError } from '@tsurara/client';
import { mockFetch } from '@tsurara/client/testing';
import { http } from 'msw';
import { setupServer } from 'msw/node';
import { handlers } from './msw/mocks/index.ts';
import { showPetByIdHandler } from './msw/mocks/operations/showPetById.ts';
import { listPets } from './msw/operations/listPets.ts';
import { showPetById } from './msw/operations/showPetById.ts';
import { routes } from './fetch/mocks/index.ts';
import { showPetByIdHandler as showPetByIdRoute } from './fetch/mocks/operations/showPetById.ts';

const check = (cond: boolean, what: string): void => {
  if (!cond) throw new Error(what);
};

const server = setupServer(...handlers);
server.listen({ onUnhandledFrame: 'error' });
const api = createClient({ baseUrl: 'http://petstore.test/v1' });
const pets = await listPets(api);
check(pets.ok && Array.isArray(pets.data) && pets.data[0]?.name === 'name', `msw list: ${JSON.stringify(pets)}`);
const pet = await showPetById(api, { path: { petId: '7' } });
check(pet.ok && pet.data.id === 1, `msw show: ${JSON.stringify(pet)}`);
server.use(showPetByIdHandler({ status: 404, body: { code: 404, message: 'gone' } }));
const gone = await showPetById(api, { path: { petId: '7' } });
check(isHttpError(gone, 404) || (gone.kind === 'http' && gone.status === 404), `msw 404: ${JSON.stringify(gone)}`);
server.close();
void http;

const mock = mockFetch([showPetByIdRoute({ status: 404, body: { code: 404, message: 'gone' } }), ...routes], {
  basePath: '/v1',
});
const local = createClient({ baseUrl: 'http://petstore.test/v1', fetch: mock.fetch });
const listed = await listPets(local);
check(listed.ok && listed.data[0]?.name === 'name', `fetch list: ${JSON.stringify(listed)}`);
const missing = await showPetById(local, { path: { petId: '7' } });
check(missing.kind === 'http' && missing.status === 404, `fetch 404: ${JSON.stringify(missing)}`);
console.log('ok');
"#;

#[test]
fn handlers_answer_the_call_functions_on_node() {
    let node_ok = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            let v = String::from_utf8_lossy(&o.stdout);
            let mut parts = v.trim().trim_start_matches('v').split('.');
            let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            (major, minor) >= (22, 18)
        });
    if !node_ok {
        assert!(
            std::env::var_os("TSURARA_REQUIRE_NODE").is_none(),
            "Node 22.18 or later is required"
        );
        eprintln!("skipping: Node 22.18 or later is required");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("node_modules/@tsurara/client");
    let built = Command::new(tsc_path().expect("tsc is installed"))
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
    std::os::unix::fs::symlink(
        repo_root().join("node_modules/msw"),
        dir.path().join("node_modules/msw"),
    )
    .unwrap();
    for (folder, mocks) in [("msw", Mocks::Msw), ("fetch", Mocks::Fetch)] {
        let opts = TypeScriptOptions {
            import_extension: ImportExtension::Ts,
            mocks: Some(mocks),
            ..TypeScriptOptions::default()
        };
        for (path, contents) in files(&petstore(), &opts) {
            let full = dir.path().join(folder).join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
        }
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), RUN).unwrap();
    let out = Command::new("node")
        .arg("run.ts")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("ok"),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
