//! A set of loaded definitions and `$ref` resolution.

use crate::fetch::{FetchError, Fetcher};
use crate::parse::{ParseError, parse_document};
use crate::pointer;
use percent_encoding::percent_decode_str;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use url::Url;

/// A location in a definition. A pair of the document URL (without fragment) and a JSON Pointer.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Loc {
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub doc: Url,
    /// Escaped JSON Pointer. The root is `""`.
    pub pointer: String,
}

impl Loc {
    pub fn new(doc: Url, pointer: impl Into<String>) -> Self {
        let mut doc = doc;
        doc.set_fragment(None);
        Self {
            doc,
            pointer: pointer.into(),
        }
    }

    pub fn root(doc: Url) -> Self {
        Self::new(doc, "")
    }

    /// Returns the location with one more token appended.
    pub fn join(&self, token: &str) -> Self {
        Self {
            doc: self.doc.clone(),
            pointer: pointer::push(&self.pointer, token),
        }
    }

    pub fn tokens(&self) -> impl Iterator<Item = String> + '_ {
        pointer::tokens(&self.pointer)
    }

    /// The last token. `None` for the root.
    pub fn last_token(&self) -> Option<String> {
        self.tokens().last()
    }

    pub fn parent(&self) -> Option<Self> {
        let idx = self.pointer.rfind('/')?;
        Some(Self {
            doc: self.doc.clone(),
            pointer: self.pointer[..idx].to_owned(),
        })
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.doc, self.pointer)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("failed to parse {url}: {source}")]
    Parse {
        url: Url,
        #[source]
        source: Box<ParseError>,
    },
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RefError {
    #[error("invalid reference `{reference}`: {message}")]
    Invalid { reference: String, message: String },
    #[error("the document {url} could not be loaded: {message}")]
    DocumentUnavailable { url: Url, message: String },
    #[error("the document {0} is not loaded")]
    UnknownDocument(Url),
    #[error("{0} does not exist")]
    NotFound(Loc),
    #[error("the anchor `{anchor}` is not defined in {url}")]
    UnknownAnchor { url: Url, anchor: String },
    #[error("circular $ref at {0}")]
    Cycle(Loc),
}

#[derive(Debug, Clone)]
enum DocState {
    Loaded(Value),
    Failed(String),
}

/// The root definition and every external definition referenced from it.
#[derive(Debug, Clone)]
pub struct DocumentSet {
    root: Url,
    docs: HashMap<Url, DocState>,
    /// Document URLs in load order. Used to keep the output order stable.
    order: Vec<Url>,
    /// Resources declared with `$id`.
    ids: HashMap<Url, Loc>,
    /// `$anchor` and `$dynamicAnchor`. The key is the resource URL and the name.
    anchors: HashMap<(Url, String), Loc>,
    dynamic_anchors: HashMap<(Url, String), Loc>,
}

impl DocumentSet {
    /// Follows `$ref` breadth-first from `root` and loads every referenced external definition.
    ///
    /// Fails if the root definition cannot be loaded. Failures of external definitions are only recorded,
    /// and become errors when a reference to them is resolved.
    pub fn load(root: &Url, fetcher: &dyn Fetcher) -> Result<Self, LoadError> {
        let mut root = root.clone();
        root.set_fragment(None);
        let bytes = fetcher.fetch(&root)?;
        let value = parse_document(&bytes).map_err(|source| LoadError::Parse {
            url: root.clone(),
            source: Box::new(source),
        })?;
        let mut set = Self::empty(root.clone());
        let mut queue = VecDeque::new();
        // Local files are loaded only for references from local documents: a definition fetched
        // over HTTP must not read the files of the machine that generates.
        let mut wanted_locally: HashSet<Url> = HashSet::new();
        let local = !is_remote(&root);
        let refs = set.add(root, value, &mut queue);
        if local {
            wanted_locally.extend(refs);
        }
        while let Some(url) = queue.pop_front() {
            if set.docs.contains_key(&url) || set.ids.contains_key(&url) {
                continue;
            }
            if !is_remote(&url) && !wanted_locally.contains(&url) {
                set.order.push(url.clone());
                set.docs.insert(
                    url,
                    DocState::Failed("a remote definition cannot refer to a local file".to_owned()),
                );
                continue;
            }
            let state = match fetcher.fetch(&url) {
                Ok(bytes) => match parse_document(&bytes) {
                    Ok(value) => {
                        let local = !is_remote(&url);
                        let refs = set.add(url, value, &mut queue);
                        if local {
                            wanted_locally.extend(refs);
                        }
                        continue;
                    }
                    Err(e) => DocState::Failed(e.to_string()),
                },
                Err(e) => DocState::Failed(e.to_string()),
            };
            set.order.push(url.clone());
            set.docs.insert(url, state);
        }
        Ok(set)
    }

