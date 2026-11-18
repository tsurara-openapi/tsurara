//! Call functions: generated functions build requests with the runtime (tsurara-client) and
//! return typed results, with the sync and the async client. Needs the runtime's virtual environment
//! (`uv sync --project runtimes/python`); skipped without it unless the Python checker is required.

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tsurara_lang_python::{PythonOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, PythonChecker, python_or_skip, repo_root};

fn fixture(name: &str) -> DocumentSet {
    let path = repo_root()
        .join("crates/tsurara-lang-typescript/tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    tsurara_loader::normalize_swagger2(&mut set).unwrap();
    set
}

fn write(dir: &Path, set: &DocumentSet) -> Vec<(String, String)> {
    let out = generate(set, &Options::default(), &PythonOptions::default()).unwrap();
    let files: Vec<(String, String)> = out
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    for (path, contents) in &files {
        let full = dir.join("api").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    files
}

fn run(python: &Path, dir: &Path, script: &str) -> String {
    std::fs::write(dir.join("script.py"), script).unwrap();
    let out = Command::new(python)
        .arg("script.py")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const CALLS: &str = r#"
import asyncio, json
import httpx
from tsurara_client import AsyncClient, Client, File, HttpError, Success, unwrap
from api import Pet, Category
from api.pet_status import PetStatus
from api.operations import add_pet, delete_pet, find_pets_by_status, get_inventory, get_pet_by_id, upload_file

sent = []

def handler(request):
    sent.append((request.method, str(request.url), request.headers.get("content-type"), request.headers.get("api_key"), request.read().decode(errors="replace")))
    if request.url.path == "/api/v3/pet/1":
        return httpx.Response(200, json={"id": 1, "name": "Rex", "photoUrls": [], "status": "sold", "category": {"id": 2, "name": "dogs"}})
    if request.url.path == "/api/v3/pet/2":
        return httpx.Response(404, json={"code": 404})
    if request.url.path == "/api/v3/pet/findByStatus":
        return httpx.Response(200, json=[{"name": "a", "photoUrls": [], "status": "lost"}])
    if request.url.path == "/api/v3/store/inventory":
        return httpx.Response(200, json={"sold": 3})
    return httpx.Response(200, json={"name": "x", "photoUrls": []})

client = Client("https://petstore3.swagger.io/api/v3", http=httpx.Client(transport=httpx.MockTransport(handler)))
r = get_pet_by_id(client, pet_id=1)
assert isinstance(r, Success), r
pet = r.data
assert isinstance(pet, Pet) and pet.photo_urls == [] and pet.status is PetStatus.SOLD, pet
assert isinstance(pet.category, Category) and pet.category.name == "dogs"
r = get_pet_by_id(client, pet_id=2)
assert r.kind == "http" and r.status == 404, r
pets = unwrap(find_pets_by_status(client, status=PetStatus.AVAILABLE))
assert pets[0].status == "lost"
assert unwrap(get_inventory(client)) == {"sold": 3}
unwrap(add_pet(client, body=Pet(name="Rex", photo_urls=["u"], status=PetStatus.SOLD)))
unwrap(add_pet(client, body=Pet(name="Rex", photo_urls=["u"]), content_type="application/x-www-form-urlencoded"))
delete_pet(client, pet_id=5, api_key="k")
delete_pet(client, pet_id=6)
unwrap(upload_file(client, pet_id=1, body=File(b"PNG", content_type="image/png")))

async def main():
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
        api = AsyncClient("https://petstore3.swagger.io/api/v3", http=http)
        pet = unwrap(await get_pet_by_id(api, pet_id=1))
        assert isinstance(pet, Pet)

asyncio.run(main())
print(json.dumps(sent))
"#;

#[test]
fn generated_functions_call_the_api_with_both_clients() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &fixture("petstore3.json"));
    let sent: Vec<Value> = serde_json::from_str(&run(&python, dir.path(), CALLS)).unwrap();
    let base = "https://petstore3.swagger.io/api/v3";
    assert_eq!(sent[0][1], format!("{base}/pet/1"));
    assert_eq!(
        sent[2][1],
        format!("{base}/pet/findByStatus?status=available")
    );
    assert_eq!(sent[4][0], "POST");
    assert_eq!(sent[4][2], "application/json");
    assert_eq!(
        sent[4][4],
        r#"{"name":"Rex","photoUrls":["u"],"status":"sold"}"#
    );
    assert_eq!(sent[5][2], "application/x-www-form-urlencoded");
    assert_eq!(sent[5][4], "name=Rex&photoUrls=u");
    assert_eq!(sent[6][0], "DELETE");
    assert_eq!(sent[6][3], "k");
    // An optional parameter left out is not sent.
    assert_eq!(sent[7][3], Value::Null);
    assert_eq!(sent[8][2], "application/octet-stream");
    assert_eq!(sent[8][4], "PNG");
    assert_eq!(sent[9][1], format!("{base}/pet/1"));
}

#[test]
fn operation_defs_validate_against_the_schema() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/operation-def.schema.json")).unwrap(),
    )
    .unwrap();
    let type_defs: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/type-def.schema.json")).unwrap(),
    )
    .unwrap();
    let registry = jsonschema::Registry::new()
        .add("https://raw.githubusercontent.com/tsurara-openapi/tsurara/main/spec/type-def.schema.json", type_defs)
        .unwrap()
        .prepare()
        .unwrap();
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .unwrap();
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &fixture(name));
        let dump = r#"
