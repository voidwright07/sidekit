//! JSON Schema: infer one from sample documents, or check a document against one.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::shape::{self, Kind, Shape};

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

// ---------------------------------------------------------------- inference

#[derive(Clone, Copy)]
pub struct InferOpts {
    /// Recognise string formats and small repeated vocabularies (as `enum`).
    pub hints: bool,
    /// Add `additionalProperties: false` to every object.
    pub closed: bool,
}

fn kind_schema(kind: &Kind, o: InferOpts) -> Map<String, Value> {
    let mut m = Map::new();
    let ty = match kind {
        Kind::Bool => "boolean",
        Kind::Int => "integer",
        Kind::Float => "number",
        Kind::Str(s) => {
            if o.hints {
                if let Some(f) = s.format {
                    m.insert("format".into(), f.into());
                } else if let Some(values) = s.vocabulary() {
                    m.insert("enum".into(), values.iter().cloned().map(Value::String).collect());
                }
            }
            "string"
        }
        Kind::Array(items) => {
            m.insert("items".into(), shape_schema(items, o));
            "array"
        }
        Kind::Object(fields) => {
            let props: Map<String, Value> = fields.iter().map(|f| (f.name.clone(), shape_schema(&f.shape, o))).collect();
            let required: Vec<Value> = fields.iter().filter(|f| !f.optional).map(|f| f.name.clone().into()).collect();
            m.insert("properties".into(), props.into());
            if !required.is_empty() {
                m.insert("required".into(), required.into());
            }
            if o.closed {
                m.insert("additionalProperties".into(), false.into());
            }
            "object"
        }
    };
    // `type` reads best first; the map keeps insertion order.
    let mut out = Map::new();
    out.insert("type".into(), ty.into());
    out.extend(m);
    out
}

fn shape_schema(s: &Shape, o: InferOpts) -> Value {
    let mut parts: Vec<Map<String, Value>> = s.kinds.iter().map(|k| kind_schema(k, o)).collect();
    match parts.len() {
        0 if s.nullable => json!({"type": "null"}),
        0 => json!({}),
        1 => {
            let mut m = parts.remove(0);
            if s.nullable {
                let ty = m["type"].clone();
                m.insert("type".into(), json!([ty, "null"]));
                if let Some(Value::Array(e)) = m.get_mut("enum") {
                    e.push(Value::Null);
                }
            }
            m.into()
        }
        // Bare types fold into one `type` list; anything richer needs `anyOf`.
        _ if parts.iter().all(|p| p.len() == 1) => {
            let mut types: Vec<Value> = parts.iter().map(|p| p["type"].clone()).collect();
            if s.nullable {
                types.push("null".into());
            }
            json!({"type": types})
        }
        _ => {
            let mut any: Vec<Value> = parts.into_iter().map(Value::Object).collect();
            if s.nullable {
                any.push(json!({"type": "null"}));
            }
            json!({"anyOf": any})
        }
    }
}

/// A schema describing every sample in `src`, and how many samples there were.
pub fn infer(src: &str, opts: InferOpts) -> Result<(Value, usize), String> {
    let (shape, n) = shape::of_documents(src)?;
    let mut root = Map::new();
    root.insert("$schema".into(), DIALECT.into());
    if let Value::Object(m) = shape_schema(&shape, opts) {
        root.extend(m);
    }
    Ok((root.into(), n))
}

// ---------------------------------------------------------------- validation

#[derive(Debug, PartialEq)]
pub struct Issue {
    /// Where in the document, as a JSONPath: `$.users[0].name`.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Report {
    /// At most one per path, in document order of discovery.
    pub issues: Vec<Issue>,
    /// Keywords the schema uses that were not checked.
    pub skipped: Vec<String>,
}

/// Keywords that describe rather than constrain, or are read by a neighbour.
const ANNOTATIONS: &[&str] = &[
    "$schema", "$id", "$defs", "definitions", "$comment", "$anchor", "title", "description", "default", "examples", "deprecated",
    "readOnly", "writeOnly", "contentEncoding", "contentMediaType", "then", "else", "minContains", "maxContains", "additionalItems",
    "id",
];

