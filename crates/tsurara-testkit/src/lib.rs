//! Test helpers for tsurara. Writes generated output to a temporary directory and runs `tsc --noEmit`.

mod cache;
pub use cache::use_checker_cache_dir;

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/tsurara-testkit is two levels below the root")
        .to_path_buf()
}

/// A TypeScript compiler to run.
#[derive(Debug, Clone)]
pub struct Tsc {
    /// `7.0.2`, `5.8`: shown in failures.
    pub label: &'static str,
    program: PathBuf,
    args: Vec<PathBuf>,
}

impl Tsc {
    fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        c
    }
}

/// Path to `node_modules/.bin/tsc` (the development version). `None` without `pnpm install`.
pub fn tsc_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "tsc.cmd" } else { "tsc" };
    let path = repo_root().join("node_modules").join(".bin").join(name);
    path.exists().then_some(path)
}

/// Every installed compiler: the development version, and the minimum supported version 5.8
/// (the `typescript58` alias, run with node; design.md chapter 17).
pub fn tsc_versions() -> Vec<Tsc> {
    let mut out = Vec::new();
    if let Some(program) = tsc_path() {
        out.push(Tsc {
            label: "7.0.2",
            program,
            args: Vec::new(),
        });
    }
    let script = repo_root().join("node_modules/typescript58/bin/tsc");
    if script.exists() {
        out.push(Tsc {
            label: "5.8",
            program: PathBuf::from("node"),
            args: vec![script],
        });
    }
    out
}

/// Whether the checker of `language` must run: `TSURARA_REQUIRE_CHECKERS` lists languages
/// (`typescript,python`). Without it, a missing toolchain skips the checks (design.md 18.12).
pub fn required(language: &str) -> bool {
    std::env::var("TSURARA_REQUIRE_CHECKERS")
        .is_ok_and(|v| v.split(',').any(|l| l.trim() == language))
}

/// Compiles or type-checks generated files with a language's toolchain.
pub trait Checker {
    fn language(&self) -> &'static str;
    /// `Ok` when the files pass, or when the toolchain is missing and the checker is not required.
    fn check(&self, files: &[(String, String)]) -> Result<(), String>;
}

/// `tsc --noEmit` with every installed TypeScript version (7.0.2 and 5.8).
#[derive(Debug, Clone, Default)]
pub struct TypeScriptChecker {
    pub options: TsOptions,
}

impl Checker for TypeScriptChecker {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn check(&self, files: &[(String, String)]) -> Result<(), String> {
        // Without the toolchain, the uncached check skips or fails as it always did.
        if !(!tsc_versions().is_empty()) {
            return self.check_uncached(files);
        }
        let key = cache::key(
            "typescript",
            &format!(
                "{:?} {:?}",
                self.options,
                tsc_versions().iter().map(|t| t.label).collect::<Vec<_>>()
            ),
            files,
            &["runtimes/typescript/src"],
        );
        if cache::hit(key.as_deref()) {
            return Ok(());
        }
        let result = self.check_uncached(files);
        if result.is_ok() {
            cache::store(key.as_deref());
        }
        result
    }
}

impl TypeScriptChecker {
    /// Runs the checker, without the cache of `check`.
    pub fn check_uncached(&self, files: &[(String, String)]) -> Result<(), String> {
        for tsc in tsc_versions_or_skip() {
            let result = tsc_check(&tsc, files, &self.options);
            if !result.success {
                return Err(format!(
                    "tsc {} failed (files are in {}):\n{}",
                    tsc.label,
                    result.dir.display(),
                    result.output
                ));
            }
            let _ = std::fs::remove_dir_all(&result.dir);
        }
        Ok(())
    }
}

/// Whether to skip tests when tsc is missing. `TSURARA_REQUIRE_CHECKERS=typescript` makes them fail
/// instead (for CI).
pub fn tsc_or_skip() -> Option<Tsc> {
    tsc_versions_or_skip().into_iter().next()
}