import importlib, json, pkgutil
import api.operations as ops
from api._typedefs import TYPE_DEFS
defs = [importlib.import_module(f"api.operations.{m.name}").DEF for m in pkgutil.iter_modules(ops.__path__)]
print(json.dumps({"defs": defs, "types": list(TYPE_DEFS["types"])}))
"#;
        let out: Value = serde_json::from_str(&run(&python, dir.path(), dump)).unwrap();
        let types: Vec<&str> = out["types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        let defs = out["defs"].as_array().unwrap();
        assert!(defs.len() > 3, "{name}");
        for def in defs {
            let errors: Vec<String> = validator
                .iter_errors(def)
                .map(|e| format!("{e} at {}", e.instance_path()))
                .collect();
            assert!(errors.is_empty(), "{name} {}: {errors:?}", def["id"]);
            // Every TypeDef reference names a declaration of the package.
            let text = def.to_string();
            for (i, _) in text.match_indices("\"ref\":\"") {
                let rest = &text[i + 7..];
                let r = &rest[..rest.find('"').unwrap()];
                assert!(types.contains(&r), "{name}: {r}");
            }
        }
    }
}

#[test]
fn generated_functions_pass_pyright() {
    let checker = PythonChecker {
        package: "api".into(),
    };
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        let out = generate(
            &fixture(name),
            &Options::default(),
            &PythonOptions::default(),
        )
        .unwrap();
        let files: Vec<(String, String)> = out
            .files
            .into_iter()
            .map(|f| (f.path, f.contents))
            .collect();
        assert!(
            files.iter().any(|(p, _)| p.starts_with("operations/")),
            "{name}"
        );
        checker
            .check(&files)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

const AUTH: &str = r#"
import httpx
from tsurara_client import Client, unwrap
from api.operations import SecuritySchemeName, get_inventory, get_pet_by_id, place_order

seen = []

def handler(request):
    seen.append((request.url.path, request.headers.get("api_key"), request.headers.get("authorization")))
    return httpx.Response(200, json={"name": "x", "photoUrls": []})

def auth(scheme, info):
    name: SecuritySchemeName = scheme.scheme
    return {"api_key": "k1"}.get(name)

client = Client("https://p/api/v3", http=httpx.Client(transport=httpx.MockTransport(handler)), auth=auth)
get_pet_by_id(client, pet_id=1)
get_inventory(client)
place_order(client)
print(seen)
"#;

#[test]
fn the_auth_hook_authenticates_operations_by_their_schemes() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &fixture("petstore3.json"));
    let out = run(&python, dir.path(), AUTH);
    // getPetById takes api_key or petstore_auth: the API key is there. getInventory takes api_key.
    // placeOrder has no security.
    assert_eq!(
        out.trim(),
        "[('/api/v3/pet/1', 'k1', None), ('/api/v3/store/inventory', 'k1', None), ('/api/v3/store/order', None, None)]"
    );
}
