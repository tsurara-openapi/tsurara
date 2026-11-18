//! `tsurara mcp` (design.md 12.1): a Model Context Protocol server over stdio. Messages are
//! JSON-RPC 2.0, one per line. The tools are thin layers over `inspect`, `generate` and `check`; the
//! guides of the installed version are resources.

use crate::inspect::{self, Common};
use crate::{Error, Format, check_report, generate_report, load_targets};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// The protocol version this server speaks when the client asks for one it does not know.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The guides, embedded at build time so that agents read the documentation of this version.
const DOCS: &[(&str, &str)] = &[
    (
        "docs/guide/ai.md",
        include_str!("../../../docs/guide/ai.md"),
    ),
    (
        "docs/guide/backends.md",
        include_str!("../../../docs/guide/backends.md"),
    ),
    (
        "docs/guide/cli.md",
        include_str!("../../../docs/guide/cli.md"),
    ),
    (
        "docs/guide/client.md",
        include_str!("../../../docs/guide/client.md"),
    ),
    (
        "docs/guide/configuration.md",
        include_str!("../../../docs/guide/configuration.md"),
    ),
    (
        "docs/guide/migration.md",
        include_str!("../../../docs/guide/migration.md"),
    ),
    (
        "docs/guide/mocks.md",
        include_str!("../../../docs/guide/mocks.md"),
    ),
    (
        "docs/guide/monorepo.md",
        include_str!("../../../docs/guide/monorepo.md"),
    ),
    (
        "docs/guide/plugins.md",
        include_str!("../../../docs/guide/plugins.md"),
    ),
    (
        "docs/guide/react.md",
        include_str!("../../../docs/guide/react.md"),
    ),
    (
        "docs/guide/types.md",
        include_str!("../../../docs/guide/types.md"),
    ),
    (
        "docs/guide/validation.md",
        include_str!("../../../docs/guide/validation.md"),
    ),
    (
        "docs/languages/go.md",
        include_str!("../../../docs/languages/go.md"),
    ),
    (
        "docs/languages/python.md",
        include_str!("../../../docs/languages/python.md"),
    ),
    (
        "docs/languages/rust.md",
        include_str!("../../../docs/languages/rust.md"),
    ),
];

pub fn serve() -> Result<(), Error> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(response) = response {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
    Ok(())
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The response to one message; `None` for notifications.
fn handle(message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": "tsurara", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Answers about the OpenAPI definition and the code tsurara generates from it, for the targets in tsurara.config.yaml. Use search_operations to find the generated function for an endpoint, then describe_operation for its parameters, responses and a call example in the target's language.",
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(name, &args) {
                Ok((summary, value)) => json!({
                    "content": [{ "type": "text", "text": summary }],
                    "structuredContent": value,
                    "isError": false,
                }),
                Err(e) => json!({
                    "content": [{ "type": "text", "text": e }],
                    "isError": true,
                }),
            }
        }
        "resources/list" => json!({
            "resources": DOCS.iter().map(|(path, _)| json!({
                "uri": format!("tsurara://{path}"),
                "name": path.rsplit('/').next(),
                "mimeType": "text/markdown",
            })).collect::<Vec<_>>(),
        }),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            match DOCS
                .iter()
                .find(|(path, _)| uri.strip_prefix("tsurara://") == Some(path))
            {
                Some((_, text)) => json!({
                    "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": text }],
                }),
                None => return Some(error(id, -32002, &format!("no resource {uri}"))),
            }
        }
        _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn tools() -> Vec<Value> {
    let target = json!({
        "type": "string",
        "description": "The target in tsurara.config.yaml; required when there are several (list_targets)",
    });
    vec![
        json!({
            "name": "list_targets",
            "description": "The targets of tsurara.config.yaml: name, language, input and output.",
            "inputSchema": object_schema(json!({}), &[]),
        }),
        json!({
            "name": "search_operations",
            "description": "Find operations whose id, method, path, summary or tags contain every word of `query` (\"find pet by id\" finds getPetById). Returns operation ids, summaries and the generated function and file.",
            "inputSchema": object_schema(json!({
                "query": { "type": "string" },
                "tag": { "type": "string" },
                "target": target,
                "offset": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1, "description": "Default 50" },
            }), &[]),
        }),
        json!({
            "name": "describe_operation",
            "description": "One operation by id or \"METHOD /path\": parameters, request body, responses, the generated function and types, and a call example in the target's language that compiles next to the generated code.",
            "inputSchema": object_schema(json!({
                "operation": { "type": "string" },
                "target": target,
            }), &["operation"]),
        }),
        json!({
            "name": "describe_schema",
            "description": "A schema by its name in the definition or its generated name: the generated name, file and declaration.",
            "inputSchema": object_schema(json!({
                "name": { "type": "string" },
                "target": target,
            }), &["name"]),
        }),
        json!({
            "name": "generate",
            "description": "Run `tsurara generate` for one target or all: writes only into the configured output directories. Returns the files written, unchanged and deleted, and the warnings.",
            "inputSchema": object_schema(json!({ "target": target }), &[]),
        }),
        json!({
            "name": "check",
            "description": "Run `tsurara check`: whether the generated files on disk match the definition and the runtime library satisfies them.",
            "inputSchema": object_schema(json!({ "target": target }), &[]),
        }),
        json!({
            "name": "explain_warning",
            "description": "What a warning code (`unresolved-ref`, ...) means and how to resolve it.",
            "inputSchema": object_schema(json!({ "code": { "type": "string" } }), &["code"]),
        }),
    ]
}

