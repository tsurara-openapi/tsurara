//! Go's naming policy is expressible as a data policy: the data form names every key of
//! the edge-case corpus as `GoPolicy` does, in every context.

use std::collections::BTreeMap;
use tsurara_lang_go::naming::GoPolicy;
use tsurara_naming::data::{Case, ContextRules, DataPolicy};
use tsurara_naming::{Context, NamingPolicy};
use tsurara_testkit::repo_root;

const INITIALISMS: &[&str] = &[
    "ACL", "API", "ASCII", "CPU", "CSS", "DNS", "EOF", "GUID", "HTML", "HTTP", "HTTPS", "ID", "IP",
    "JSON", "LHS", "QPS", "RAM", "RHS", "RPC", "SLA", "SMTP", "SQL", "SSH", "TCP", "TLS", "TTL",
    "UDP", "UI", "UID", "UUID", "URI", "URL", "UTF8", "VM", "XML", "XMPP", "XSRF", "XSS",
];

fn rules(case: Case, fallback: &str, reserved: &[&str], suffix: &str) -> ContextRules {
    ContextRules {
        case,
        // File names may start with a digit.
        digit_prefix: if case == Case::Snake {
            String::new()
        } else {
            "V".into()
        },
        upper_prefix: if case == Case::Pascal {
            "X".into()
        } else {
            String::new()
        },
        fallback: fallback.into(),
        ascii: case == Case::Snake,
        reserved: reserved.iter().map(|s| (*s).to_owned()).collect(),
        reserved_suffix: suffix.into(),
    }
}

fn go_like() -> DataPolicy {
    DataPolicy {
        contexts: BTreeMap::from([
            ("type".into(), rules(Case::Pascal, "Value", &[], "Type")),
            ("function".into(), rules(Case::Pascal, "Value", &[], "Type")),
            (
                "parameter".into(),
                rules(Case::Pascal, "Value", &[], "Type"),
            ),
            (
                "member".into(),
                rules(
                    Case::Pascal,
                    "Value",
                    &["MarshalJSON", "UnmarshalJSON"],
                    "Field",
                ),
            ),
            ("constant".into(), rules(Case::Pascal, "Empty", &[], "Type")),
            ("file".into(), rules(Case::Snake, "value", &[], "_")),
        ]),
        initialisms: INITIALISMS.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn keys(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                out.push(k.clone());
                keys(x, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        serde_json::Value::String(s) if s.len() < 40 => out.push(s.clone()),
        _ => {}
    }
}

#[test]
fn the_data_form_names_like_go_policy() {
    let mut raws = Vec::new();
    for entry in std::fs::read_dir(repo_root().join("corpus/edge-cases")).unwrap() {
        let text = std::fs::read(entry.unwrap().path()).unwrap();
        let doc = tsurara_loader::parse_document(&text).unwrap();
        keys(&doc, &mut raws);
    }
    raws.extend(
        [
            "1st",
            "日本語",
            "S日本語",
            "--",
            "MarshalJSON",
            "ids",
            "photoUrls",
        ]
        .map(String::from),
    );
    assert!(raws.len() > 500, "{}", raws.len());
    let data = go_like();
    for context in [
        Context::Type,
        Context::Function,
        Context::Member,
        Context::Parameter,
        Context::Constant,
        Context::File,
    ] {
        for raw in &raws {
            let go = GoPolicy.convert(context, raw, false);
            assert_eq!(data.convert(context, raw, false), go, "{context:?} {raw:?}");
            assert_eq!(
                data.is_reserved(context, &go),
                GoPolicy.is_reserved(context, &go)
            );
            assert_eq!(
                data.reserved_suffix(context),
                GoPolicy.reserved_suffix(context)
            );
        }
    }
}
