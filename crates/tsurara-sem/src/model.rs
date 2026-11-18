//! IR of a whole definition. The IR stage of plugins rewrites it.

use crate::Error;
use crate::operation::{OpKind, Operation, OperationBuilder};
use crate::options::Options;
use crate::schema::{Converter, Mode, TypeDecl};
use crate::warning::Warning;
use serde_json::Value;
use tsurara_loader::{DocumentSet, SpecVersion, detect_version};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerVariable {
    pub name: String,
    pub default: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub url: String,
    pub description: Option<String>,
    pub variables: Vec<ServerVariable>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Version of this JSON form (`SEM_VERSION`).
    pub sem_version: u32,
    pub version: SpecVersion,
    pub title: Option<String>,
    /// Named types, in the order of components, followed by the hoisted types.
    pub schemas: Vec<TypeDecl>,
    pub operations: Vec<Operation>,
    pub webhooks: Vec<Operation>,
    /// The channels of `x-event-channels`, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<crate::operation::Channel>,
    pub servers: Vec<Server>,
    /// The names in `components.securitySchemes`, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_schemes: Vec<String>,
    pub warnings: Vec<Warning>,
    /// Name requests, in the order backends must claim them (design.md 18.4.6).
    pub names: Vec<crate::naming::NameRequest>,
}

