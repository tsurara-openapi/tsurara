use tsurara_testkit::{TsOptions, tsc_check, tsc_or_skip};

#[test]
fn tsc_reports_success_and_failure() {
    let Some(tsc) = tsc_or_skip() else { return };
    let ok = vec![(
        "a.ts".to_owned(),
        "export type A = { a: string };\nexport type C = typeof import('@tsurara/client');\n"
            .to_owned(),
    )];
    let r = tsc_check(&tsc, &ok, &TsOptions::default());
    assert!(r.success, "{}", r.output);
    let bad = vec![(
        "a.ts".to_owned(),
        "export const a: number = 'x';\n".to_owned(),
    )];
    let r = tsc_check(&tsc, &bad, &TsOptions::default());
    assert!(!r.success);
    assert!(r.output.contains("TS2322"), "{}", r.output);
}
