//! External command hooks (design.md chapter 11). `transformDocument`, `transformModel`, `extend`
//! and `transformFiles` are plugins
//! that exchange JSON with a command over stdin and stdout; `afterWrite` runs after writing.

use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use tsurara_backend::OutputFile;
use tsurara_backend::plugin::{ExtendInput, Plugin, PluginError};
use tsurara_config::Command;
use tsurara_sem::{Model, SEM_VERSION};

enum Stage {
    Document,
    Model,
    Extend,
    Files,
}

pub struct CommandPlugin {
    stage: Stage,
    command: Command,
    label: String,
}

impl CommandPlugin {
    pub fn document(command: &Command) -> Self {
        Self::new(Stage::Document, command)
    }

    pub fn model(command: &Command) -> Self {
        Self::new(Stage::Model, command)
    }

    pub fn extend(command: &Command) -> Self {
        Self::new(Stage::Extend, command)
    }

    pub fn files(command: &Command) -> Self {
        Self::new(Stage::Files, command)
    }

    fn new(stage: Stage, command: &Command) -> Self {
        Self {
            stage,
            command: command.clone(),
            label: command.args.join(" "),
        }
    }
}

impl Plugin for CommandPlugin {
    fn name(&self) -> &str {
        &self.label
    }

    fn transform_document(&self, root: &mut Value) -> Result<(), PluginError> {
        if let Stage::Document = self.stage {
            *root = exchange(&self.command, root)?;
        }
        Ok(())
    }

    fn transform_model(&self, model: &mut Model) -> Result<(), PluginError> {
        if let Stage::Model = self.stage {
            let input = serde_json::to_value(&*model).expect("the model serializes");
            let output = exchange(&self.command, &input)?;
            let version = output.get("semVersion").and_then(Value::as_u64);
            if version != Some(u64::from(SEM_VERSION)) {
                return Err(PluginError::new(format!(
                    "the model on stdout has semVersion {}, expected {SEM_VERSION}",
                    version.map_or_else(|| "missing".to_owned(), |v| v.to_string())
                )));
            }
            *model = serde_json::from_value(output)
                .map_err(|e| PluginError::new(format!("the model on stdout is not valid: {e}")))?;
        }
        Ok(())
    }

    fn extend(&self, input: &ExtendInput) -> Result<Vec<OutputFile>, PluginError> {
        if let Stage::Extend = self.stage {
            let input = serde_json::to_value(input).expect("the input serializes");
            return parse_files(exchange(&self.command, &input)?);
        }
        Ok(Vec::new())
    }

    fn transform_files(&self, files: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        if let Stage::Files = self.stage {
            let input: Vec<Value> = files
                .iter()
                .map(|f| json!({ "path": f.path, "contents": f.contents }))
                .collect();
            let output = exchange(&self.command, &Value::Array(input))?;
            *files = parse_files(output)?;
        }
        Ok(())
    }
}

fn parse_files(value: Value) -> Result<Vec<OutputFile>, PluginError> {
    let bad = || PluginError::new("expected a list of `{ path, contents }` objects on stdout");
    let Value::Array(items) = value else {
        return Err(bad());
    };
    items
        .into_iter()
        .map(|item| {
            let path = item.get("path").and_then(Value::as_str).ok_or_else(bad)?;
            let contents = item
                .get("contents")
                .and_then(Value::as_str)
                .ok_or_else(bad)?;
            Ok(OutputFile {
                path: path.to_owned(),
                contents: contents.to_owned(),
            })
        })
        .collect()
}

/// Sends `input` as JSON on stdin and parses stdout as JSON.
fn exchange(command: &Command, input: &Value) -> Result<Value, PluginError> {
    let stdin = serde_json::to_vec(input).expect("JSON values serialize");
    let stdout = run(command, &command.args, Some(&stdin))?;
    serde_json::from_slice(&stdout)
        .map_err(|e| PluginError::new(format!("stdout is not valid JSON: {e}")))
}

/// Runs `args` in the command's directory. Fails with the exit code and stderr unless it succeeds.
pub fn run(
    command: &Command,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, PluginError> {
    let (program, rest) = args.split_first().expect("commands are not empty");
    let mut child = std::process::Command::new(program)
        .args(rest)
        .current_dir(&command.dir)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PluginError::new(format!("could not start: {e}")))?;
    let writer = stdin.map(|input| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let input = input.to_vec();
        // Written from another thread, so that a command that prints before reading cannot deadlock.
        std::thread::spawn(move || pipe.write_all(&input))
    });
    let output = child
        .wait_with_output()
        .map_err(|e| PluginError::new(format!("could not run: {e}")))?;
    if let Some(w) = writer {
        // A command may exit without reading all of stdin; its exit status decides.
        let _ = w.join();
    }
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map_or_else(|| "a signal".to_owned(), |c| format!("exit code {c}"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PluginError::new(format!(
            "exited with {code}: {}",
            stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// Runs the `afterWrite` commands with `{output}` replaced by the output directory.
pub fn after_write(commands: &[Command], output: &Path) -> Result<(), (String, PluginError)> {
    let dir = output.to_string_lossy();
    for command in commands {
        let args: Vec<String> = command
            .args
            .iter()
            .map(|a| a.replace("{output}", &dir))
            .collect();
        let stdout = run(command, &args, None).map_err(|e| (command.args.join(" "), e))?;
        std::io::stdout().write_all(&stdout).ok();
    }
    Ok(())
}