    /// Builds the set without I/O. The first document is the root. References to missing documents fail at resolution.
    pub fn from_documents(docs: impl IntoIterator<Item = (Url, Value)>) -> Self {
        let mut iter = docs.into_iter().peekable();
        let root = iter
            .peek()
            .map(|(u, _)| u.clone())
            .unwrap_or_else(|| Url::parse("memory:///").expect("valid URL"));
        let mut set = Self::empty(root);
        let mut queue = VecDeque::new();
        for (mut url, value) in iter {
            url.set_fragment(None);
            set.add(url, value, &mut queue);
        }
        set
    }

    /// A set made of a single document. Its URL is `memory:///openapi.json`.
    pub fn from_value(value: Value) -> Self {
        Self::from_documents([(
            Url::parse("memory:///openapi.json").expect("valid URL"),
            value,
        )])
    }

    fn empty(root: Url) -> Self {
        Self {
            root,
            docs: HashMap::new(),
            order: Vec::new(),
            ids: HashMap::new(),
            anchors: HashMap::new(),
            dynamic_anchors: HashMap::new(),
        }
    }

    /// Adds a document and queues the documents it refers to, which it returns.
    fn add(&mut self, url: Url, value: Value, queue: &mut VecDeque<Url>) -> Vec<Url> {
        let mut refs = Vec::new();
        self.index(&url, &value, &mut refs);
        refs.retain(|r| !self.docs.contains_key(r) && *r != url);
        queue.extend(refs.iter().cloned());
        if !self.docs.contains_key(&url) {
            self.order.push(url.clone());
        }
        self.docs.insert(url, DocState::Loaded(value));
        refs
    }