/// Every compiler, or none to skip. When the TypeScript checker is required, both versions are.
pub fn tsc_versions_or_skip() -> Vec<Tsc> {
    let versions = tsc_versions();
    if required("typescript") && versions.len() < 2 {
        panic!(
            "tsc 7.0.2 and 5.8 are required but only {:?} were found; run `pnpm install`",
            versions.iter().map(|t| t.label).collect::<Vec<_>>()
        );
    }
    if versions.is_empty() {
        eprintln!("skipping: node_modules/.bin/tsc was not found; run `pnpm install`");
    }
    versions
}

/// Kind of `moduleResolution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    NodeNext,
    Bundler,
}

/// Settings passed to tsc. The default is the strictest combination from design.md chapter 13.
#[derive(Debug, Clone)]
pub struct TsOptions {
    pub resolution: Resolution,
    pub exact_optional_property_types: bool,
    pub no_unchecked_indexed_access: bool,
    pub isolated_declarations: bool,
    /// Consumer-side setting for output with `.ts` import extensions that is compiled to JavaScript.
    pub rewrite_relative_import_extensions: bool,
    /// Skips checking declaration files of dependencies, for a library whose own declarations do not
    /// pass the strict options (`@tanstack/vue-query` under `exactOptionalPropertyTypes`).
    pub skip_lib_check: bool,
}

impl Default for TsOptions {
    fn default() -> Self {
        Self {
            resolution: Resolution::NodeNext,
            exact_optional_property_types: true,
            no_unchecked_indexed_access: true,
            isolated_declarations: true,
            rewrite_relative_import_extensions: false,
            skip_lib_check: false,
        }
    }
}

impl TsOptions {
    /// The tsconfig matrix from chapter 13.
    pub fn matrix() -> Vec<(&'static str, TsOptions)> {
        vec![
            ("nodenext-strictest", TsOptions::default()),
            (
                "bundler-strictest",
                TsOptions {
                    resolution: Resolution::Bundler,
                    ..TsOptions::default()
                },
            ),
            (
                "nodenext-basic",
                TsOptions {
                    exact_optional_property_types: false,
                    no_unchecked_indexed_access: false,
                    isolated_declarations: false,
                    ..TsOptions::default()
                },
            ),
        ]
    }

    fn tsconfig(&self, client: &Path) -> Value {
        let (module, resolution) = match self.resolution {
            Resolution::NodeNext => ("nodenext", "nodenext"),
            Resolution::Bundler => ("esnext", "bundler"),
        };
        json!({
            "compilerOptions": {
                "target": "es2022",
                "module": module,
                "moduleResolution": resolution,
                "lib": ["es2022", "dom", "dom.iterable"],
                "types": [],
                "strict": true,
                "noEmit": true,
                "skipLibCheck": self.skip_lib_check,
                "erasableSyntaxOnly": true,
                "verbatimModuleSyntax": true,
                "allowImportingTsExtensions": true,
                "exactOptionalPropertyTypes": self.exact_optional_property_types,
                "noUncheckedIndexedAccess": self.no_unchecked_indexed_access,
                "isolatedDeclarations": self.isolated_declarations,
                "declaration": self.isolated_declarations,
                "rewriteRelativeImportExtensions": self.rewrite_relative_import_extensions,
                "paths": {
                    "@tsurara/client": [client.to_string_lossy()],
                    // Zod schemas (`typescript.validators: zod`) import `zod`, a devDependency.
                    "zod": [repo_root().join("node_modules/zod/index.d.ts").to_string_lossy()],
                    // Query options (`typescript.query: tanstack`) import `@tanstack/query-core`;
                    // tests use them with `@tanstack/react-query`, devDependencies too.
                    "@tanstack/query-core": [repo_root().join("node_modules/@tanstack/query-core/build/modern/index.d.ts").to_string_lossy()],
                    "@tanstack/react-query": [repo_root().join("node_modules/@tanstack/react-query/build/modern/index.d.ts").to_string_lossy()],
                    "react": [repo_root().join("node_modules/@types/react/index.d.ts").to_string_lossy()],
                    // A second adapter, to check that the options are framework-neutral.
                    "@tanstack/vue-query": [repo_root().join("node_modules/@tanstack/vue-query/build/modern/index.d.ts").to_string_lossy()],
                    "vue": [repo_root().join("node_modules/vue/dist/vue.d.ts").to_string_lossy()],
                    // Mock handlers (`typescript.mocks: msw`) import `msw`; `@tsurara/client/testing`
                    // is the runtime's mock transport (`typescript.mocks: fetch`).
                    "msw": [repo_root().join("node_modules/msw/lib/core/index.d.ts").to_string_lossy()],
                    "msw/node": [repo_root().join("node_modules/msw/lib/node/index.d.ts").to_string_lossy()],
                    "@tsurara/client/testing": [repo_root().join("runtimes/typescript/src/testing.ts").to_string_lossy()],
                    // The SWR example plugin's hooks (`examples/plugin-swr`).
                    "swr": [repo_root().join("node_modules/swr/dist/index/index.d.mts").to_string_lossy()]
                }
            },
            "include": ["**/*.ts"]
        })
    }
}

