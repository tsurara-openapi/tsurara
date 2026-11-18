//! Backends written as external programs (design.md 18.11): `describe` once, then `generate`.

use crate::hooks::run;
use tsurara_backend::external::{Describe, GenerateRequest, GenerateResponse, PROTOCOLS};
use tsurara_backend::{Backend, Emitted, Error, Markers};
use tsurara_config::Command;
use tsurara_sem::{Model, Options, SEM_VERSION};

pub struct ExternalBackend {
    command: Command,
    options: serde_json::Value,
    markers: Markers,
    label: String,
    /// The protocol version the program speaks.
    protocol: u32,
    lowering: Option<tsurara_backend::external::Lowering>,
    naming: Option<tsurara_naming::data::DataPolicy>,
}

impl ExternalBackend {
    /// Runs `describe` and checks that the program speaks this protocol and accepts this model.
    pub fn describe(command: &Command, options: &serde_json::Value) -> Result<Self, String> {
        let label = command.args.join(" ");
        let fail = |e: String| format!("backend `{label}`: {e}");
        let args = with(&command.args, "describe");
        let stdout = run(command, &args, None).map_err(|e| fail(e.to_string()))?;
        let d: Describe = serde_json::from_slice(&stdout)
            .map_err(|e| fail(format!("`describe` printed an invalid answer: {e}")))?;
        if !PROTOCOLS.contains(&d.protocol) {
            return Err(fail(format!(
                "speaks protocol {}, but tsurara speaks protocols {PROTOCOLS:?}",
                d.protocol
            )));
        }
        if !d.sem_versions.contains(&SEM_VERSION) {
            return Err(fail(format!(
                "does not accept model version {SEM_VERSION} (it accepts {:?})",
                d.sem_versions
            )));
        }
        if d.naming.is_some() && d.lowering.is_none() {
            return Err(fail(
                "describes `naming` without `lowering`: names are allocated for the lowered model"
                    .into(),
            ));
        }
        Ok(Self {
            command: command.clone(),
            options: options.clone(),
            markers: d.markers,
            label,
            protocol: d.protocol,
            lowering: d.lowering,
            naming: d.naming,
        })
    }
}

fn with(args: &[String], mode: &str) -> Vec<String> {
    let mut a = args.to_vec();
    a.push(mode.to_owned());
    a
}

impl Backend for ExternalBackend {
    fn id(&self) -> &'static str {
        "external"
    }

    fn markers(&self) -> Markers {
        self.markers.clone()
    }

    fn options_json(&self) -> serde_json::Value {
        self.options.clone()
    }

    fn emit(&self, model: &Model, opts: &Options) -> Result<Emitted, Error> {
        let fail = |e: String| Error::External(format!("backend `{}`: {e}", self.label));
        // The shared lowering and naming, for programs that ask for them.
        let lowered = self
            .lowering
            .as_ref()
            .map(|l| tsurara_lower::lower(model, &l.capabilities));
        let names = match (&lowered, &self.naming) {
            (Some((module, _)), Some(policy)) => {
                Some(tsurara_lower::allocate(model, module, policy))
            }
            _ => None,
        };
        let request = GenerateRequest {
            protocol: self.protocol,
            sem_version: SEM_VERSION,
            model,
            common: opts,
            options: &self.options,
            lowered: lowered.as_ref().map(|(m, _)| m),
            names: names.as_ref(),
        };
        let stdin = serde_json::to_vec(&request).expect("the request serializes");
        let stdout = run(
            &self.command,
            &with(&self.command.args, "generate"),
            Some(&stdin),
        )
        .map_err(|e| fail(e.to_string()))?;
        let mut r: GenerateResponse = serde_json::from_slice(&stdout)
            .map_err(|e| fail(format!("`generate` printed an invalid answer: {e}")))?;
        r.files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut warnings = lowered.map(|(_, w)| w).unwrap_or_default();
        warnings.extend(r.warnings);
        Ok(Emitted {
            files: r.files,
            warnings,
            // Protocol 1 has no manifest.
            manifest: if self.protocol >= 2 { r.manifest } else { None },
        })
    }
}
