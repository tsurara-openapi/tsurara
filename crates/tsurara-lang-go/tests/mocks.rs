//! Mock values and routes (`go.mocks`): the petstore call functions answered by the
//! generated routes through `tsuraratest`, with the same samples as the TypeScript mocks.

use tsurara_lang_go::{Go, GoOptions};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, GoChecker, repo_root};

fn files(mocks: bool) -> Vec<(String, String)> {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml");
    let set = DocumentSet::from_value(parse_document(&std::fs::read(path).unwrap()).unwrap());
    let backend = Go {
        options: GoOptions {
            mocks,
            ..GoOptions::default()
        },
    };
    tsurara_backend::generate(&backend, &set, &Options::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

#[test]
fn without_the_option_there_are_no_mocks() {
    assert!(files(false).iter().all(|(p, _)| !p.starts_with("mocks")));
}

const TEST: &str = "//go:build tsurara_mocks

package api

import (
\t\"context\"
\t\"errors\"
\t\"testing\"

\ttsurara \"github.com/tsurara-openapi/tsurara/runtimes/go\"
\t\"github.com/tsurara-openapi/tsurara/runtimes/go/tsuraratest\"
)

func TestMockRoutes(t *testing.T) {
\tm := tsuraratest.NewMockTransport(MockRoutes()...).WithBasePath(\"/v1\")
\tc := tsurara.NewClient(\"https://petstore.test/v1\", tsurara.WithTransport(m))
\tres, err := ShowPetByID(context.Background(), c, \"7\", nil)
\tif err != nil || res.Data.ID != 1 || res.Data.Name != \"name\" || *res.Data.Tag != \"tag\" {
\t\tt.Fatalf(\"%+v %v\", res, err)
\t}
\tif MockPet().Name != \"name\" {
\t\tt.Fatal(\"MockPet\")
\t}
\tgone := ShowPetByIDRoute(func(*tsuraratest.Request) *tsuraratest.Response {
\t\treturn ShowPetByIDMockDefault(404, Error{Code: 404, Message: \"gone\"})
\t})
\tc = tsurara.NewClient(\"https://petstore.test/v1\", tsurara.WithTransport(tsuraratest.NewMockTransport(gone).WithBasePath(\"/v1\")))
\t_, err = ShowPetByID(context.Background(), c, \"7\", nil)
\tvar failure *tsurara.Failure[Error]
\tif !errors.As(err, &failure) || failure.Status != 404 || failure.Body.Message != \"gone\" {
\t\tt.Fatalf(\"%v\", err)
\t}
}
";

#[test]
fn routes_answer_the_call_functions() {
    let checker = GoChecker {
        tests: vec![("mocks".into(), TEST.into())],
        tags: vec!["tsurara_mocks".into()],
    };
    if let Err(e) = checker.check(&files(true)) {
        panic!("{e}");
    }
}
