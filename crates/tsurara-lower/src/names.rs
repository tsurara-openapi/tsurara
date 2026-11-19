//! Names allocated for a lowered module with a naming policy, for backends outside this repository
//!: the CLI allocates them with the policy a backend describes as data, and sends them
//! with `tir`. Built-in backends allocate their own names, with derived names and more scopes.

use crate::tir::{DeclKind, Module};
use tsurara_naming::{Context, Names, NamingPolicy};
use tsurara_sem::Model;
use tsurara_sem::ir::Literal;

/// The names of a lowered module, by position.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AllocatedNames {
    /// One entry per declaration of `tir`, in order.
    pub decls: Vec<DeclNames>,
    /// One entry per operation of the model, in order.
    pub operations: Vec<OperationNames>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DeclNames {
    /// The type name (context `type`; one scope for all declarations).
    pub name: String,
    /// The file name without extension (context `file`, from the type name; one scope, where names
    /// that differ only in case collide).
    pub file: String,
    /// Fields of a record (context `member`, from the wire names) or the values of an enum (context
    /// `constant`), in order; each declaration is its own scope. Empty for other declarations.
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OperationNames {
    /// The call function (context `function`, from the operation id; one scope, shared with no type).
    pub function: String,
}

fn literal(l: &Literal) -> String {
    match l {
        Literal::String(s) => s.clone(),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}

/// Allocates the names of `module` (lowered from `model`) with `policy`, in order: the same input
/// always yields the same names.
pub fn allocate(model: &Model, module: &Module, policy: &dyn NamingPolicy) -> AllocatedNames {
    let mut types = Names::new();
    let mut files = Names::new();
    let mut decls = Vec::with_capacity(module.decls.len());
    for d in &module.decls {
        let owner = d.loc.to_string();
        let ident = policy.convert(Context::Type, &d.preferred_name, false);
        let name = types
            .claim(policy, Context::Type, "types", &ident, &owner)
            .name;
        let file_ident = policy.convert(Context::File, &name, false);
        let file = files
            .claim(policy, Context::File, "files", &file_ident, &owner)
            .name;
        let (context, raws): (Context, Vec<String>) = match &d.kind {
            DeclKind::Record(r) => (
                Context::Member,
                r.fields.iter().map(|f| f.wire_name.clone()).collect(),
            ),
            DeclKind::Enum(e) => (
                Context::Constant,
                e.values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| e.name(i).map_or_else(|| literal(v), str::to_owned))
                    .collect(),
            ),
            _ => (Context::Member, Vec::new()),
        };
        let mut scope = Names::new();
        let members = raws
            .iter()
            .map(|raw| {
                let ident = policy.convert(context, raw, false);
                scope.claim(policy, context, "members", &ident, raw).name
            })
            .collect();
        decls.push(DeclNames {
            name,
            file,
            members,
        });
    }
    let mut functions = Names::new();
    let operations = model
        .operations
        .iter()
        .map(|op| {
            let ident = policy.convert(Context::Function, &op.id, false);
            OperationNames {
                function: functions
                    .claim(policy, Context::Function, "functions", &ident, &op.id)
                    .name,
            }
        })
        .collect();
    AllocatedNames { decls, operations }
}
