//! The session protocol of plugin modules (`plugins`): tsurara starts the module once per
//! run and exchanges one JSON message per line over stdin and stdout, so a plugin that takes part in
//! several stages runs in one process. The SDKs (`@tsurara/plugin`) implement the other end.
//!
//! 1. tsurara sends `{ "type": "hello", "protocol": 1 }`; the plugin answers with `hello`, its protocol,
//!    its name and the stages it takes part in. A different protocol stops the run with an error that
//!    names both.
//! 2. Per stage the plugin declared, tsurara sends `call` with the stage and its input; the plugin
//!    answers with `result` and the output, or `error` and a message.
//! 3. tsurara closes stdin at the end of the run; the plugin exits.
//!
//! Inputs and outputs: `transformDocument` the root definition (JSON); `transformModel` the model
//! (`sem-model.schema.json`); `extend` an `ExtendInput` in, files out; `transformFiles` files in and out.

/// The version of this protocol.
pub const SESSION_PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    TransformDocument,
    TransformModel,
    Extend,
    TransformFiles,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::TransformDocument => "transformDocument",
            Stage::TransformModel => "transformModel",
            Stage::Extend => "extend",
            Stage::TransformFiles => "transformFiles",
        }
    }
}

/// What tsurara sends.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionRequest {
    Hello {
        protocol: u32,
    },
    Call {
        stage: Stage,
        input: serde_json::Value,
    },
}

/// What a plugin answers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionResponse {
    Hello {
        protocol: u32,
        name: String,
        stages: Vec<Stage>,
    },
    Result {
        output: serde_json::Value,
    },
    Error {
        message: String,
    },
}