fn string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn common(args: &Value) -> Common {
    Common {
        config: None::<PathBuf>,
        target: string(args, "target"),
        format: Format::Json,
    }
}

/// Runs a tool: a short text summary and the structured result.
fn call(name: &str, args: &Value) -> Result<(String, Value), String> {
    let fail = |e: Error| e.to_string();
    match name {
        "list_targets" => {
            let targets = load_targets(None, None).map_err(fail)?;
            let list: Vec<Value> = targets
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "language": crate::backend(t).map(|b| b.id().to_owned()).unwrap_or_default(),
                        "input": t.input.url.to_string(),
                        "output": t.output.display().to_string(),
                    })
                })
                .collect();
            let summary = list
                .iter()
                .map(|t| {
                    format!(
                        "{} ({})",
                        t["name"].as_str().unwrap_or(""),
                        t["language"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok((summary, json!({ "targets": list })))
        }
        "search_operations" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let report = inspect::operations(
                &common(args),
                string(args, "tag").as_deref(),
                string(args, "query").as_deref(),
                offset,
                Some(limit),
            )
            .map_err(fail)?;
            let lines: Vec<String> = report["operations"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    format!(
                        "{} {} {} -> {}",
                        o["method"].as_str().unwrap_or(""),
                        o["path"].as_str().unwrap_or(""),
                        o["operationId"].as_str().unwrap_or(""),
                        o["function"]["identifier"]
                            .as_str()
                            .unwrap_or("(no function)")
                    )
                })
                .collect();
            let summary = format!(
                "{} of {} matching operations\n{}",
                lines.len(),
                report["total"],
                lines.join("\n")
            );
            Ok((summary, report))
        }
        "describe_operation" => {
            let key = string(args, "operation").ok_or("`operation` is required")?;
            let report = inspect::operation(&common(args), &key).map_err(fail)?;
            let mut summary = format!(
                "{}: {} {} -> {}",
                report["operationId"].as_str().unwrap_or(""),
                report["method"].as_str().unwrap_or(""),
                report["path"].as_str().unwrap_or(""),
                report["function"]["identifier"]
                    .as_str()
                    .unwrap_or("(no function)")
            );
            if let Some(code) = report["example"]["code"].as_str() {
                summary.push_str(&format!(
                    "\n\nexample ({}):\n{code}",
                    report["example"]["file"].as_str().unwrap_or("")
                ));
            }
            Ok((summary, report))
        }
        "describe_schema" => {
            let name = string(args, "name").ok_or("`name` is required")?;
            let report = inspect::schema(&common(args), &name).map_err(fail)?;
            let summary = format!(
                "{} -> {} ({})",
                report["name"].as_str().unwrap_or(""),
                report["identifier"].as_str().unwrap_or(""),
                report["file"].as_str().unwrap_or("")
            );
            Ok((summary, report))
        }
        "generate" => {
            let (report, result) =
                generate_report(None, string(args, "target").as_deref(), false, Format::Json);
            let value = serde_json::to_value(&report).expect("reports serialize");
            result.map_err(fail)?;
            let summary = report
                .targets
                .iter()
                .map(|t| {
                    format!(
                        "{}: {} written, {} unchanged, {} deleted, {} warnings",
                        t.name,
                        t.written.len(),
                        t.unchanged.len(),
                        t.deleted.len(),
                        t.warnings.len()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok((summary, value))
        }
        "check" => {
            let (report, result) =
                check_report(None, string(args, "target").as_deref(), Format::Json);
            let value = serde_json::to_value(&report).expect("reports serialize");
            // Drift is an answer, not a failure of the tool.
            if let Err(e) = result {
                if !matches!(e, Error::Drift { .. }) {
                    return Err(e.to_string());
                }
            }
            let summary = report
                .targets
                .iter()
                .map(|t| {
                    let state = if t.up_to_date {
                        "up to date"
                    } else {
                        "differs"
                    };
                    format!("{}: {state}", t.name)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok((summary, value))
        }
        "explain_warning" => {
            let code = string(args, "code").ok_or("`code` is required")?;
            let report = inspect::warning(&code).map_err(fail)?;
            Ok((
                report["explanation"].as_str().unwrap_or("").to_owned(),
                report,
            ))
        }
        _ => Err(format!("no tool `{name}`")),
    }
}
