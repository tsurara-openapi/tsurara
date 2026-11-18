//! The embedded sources are the files of `runtimes/`, without tests.

use std::path::Path;

fn check(list: &[(&str, &str)], dir: &str, expected: &[&str]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtimes")
        .join(dir);
    for (path, contents) in list {
        let on_disk = std::fs::read_to_string(root.join(path)).unwrap();
        assert_eq!(&on_disk, contents, "{dir}/{path}");
        assert!(!path.contains("test.") && !path.contains("_test"), "{path}");
    }
    for name in expected {
        assert!(
            list.iter().any(|(p, _)| p == name),
            "{dir}/{name} is missing"
        );
    }
}

#[test]
fn embedded_files_equal_the_runtimes() {
    check(
        tsurara_runtimes::TYPESCRIPT,
        "typescript/src",
        &["index.ts", "testing.ts"],
    );
    check(
        tsurara_runtimes::PYTHON,
        "python/src/tsurara_client",
        &["__init__.py", "py.typed", "testing.py"],
    );
    check(
        tsurara_runtimes::RUST,
        "rust/src",
        &["lib.rs", "client.rs", "macros.rs"],
    );
    check(
        tsurara_runtimes::GO,
        "go",
        &["client.go", "types.go", "tsuraratest/mock.go"],
    );
}
