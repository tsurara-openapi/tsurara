//! Call examples for `tsurara inspect operation`: a module at the output root that calls
//! the generated function with the operation's sample values and narrows the Result.

use crate::options::TypeScriptOptions;
use serde_json::Value;
use std::fmt::Write as _;
use tsurara_backend::{Example, ExampleInput};
use tsurara_sem::operation::BodyKind;
use tsurara_sem::print::property_key;

pub fn example(input: &ExampleInput, opts: &TypeScriptOptions) -> Option<Example> {
    let base_url = tsurara_backend::example_base_url(input.model);
    let op = input.operation;
    let entry = input
        .manifest
        .operations
        .iter()
        .find(|e| e.operation_id == op.id)?;
    let function = entry.function.as_ref()?;
    let module = |file: &str| {
        format!(
            "./{}{}",
            file.strip_suffix(".ts").unwrap_or(file),
            opts.import_extension.suffix()
        )
    };
    let mut imports = format!(
        "import {{ createClient }} from '{}';\nimport {{ {} }} from '{}';\n",
        opts.client_module,
        function.identifier,
        module(&function.file)
    );

    // Required parameters by group, then the body.
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for p in input.samples.params.iter().filter(|p| p.required) {
        let group = p.location.group();
        let field = format!("{}: {}", property_key(&p.name), literal(&p.value, 3));
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, fields)) => fields.push(field),
            None => groups.push((group, vec![field])),
        }
    }
    let mut fields: Vec<String> = groups
        .into_iter()
        .map(|(group, fields)| format!("{group}: {{ {} }}", fields.join(", ")))
        .collect();
    if let Some(body) = &input.samples.body {
        let json = op.body.as_ref().and_then(|b| {
            b.contents
                .iter()
                .find(|c| c.media_type == body.media_type)
                .map(|c| matches!(c.body_kind, BodyKind::Json | BodyKind::Auto))
        });
        if json == Some(true) {
            fields.push(format!("body: {}", literal(&body.value, 2)));
        } else if body.required {
            // Files and form data are values the app has; the sample cannot be one.
            let data = entry.types.get("data")?;
            writeln!(
                imports,
                "import type {{ {} }} from '{}';",
                data.identifier,
                module(&data.file)
            )
            .unwrap();
            fields.push(format!(
                "body: undefined as unknown as {}['body'], // a Blob, File or form fields",
                data.identifier
            ));
        }
    }
    let args = if fields.is_empty() {
        String::new()
    } else {
        format!(", {{\n    {},\n  }}", fields.join(",\n    "))
    };
    let code = format!(
        "{imports}\nexport async function example(): Promise<void> {{\n  const client = createClient({{ baseUrl: '{base_url}' }});\n  const result = await {}(client{args});\n  if (result.ok) {{\n    console.log(result.status, result.data);\n  }} else if (result.kind === 'http') {{\n    console.error(result.status, result.error);\n  }} else {{\n    console.error(result.kind);\n  }}\n}}\n",
        function.identifier
    );
    let (query_imports, query_code) = query_example(entry, &args, &base_url, &module);
    let code = code.replacen(&imports, &format!("{imports}{query_imports}"), 1) + &query_code;
    Some(Example {
        file: "example.ts".into(),
        code,
        note: None,
    })
}

/// With TanStack Query options: the same call through them, and where the other
/// helpers fit, as (imports, code). `args` is the call's data argument (`, { … }` or nothing).
fn query_example(
    entry: &tsurara_backend::manifest::OperationEntry,
    args: &str,
    base_url: &str,
    module: &dyn Fn(&str) -> String,
) -> (String, String) {
    let h = &entry.helpers;
    let mut imports = String::new();
    let mut code = String::new();
    if let Some(options) = h.get("queryOptions") {
        write!(
            imports,
            "import type {{ QueryClient }} from '@tanstack/query-core';\nimport {{ {} }} from '{}';\n",
            options.identifier,
            module(&options.file)
        )
        .unwrap();
        writeln!(
            code,
            "\n// With TanStack Query, the generated options instead of a hand-written queryFn and key:\n//   const query = useQuery({}(client{}));",
            options.identifier,
            if args.is_empty() { "" } else { ", …" }
        )
        .unwrap();
        if let Some(infinite) = h.get("infiniteQueryOptions") {
            writeln!(
                code,
                "//   const pages = useInfiniteQuery({}(client));",
                infinite.identifier
            )
            .unwrap();
        }
        for (role, how) in [
            ("queryFilter", "queryClient.invalidateQueries({}())"),
            ("setQueriesData", "{}(queryClient, (old) => old)"),
        ] {
            if let Some(e) = h.get(role) {
                writeln!(code, "//   {}", how.replace("{}", &e.identifier)).unwrap();
            }
        }
        write!(
            code,
            "export async function exampleQuery(queryClient: QueryClient): Promise<void> {{\n  const client = createClient({{ baseUrl: '{base_url}' }});\n  const data = await queryClient.fetchQuery({}(client{args}));\n  console.log(data);\n}}\n",
            options.identifier
        )
        .unwrap();
    } else if let Some(options) = h.get("mutationOptions") {
        writeln!(
            imports,
            "import {{ {} }} from '{}';",
            options.identifier,
            module(&options.file)
        )
        .unwrap();
        write!(
            code,
            "\n// With TanStack Query, the generated options instead of a hand-written mutationFn:\n//   const mutation = useMutation({}(client));\nexport function exampleMutation(): void {{\n  const client = createClient({{ baseUrl: '{base_url}' }});\n  void {}(client);\n}}\n",
            options.identifier,
            options.identifier
        )
        .unwrap();
    }
    if let Some(handler) = h.get("mockHandler") {
        write!(
            code,
            "\n// In tests, the generated mock instead of a hand-written one: {}() answers with the first\n// documented success and the sample values ({}).\n",
            handler.identifier,
            handler.file
        )
        .unwrap();
    }
    (imports, code)
}

/// `v` as a TypeScript expression: JSON, with object keys unquoted where they can be.
fn literal(v: &Value, indent: usize) -> String {
    match v {
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => {
            let pad = "  ".repeat(indent);
            let inner: Vec<String> = o
                .iter()
                .map(|(k, v)| format!("{pad}  {}: {}", property_key(k), literal(v, indent + 1)))
                .collect();
            format!("{{\n{},\n{pad}}}", inner.join(",\n"))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(|v| literal(v, indent))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        v => v.to_string(),
    }
}
