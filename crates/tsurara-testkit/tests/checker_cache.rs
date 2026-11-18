//! The cache of passed checks: a pass is recorded and a failure is not, so a check
//! that failed runs (and fails) again.

use tsurara_testkit::{Checker, TsOptions, TypeScriptChecker, tsc_versions, use_checker_cache_dir};

fn markers(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn passes_are_recorded_and_failures_are_not() {
    if tsc_versions().is_empty() || std::env::var("TSURARA_CHECKER_CACHE").is_ok_and(|v| v == "0") {
        return;
    }
    // A cache of its own, so that other tests running at the same time do not change the count.
    let dir = tempfile::tempdir().unwrap();
    use_checker_cache_dir(dir.path());
    let checker = TypeScriptChecker {
        options: TsOptions::default(),
    };
    let nonce = "checker cache";
    let bad = vec![(
        "bad.ts".to_owned(),
        format!("// {nonce}\nexport const x: number = 'not a number';\n"),
    )];
    assert!(checker.check(&bad).is_err());
    assert!(checker.check(&bad).is_err(), "a failure must not be cached");
    assert_eq!(markers(dir.path()), 0);
    let good = vec![(
        "good.ts".to_owned(),
        format!("// {nonce}\nexport const x: number = 1;\n"),
    )];
    assert!(checker.check(&good).is_ok());
    assert_eq!(markers(dir.path()), 1, "a pass is recorded");
    assert!(checker.check(&good).is_ok());
    assert_eq!(markers(dir.path()), 1, "the second pass is a hit");
}