const KEYWORDS: &[&str] = &[
    "$ref", "type", "enum", "const", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "minLength",
    "maxLength", "pattern", "format", "items", "prefixItems", "contains", "minItems", "maxItems", "uniqueItems", "properties",
    "required", "additionalProperties", "patternProperties", "propertyNames", "minProperties", "maxProperties", "dependentRequired",
    "dependentSchemas", "dependencies", "allOf", "anyOf", "oneOf", "not", "if",
];

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if is_integer(n) => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_integer(n: &serde_json::Number) -> bool {
    n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.)
}

/// Equality as JSON Schema means it: `1` and `1.0` are the same number.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| same(p, q))),
        _ => a == b,
    }
}

fn brief(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    match s.char_indices().nth(60) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

fn child(path: &str, key: &str) -> String {
    let plain = key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain { format!("{path}.{key}") } else { format!("{path}[{}]", serde_json::to_string(key).unwrap_or_default()) }
}

fn format_ok(format: &str, s: &str) -> Option<bool> {
    Some(match format {
        "date-time" => chrono::DateTime::parse_from_rfc3339(s).is_ok(),
        "date" => s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok(),
        "time" => chrono::DateTime::parse_from_rfc3339(&format!("1970-01-01T{s}")).is_ok(),
        "email" | "idn-email" => shape::string_format(s) == Some("email"),
        "uuid" => shape::string_format(s) == Some("uuid"),
        "ipv4" => s.parse::<std::net::Ipv4Addr>().is_ok(),
        "ipv6" => s.parse::<std::net::Ipv6Addr>().is_ok(),
        "uri" | "iri" => s.split_once(':').is_some_and(|(sc, _)| {
            sc.starts_with(|c: char| c.is_ascii_alphabetic()) && sc.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        }),
        "regex" => fancy_regex::Regex::new(s).is_ok(),
        _ => return None,
    })
}

struct Checker<'a> {
    root: &'a Value,
    skipped: BTreeSet<String>,
    /// Schemas entered and not yet left, to stop a `$ref` that loops on itself.
    depth: usize,
}