/// Result of tsc.
#[derive(Debug)]
pub struct TscOutput {
    pub success: bool,
    pub output: String,
    pub dir: PathBuf,
}

/// Writes files to a temporary directory and runs `tsc --noEmit`.
/// `@tsurara/client` points to `runtimes/typescript/src`.
pub fn tsc_check(tsc: &Tsc, files: &[(String, String)], options: &TsOptions) -> TscOutput {
    let dir = tempfile::Builder::new()
        .prefix("tsurara-tsc-")
        .tempdir()
        .expect("temp dir")
        .keep();
    tsc_check_in(tsc, &dir, files, options)
}

/// Writes to `dir` and runs. The directory is kept so its contents can be inspected on failure.
pub fn tsc_check_in(
    tsc: &Tsc,
    dir: &Path,
    files: &[(String, String)],
    options: &TsOptions,
) -> TscOutput {
    for (path, contents) in files {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().expect("file has a parent")).expect("mkdir");
        std::fs::write(&full, contents).expect("write");
    }
    let client = repo_root().join("runtimes/typescript/src/index.ts");
    let tsconfig = options.tsconfig(&client);
    std::fs::write(
        dir.join("tsconfig.json"),
        serde_json::to_string_pretty(&tsconfig).expect("json"),
    )
    .expect("write tsconfig");
    std::fs::write(dir.join("package.json"), r#"{ "type": "module" }"#).expect("write");
    let out = tsc
        .command()
        .arg("-p")
        .arg(dir.join("tsconfig.json"))
        .arg("--pretty")
        .arg("false")
        .output()
        .expect("run tsc");
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    TscOutput {
        success: out.status.success(),
        output,
        dir: dir.to_path_buf(),
    }
}

/// Fails, printing the errors and the output location, unless tsc passes.
/// Runs every installed compiler (7.0.2 and 5.8).
pub fn assert_tsc_passes(files: &[(String, String)], options: &TsOptions) {
    let checker = TypeScriptChecker {
        options: options.clone(),
    };
    if let Err(e) = checker.check(files) {
        panic!("{e}");
    }
}

/// `node_modules/.bin/pyright`, or `None` without `pnpm install`.
pub fn pyright_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "pyright.cmd"
    } else {
        "pyright"
    };
    let path = repo_root().join("node_modules").join(".bin").join(name);
    path.exists().then_some(path)
}

/// The virtual environment of the Python runtime (`uv sync --project runtimes/python`), where
/// `tsurara_client` and httpx are installed.
pub fn python_runtime_venv() -> PathBuf {
    repo_root().join("runtimes/python/.venv")
}