pub fn build_model(set: &DocumentSet, opts: &Options) -> Result<Model, Error> {
    let version = detect_version(set.root())?;
    if version == SpecVersion::Swagger2 {
        return Err(Error::Swagger2);
    }
    let mut conv = Converter::new(set, opts, version);
    conv.find_multipart_only();
    let components = conv.register_components();
    if opts.filter.is_empty() {
        for loc in &components {
            conv.enqueue(loc, Mode::Read);
        }
    }

    let mut builder = OperationBuilder {
        conv: &mut conv,
        incoming: false,
    };
    let mut selected = Vec::new();
    let paths = if opts.operations {
        builder.collect_paths()
    } else {
        Vec::new()
    };
    for (loc, method, path) in paths {
        if !opts.filter.is_empty() {
            let (id, tags) = builder.peek(&loc);
            if !opts.filter.selects(id.as_deref(), &tags) {
                continue;
            }
        }
        let names = builder.name_operation(&loc, &method, &path, OpKind::Operation, None);
        selected.push((loc, method, path, names));
    }
    let mut hooks = Vec::new();
    if opts.filter.is_empty() {
        for (loc, method, name) in builder.collect_webhooks() {
            let names = builder.name_operation(&loc, &method, &name, OpKind::Webhook, None);
            hooks.push((loc, method, name, names));
        }
    }
    let operations: Vec<Operation> = selected
        .into_iter()
        .map(|(loc, method, path, (id, name_id))| {
            let mut op = builder.build(&loc, &method, &path, id, name_id);
            builder.check_path_template(&mut op);
            op
        })
        .collect();
    builder.incoming = true;
    let webhooks: Vec<Operation> = hooks
        .into_iter()
        .map(|(loc, method, name, (id, name_id))| builder.build(&loc, &method, &name, id, name_id))
        .collect();
    builder.incoming = false;
    let channels = if opts.filter.is_empty() {
        builder.collect_channels()
    } else {
        Vec::new()
    };
    conv.drain();
    break_alias_cycles(&mut conv.decls, &mut conv.warnings);

    Ok(Model {
        sem_version: crate::SEM_VERSION,
        version,
        title: set
            .root()
            .get("info")
            .and_then(|i| i.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        schemas: conv.decls,
        operations,
        webhooks,
        channels,
        servers: servers(set.root()),
        security_schemes: set
            .root()
            .get("components")
            .and_then(|c| c.get("securitySchemes"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
        warnings: conv.warnings,
        names: conv.names,
    })
}

/// A declaration that refers back to itself without an object, array or tuple in between
/// (`A: { $ref: A }`, `A: { anyOf: [{ $ref: A }, { type: string }] }`, or `A` and `B` doing so through
/// each other) is rejected by languages (TS2456 in TypeScript) and adds no values. Those references
/// become `never`, so `A | string` is `string`; a declaration with nothing else becomes unknown. Both
/// are warned about.
fn break_alias_cycles(decls: &mut [TypeDecl], warnings: &mut Vec<Warning>) {
    use crate::ir::Type;
    use crate::naming::NameId;
    use crate::warning::WarningKind;
    use std::collections::{HashMap, HashSet};

    /// References that are not inside an object, array or tuple.
    fn direct(t: &Type, out: &mut Vec<NameId>) {
        match t {
            Type::Named(n) => out.push(n.id),
            Type::Union(ts) | Type::Intersection(ts) => ts.iter().for_each(|t| direct(t, out)),
            Type::Choice(c) => c.members.iter().for_each(|t| direct(t, out)),
            Type::Inline(i) => direct(&i.ty, out),
            _ => {}
        }
    }
    fn cut(t: &Type, cyclic: &HashSet<NameId>) -> Type {
        match t {
            Type::Named(n) if cyclic.contains(&n.id) => Type::Never,
            Type::Union(ts) => Type::Union(ts.iter().map(|t| cut(t, cyclic)).collect()),
            Type::Intersection(ts) => {
                Type::Intersection(ts.iter().map(|t| cut(t, cyclic)).collect())
            }
            Type::Choice(c) => {
                let mut c = c.clone();
                c.members = c.members.iter().map(|t| cut(t, cyclic)).collect();
                Type::Choice(c)
            }
            Type::Inline(i) => {
                let mut i = (**i).clone();
                i.ty = cut(&i.ty, cyclic);
                Type::Inline(Box::new(i))
            }
            other => other.clone(),
        }
    }
    /// Whether nothing but `never` is left.
    fn empty(t: &Type) -> bool {
        match t {
            Type::Never => true,
            Type::Union(ts) | Type::Intersection(ts) => ts.iter().all(empty),
            Type::Choice(c) => c.members.iter().all(empty),
            Type::Inline(i) => empty(&i.ty),
            _ => false,
        }
    }

    let edges: HashMap<NameId, Vec<NameId>> = decls
        .iter()
        .map(|d| {
            let mut out = Vec::new();
            direct(&d.ty, &mut out);
            (d.id, out)
        })
        .collect();
    let reaches_itself = |start: NameId| {
        let mut seen = HashSet::new();
        let mut stack = edges.get(&start).cloned().unwrap_or_default();
        while let Some(id) = stack.pop() {
            if id == start {
                return true;
            }
            if seen.insert(id) {
                stack.extend(edges.get(&id).cloned().unwrap_or_default());
            }
        }
        false
    };
    let cyclic: HashSet<NameId> = decls
        .iter()
        .map(|d| d.id)
        .filter(|id| reaches_itself(*id))
        .collect();
    for d in decls.iter_mut().filter(|d| cyclic.contains(&d.id)) {
        let ty = cut(&d.ty, &cyclic);
        let message = if empty(&ty) {
            d.ty = Type::Unknown;
            format!(
                "`{}` only refers to itself through references; its type is unknown",
                d.preferred_name
            )
        } else {
            d.ty = ty;
            format!(
                "`{}` refers to itself without an object or array in between; those references are left out",
                d.preferred_name
            )
        };
        warnings.push(Warning {
            kind: WarningKind::InvalidDefinition,
            loc: d.loc.clone(),
            message,
            language: None,
        });
    }
}

fn servers(root: &Value) -> Vec<Server> {
    let Some(Value::Array(list)) = root.get("servers") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|s| {
            let url = s.get("url")?.as_str()?.to_owned();
            let variables = match s.get("variables") {
                Some(Value::Object(vars)) => vars
                    .iter()
                    .map(|(name, v)| ServerVariable {
                        name: name.clone(),
                        default: v
                            .get("default")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        values: v
                            .get("enum")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Some(Server {
                url,
                description: s
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                variables,
            })
        })
        .collect()
}
