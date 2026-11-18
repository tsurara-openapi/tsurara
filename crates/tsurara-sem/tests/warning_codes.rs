//! Every warning code has an explanation for `tsurara inspect warning` and is named in design.md 6.8
//!.

use tsurara_sem::WarningKind;

#[test]
fn every_warning_code_is_explained_and_documented() {
    let design =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/design.md"))
            .unwrap();
    let section = design
        .split("### 6.8 Warnings")
        .nth(1)
        .and_then(|s| s.split("\n## ").next())
        .unwrap();
    for kind in WarningKind::ALL {
        let code = kind.as_str();
        assert!(kind.explanation().len() > 40, "{code}");
        assert_eq!(WarningKind::from_code(code), Some(*kind));
        assert!(
            section.contains(&format!("`{code}`")),
            "design.md 6.8 does not name `{code}`"
        );
    }
}
