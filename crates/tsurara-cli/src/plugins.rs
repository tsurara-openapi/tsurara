//! Plugin modules (`plugins`, design.md 11.2): one process per module and run, speaking
//! the session protocol of `tsurara_backend::session` over stdio.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use tsurara_backend::OutputFile;
use tsurara_backend::plugin::{ExtendInput, Plugin, PluginError};
use tsurara_backend::session::{SESSION_PROTOCOL, SessionRequest, SessionResponse, Stage};
use tsurara_config::PluginModule;
use tsurara_sem::{Model, SEM_VERSION};

/// The command that runs a module with the default runtime: Node imports the runner of
/// `@tsurara/plugin` (resolved from the configuration's directory) and gives it the module.
pub const NODE_RUNNER: &str =
    "import('@tsurara/plugin/runner').then((m) => m.run(process.argv[1]))";

struct Pipes {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

pub struct PluginProcess {
    name: String,
    stages: Vec<Stage>,
    pipes: Mutex<Pipes>,
}

impl PluginProcess {
    /// Starts the module and exchanges `hello`.
    pub fn start(module: &PluginModule) -> Result<Self, String> {
        let label = module.module.display().to_string();
        let mut args: Vec<String> = module.runtime.clone();
        if args == ["node"] {
            args.extend([
                "--input-type=module".into(),
                "-e".into(),
                NODE_RUNNER.into(),
            ]);
        }
        args.push(module.module.to_string_lossy().into_owned());
        let (program, rest) = args.split_first().expect("the runtime is not empty");
        let mut child = Command::new(program)
            .args(rest)
            .current_dir(&module.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("plugin `{label}`: could not start `{program}`: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut process = PluginProcess {
            name: label.clone(),
            stages: Vec::new(),
            pipes: Mutex::new(Pipes {
                child,
                stdin,
                stdout,
            }),
        };
        match process.exchange(&SessionRequest::Hello {
            protocol: SESSION_PROTOCOL,
        })? {
            SessionResponse::Hello {
                protocol,
                name,
                stages,
            } => {
                if protocol != SESSION_PROTOCOL {
                    return Err(format!(
                        "plugin `{label}` speaks plugin protocol {protocol}, but tsurara speaks {SESSION_PROTOCOL}; use a version of the plugin SDK for this tsurara"
                    ));
                }
                if !name.is_empty() {
                    process.name = name;
                }
                process.stages = stages;
                Ok(process)
            }
            other => Err(format!(
                "plugin `{label}`: expected `hello`, got {}",
                serde_json::to_string(&other).unwrap_or_default()
            )),
        }
    }

    fn exchange(&self, request: &SessionRequest) -> Result<SessionResponse, String> {
        let mut pipes = self.pipes.lock().expect("not poisoned");
        let line = serde_json::to_string(request).expect("requests serialize");
        let stdin = pipes.stdin.as_mut().ok_or("the plugin was closed")?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("plugin `{}`: could not write: {e}", self.name))?;
        let mut answer = String::new();
        let n = pipes
            .stdout
            .read_line(&mut answer)
            .map_err(|e| format!("plugin `{}`: could not read: {e}", self.name))?;
        if n == 0 {
            return Err(format!("plugin `{}` exited without answering", self.name));
        }
        serde_json::from_str(&answer).map_err(|e| {
            format!(
                "plugin `{}` answered something that is not a session message ({e}): {}",
                self.name,
                answer.trim()
            )
        })
    }

    fn call(&self, stage: Stage, input: Value) -> Result<Option<Value>, PluginError> {
        if !self.stages.contains(&stage) {
            return Ok(None);
        }
        match self.exchange(&SessionRequest::Call { stage, input }) {
            Ok(SessionResponse::Result { output }) => Ok(Some(output)),
            Ok(SessionResponse::Error { message }) => {
                Err(PluginError::new(format!("{}: {message}", stage.as_str())))
            }
            Ok(_) => Err(PluginError::new(format!(
                "{}: expected `result` or `error`",
                stage.as_str()
            ))),
            Err(e) => Err(PluginError::new(e)),
        }
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        let mut pipes = self.pipes.lock().expect("not poisoned");
        // Closing stdin ends the session; the plugin exits.
        pipes.stdin.take();
        let _ = pipes.child.wait();
    }
}

fn files_of(value: Value) -> Result<Vec<OutputFile>, PluginError> {
    serde_json::from_value(value)
        .map_err(|e| PluginError::new(format!("expected a list of `{{ path, contents }}`: {e}")))
}

impl Plugin for PluginProcess {
    fn name(&self) -> &str {
        &self.name
    }

    fn transform_document(&self, root: &mut Value) -> Result<(), PluginError> {
        if let Some(out) = self.call(Stage::TransformDocument, root.clone())? {
            *root = out;
        }
        Ok(())
    }

    fn transform_model(&self, model: &mut Model) -> Result<(), PluginError> {
        let input = serde_json::to_value(&*model).expect("the model serializes");
        if let Some(out) = self.call(Stage::TransformModel, input)? {
            let version = out.get("semVersion").and_then(Value::as_u64);
            if version != Some(u64::from(SEM_VERSION)) {
                return Err(PluginError::new(format!(
                    "transformModel: the model has semVersion {}, expected {SEM_VERSION}",
                    version.map_or_else(|| "missing".to_owned(), |v| v.to_string())
                )));
            }
            *model = serde_json::from_value(out)
                .map_err(|e| PluginError::new(format!("transformModel: invalid model: {e}")))?;
        }
        Ok(())
    }

    fn extend(&self, input: &ExtendInput) -> Result<Vec<OutputFile>, PluginError> {
        let input = serde_json::to_value(input).expect("the input serializes");
        match self.call(Stage::Extend, input)? {
            Some(out) => files_of(out),
            None => Ok(Vec::new()),
        }
    }

    fn transform_files(&self, files: &mut Vec<OutputFile>) -> Result<(), PluginError> {
        let input = serde_json::to_value(&*files).expect("files serialize");
        if let Some(out) = self.call(Stage::TransformFiles, input)? {
            *files = files_of(out)?;
        }
        Ok(())
    }
}
