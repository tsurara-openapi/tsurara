//! Every published package carries the license text, which the MIT License asks of copies: each
//! package directory has a copy of the root `LICENSE`, and the third-party files in the repository
//! carry theirs (THIRD_PARTY_NOTICES.md).

use tsurara_testkit::repo_root;

/// Directories that are published on their own (crates, npm, PyPI and Go modules).
fn packages() -> Vec<String> {
    let mut dirs: Vec<String> = [
        "runtimes/typescript",
        "runtimes/python",
        "runtimes/rust",
        "runtimes/go",
        "packages/plugin",
        "packages/tsurara",
        "packages/plugin-python",
        "packages/plugin-go",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    for entry in std::fs::read_dir(repo_root().join("crates")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        // The test kit is not published.
        if name != "tsurara-testkit" {
            dirs.push(format!("crates/{name}"));
        }
    }
    dirs.sort();
    dirs
}

#[test]
fn every_package_has_the_license() {
    let root = repo_root();
    let license = std::fs::read_to_string(root.join("LICENSE")).unwrap();
    for dir in packages() {
        let copy = std::fs::read_to_string(root.join(&dir).join("LICENSE"))
            .unwrap_or_else(|_| panic!("{dir}/LICENSE is missing; copy the root LICENSE there"));
        assert_eq!(copy, license, "{dir}/LICENSE differs from the root LICENSE");
    }
}

#[test]
fn third_party_files_have_their_licenses() {
    let root = repo_root();
    for path in [
        "LICENSES/Apache-2.0.txt",
        "LICENSES/CC-BY-4.0.txt",
        "runtimes/typescript/hermes/rn/LICENSE",
        "THIRD_PARTY_NOTICES.md",
    ] {
        assert!(root.join(path).exists(), "{path} is missing");
    }
}