/// A Python 3.11 or later interpreter with the runtime installed (`TSURARA_PYTHON`, or the runtime's
/// virtual environment), or `None` to skip. Required when the Python checker is.
pub fn python_or_skip() -> Option<PathBuf> {
    let venv = python_runtime_venv().join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    let program = std::env::var_os("TSURARA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or(venv);
    let version = Command::new(&program)
        .args([
            "-c",
            "import sys; print(sys.version_info[0], sys.version_info[1])",
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let ok = version.as_deref().is_some_and(|v| {
        let mut p = v.split(' ').filter_map(|x| x.parse::<u32>().ok());
        matches!((p.next(), p.next()), (Some(3), Some(m)) if m >= 11)
    });
    match (ok, required("python")) {
        (true, _) => Some(program),
        (false, true) => panic!(
            "Python 3.11 or later with the runtime is required, found {version:?} at {}; run `uv sync --project runtimes/python`",
            program.display()
        ),
        (false, false) => {
            eprintln!("skipping: Python 3.11 or later was not found ({version:?})");
            None
        }
    }
}

/// `pyright` on a generated package, as Python 3.11 (the minimum version of generated code).
#[derive(Debug, Clone)]
pub struct PythonChecker {
    /// The package directory the files are written into.
    pub package: String,
}

impl Checker for PythonChecker {
    fn language(&self) -> &'static str {
        "python"
    }

    fn check(&self, files: &[(String, String)]) -> Result<(), String> {
        // Without the toolchain, the uncached check skips or fails as it always did.
        if !(pyright_path().is_some()) {
            return self.check_uncached(files);
        }
        let key = cache::key(
            "python",
            &format!("{self:?}"),
            files,
            &[
                "runtimes/python/src",
                "runtimes/python/pyproject.toml",
                "runtimes/python/uv.lock",
            ],
        );
        if cache::hit(key.as_deref()) {
            return Ok(());
        }
        let result = self.check_uncached(files);
        if result.is_ok() {
            cache::store(key.as_deref());
        }
        result
    }
}

impl PythonChecker {
    /// Runs the checker, without the cache of `check`.
    pub fn check_uncached(&self, files: &[(String, String)]) -> Result<(), String> {
        let Some(pyright) = pyright_path() else {
            if required("python") {
                return Err(
                    "pyright is required but node_modules/.bin/pyright was not found".into(),
                );
            }
            eprintln!("skipping: node_modules/.bin/pyright was not found; run `pnpm install`");
            return Ok(());
        };
        let dir = tempfile::Builder::new()
            .prefix("tsurara-pyright-")
            .tempdir()
            .expect("temp dir")
            .keep();
        for (path, contents) in files {
            let full = dir.join(&self.package).join(path);
            std::fs::create_dir_all(full.parent().expect("file has a parent")).expect("mkdir");
            std::fs::write(&full, contents).expect("write");
        }
        let config = json!({
            "include": [self.package],
            "pythonVersion": "3.11",
            "typeCheckingMode": "standard",
            "reportMissingModuleSource": false,
            // Generated code imports the runtime, installed in its virtual environment.
            "venvPath": repo_root().join("runtimes/python"),
            "venv": ".venv",
        });
        std::fs::write(dir.join("pyrightconfig.json"), config.to_string()).expect("write");
        let out = Command::new(pyright)
            .arg("-p")
            .arg(dir.join("pyrightconfig.json"))
            .output()
            .expect("run pyright");
        if out.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            Ok(())
        } else {
            Err(format!(
                "pyright failed (files are in {}):\n{}",
                dir.display(),
                String::from_utf8_lossy(&out.stdout)
            ))
        }
    }
}

/// Generated Rust in a temporary crate that depends on the runtime (`runtimes/rust`), checked with
/// `cargo clippy -D warnings`; with `tests`, also `cargo test`. The target directory is shared
/// (`target/rust-checker`), so the runtime and its dependencies are built once.
#[derive(Debug, Clone, Default)]
pub struct RustChecker {
    /// Test files (`tests/<name>.rs`) to run against the generated module `api`.
    pub tests: Vec<(String, String)>,
    /// Enable the runtime's reqwest transport, for tests that call a server.
    pub http: bool,
    /// Give the crate a `mocks` feature that enables the runtime's `testing` feature, and check with
    /// it (generated mocks, `rust.mocks`).
    pub mocks: bool,
    /// Depend on the runtime without its `client` feature (only the types layer: no tokio, no
    /// reqwest), for output generated with `operations: false`.
    pub types_only: bool,
    /// The output has a vendored runtime (`runtime: vendor`): the crate depends on these lines of
    /// `[dependencies]` instead of the runtime crate.
    pub vendored: Vec<String>,
}

impl Checker for RustChecker {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn check(&self, files: &[(String, String)]) -> Result<(), String> {
        // Without the toolchain, the uncached check skips or fails as it always did.
        if !(Command::new("cargo").arg("--version").output().is_ok()) {
            return self.check_uncached(files);
        }
        let key = cache::key(
            "rust",
            &format!("{self:?}"),
            files,
            &[
                "runtimes/rust/src",
                "runtimes/rust/Cargo.toml",
                "runtimes/rust/Cargo.lock",
            ],
        );
        if cache::hit(key.as_deref()) {
            return Ok(());
        }
        let result = self.check_uncached(files);
        if result.is_ok() {
            cache::store(key.as_deref());
        }
        result
    }
}

impl RustChecker {
    /// Runs the checker, without the cache of `check`.
    pub fn check_uncached(&self, files: &[(String, String)]) -> Result<(), String> {
        if Command::new("cargo").arg("--version").output().is_err() {
            if required("rust") {
                return Err("cargo is required".into());
            }
            eprintln!("skipping: cargo was not found");
            return Ok(());
        }
        let dir = tempfile::Builder::new()
            .prefix("tsurara-rust-")
            .tempdir()
            .expect("temp dir")
            .keep();
        let runtime = repo_root().join("runtimes/rust");
        std::fs::create_dir_all(dir.join("src/api")).expect("mkdir");
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"tsurara-generated\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n\n[features]\nmocks = [{}]\n\n[dependencies]\n{}\n\n[dev-dependencies]\ntokio = {{ version = \"1\", features = [\"io-util\", \"macros\", \"net\", \"rt-multi-thread\", \"time\"] }}\n",
                if self.vendored.is_empty() {
                    "\"tsurara-client/testing\""
                } else {
                    ""
                },
                if self.vendored.is_empty() {
                    format!(
                        "tsurara-client = {{ path = {:?}, default-features = false, features = [{}] }}",
                        runtime.to_string_lossy(),
                        match (self.types_only, self.http) {
                            (true, _) => "",
                            (false, false) => "\"client\"",
                            (false, true) => "\"client\", \"reqwest\"",
                        }
                    )
                } else {
                    self.vendored.join("\n")
                }
            ),
        )
        .expect("write");
        // The runtime's lock file pins the same dependency versions without resolving again.
        let _ = std::fs::copy(runtime.join("Cargo.lock"), dir.join("Cargo.lock"));
        std::fs::write(dir.join("src/lib.rs"), "pub mod api;\n").expect("write");
        for (path, contents) in files {
            let full = dir.join("src/api").join(path);
            std::fs::create_dir_all(full.parent().expect("file has a parent")).expect("mkdir");
            std::fs::write(&full, contents).expect("write");
        }
        for (name, contents) in &self.tests {
            std::fs::create_dir_all(dir.join("tests")).expect("mkdir");
            std::fs::write(dir.join("tests").join(format!("{name}.rs")), contents).expect("write");
        }
        let target = repo_root().join("target/rust-checker");
        let run = |args: &[&str]| {
            Command::new("cargo")
                .args(args)
                .current_dir(&dir)
                .env("CARGO_TARGET_DIR", &target)
                .output()
                .expect("run cargo")
        };
        let features: &[&str] = if self.mocks {
            &["--features", "mocks"]
        } else {
            &[]
        };
        let mut clippy = vec!["clippy", "--quiet", "--all-targets"];
        clippy.extend(features);
        clippy.extend(["--", "-D", "warnings"]);
        let mut outputs = vec![run(&clippy)];
        if outputs[0].status.success() && !self.tests.is_empty() {
            let mut test = vec!["test", "--quiet"];
            test.extend(features);
            outputs.push(run(&test));
        }
        match outputs.iter().find(|o| !o.status.success()) {
            None => {
                let _ = std::fs::remove_dir_all(&dir);
                Ok(())
            }
            Some(out) => Err(format!(
                "cargo failed (the crate is in {}):\n{}{}",
                dir.display(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )),
        }
    }
}