impl<'a> Checker<'a> {
    fn resolve(&mut self, reference: &str) -> Option<&'a Value> {
        let pointer = reference.strip_prefix('#')?;
        if pointer.is_empty() {
            return Some(self.root);
        }
        if !pointer.starts_with('/') {
            return None;
        }
        let pointer = super::url_dec(pointer).unwrap_or_else(|_| pointer.to_string());
        self.root.pointer(&pointer)
    }

    /// Whether `v` satisfies `s`, without reporting why not.
    fn passes(&mut self, v: &Value, s: &'a Value, path: &str) -> bool {
        let mut scratch = Vec::new();
        self.check(v, s, path, &mut scratch);
        scratch.is_empty()
    }

    fn check(&mut self, v: &Value, s: &'a Value, path: &str, out: &mut Vec<Issue>) {
        let schema = match s {
            Value::Bool(true) => return,
            Value::Bool(false) => return out.push(Issue { path: path.into(), message: "No value is allowed here".into() }),
            Value::Object(m) => m,
            _ => return,
        };
        if self.depth > 96 {
            self.skipped.insert("$ref (nested too deeply to follow)".into());
            return;
        }
        self.depth += 1;
        self.keywords(v, schema, path, out);
        self.depth -= 1;
    }

    fn keywords(&mut self, v: &Value, schema: &'a Map<String, Value>, path: &str, out: &mut Vec<Issue>) {
        let fail = |out: &mut Vec<Issue>, message: String| out.push(Issue { path: path.into(), message });
        for key in schema.keys() {
            if !KEYWORDS.contains(&key.as_str()) && !ANNOTATIONS.contains(&key.as_str()) {
                self.skipped.insert(key.clone());
            }
        }

        if let Some(Value::String(r)) = schema.get("$ref") {
            match self.resolve(r) {
                Some(target) => self.check(v, target, path, out),
                None => {
                    self.skipped.insert(format!("$ref \"{r}\" (only #/… pointers inside this schema are followed)"));
                }
            }
        }

        if let Some(t) = schema.get("type") {
            let actual = type_name(v);
            let fits = |want: &str| want == actual || (want == "number" && actual == "integer");
            let (ok, wanted) = match t {
                Value::String(w) => (fits(w), w.clone()),
                Value::Array(a) => {
                    let names: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                    (names.iter().any(|w| fits(w)), names.join(" or "))
                }
                _ => (true, String::new()),
            };
            if !ok {
                fail(out, format!("Expected {wanted}, found {actual}"));
                // The remaining keywords assume the right type.
                return;
            }
        }
        if let Some(Value::Array(options)) = schema.get("enum") {
            if !options.iter().any(|o| same(o, v)) {
                let list = options.iter().take(8).map(brief).collect::<Vec<_>>().join(", ");
                let more = if options.len() > 8 { ", …" } else { "" };
                fail(out, format!("{} is not one of {list}{more}", brief(v)));
            }
        }
        if let Some(c) = schema.get("const") {
            if !same(c, v) {
                fail(out, format!("Must be exactly {}", brief(c)));
            }
        }

        match v {
            Value::Number(n) => {
                let x = n.as_f64().unwrap_or(f64::NAN);
                let num = |k: &str| schema.get(k).and_then(Value::as_f64);
                // Draft 4 spelled exclusivity as a boolean beside minimum / maximum.
                let flag = |k: &str| schema.get(k).and_then(Value::as_bool).unwrap_or(false);
                if let Some(min) = num("minimum") {
                    if x < min || (flag("exclusiveMinimum") && x == min) {
                        fail(out, format!("{n} is below the minimum of {min}"));
                    }
                }
                if let Some(max) = num("maximum") {
                    if x > max || (flag("exclusiveMaximum") && x == max) {
                        fail(out, format!("{n} is above the maximum of {max}"));
                    }
                }
                if let Some(min) = num("exclusiveMinimum") {
                    if x <= min {
                        fail(out, format!("{n} must be greater than {min}"));
                    }
                }
                if let Some(max) = num("exclusiveMaximum") {
                    if x >= max {
                        fail(out, format!("{n} must be less than {max}"));
                    }
                }
                if let Some(step) = num("multipleOf").filter(|s| *s > 0.) {
                    let q = x / step;
                    if (q - q.round()).abs() > 1e-9 * q.abs().max(1.) {
                        fail(out, format!("{n} is not a multiple of {step}"));
                    }
                }
            }
            Value::String(text) => {
                let len = text.chars().count() as u64;
                if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                    if len < min {
                        fail(out, format!("Shorter than {min} characters ({len})"));
                    }
                }
                if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
                    if len > max {
                        fail(out, format!("Longer than {max} characters ({len})"));
                    }
                }
                if let Some(Value::String(p)) = schema.get("pattern") {
                    match fancy_regex::Regex::new(p) {
                        Ok(re) => {
                            if !re.is_match(text).unwrap_or(true) {
                                fail(out, format!("Does not match the pattern {p}"));
                            }
                        }
                        Err(_) => {
                            self.skipped.insert(format!("pattern \"{p}\" (not a valid regular expression)"));
                        }
                    }
                }
                if let Some(Value::String(f)) = schema.get("format") {
                    match format_ok(f, text) {
                        Some(true) => {}
                        Some(false) => fail(out, format!("Not a valid {f}")),
                        None => {
                            self.skipped.insert(format!("format \"{f}\""));
                        }
                    }
                }
            }
            Value::Array(items) => self.array(items, schema, path, out),
            Value::Object(map) => self.object(map, schema, path, out),
            _ => {}
        }

        if let Some(Value::Array(all)) = schema.get("allOf") {
            for sub in all {
                self.check(v, sub, path, out);
            }
        }
        if let Some(Value::Array(any)) = schema.get("anyOf") {
            if !any.iter().any(|sub| self.passes(v, sub, path)) {
                // With a single alternative its own errors say more.
                match any.as_slice() {
                    [only] => self.check(v, only, path, out),
                    _ => out.push(Issue { path: path.into(), message: format!("Matches none of the {} anyOf alternatives", any.len()) }),
                }
            }
        }
        if let Some(Value::Array(one)) = schema.get("oneOf") {
            let hits = one.iter().filter(|sub| self.passes(v, sub, path)).count();
            if hits != 1 {
                let found = if hits == 0 { "none".to_string() } else { hits.to_string() };
                out.push(Issue { path: path.into(), message: format!("Must match exactly one oneOf alternative, but matches {found}") });
            }
        }
        if let Some(not) = schema.get("not") {
            if self.passes(v, not, path) {
                out.push(Issue { path: path.into(), message: "Matches a schema it must not match (not)".into() });
            }
        }
        if let Some(cond) = schema.get("if") {
            let branch = if self.passes(v, cond, path) { "then" } else { "else" };
            if let Some(sub) = schema.get(branch) {
                self.check(v, sub, path, out);
            }
        }
    }

    fn array(&mut self, items: &[Value], schema: &'a Map<String, Value>, path: &str, out: &mut Vec<Issue>) {
        let n = items.len() as u64;
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if n < min {
                out.push(Issue { path: path.into(), message: format!("Has {n} items, fewer than the minimum of {min}") });
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if n > max {
                out.push(Issue { path: path.into(), message: format!("Has {n} items, more than the maximum of {max}") });
            }
        }
        if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
            if let Some(i) = (1..items.len()).find(|i| items[..*i].iter().any(|x| same(x, &items[*i]))) {
                out.push(Issue { path: format!("{path}[{i}]"), message: "Duplicates an earlier item (uniqueItems)".into() });
            }
        }
        // Tuple schemas: `prefixItems`, or before 2020-12 an array under `items`.
        let (prefix, rest) = match (schema.get("prefixItems"), schema.get("items")) {
            (Some(Value::Array(p)), rest) => (p.as_slice(), rest),
            (_, Some(Value::Array(p))) => (p.as_slice(), schema.get("additionalItems")),
            (_, rest) => (&[][..], rest),
        };
        for (i, item) in items.iter().enumerate() {
            if let Some(sub) = prefix.get(i).or(rest) {
                self.check(item, sub, &format!("{path}[{i}]"), out);
            }
        }
        if let Some(sub) = schema.get("contains") {
            let hits = items.iter().enumerate().filter(|(i, x)| self.passes(x, sub, &format!("{path}[{i}]"))).count() as u64;
            let min = schema.get("minContains").and_then(Value::as_u64).unwrap_or(1);
            let max = schema.get("maxContains").and_then(Value::as_u64);
            if hits < min {
                out.push(Issue { path: path.into(), message: format!("Needs at least {min} item(s) matching contains, has {hits}") });
            } else if max.is_some_and(|m| hits > m) {
                out.push(Issue { path: path.into(), message: format!("Has {hits} items matching contains, more than {}", max.unwrap()) });
            }
        }
    }

    fn object(&mut self, map: &Map<String, Value>, schema: &'a Map<String, Value>, path: &str, out: &mut Vec<Issue>) {
        let n = map.len() as u64;
        if let Some(min) = schema.get("minProperties").and_then(Value::as_u64) {
            if n < min {
                out.push(Issue { path: path.into(), message: format!("Has {n} properties, fewer than the minimum of {min}") });
            }
        }
        if let Some(max) = schema.get("maxProperties").and_then(Value::as_u64) {
            if n > max {
                out.push(Issue { path: path.into(), message: format!("Has {n} properties, more than the maximum of {max}") });
            }
        }
        if let Some(Value::Array(required)) = schema.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !map.contains_key(name) {
                    out.push(Issue { path: child(path, name), message: "Required property is missing".into() });
                }
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        let patterns: Vec<(fancy_regex::Regex, &'a Value)> = schema
            .get("patternProperties")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(p, sub)| Some((fancy_regex::Regex::new(p).ok()?, sub))).collect())
            .unwrap_or_default();
        let additional = schema.get("additionalProperties");
        let names = schema.get("propertyNames");
        for (key, value) in map {
            let at = child(path, key);
            let mut known = false;
            if let Some(sub) = props.and_then(|p| p.get(key)) {
                known = true;
                self.check(value, sub, &at, out);
            }
            for (re, sub) in &patterns {
                if re.is_match(key).unwrap_or(false) {
                    known = true;
                    self.check(value, sub, &at, out);
                }
            }
            if !known {
                match additional {
                    Some(Value::Bool(false)) => out.push(Issue { path: at.clone(), message: "Property is not allowed (additionalProperties)".into() }),
                    Some(sub) => self.check(value, sub, &at, out),
                    None => {}
                }
            }
            if let Some(sub) = names {
                if !self.passes(&Value::String(key.clone()), sub, &at) {
                    out.push(Issue { path: at, message: "Property name is not allowed (propertyNames)".into() });
                }
            }
        }
        // `dependencies` is the older spelling of both dependent keywords.
        for keyword in ["dependentRequired", "dependentSchemas", "dependencies"] {
            let Some(Value::Object(deps)) = schema.get(keyword) else { continue };
            for (trigger, dep) in deps.iter().filter(|(k, _)| map.contains_key(*k)) {
                match dep {
                    Value::Array(needed) => {
                        for name in needed.iter().filter_map(Value::as_str).filter(|n| !map.contains_key(*n)) {
                            out.push(Issue { path: child(path, name), message: format!("Required because \"{trigger}\" is present") });
                        }
                    }
                    sub => self.check(&Value::Object(map.clone()), sub, path, out),
                }
            }
        }
    }
}

