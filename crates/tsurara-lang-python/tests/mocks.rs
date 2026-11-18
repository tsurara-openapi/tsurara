//! Mock factories and routes (`python.mocks`): the petstore call functions answered by the
//! generated routes through `MockRouter`, with the same samples as the TypeScript mocks.

use std::path::Path;
use std::process::Command;
use tsurara_lang_python::{PythonOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, PythonChecker, python_or_skip, repo_root};

fn petstore() -> DocumentSet {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml");
    DocumentSet::from_value(parse_document(&std::fs::read(path).unwrap()).unwrap())
}

fn files(mocks: bool) -> Vec<(String, String)> {
    let opts = PythonOptions {
        mocks,
        ..PythonOptions::default()
    };
    generate(&petstore(), &Options::default(), &opts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

#[test]
fn without_the_option_there_are_no_mocks() {
    assert!(files(false).iter().all(|(p, _)| !p.starts_with("mocks/")));
}

const SCRIPT: &str = r#"
import httpx
from tsurara_client import Client, HttpError, Success
from tsurara_client.testing import MockRouter

from api.error import Error
from api.mocks.operations import list_pets_route, routes, show_pet_by_id_default, show_pet_by_id_route
from api.mocks.schemas import mock_pet
from api.operations.list_pets import list_pets
from api.operations.show_pet_by_id import show_pet_by_id
from api.pet import Pet

router = MockRouter(base_path="/v1")
router.extend(routes())
with Client("https://petstore.test/v1", http=httpx.Client(transport=httpx.MockTransport(router))) as client:
    shown = show_pet_by_id(client, pet_id="7")
    assert isinstance(shown, Success), shown
    assert shown.data == Pet(id=1, name="name", tag="tag"), shown.data
    listed = list_pets(client)
    assert isinstance(listed, Success) and listed.data == [Pet(id=1, name="name", tag="tag")], listed

assert mock_pet(name="Tama") == Pet(id=1, name="Tama", tag="tag")

router = MockRouter(base_path="/v1")
router.extend([show_pet_by_id_route(show_pet_by_id_default(404, Error(code=404, message="gone"))), list_pets_route()])
with Client("https://petstore.test/v1", http=httpx.Client(transport=httpx.MockTransport(router))) as client:
    gone = show_pet_by_id(client, pet_id="7")
    assert isinstance(gone, HttpError) and gone.status == 404 and gone.error == Error(code=404, message="gone"), gone
print("ok")
"#;

#[test]
fn routes_answer_the_call_functions() {
    let files = files(true);
    let mut checked = files.clone();
    checked.push((
        "check_mocks.py".into(),
        SCRIPT.replace("from api.", "from .").replace("api.", ""),
    ));
    if let Err(e) = (PythonChecker {
        package: "api".into(),
    })
    .check(&checked)
    {
        panic!("{e}");
    }
    let Some(python) = python_or_skip() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in &files {
        let full = dir.path().join("api").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    std::fs::write(dir.path().join("script.py"), SCRIPT).unwrap();
    let out = Command::new(python)
        .arg("script.py")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("ok"),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = Path::new("");
}