/// Writes Go output into a temporary module (package directory `api`, with the runtime in
/// `runtimes/go` replaced locally) and requires `gofmt -l` to print nothing, then runs `go vet` and
/// `go build`, and `go test` when there are tests. `go` runs from the repository root, so that the
/// version in `mise.toml` is used.
#[derive(Debug, Default)]
pub struct GoChecker {
    /// Test files (`<name>_test.go`) in the package directory, run with `go test`.
    pub tests: Vec<(String, String)>,
    /// Build tags for `go vet`, `go build` and `go test` (`tsurara_mocks` for generated mocks).
    pub tags: Vec<String>,
}

impl Checker for GoChecker {
    fn language(&self) -> &'static str {
        "go"
    }

    fn check(&self, files: &[(String, String)]) -> Result<(), String> {
        // Without the toolchain, the uncached check skips or fails as it always did.
        if !(Command::new("go")
            .arg("version")
            .current_dir(repo_root())
            .env("GOTOOLCHAIN", "local")
            .output()
            .is_ok_and(|o| o.status.success()))
        {
            return self.check_uncached(files);
        }
        let key = cache::key("go", &format!("{self:?}"), files, &["runtimes/go"]);
        if cache::hit(key.as_deref()) {
            return Ok(());
        }
        let result = self.check_uncached(files);
        if result.is_ok() {
            cache::store(key.as_deref());
        }
        result
    }
}