pub fn validate(doc: &str, schema: &str) -> Result<Report, String> {
    let doc: Value = serde_json::from_str(doc).map_err(|e| format!("The document is not valid JSON: {}", super::json_err(&e)))?;
    let schema: Value = serde_json::from_str(schema).map_err(|e| format!("The schema is not valid JSON: {}", super::json_err(&e)))?;
    if !schema.is_object() && !schema.is_boolean() {
        return Err("A schema is a JSON object (or true / false)".into());
    }
    let mut checker = Checker { root: &schema, skipped: BTreeSet::new(), depth: 0 };
    let mut found = Vec::new();
    checker.check(&doc, &schema, "$", &mut found);
    let mut seen = BTreeSet::new();
    let issues = found.into_iter().filter(|i| seen.insert(i.path.clone())).collect();
    Ok(Report { issues, skipped: checker.skipped.into_iter().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINTS: InferOpts = InferOpts { hints: true, closed: false };

    #[test]
    fn infers_optional_nullable_and_formats() {
        let src = r#"[
            {"id": 1, "email": "ada@example.com", "role": "admin", "seen": "2026-10-01T10:00:00Z", "tags": ["a"]},
            {"id": 2, "email": "alan@example.com", "role": "user", "seen": null, "tags": [], "score": 1.5},
            {"id": 3, "email": "g@example.com", "role": "user", "seen": "2026-10-02T10:00:00Z", "tags": ["b", "c"]},
            {"id": 4, "email": "e@example.com", "role": "admin", "seen": "2026-10-03T10:00:00Z", "tags": []}
        ]"#;
        let (schema, n) = infer(src, HINTS).unwrap();
        assert_eq!(n, 1);
        assert_eq!(schema["$schema"], DIALECT);
        assert_eq!(schema["type"], "array");
        let item = &schema["items"];
        assert_eq!(item["required"], json!(["id", "email", "role", "seen", "tags"]));
        assert_eq!(item["properties"]["id"], json!({"type": "integer"}));
        assert_eq!(item["properties"]["email"], json!({"type": "string", "format": "email"}));
        assert_eq!(item["properties"]["role"], json!({"type": "string", "enum": ["admin", "user"]}));
        assert_eq!(item["properties"]["seen"], json!({"type": ["string", "null"], "format": "date-time"}));
        assert_eq!(item["properties"]["tags"], json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(item["properties"]["score"], json!({"type": "number"}));

        let (plain, _) = infer(src, InferOpts { hints: false, closed: true }).unwrap();
        assert_eq!(plain["items"]["properties"]["role"], json!({"type": "string"}));
        assert_eq!(plain["items"]["additionalProperties"], false);
    }

    #[test]
    fn infers_unions_and_empties() {
        let (s, _) = infer(r#"{"a": [1, "x", null], "b": [], "c": null, "d": [{"x": 1}, "y"]}"#, HINTS).unwrap();
        assert_eq!(s["properties"]["a"]["items"], json!({"type": ["integer", "string", "null"]}));
        assert_eq!(s["properties"]["b"], json!({"type": "array", "items": {}}));
        assert_eq!(s["properties"]["c"], json!({"type": "null"}));
        assert_eq!(s["properties"]["d"]["items"]["anyOf"].as_array().unwrap().len(), 2);
        let (many, n) = infer("{\"a\": 1}\n{\"b\": 2}", HINTS).unwrap();
        assert_eq!(n, 2);
        assert!(many.get("required").is_none());
    }

    #[test]
    fn an_inferred_schema_accepts_its_samples() {
        let src = r#"{"users": [{"id": 1, "name": "Ada", "tags": ["x"]}, {"id": 2, "name": null}], "ok": true}"#;
        let (schema, _) = infer(src, InferOpts { hints: true, closed: true }).unwrap();
        let report = validate(src, &schema.to_string()).unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        assert!(report.skipped.is_empty());
    }

    fn issues(doc: &str, schema: &str) -> Vec<(String, String)> {
        validate(doc, schema).unwrap().issues.into_iter().map(|i| (i.path, i.message)).collect()
    }

    #[test]
    fn reports_one_issue_per_path() {
        let schema = r#"{
            "type": "object",
            "required": ["id", "name"],
            "additionalProperties": false,
            "properties": {
                "id": {"type": "integer", "minimum": 1},
                "name": {"type": "string", "minLength": 2, "pattern": "^[A-Z]"},
                "tags": {"type": "array", "items": {"enum": ["a", "b"]}, "uniqueItems": true, "maxItems": 3},
                "when": {"type": "string", "format": "date"}
            }
        }"#;
        let got = issues(r#"{"id": 0, "tags": ["a", "z", "a"], "when": "tomorrow", "extra": 1, "name": "x"}"#, schema);
        let at = |p: &str| got.iter().find(|i| i.0 == p).map(|i| i.1.as_str());
        assert_eq!(at("$.id"), Some("0 is below the minimum of 1"));
        // Two things are wrong with the name; only the first is listed.
        assert_eq!(at("$.name"), Some("Shorter than 2 characters (1)"));
        assert_eq!(got.iter().filter(|i| i.0 == "$.name").count(), 1);
        assert_eq!(at("$.tags[1]"), Some("\"z\" is not one of \"a\", \"b\""));
        assert_eq!(at("$.tags[2]"), Some("Duplicates an earlier item (uniqueItems)"));
        assert_eq!(at("$.when"), Some("Not a valid date"));
        assert_eq!(at("$.extra"), Some("Property is not allowed (additionalProperties)"));
        assert_eq!(issues("{}", schema), [("$.id".to_string(), "Required property is missing".to_string()), ("$.name".into(), "Required property is missing".into())]);
        assert_eq!(issues("[]", schema)[0].1, "Expected object, found array");
    }

    #[test]
    fn combinators_refs_and_conditions() {
        let schema = r##"{
            "$defs": {"port": {"type": "integer", "minimum": 1, "maximum": 65535}},
            "type": "object",
            "properties": {
                "port": {"$ref": "#/$defs/port"},
                "host": {"anyOf": [{"type": "string", "format": "ipv4"}, {"type": "string", "pattern": "^[a-z.]+$"}]},
                "mode": {"oneOf": [{"const": "a"}, {"const": "b"}]},
                "n": {"not": {"type": "string"}, "multipleOf": 0.5}
            },
            "if": {"properties": {"mode": {"const": "a"}}, "required": ["mode"]},
            "then": {"required": ["port"]},
            "dependentRequired": {"host": ["port"]}
        }"##;
        assert!(issues(r#"{"port": 8080, "host": "10.0.0.1", "mode": "a", "n": 1.5}"#, schema).is_empty());
        assert!(issues(r#"{"host": "example.com", "port": 1.0}"#, schema).is_empty());
        let got = issues(r#"{"port": 70000, "host": "Nope!", "mode": "c", "n": "x"}"#, schema);
        assert_eq!(got[0], ("$.port".into(), "70000 is above the maximum of 65535".into()));
        assert_eq!(got[1].1, "Matches none of the 2 anyOf alternatives");
        assert_eq!(got[2].1, "Must match exactly one oneOf alternative, but matches none");
        assert_eq!(got[3].1, "Matches a schema it must not match (not)");
        assert_eq!(issues(r#"{"mode": "a"}"#, schema), [("$.port".to_string(), "Required property is missing".to_string())]);
        assert_eq!(issues(r#"{"host": "a.b"}"#, schema), [("$.port".to_string(), "Required because \"host\" is present".to_string())]);
    }

    #[test]
    fn tuples_contains_and_numbers() {
        let tuple = r#"{"prefixItems": [{"type": "string"}, {"type": "integer"}], "items": false}"#;
        assert!(issues(r#"["a", 1]"#, tuple).is_empty());
        assert_eq!(issues(r#"["a", 1, 2]"#, tuple), [("$[2]".to_string(), "No value is allowed here".to_string())]);
        let old = r#"{"items": [{"type": "string"}], "additionalItems": {"type": "boolean"}}"#;
        assert_eq!(issues(r#"["a", 1]"#, old)[0].0, "$[1]");
        let contains = r#"{"contains": {"type": "integer"}, "minContains": 2}"#;
        assert_eq!(issues("[1, \"a\"]", contains)[0].1, "Needs at least 2 item(s) matching contains, has 1");
        assert!(issues("[1, 2]", contains).is_empty());
        assert!(issues("3", r#"{"type": "integer", "exclusiveMinimum": 2}"#).is_empty());
        assert_eq!(issues("2", r#"{"minimum": 2, "exclusiveMinimum": true}"#)[0].1, "2 is below the minimum of 2");
        assert!(issues("1", r#"{"enum": [1.0]}"#).is_empty());
        assert_eq!(issues(r#"{"a b": 1}"#, r#"{"propertyNames": {"pattern": "^\\S+$"}}"#)[0].0, "$[\"a b\"]");
    }

    #[test]
    fn names_what_it_did_not_check() {
        let r = validate("{}", r##"{"unevaluatedProperties": false, "x-team": "a", "$ref": "other.json#/a", "properties": {"a": {"format": "hostname"}}, "title": "T"}"##).unwrap();
        assert_eq!(r.skipped, ["$ref \"other.json#/a\" (only #/… pointers inside this schema are followed)", "unevaluatedProperties", "x-team"]);
        let r = validate(r#"{"a": "x"}"#, r#"{"properties": {"a": {"format": "hostname", "pattern": "("}}}"#).unwrap();
        assert_eq!(r.skipped, ["format \"hostname\"", "pattern \"(\" (not a valid regular expression)"]);
        // A schema that refers to itself must not hang.
        assert!(validate("1", r##"{"$ref": "#"}"##).unwrap().issues.is_empty());
        assert!(validate("{", "{}").is_err());
        assert!(validate("{}", "[]").is_err());
    }
}