    /// Walks a document, indexes `$id`s and anchors, and collects external reference targets into `refs`.
    fn index(&mut self, url: &Url, value: &Value, refs: &mut Vec<Url>) {
        let mut stack: Vec<(&Value, String, Url)> = vec![(value, String::new(), url.clone())];
        while let Some((value, ptr, base)) = stack.pop() {
            match value {
                Value::Object(map) => {
                    let mut base = base;
                    if let Some(Value::String(id)) = map.get("$id") {
                        if let Ok(mut id_url) = base.join(id) {
                            id_url.set_fragment(None);
                            self.ids
                                .entry(id_url.clone())
                                .or_insert_with(|| Loc::new(url.clone(), ptr.clone()));
                            base = id_url;
                        }
                    }
                    if let Some(Value::String(anchor)) = map.get("$anchor") {
                        self.anchors
                            .entry((base.clone(), anchor.clone()))
                            .or_insert_with(|| Loc::new(url.clone(), ptr.clone()));
                    }
                    if let Some(Value::String(anchor)) = map.get("$dynamicAnchor") {
                        let loc = Loc::new(url.clone(), ptr.clone());
                        self.anchors
                            .entry((base.clone(), anchor.clone()))
                            .or_insert_with(|| loc.clone());
                        self.dynamic_anchors
                            .entry((base.clone(), anchor.clone()))
                            .or_insert(loc);
                    }
                    for key in ["$ref", "$dynamicRef"] {
                        if let Some(Value::String(r)) = map.get(key) {
                            if let Ok(mut target) = base.join(r) {
                                target.set_fragment(None);
                                if target != *url {
                                    refs.push(target);
                                }
                            }
                        }
                    }
                    for (k, v) in map.iter().rev() {
                        if is_data_key(k, v) {
                            continue;
                        }
                        if k == "examples" {
                            // Only follows references to OpenAPI Example Objects and does not look inside the values.
                            if let Value::Object(examples) = v {
                                for ex in examples.values() {
                                    if let Some(Value::String(r)) = ex.get("$ref") {
                                        if let Ok(mut target) = base.join(r) {
                                            target.set_fragment(None);
                                            if target != *url {
                                                refs.push(target);
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        stack.push((v, pointer::push(&ptr, k), base.clone()));
                    }
                }
                Value::Array(items) => {
                    for (i, v) in items.iter().enumerate().rev() {
                        stack.push((v, pointer::push(&ptr, &i.to_string()), base.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    pub fn root_url(&self) -> &Url {
        &self.root
    }

    pub fn root_loc(&self) -> Loc {
        Loc::root(self.root.clone())
    }

    /// The root definition.
    pub fn root(&self) -> &Value {
        match self.docs.get(&self.root) {
            Some(DocState::Loaded(v)) => v,
            _ => &Value::Null,
        }
    }

    /// Returns the successfully loaded documents in load order.
    pub fn documents(&self) -> impl Iterator<Item = (&Url, &Value)> {
        self.order.iter().filter_map(|u| match self.docs.get(u) {
            Some(DocState::Loaded(v)) => Some((u, v)),
            _ => None,
        })
    }

    /// Documents that failed to load, with the reasons.
    pub fn failures(&self) -> impl Iterator<Item = (&Url, &str)> {
        self.order.iter().filter_map(|u| match self.docs.get(u) {
            Some(DocState::Failed(m)) => Some((u, m.as_str())),
            _ => None,
        })
    }

    /// Rewrites a document. The index is rebuilt afterwards.
    pub fn transform(&mut self, mut f: impl FnMut(&Url, &mut Value)) {
        let order = self.order.clone();
        for url in &order {
            if let Some(DocState::Loaded(v)) = self.docs.get_mut(url) {
                f(url, v);
            }
        }
        self.reindex();
    }

    /// Replaces the root definition.
    pub fn replace_root(&mut self, value: Value) {
        self.docs.insert(self.root.clone(), DocState::Loaded(value));
        self.reindex();
    }

    fn reindex(&mut self) {
        self.ids.clear();
        self.anchors.clear();
        self.dynamic_anchors.clear();
        let docs: Vec<(Url, Value)> = self
            .documents()
            .map(|(u, v)| (u.clone(), v.clone()))
            .collect();
        for (url, value) in &docs {
            let mut refs = Vec::new();
            self.index(url, value, &mut refs);
        }
    }

    pub fn get(&self, loc: &Loc) -> Result<&Value, RefError> {
        let doc = self.document(&loc.doc)?;
        pointer::get(doc, &loc.pointer).ok_or_else(|| RefError::NotFound(loc.clone()))
    }

    fn document(&self, url: &Url) -> Result<&Value, RefError> {
        match self.docs.get(url) {
            Some(DocState::Loaded(v)) => Ok(v),
            Some(DocState::Failed(m)) => Err(RefError::DocumentUnavailable {
                url: url.clone(),
                message: m.clone(),
            }),
            None => Err(RefError::UnknownDocument(url.clone())),
        }
    }

    /// Base URI at `loc`. Applies the `$id`s along the path in order.
    pub fn base_uri(&self, loc: &Loc) -> Url {
        let mut base = loc.doc.clone();
        let Ok(mut current) = self.document(&loc.doc) else {
            return base;
        };
        let apply = |v: &Value, base: &mut Url| {
            if let Some(Value::String(id)) = v.get("$id") {
                if let Ok(mut u) = base.join(id) {
                    u.set_fragment(None);
                    *base = u;
                }
            }
        };
        apply(current, &mut base);
        for token in loc.tokens() {
            let next = match current {
                Value::Object(m) => m.get(&token),
                Value::Array(a) => token.parse::<usize>().ok().and_then(|i| a.get(i)),
                _ => None,
            };
            let Some(next) = next else { break };
            current = next;
            apply(current, &mut base);
        }
        base
    }

    /// Returns the target of the reference `reference` written at `from`.
    pub fn resolve_ref(&self, from: &Loc, reference: &str) -> Result<Loc, RefError> {
        let base = self.base_uri(from);
        self.resolve_url(&base, reference)
    }

    /// Resolves the reference `reference` against the base URI `base`.
    pub fn resolve_url(&self, base: &Url, reference: &str) -> Result<Loc, RefError> {
        let target = base.join(reference).map_err(|e| RefError::Invalid {
            reference: reference.to_owned(),
            message: e.to_string(),
        })?;
        let fragment = target.fragment().unwrap_or("").to_owned();
        let mut resource = target;
        resource.set_fragment(None);
        let resource_loc = match self.ids.get(&resource) {
            Some(loc) => loc.clone(),
            None => {
                self.document(&resource)?;
                Loc::root(resource.clone())
            }
        };
        let fragment = percent_decode_str(&fragment)
            .decode_utf8()
            .map_err(|e| RefError::Invalid {
                reference: reference.to_owned(),
                message: e.to_string(),
            })?
            .into_owned();
        if fragment.is_empty() {
            self.get(&resource_loc)?;
            return Ok(resource_loc);
        }
        if fragment.starts_with('/') {
            let loc = Loc::new(
                resource_loc.doc.clone(),
                format!("{}{}", resource_loc.pointer, fragment),
            );
            self.get(&loc)?;
            return Ok(loc);
        }
        self.anchors
            .get(&(resource.clone(), fragment.clone()))
            .cloned()
            .ok_or(RefError::UnknownAnchor {
                url: resource,
                anchor: fragment,
            })
    }

    /// Looks up a `$dynamicAnchor`. `resource` is the URL of a resource in the dynamic scope.
    pub fn dynamic_anchor(&self, resource: &Url, name: &str) -> Option<&Loc> {
        self.dynamic_anchors
            .get(&(resource.clone(), name.to_owned()))
    }

    /// Resources that declare a `$dynamicAnchor` (per name, in no particular order).
    pub fn dynamic_anchors_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Loc> {
        self.dynamic_anchors
            .iter()
            .filter(move |((_, n), _)| n == name)
            .map(|(_, l)| l)
    }

    /// Follows references while the value at `loc` is an object with only `$ref`, and returns the final location and value.
    /// Stops at a `$ref` with sibling keys (the caller handles it).
    pub fn deref(&self, loc: &Loc) -> Result<(Loc, &Value), RefError> {
        self.deref_with(loc, |map| map.len() == 1)
    }

    /// Follows `$ref` regardless of sibling keys. Used where sibling keys may be ignored,
    /// such as OpenAPI 3.0 Reference Objects.
    pub fn deref_all(&self, loc: &Loc) -> Result<(Loc, &Value), RefError> {
        self.deref_with(loc, |_| true)
    }

    fn deref_with(
        &self,
        loc: &Loc,
        follow: impl Fn(&serde_json::Map<String, Value>) -> bool,
    ) -> Result<(Loc, &Value), RefError> {
        let mut seen = HashSet::new();
        let mut loc = loc.clone();
        loop {
            let value = self.get(&loc)?;
            let next = match value {
                Value::Object(map) if follow(map) => match map.get("$ref") {
                    Some(Value::String(r)) => Some(self.resolve_ref(&loc, r)?),
                    _ => None,
                },
                _ => None,
            };
            match next {
                None => return Ok((loc, value)),
                Some(next) => {
                    if !seen.insert(loc.clone()) {
                        return Err(RefError::Cycle(loc));
                    }
                    loc = next;
                }
            }
        }
    }
}

/// Keys whose values are data, where `$ref` and `$id` inside must not be treated as keywords.
fn is_data_key(key: &str, value: &Value) -> bool {
    match key {
        "example" | "default" | "const" | "enum" => true,
        // JSON Schema examples is an array of values.
        "examples" => value.is_array(),
        k => k.starts_with("x-"),
    }
}

/// Whether a document comes from the network rather than from the machine that generates.
fn is_remote(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::MemoryFetcher;
    use serde_json::json;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn internal_ref() {
        let set = DocumentSet::from_value(json!({
            "components": { "schemas": {
                "Pet": { "type": "object" },
                "Alias": { "$ref": "#/components/schemas/Pet" },
                "A/B": { "type": "string" },
            }}
        }));
        let from = set
            .root_loc()
            .join("components")
            .join("schemas")
            .join("Alias");
        let loc = set.resolve_ref(&from, "#/components/schemas/Pet").unwrap();
        assert_eq!(loc.pointer, "/components/schemas/Pet");
        let (loc, value) = set.deref(&from).unwrap();
        assert_eq!(loc.pointer, "/components/schemas/Pet");
        assert_eq!(value, &json!({ "type": "object" }));
        let loc = set.resolve_ref(&from, "#/components/schemas/A~1B").unwrap();
        assert_eq!(set.get(&loc).unwrap(), &json!({ "type": "string" }));
        let loc = set
            .resolve_ref(&from, "#/components/schemas/A%7E1B")
            .unwrap();
        assert_eq!(set.get(&loc).unwrap(), &json!({ "type": "string" }));
    }

    #[test]
    fn external_file_refs_are_loaded() {
        let mut f = MemoryFetcher::new();
        f.insert(
            "file:///spec/openapi.yaml",
            "components:\n  schemas:\n    Pet:\n      $ref: './schemas/pet.yaml#/Pet'\n",
        );
        f.insert(
            "file:///spec/schemas/pet.yaml",
            "Pet:\n  type: object\n  properties:\n    owner:\n      $ref: '../common/owner.json'\n",
        );
        f.insert("file:///spec/common/owner.json", r#"{"type":"string"}"#);
        let set = DocumentSet::load(&url("file:///spec/openapi.yaml"), &f).unwrap();
        let urls: Vec<_> = set
            .documents()
            .map(|(u, _)| u.as_str().to_owned())
            .collect();
        assert_eq!(
            urls,
            [
                "file:///spec/openapi.yaml",
                "file:///spec/schemas/pet.yaml",
                "file:///spec/common/owner.json"
            ]
        );
        let pet = set
            .root_loc()
            .join("components")
            .join("schemas")
            .join("Pet");
        let (loc, _) = set.deref(&pet).unwrap();
        assert_eq!(loc.to_string(), "file:///spec/schemas/pet.yaml#/Pet");
        let owner = loc.join("properties").join("owner");
        let (loc, v) = set.deref(&owner).unwrap();
        assert_eq!(loc.to_string(), "file:///spec/common/owner.json#");
        assert_eq!(v, &json!({ "type": "string" }));
    }

    #[test]
    fn remote_documents_cannot_refer_to_local_files() {
        let mut f = MemoryFetcher::new();
        f.insert(
            "https://api.example.com/openapi.yaml",
            "components:\n  schemas:\n    Secret: { $ref: 'file:///home/me/secret.yaml' }\n    Viaid: { $id: 'file:///home/me/', properties: { x: { $ref: 'secret.yaml' } } }\n",
        );
        f.insert(
            "file:///home/me/secret.yaml",
            "type: string\ndefault: s3cret\n",
        );
        let set = DocumentSet::load(&url("https://api.example.com/openapi.yaml"), &f).unwrap();
        let schemas = set.root_loc().join("components").join("schemas");
        for (name, loc) in [
            ("Secret", schemas.join("Secret")),
            ("Viaid", schemas.join("Viaid").join("properties").join("x")),
        ] {
            let err = set.deref(&loc).unwrap_err().to_string();
            assert!(err.contains("remote"), "{name}: {err}");
        }
        assert!(
            set.documents()
                .all(|(u, _)| u.as_str() != "file:///home/me/secret.yaml")
        );
        // A local document may still refer to local files, and to remote ones.
        let mut f = MemoryFetcher::new();
        f.insert(
            "file:///spec/openapi.yaml",
            "components:\n  schemas:\n    A: { $ref: 'a.yaml' }\n",
        );
        f.insert("file:///spec/a.yaml", "type: string\n");
        let set = DocumentSet::load(&url("file:///spec/openapi.yaml"), &f).unwrap();
        let loc = set.root_loc().join("components").join("schemas").join("A");
        assert!(set.deref(&loc).is_ok());
    }

    #[test]
    fn id_based_refs() {
        let set = DocumentSet::from_value(json!({
            "components": { "schemas": {
                "Pet": {
                    "$id": "https://example.com/schemas/pet",
                    "type": "object",
                    "properties": {
                        "tag": { "$ref": "tag" },
                        "self": { "$ref": "#/properties/tag" }
                    }
                },
                "Tag": { "$id": "https://example.com/schemas/tag", "type": "string" },
                "UsesPet": { "$ref": "https://example.com/schemas/pet" }
            }}
        }));
        let schemas = set.root_loc().join("components").join("schemas");
        let tag_ref = schemas.join("Pet").join("properties").join("tag");
        let (loc, _) = set.deref(&tag_ref).unwrap();
        assert_eq!(loc, schemas.join("Tag"));
        let self_ref = schemas.join("Pet").join("properties").join("self");
        let loc = set.resolve_ref(&self_ref, "#/properties/tag").unwrap();
        assert_eq!(loc, tag_ref);
        let (loc, _) = set.deref(&schemas.join("UsesPet")).unwrap();
        assert_eq!(loc, schemas.join("Pet"));
    }

    #[test]
    fn anchor_refs() {
        let set = DocumentSet::from_value(json!({
            "$defs": {
                "a": { "$anchor": "item", "type": "string" },
                "b": { "$dynamicAnchor": "node", "type": "object" },
                "nested": {
                    "$id": "https://example.com/n",
                    "$defs": { "x": { "$anchor": "item", "type": "number" } }
                }
            }
        }));
        let root = set.root_loc();
        assert_eq!(set.resolve_ref(&root, "#item").unwrap().pointer, "/$defs/a");
        assert_eq!(set.resolve_ref(&root, "#node").unwrap().pointer, "/$defs/b");
        assert_eq!(
            set.resolve_ref(&root, "https://example.com/n#item")
                .unwrap()
                .pointer,
            "/$defs/nested/$defs/x"
        );
        assert!(matches!(
            set.resolve_ref(&root, "#missing"),
            Err(RefError::UnknownAnchor { .. })
        ));
        assert!(set.dynamic_anchor(set.root_url(), "node").is_some());
    }

    #[test]
    fn fake_refs_in_examples_do_not_stop_loading() {
        let mut f = MemoryFetcher::new();
        f.insert(
            "file:///spec/openapi.json",
            json!({
                "components": { "schemas": {
                    "Doc": {
                        "type": "object",
                        "example": { "$ref": "https://unreachable.invalid/x.json" },
                        "x-sample": { "$ref": "nope.json" },
                        "properties": {
                            "a": { "type": "string", "default": { "$ref": "nope2.json" } },
                            "$ref": { "type": "string" }
                        }
                    }
                }},
                "paths": { "/a": { "get": { "responses": { "200": {
                    "description": "ok",
                    "content": { "application/json": { "examples": {
                        "one": { "value": { "$ref": "nope3.json" } }
                    }}}
                }}}}},
                "x-broken": { "$ref": "missing.json#/x" }
            })
            .to_string(),
        );
        let set = DocumentSet::load(&url("file:///spec/openapi.json"), &f).unwrap();
        assert_eq!(set.failures().count(), 0);
    }

    #[test]
    fn failed_documents_error_on_resolution() {
        let mut f = MemoryFetcher::new();
        f.insert(
            "file:///spec/openapi.json",
            r##"{"components":{"schemas":{"A":{"$ref":"missing.json#/A"}}}}"##,
        );
        let set = DocumentSet::load(&url("file:///spec/openapi.json"), &f).unwrap();
        assert_eq!(set.failures().count(), 1);
        let a = set.root_loc().join("components").join("schemas").join("A");
        assert!(matches!(
            set.deref(&a),
            Err(RefError::DocumentUnavailable { .. })
        ));
    }

    #[test]
    fn root_failure_is_an_error() {
        let f = MemoryFetcher::new();
        assert!(DocumentSet::load(&url("file:///spec/openapi.json"), &f).is_err());
    }

    #[test]
    fn deref_detects_cycles() {
        let set = DocumentSet::from_value(json!({
            "a": { "$ref": "#/b" },
            "b": { "$ref": "#/a" },
        }));
        assert!(matches!(
            set.deref(&set.root_loc().join("a")),
            Err(RefError::Cycle(_))
        ));
    }

    #[test]
    fn deref_stops_at_refs_with_siblings() {
        let set = DocumentSet::from_value(json!({
            "a": { "$ref": "#/b", "description": "x" },
            "b": { "type": "string" },
        }));
        let (loc, _) = set.deref(&set.root_loc().join("a")).unwrap();
        assert_eq!(loc.pointer, "/a");
        let (loc, _) = set.deref_all(&set.root_loc().join("a")).unwrap();
        assert_eq!(loc.pointer, "/b");
    }
}
