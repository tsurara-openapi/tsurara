//! A path variable without a path parameter is an argument of the call function in every language,
//! and a path parameter that is not in the template is in none (`corpus/edge-cases/path-template.yaml`).

use tsurara_loader::DocumentSet;
use tsurara_sem::Options;
use tsurara_testkit::repo_root;

fn files(output: tsurara_backend::Output) -> Vec<(String, String)> {
    output
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn contents(files: &[(String, String)], path: &str) -> String {
    files
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| {
            panic!(
                "no {path} in {:?}",
                files.iter().map(|f| &f.0).collect::<Vec<_>>()
            )
        })
        .1
        .clone()
}

#[test]
fn every_language_takes_the_variable_and_drops_the_stray_parameter() {
    let text = std::fs::read(repo_root().join("corpus/edge-cases/path-template.yaml")).unwrap();
    let set = DocumentSet::from_value(tsurara_loader::parse_document(&text).unwrap());
    let opts = Options::default();
    let ts = files(tsurara_lang_typescript::generate(&set, &opts, &Default::default()).unwrap());
    let py = files(tsurara_lang_python::generate(&set, &opts, &Default::default()).unwrap());
    let rs = files(tsurara_lang_rust::generate(&set, &opts, &Default::default()).unwrap());
    let go = files(tsurara_lang_go::generate(&set, &opts, &Default::default()).unwrap());

    assert!(contents(&ts, "operations/echoPost.ts").contains("path: { id: string };"));
    assert!(contents(&py, "operations/echo_post.py").contains("*, id: str,"));
    assert!(
        contents(&rs, "operations/echo_post.rs")
            .contains("pub async fn echo_post(client: &tsurara_client::Client, id: String,")
    );
    assert!(
        contents(&go, "echo_post_op_gen.go")
            .contains("func EchoPost(ctx context.Context, c *tsurara.Client, id string,")
    );
    for (lang, files) in [("ts", &ts), ("py", &py), ("rs", &rs), ("go", &go)] {
        for (path, text) in files.iter() {
            let lower = text.to_ascii_lowercase();
            assert!(
                !lower.contains("ownerid") && !lower.contains("owner_id"),
                "{lang} {path} mentions the stray parameter"
            );
        }
    }
}
