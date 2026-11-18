//! The agent skill (`skills/tsurara`): every code block of a language reference compiles
//! next to the petstore output of that language, and the Claude Code plugin at the repository root
//! carries the skill.

use tsurara_backend::{Backend, OutputFile, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, repo_root,
};

fn generated(backend: &dyn Backend) -> Vec<(String, String)> {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml");
    let doc = parse_document(&std::fs::read(path).unwrap()).unwrap();
    let out = generate(backend, &DocumentSet::from_value(doc), &Options::default()).unwrap();
    out.files
        .into_iter()
        .map(|OutputFile { path, contents }| (path, contents))
        .collect()
}

/// The fenced code blocks of `reference` whose info string is exactly `language`.
fn blocks(reference: &str, language: &str) -> Vec<String> {
    let text = std::fs::read_to_string(
        repo_root()
            .join("skills/tsurara/references")
            .join(reference),
    )
    .unwrap();
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match &mut current {
            None if line == format!("```{language}") => current = Some(String::new()),
            Some(code) if line == "```" => {
                out.push(std::mem::take(code));
                current = None;
            }
            Some(code) => {
                code.push_str(line);
                code.push('\n');
            }
            None => {}
        }
    }
    assert!(!out.is_empty(), "{reference} has no {language} block");
    out
}

#[test]
fn typescript_reference_compiles() {
    let mut files = generated(&tsurara_lang_typescript::TypeScript::default());
    for (i, code) in blocks("typescript.md", "ts").into_iter().enumerate() {
        files.push((format!("skill{i}.ts"), code));
    }
    if let Err(e) = (TypeScriptChecker {
        options: TsOptions::default(),
    })
    .check(&files)
    {
        panic!("{e}");
    }
}

/// The TanStack Query and mock reference compiles against the output with query options, MSW
/// handlers and Zod schemas.
#[test]
fn tanstack_query_reference_compiles() {
    let backend = tsurara_lang_typescript::TypeScript {
        options: tsurara_lang_typescript::TypeScriptOptions {
            query: Some(tsurara_lang_typescript::options::Query::Tanstack),
            mocks: Some(tsurara_lang_typescript::options::Mocks::Msw),
            validators: Some(tsurara_lang_typescript::options::Validators::Zod),
            ..Default::default()
        },
    };
    let mut files = generated(&backend);
    for (i, code) in blocks("tanstack-query.md", "ts").into_iter().enumerate() {
        files.push((format!("skill{i}.ts"), code));
    }
    if let Err(e) = (TypeScriptChecker {
        options: TsOptions::default(),
    })
    .check(&files)
    {
        panic!("{e}");
    }
}

#[test]
fn python_reference_compiles() {
    let mut files = generated(&tsurara_lang_python::Python::default());
    for (i, code) in blocks("python.md", "python").into_iter().enumerate() {
        files.push((format!("skill{i}.py"), code));
    }
    if let Err(e) = (PythonChecker {
        package: "api".into(),
    })
    .check(&files)
    {
        panic!("{e}");
    }
}

#[test]
fn rust_reference_compiles() {
    let mut files = generated(&tsurara_lang_rust::Rust::default());
    let mut modules = String::new();
    for (i, code) in blocks("rust.md", "rust").into_iter().enumerate() {
        files.push((format!("skill{i}.rs"), code));
        modules.push_str(&format!("pub mod skill{i};\n"));
    }
    files
        .iter_mut()
        .find(|(p, _)| p == "mod.rs")
        .unwrap()
        .1
        .push_str(&modules);
    if let Err(e) = RustChecker::default().check(&files) {
        panic!("{e}");
    }
}

#[test]
fn go_reference_compiles() {
    let mut files = generated(&tsurara_lang_go::Go::default());
    for (i, code) in blocks("go.md", "go").into_iter().enumerate() {
        files.push((format!("skill{i}.go"), code));
    }
    if let Err(e) = GoChecker::default().check(&files) {
        panic!("{e}");
    }
}

#[test]
fn the_plugin_carries_the_skill_and_the_server() {
    let plugin: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join(".claude-plugin/plugin.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(plugin["mcpServers"]["tsurara"]["args"][0], "mcp");
    // Claude Code finds skills in `skills/<name>/SKILL.md` at the plugin root.
    let skill = std::fs::read_to_string(repo_root().join("skills/tsurara/SKILL.md")).unwrap();
    assert!(
        skill.starts_with("---\nname: tsurara\ndescription: "),
        "{skill}"
    );
    for reference in ["typescript", "python", "rust", "go", "configuration"] {
        assert!(
            skill.contains(&format!("(references/{reference}.md)")),
            "SKILL.md does not link {reference}"
        );
    }
}