impl GoChecker {
    /// Runs the checker, without the cache of `check`.
    pub fn check_uncached(&self, files: &[(String, String)]) -> Result<(), String> {
        let root = repo_root();
        let go = |args: &[&str]| {
            Command::new("go")
                .args(args)
                .current_dir(&root)
                .env("GOTOOLCHAIN", "local")
                .env("GOFLAGS", "-mod=mod")
                .output()
        };
        if !go(&["version"]).is_ok_and(|o| o.status.success()) {
            if required("go") {
                return Err("go is required".into());
            }
            eprintln!("skipping: go was not found");
            return Ok(());
        }
        let dir = tempfile::Builder::new()
            .prefix("tsurara-go-")
            .tempdir()
            .expect("temp dir")
            .keep();
        let runtime = root.join("runtimes/go");
        std::fs::write(
            dir.join("go.mod"),
            format!(
                "module example.com/gen\n\ngo 1.24\n\nrequire github.com/tsurara-openapi/tsurara/runtimes/go v0.0.0\n\nreplace github.com/tsurara-openapi/tsurara/runtimes/go => {}\n",
                runtime.display()
            ),
        )
        .expect("write");
        let package = dir.join("api");
        for (path, contents) in files {
            let full = package.join(path);
            std::fs::create_dir_all(full.parent().expect("file has a parent")).expect("mkdir");
            std::fs::write(&full, contents).expect("write");
        }
        for (name, contents) in &self.tests {
            std::fs::write(package.join(format!("{name}_test.go")), contents).expect("write");
        }
        let fail = |what: &str, out: &std::process::Output| {
            Err(format!(
                "{what} failed (the module is in {}):\n{}{}",
                dir.display(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ))
        };
        let module = dir.to_string_lossy().into_owned();
        let gofmt = Command::new("gofmt")
            .arg("-l")
            .arg(&package)
            .current_dir(&root)
            .output()
            .expect("run gofmt");
        if !gofmt.status.success() || !gofmt.stdout.is_empty() {
            return fail("gofmt -l", &gofmt);
        }
        let tags = self.tags.join(",");
        let with_tags = |mut args: Vec<&'static str>, tags: &str| -> Vec<String> {
            let mut out: Vec<String> = args.drain(..1).map(str::to_owned).collect();
            if !tags.is_empty() {
                out.push(format!("-tags={tags}"));
            }
            out.extend(args.into_iter().map(str::to_owned));
            out
        };
        for args in [
            with_tags(vec!["vet", "./..."], &tags),
            with_tags(vec!["build", "./..."], &tags),
        ] {
            let mut full = vec!["-C".to_owned(), module.clone()];
            full.extend(args);
            let args: Vec<&str> = full.iter().map(String::as_str).collect();
            let out = go(&args).expect("run go");
            if !out.status.success() {
                return fail(&format!("go {}", args[2]), &out);
            }
        }
        if !self.tests.is_empty() {
            let mut full = vec!["-C".to_owned(), module.clone()];
            full.extend(with_tags(vec!["test", "./..."], &tags));
            let args: Vec<&str> = full.iter().map(String::as_str).collect();
            let out = go(&args).expect("run go");
            if !out.status.success() {
                return fail("go test", &out);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
