//! `llms.txt` and `llms-full.txt` at the repository root: an index of the documentation for
//! language models, and all of it in one file. Generated from `README.md`, `docs/guide/`,
//! `docs/languages/` and the skill; `TSURARA_UPDATE_LLMS=1` rewrites them, and the test fails when
//! they are out of date.

use std::path::Path;
use tsurara_testkit::repo_root;

const SUMMARY: &str = "tsurara generates types and one call function per operation from OpenAPI 3.0, 3.1 and 3.2 (and Swagger 2.0) for TypeScript, Python, Rust and Go. The generator is a single binary; generated code calls a small runtime library per language and returns Results instead of throwing.";

/// The markdown files of `dir`, sorted.
fn docs(dir: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo_root().join(dir))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .map(|n| format!("{dir}/{n}"))
        .collect();
    out.sort();
    out
}

/// The title (first heading) and the first paragraph of a markdown file.
fn title_and_summary(path: &str) -> (String, String) {
    let text = std::fs::read_to_string(repo_root().join(path)).unwrap();
    let mut lines = text.lines();
    let title = lines
        .by_ref()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(path)
        .to_owned();
    let paragraph: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .collect();
    let mut summary = paragraph.join(" ");
    if summary.starts_with('|') || summary.starts_with("```") || summary.starts_with('#') {
        summary.clear();
    }
    if summary.chars().count() > 240 {
        summary = summary.chars().take(237).collect::<String>() + "...";
    }
    (title, summary)
}

fn llms() -> (String, String) {
    let sections = [
        ("Guides", docs("docs/guide")),
        ("Languages", docs("docs/languages")),
        (
            "Agents",
            vec!["skills/tsurara/SKILL.md".to_owned()]
                .into_iter()
                .chain(docs("skills/tsurara/references"))
                .collect(),
        ),
    ];
    let mut index = format!(
        "# tsurara\n\n> {SUMMARY}\n\n- [README](README.md): what tsurara is, and a first TypeScript target\n"
    );
    let mut full = format!("# tsurara\n\n> {SUMMARY}\n");
    let mut append = |path: &str| {
        let text = std::fs::read_to_string(repo_root().join(path)).unwrap();
        full.push_str(&format!("\n\n<!-- {path} -->\n\n{}", text.trim_end()));
    };
    append("README.md");
    for (heading, paths) in &sections {
        index.push_str(&format!("\n## {heading}\n\n"));
        for path in paths {
            let (title, summary) = title_and_summary(path);
            if summary.is_empty() {
                index.push_str(&format!("- [{title}]({path})\n"));
            } else {
                index.push_str(&format!("- [{title}]({path}): {summary}\n"));
            }
            append(path);
        }
    }
    full.push('\n');
    (index, full)
}

#[test]
fn llms_txt_is_up_to_date() {
    let (index, full) = llms();
    let root = repo_root();
    let mut stale = Vec::new();
    for (name, want) in [("llms.txt", index), ("llms-full.txt", full)] {
        let path = Path::new(&root).join(name);
        if std::env::var_os("TSURARA_UPDATE_LLMS").is_some() {
            std::fs::write(&path, &want).unwrap();
        } else if std::fs::read_to_string(&path).unwrap_or_default() != want {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "{stale:?} out of date; run `TSURARA_UPDATE_LLMS=1 cargo test -p tsurara-cli --test llms`"
    );
}
