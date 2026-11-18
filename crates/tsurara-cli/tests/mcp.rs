//! `tsurara mcp`: a Model Context Protocol server over stdio, driven through the real
//! binary with newline-delimited JSON-RPC.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tsurara_testkit::repo_root;

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Server {
    fn start(cwd: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tsurara"))
            .arg("mcp")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            stdin,
            stdout,
            next: 1,
        };
        let init = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1" },
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "tsurara");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        server.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        server
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(response["id"], json!(id), "{response}");
        response
    }

    /// Calls a tool and returns its structured content; panics on a tool error.
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &r["result"];
        assert_ne!(result["isError"], true, "{name}: {result}");
        assert!(result["content"][0]["text"].is_string(), "{result}");
        result["structuredContent"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/petstore3.json"),
        tmp.path().join("openapi.json"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("tsurara.config.yaml"),
        "input: openapi.json\ntargets:\n  - { name: ts, output: ts }\n  - { name: py, output: py, language: python }\n",
    )
    .unwrap();
    tmp
}

fn inspect(cwd: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .args(["inspect"])
        .args(args)
        .args(["--format", "json"])
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn tools_answer_like_inspect() {
    let tmp = project();
    let mut server = Server::start(tmp.path());
    let tools = server.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for tool in [
        "list_targets",
        "search_operations",
        "describe_operation",
        "describe_schema",
        "generate",
        "check",
        "explain_warning",
    ] {
        assert!(names.contains(&tool), "{names:?}");
    }
    for t in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
    }

    let targets = server.call("list_targets", json!({}));
    assert_eq!(targets["targets"][1]["language"], "python");

    let found = server.call(
        "search_operations",
        json!({ "query": "find pet by id", "target": "ts" }),
    );
    let ids: Vec<&str> = found["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["operationId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["getPetById"]);

    for target in ["ts", "py"] {
        let described = server.call(
            "describe_operation",
            json!({ "operation": "getPetById", "target": target }),
        );
        let cli = inspect(tmp.path(), &["operation", "getPetById", "--target", target]);
        assert_eq!(described["example"], cli["example"], "{target}");
        assert!(described["example"]["code"].is_string());
    }

    let schema = server.call("describe_schema", json!({ "name": "Pet", "target": "py" }));
    assert_eq!(schema["identifier"], "Pet");
    let warning = server.call("explain_warning", json!({ "code": "renamed" }));
    assert_eq!(warning["code"], "renamed");

    let missing = server.request(
        "tools/call",
        json!({ "name": "describe_operation", "arguments": { "operation": "nope", "target": "ts" } }),
    );
    assert_eq!(missing["result"]["isError"], true);
    let unknown = server.request("no/such/method", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn generate_through_mcp_writes_what_the_cli_writes() {
    let tmp = project();
    let mut server = Server::start(tmp.path());
    let generated = server.call("generate", json!({ "target": "ts" }));
    assert!(
        !generated["targets"][0]["written"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!tmp.path().join("py").exists(), "only the target asked for");
    let checked = server.call("check", json!({ "target": "ts" }));
    assert_eq!(checked["targets"][0]["upToDate"], true);
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .args(["generate", "--target", "ts"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(" 0 written"), "{text}");
}

#[test]
fn guides_are_resources() {
    let tmp = project();
    let mut server = Server::start(tmp.path());
    let list = server.request("resources/list", json!({}));
    let resources = list["result"]["resources"].as_array().unwrap();
    let uris: Vec<&str> = resources
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(
        uris.contains(&"tsurara://docs/guide/configuration.md"),
        "{uris:?}"
    );
    assert!(
        uris.contains(&"tsurara://docs/languages/python.md"),
        "{uris:?}"
    );
    // Every guide is embedded.
    for dir in ["docs/guide", "docs/languages"] {
        for entry in std::fs::read_dir(repo_root().join(dir)).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            let uri = format!("tsurara://{dir}/{name}");
            assert!(uris.contains(&uri.as_str()), "{uri} is not embedded");
        }
    }
    let read = server.request(
        "resources/read",
        json!({ "uri": "tsurara://docs/guide/configuration.md" }),
    );
    let text = read["result"]["contents"][0]["text"].as_str().unwrap();
    assert_eq!(
        text,
        std::fs::read_to_string(repo_root().join("docs/guide/configuration.md")).unwrap()
    );
}
