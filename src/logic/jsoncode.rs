//! Typed models from sample JSON, in a handful of languages.

use super::shape::{self, Kind, Shape};
use super::split_words;

pub const LANGS: &[&str] = &["TypeScript", "Rust", "Go", "Python", "Swift", "Kotlin", "C#", "Java"];

/// The editor grammar for each of `LANGS`, where one is bundled.
pub const GRAMMARS: &[&str] = &["typescript", "rust", "go", "python", "swift", "kotlin", "csharp", "java"];

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Any,
    Bool,
    Int,
    Float,
    Str,
    List(Box<Ty>),
    /// An index into the model list.
    Ref(usize),
    Union(Vec<Ty>),
    /// `null` is among the values (array items only; fields carry a flag).
    Opt(Box<Ty>),
}

#[derive(Clone, Debug, PartialEq)]
struct Prop {
    key: String,
    ty: Ty,
    /// The key may be absent.
    optional: bool,
    nullable: bool,
}

struct Model {
    name: String,
    props: Vec<Prop>,
}

fn pascal(s: &str) -> String {
    split_words(s)
        .iter()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn snake(s: &str) -> String {
    split_words(s).iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_")
}

/// An identifier out of `raw`: never empty, never starting with a digit.
fn ident(raw: String, fallback: &str) -> String {
    match raw.chars().next() {
        None => fallback.to_string(),
        Some(c) if c.is_ascii_digit() => format!("_{raw}"),
        _ => raw,
    }
}

/// `line_items` → `line_item`, `categories` → `category`, `addresses` → `address`.
fn singular(s: &str) -> String {
    let lower = s.to_lowercase();
    let cut = |n: usize| s[..s.len() - n].to_string();
    if lower.ends_with("ies") && s.len() > 4 {
        format!("{}y", cut(3))
    } else if ["sses", "xes", "ches", "shes"].iter().any(|e| lower.ends_with(e)) {
        cut(2)
    } else if lower.ends_with('s') && s.len() > 2 && !["ss", "us", "is"].iter().any(|e| lower.ends_with(e)) {
        cut(1)
    } else {
        s.to_string()
    }
}

/// A type name for the objects found under `key`: `line_items` → `LineItem`.
fn type_name(key: &str, in_list: bool) -> String {
    let base = if in_list { singular(key) } else { key.to_string() };
    ident(pascal(&base), "Item")
}

struct Builder {
    models: Vec<Model>,
}

impl Builder {
    fn ty(&mut self, s: &Shape, key: &str, in_list: bool) -> Ty {
        let mut kinds: Vec<Ty> = s.kinds.iter().map(|k| self.kind(k, key, in_list)).collect();
        match kinds.len() {
            0 => Ty::Any,
            1 => kinds.remove(0),
            _ => Ty::Union(kinds),
        }
    }

    fn kind(&mut self, k: &Kind, key: &str, in_list: bool) -> Ty {
        match k {
            Kind::Bool => Ty::Bool,
            Kind::Int => Ty::Int,
            Kind::Float => Ty::Float,
            Kind::Str(_) => Ty::Str,
            Kind::Array(items) => {
                let inner = self.ty(items, key, true);
                Ty::List(Box::new(if items.nullable && !items.kinds.is_empty() { Ty::Opt(Box::new(inner)) } else { inner }))
            }
            Kind::Object(fields) => Ty::Ref(self.model(fields, type_name(key, in_list))),
        }
    }

    fn model(&mut self, fields: &[shape::Field], wanted: String) -> usize {
        // Reserve the slot first so parents are listed before their children.
        let at = self.models.len();
        let mut name = wanted.clone();
        let mut n = 2;
        while self.models.iter().any(|m| m.name == name) {
            name = format!("{wanted}{n}");
            n += 1;
        }
        self.models.push(Model { name, props: Vec::new() });
        let props: Vec<Prop> = fields
            .iter()
            .map(|f| Prop { key: f.name.clone(), ty: self.ty(&f.shape, &f.name, false), optional: f.optional, nullable: f.shape.nullable })
            .collect();
        // The same structure seen again under another key is one type, not two.
        if let Some(twin) = self.models[..at].iter().position(|m| m.props == props && !props.is_empty()) {
            if self.models.len() == at + 1 {
                self.models.pop();
                return twin;
            }
        }
        self.models[at].props = props;
        at
    }
}

fn build(src: &str, root: &str) -> Result<Vec<Model>, String> {
    let (shape, _) = shape::of_documents(src)?;
    let root = ident(pascal(root), "Root");
    let mut b = Builder { models: Vec::new() };
    // A list of records is modelled by its record.
    let mut top = &shape;
    while let [Kind::Array(items)] = top.kinds.as_slice() {
        top = items;
    }
    match top.kinds.as_slice() {
        [Kind::Object(fields)] => {
            b.model(fields, root);
            Ok(b.models)
        }
        _ => Err("Models are generated from a JSON object, or an array of objects".into()),
    }
}

/// Columns padded to line up, the way gofmt lays out struct fields.
fn pad(s: &str, width: usize) -> String {
    format!("{s:<width$}")
}

fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

// ---------------------------------------------------------------- TypeScript

fn ts_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any => "unknown".into(),
        Ty::Bool => "boolean".into(),
        Ty::Int | Ty::Float => "number".into(),
        Ty::Str => "string".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Union(u) => u.iter().map(|t| ts_type(t, m)).collect::<Vec<_>>().join(" | "),
        Ty::Opt(t) => format!("{} | null", ts_type(t, m)),
        Ty::List(t) => match **t {
            Ty::Union(_) | Ty::Opt(_) => format!("({})[]", ts_type(t, m)),
            _ => format!("{}[]", ts_type(t, m)),
        },
    }
}

fn typescript(m: &[Model]) -> String {
    let mut out = Vec::new();
    for model in m {
        let mut s = format!("export interface {} {{\n", model.name);
        for p in &model.props {
            let plain = p.key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
                && p.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            let key = if plain { p.key.clone() } else { quoted(&p.key) };
            let mut ty = ts_type(&p.ty, m);
            if p.nullable && p.ty != Ty::Any {
                ty.push_str(" | null");
            } else if p.nullable {
                ty = "null".into();
            }
            s.push_str(&format!("  {key}{}: {ty};\n", if p.optional { "?" } else { "" }));
        }
        s.push('}');
        out.push(s);
    }
    out.join("\n\n")
}

// ---------------------------------------------------------------- Rust

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
    "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "try", "type",
    "unsafe", "use", "where", "while", "yield",
];

fn rust_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "serde_json::Value".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "i64".into(),
        Ty::Float => "f64".into(),
        Ty::Str => "String".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => format!("Option<{}>", rust_type(t, m)),
        Ty::List(t) => format!("Vec<{}>", rust_type(t, m)),
    }
}

fn rust(m: &[Model]) -> String {
    let mut out = vec!["use serde::{Deserialize, Serialize};".to_string()];
    for model in m {
        let mut s = format!("#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {} {{\n", model.name);
        for (i, p) in model.props.iter().enumerate() {
            let mut name = ident(snake(&p.key), &format!("field_{}", i + 1));
            // `r#self` and friends are not allowed, so those get a suffix instead.
            if ["self", "super", "crate"].contains(&name.as_str()) {
                name.push('_');
            }
            if name != p.key {
                s.push_str(&format!("    #[serde(rename = {})]\n", quoted(&p.key)));
            }
            if RUST_KEYWORDS.contains(&name.as_str()) {
                name = format!("r#{name}");
            }
            let ty = rust_type(&p.ty, m);
            let ty = if p.optional || p.nullable { format!("Option<{ty}>") } else { ty };
            s.push_str(&format!("    pub {name}: {ty},\n"));
        }
        s.push('}');
        out.push(s);
    }
    out.join("\n\n")
}

// ---------------------------------------------------------------- Go

const GO_INITIALISMS: &[&str] = &["id", "url", "uri", "api", "http", "https", "json", "xml", "html", "ip", "uuid", "sql"];

fn go_name(key: &str, fallback: &str) -> String {
    let name: String = split_words(key)
        .iter()
        .map(|w| if GO_INITIALISMS.contains(&w.to_lowercase().as_str()) { w.to_uppercase() } else { pascal(w) })
        .collect();
    match name.chars().next() {
        None => fallback.to_string(),
        Some(c) if c.is_ascii_digit() => format!("F{name}"),
        _ => name,
    }
}

fn go_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "any".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "int64".into(),
        Ty::Float => "float64".into(),
        Ty::Str => "string".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => format!("*{}", go_type(t, m)),
        Ty::List(t) => format!("[]{}", go_type(t, m)),
    }
}

fn go(m: &[Model]) -> String {
    let mut out = Vec::new();
    for model in m {
        let rows: Vec<(String, String, String)> = model
            .props
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = go_type(&p.ty, m);
                // Slices and `any` are already nil-able; the rest need a pointer.
                let pointer = (p.optional || p.nullable) && !matches!(p.ty, Ty::List(_) | Ty::Any | Ty::Union(_));
                let ty = if pointer { format!("*{ty}") } else { ty };
                let tag = format!("`json:\"{}{}\"`", p.key.replace('"', "\\\""), if p.optional { ",omitempty" } else { "" });
                (go_name(&p.key, &format!("Field{}", i + 1)), ty, tag)
            })
            .collect();
        let (w0, w1) = (rows.iter().map(|r| r.0.len()).max().unwrap_or(0), rows.iter().map(|r| r.1.len()).max().unwrap_or(0));
        let mut s = format!("type {} struct {{\n", model.name);
        for (name, ty, tag) in rows {
            s.push_str(&format!("\t{} {} {tag}\n", pad(&name, w0), pad(&ty, w1)));
        }
        s.push('}');
        out.push(s);
    }
    out.join("\n\n")
}

// ---------------------------------------------------------------- Python

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
    "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield",
];

fn py_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any => "Any".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "int".into(),
        Ty::Float => "float".into(),
        Ty::Str => "str".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Union(u) => u.iter().map(|t| py_type(t, m)).collect::<Vec<_>>().join(" | "),
        Ty::Opt(t) => format!("{} | None", py_type(t, m)),
        Ty::List(t) => format!("list[{}]", py_type(t, m)),
    }
}

fn python(m: &[Model]) -> String {
    let mut body = Vec::new();
    for model in m {
        let mut s = format!("@dataclass\nclass {}:\n", model.name);
        // Fields with defaults have to come after those without.
        let ordered = model.props.iter().enumerate().filter(|(_, p)| !(p.optional || p.nullable)).chain(model.props.iter().enumerate().filter(|(_, p)| p.optional || p.nullable));
        for (i, p) in ordered {
            let mut name = ident(snake(&p.key), &format!("field_{}", i + 1));
            if PY_KEYWORDS.contains(&name.as_str()) {
                name.push('_');
            }
            let ty = py_type(&p.ty, m);
            let line = if p.optional || p.nullable {
                if p.ty == Ty::Any { format!("    {name}: Any = None") } else { format!("    {name}: {ty} | None = None") }
            } else {
                format!("    {name}: {ty}")
            };
            let note = if name != p.key { format!("  # JSON key: {}", quoted(&p.key)) } else { String::new() };
            s.push_str(&format!("{line}{note}\n"));
        }
        if model.props.is_empty() {
            s.push_str("    pass\n");
        }
        body.push(s.trim_end().to_string());
    }
    let body = body.join("\n\n\n");
    let any = if body.contains("Any") { "from typing import Any\n" } else { "" };
    format!("from __future__ import annotations\n\nfrom dataclasses import dataclass\n{any}\n\n{body}")
}

// ---------------------------------------------------------------- Swift

const SWIFT_KEYWORDS: &[&str] = &[
    "associatedtype", "class", "deinit", "enum", "extension", "func", "import", "init", "inout", "internal", "let", "operator",
    "private", "protocol", "public", "static", "struct", "subscript", "typealias", "var", "break", "case", "continue", "default",
    "defer", "do", "else", "fallthrough", "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as", "catch",
    "false", "is", "nil", "rethrows", "super", "self", "throw", "throws", "true", "try",
];

fn swift_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "AnyCodable".into(),
        Ty::Bool => "Bool".into(),
        Ty::Int => "Int".into(),
        Ty::Float => "Double".into(),
        Ty::Str => "String".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => format!("{}?", swift_type(t, m)),
        Ty::List(t) => format!("[{}]", swift_type(t, m)),
    }
}

fn swift(m: &[Model]) -> String {
    let mut out = Vec::new();
    for model in m {
        let names: Vec<String> = model
            .props
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let n = ident(camel(&p.key), &format!("field{}", i + 1));
                if SWIFT_KEYWORDS.contains(&n.as_str()) { format!("`{n}`") } else { n }
            })
            .collect();
        let mut s = format!("struct {}: Codable {{\n", model.name);
        for (p, name) in model.props.iter().zip(&names) {
            let ty = swift_type(&p.ty, m);
            s.push_str(&format!("    let {name}: {ty}{}\n", if p.optional || p.nullable { "?" } else { "" }));
        }
        if model.props.iter().zip(&names).any(|(p, n)| n.trim_matches('`') != p.key) {
            s.push_str("\n    enum CodingKeys: String, CodingKey {\n");
            for (p, name) in model.props.iter().zip(&names) {
                if name.trim_matches('`') == p.key {
                    s.push_str(&format!("        case {name}\n"));
                } else {
                    s.push_str(&format!("        case {name} = {}\n", quoted(&p.key)));
                }
            }
            s.push_str("    }\n");
        }
        s.push('}');
        out.push(s);
    }
    let body = out.join("\n\n");
    if body.contains("AnyCodable") {
        format!("// AnyCodable stands for a type-erased Codable value; Foundation has none built in.\n\n{body}")
    } else {
        body
    }
}

// ---------------------------------------------------------------- Kotlin

const KOTLIN_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null", "object",
    "package", "return", "super", "this", "throw", "true", "try", "typealias", "typeof", "val", "var", "when", "while",
];

fn kotlin_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "JsonElement".into(),
        Ty::Bool => "Boolean".into(),
        Ty::Int => "Long".into(),
        Ty::Float => "Double".into(),
        Ty::Str => "String".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => format!("{}?", kotlin_type(t, m)),
        Ty::List(t) => format!("List<{}>", kotlin_type(t, m)),
    }
}

fn kotlin(m: &[Model]) -> String {
    let mut out = Vec::new();
    for model in m {
        let mut s = format!("@Serializable\ndata class {}(\n", model.name);
        for (i, p) in model.props.iter().enumerate() {
            let name = ident(camel(&p.key), &format!("field{}", i + 1));
            let rename = if name != p.key { format!("@SerialName({}) ", quoted(&p.key).replace('$', "\\$")) } else { String::new() };
            let name = if KOTLIN_KEYWORDS.contains(&name.as_str()) { format!("`{name}`") } else { name };
            let ty = kotlin_type(&p.ty, m);
            let ty = if p.optional || p.nullable { format!("{ty}? = null") } else { ty };
            s.push_str(&format!("    {rename}val {name}: {ty},\n"));
        }
        s.push(')');
        out.push(s);
    }
    let body = out.join("\n\n");
    let mut imports = String::new();
    if body.contains("@SerialName") {
        imports.push_str("import kotlinx.serialization.SerialName\n");
    }
    imports.push_str("import kotlinx.serialization.Serializable\n");
    if body.contains("JsonElement") {
        imports.push_str("import kotlinx.serialization.json.JsonElement\n");
    }
    format!("{imports}\n{body}")
}

// ---------------------------------------------------------------- C#

fn csharp_type(t: &Ty, m: &[Model]) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "JsonElement".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "long".into(),
        Ty::Float => "double".into(),
        Ty::Str => "string".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => format!("{}?", csharp_type(t, m)),
        Ty::List(t) => format!("List<{}>", csharp_type(t, m)),
    }
}

fn csharp(m: &[Model]) -> String {
    let mut out = Vec::new();
    for model in m {
        let mut props = Vec::new();
        for (i, p) in model.props.iter().enumerate() {
            let mut name = ident(pascal(&p.key), &format!("Field{}", i + 1));
            // A member cannot share its enclosing type's name.
            if name == model.name {
                name.push_str("Value");
            }
            let ty = csharp_type(&p.ty, m);
            let nullable = p.optional || p.nullable;
            let init = match &p.ty {
                _ if nullable => "",
                Ty::Str => " = \"\";",
                Ty::List(_) | Ty::Ref(_) => " = new();",
                _ => "",
            };
            props.push(format!(
                "    [JsonPropertyName({})]\n    public {ty}{} {name} {{ get; set; }}{init}",
                quoted(&p.key),
                if nullable { "?" } else { "" }
            ));
        }
        out.push(format!("public class {}\n{{\n{}\n}}", model.name, props.join("\n\n")));
    }
    let body = out.join("\n\n");
    let mut usings = String::new();
    if body.contains("List<") {
        usings.push_str("using System.Collections.Generic;\n");
    }
    if body.contains("JsonElement") {
        usings.push_str("using System.Text.Json;\n");
    }
    usings.push_str("using System.Text.Json.Serialization;\n");
    format!("{usings}\n{body}")
}

// ---------------------------------------------------------------- Java

const JAVA_KEYWORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do",
    "double", "else", "enum", "extends", "final", "finally", "float", "for", "goto", "if", "implements", "import", "instanceof",
    "int", "interface", "long", "native", "new", "package", "private", "protected", "public", "return", "short", "static",
    "strictfp", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void", "volatile", "while",
    "true", "false", "null", "record", "var",
];

/// `boxed` inside generics and wherever null is a possible value.
fn java_type(t: &Ty, m: &[Model], boxed: bool) -> String {
    match t {
        Ty::Any | Ty::Union(_) => "Object".into(),
        Ty::Bool => if boxed { "Boolean" } else { "boolean" }.into(),
        Ty::Int => if boxed { "Long" } else { "long" }.into(),
        Ty::Float => if boxed { "Double" } else { "double" }.into(),
        Ty::Str => "String".into(),
        Ty::Ref(i) => m[*i].name.clone(),
        Ty::Opt(t) => java_type(t, m, true),
        Ty::List(t) => format!("List<{}>", java_type(t, m, true)),
    }
}

fn java(m: &[Model]) -> String {
    let mut out = Vec::new();
    for (mi, model) in m.iter().enumerate() {
        let mut parts = Vec::new();
        for (i, p) in model.props.iter().enumerate() {
            let mut name = ident(camel(&p.key), &format!("field{}", i + 1));
            if JAVA_KEYWORDS.contains(&name.as_str()) {
                name.push('_');
            }
            let rename = if name != p.key { format!("@JsonProperty({}) ", quoted(&p.key)) } else { String::new() };
            parts.push(format!("    {rename}{} {name}", java_type(&p.ty, m, p.optional || p.nullable)));
        }
        // One public type per file; the rest stay package-private.
        let vis = if mi == 0 { "public " } else { "" };
        out.push(format!("{vis}record {}(\n{}\n) {{}}", model.name, parts.join(",\n")));
    }
    let body = out.join("\n\n");
    let mut imports = String::new();
    if body.contains("@JsonProperty") {
        imports.push_str("import com.fasterxml.jackson.annotation.JsonProperty;\n");
    }
    if body.contains("List<") {
        imports.push_str("import java.util.List;\n");
    }
    if imports.is_empty() { body } else { format!("{imports}\n{body}") }
}

/// Model code for the sample in `src` in `LANGS[lang]`, and the number of types.
pub fn generate(src: &str, lang: usize, root: &str) -> Result<(String, usize), String> {
    let models = build(src, if root.trim().is_empty() { "Root" } else { root })?;
    let code = match lang {
        0 => typescript(&models),
        1 => rust(&models),
        2 => go(&models),
        3 => python(&models),
        4 => swift(&models),
        5 => kotlin(&models),
        6 => csharp(&models),
        _ => java(&models),
    };
    Ok((code, models.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "id": 7,
        "user_name": "ada",
        "type": "admin",
        "score": 1.5,
        "active": true,
        "nick": null,
        "tags": ["a", "b"],
        "address": {"city": "London", "geo": {"lat": 51.5, "lng": -0.1}},
        "line_items": [{"sku": "A1", "qty": 2}, {"sku": "B2", "qty": 1, "note": "gift"}],
        "mixed": [1, "x"],
        "first-name": "Ada"
    }"#;

    fn code(lang: &str) -> String {
        generate(SAMPLE, LANGS.iter().position(|l| *l == lang).unwrap(), "Order").unwrap().0
    }

    #[test]
    fn typescript_models() {
        assert_eq!(
            code("TypeScript"),
            "export interface Order {
  id: number;
  user_name: string;
  type: string;
  score: number;
  active: boolean;
  nick: null;
  tags: string[];
  address: Address;
  line_items: LineItem[];
  mixed: (number | string)[];
  \"first-name\": string;
}

export interface Address {
  city: string;
  geo: Geo;
}

export interface Geo {
  lat: number;
  lng: number;
}

export interface LineItem {
  sku: string;
  qty: number;
  note?: string;
}"
        );
    }

    #[test]
    fn rust_models() {
        let c = code("Rust");
        assert!(c.starts_with("use serde::{Deserialize, Serialize};\n\n#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct Order {\n    pub id: i64,\n    pub user_name: String,\n    pub r#type: String,\n    pub score: f64,"), "{c}");
        assert!(c.contains("    pub nick: Option<serde_json::Value>,\n"));
        assert!(c.contains("    pub line_items: Vec<LineItem>,\n    pub mixed: Vec<serde_json::Value>,\n    #[serde(rename = \"first-name\")]\n    pub first_name: String,\n}"));
        assert!(c.contains("pub struct LineItem {\n    pub sku: String,\n    pub qty: i64,\n    pub note: Option<String>,\n}"));
    }

    #[test]
    fn go_models() {
        let c = code("Go");
        assert!(c.contains("type Order struct {\n\tID        int64      `json:\"id\"`\n\tUserName  string     `json:\"user_name\"`\n"), "{c}");
        assert!(c.contains("\tNick      any        `json:\"nick\"`\n"));
        assert!(c.contains("\tLineItems []LineItem `json:\"line_items\"`\n"));
        assert!(c.contains("type LineItem struct {\n\tSku  string  `json:\"sku\"`\n\tQty  int64   `json:\"qty\"`\n\tNote *string `json:\"note,omitempty\"`\n}"), "{c}");
    }

    #[test]
    fn python_models() {
        let c = code("Python");
        assert!(c.starts_with("from __future__ import annotations\n\nfrom dataclasses import dataclass\nfrom typing import Any\n\n\n@dataclass\nclass Order:\n    id: int\n    user_name: str\n    type: str\n"), "{c}");
        // Fields with defaults come last.
        assert!(c.contains("    first_name: str  # JSON key: \"first-name\"\n    nick: Any = None\n\n\n@dataclass\nclass Address:"), "{c}");
        assert!(c.contains("    mixed: list[int | str]\n"));
        assert!(c.ends_with("class LineItem:\n    sku: str\n    qty: int\n    note: str | None = None"));
    }

    #[test]
    fn swift_kotlin_csharp_java_models() {
        let s = code("Swift");
        assert!(s.contains("struct Order: Codable {\n    let id: Int\n    let userName: String\n    let type: String\n"), "{s}");
        assert!(s.contains("        case userName = \"user_name\"\n        case type\n"));
        assert!(s.contains("struct Geo: Codable {\n    let lat: Double\n    let lng: Double\n}"));
        assert!(s.starts_with("// AnyCodable"));

        let k = code("Kotlin");
        assert!(k.starts_with("import kotlinx.serialization.SerialName\nimport kotlinx.serialization.Serializable\nimport kotlinx.serialization.json.JsonElement\n\n@Serializable\ndata class Order(\n    val id: Long,\n    @SerialName(\"user_name\") val userName: String,\n"), "{k}");
        assert!(k.contains("    val note: String? = null,\n)"));

        let c = code("C#");
        assert!(c.contains("public class Order\n{\n    [JsonPropertyName(\"id\")]\n    public long Id { get; set; }\n\n    [JsonPropertyName(\"user_name\")]\n    public string UserName { get; set; } = \"\";"), "{c}");
        assert!(c.contains("    public List<LineItem> LineItems { get; set; } = new();"));
        assert!(c.contains("    public string? Note { get; set; }\n}"));

        let j = code("Java");
        assert!(j.starts_with("import com.fasterxml.jackson.annotation.JsonProperty;\nimport java.util.List;\n\npublic record Order(\n    long id,\n    @JsonProperty(\"user_name\") String userName,\n"), "{j}");
        assert!(j.contains("record LineItem(\n    String sku,\n    long qty,\n    String note\n) {}"));
    }

    #[test]
    fn roots_names_and_twins() {
        // A top-level array models its element; identical structures share a type.
        let (c, n) = generate(r#"[{"home": {"lat": 1, "lng": 2}, "work": {"lat": 3, "lng": 4}, "class": 1, "2fa": true, "categories": [{"id": 1}]}]"#, 0, "geo point").unwrap();
        assert_eq!(n, 3);
        assert!(c.starts_with("export interface GeoPoint {\n  home: Home;\n  work: Home;\n  class: number;\n  \"2fa\": boolean;\n  categories: Category[];\n}"), "{c}");
        let (r, _) = generate(r#"{"2fa": true, "self": 1, "": 2}"#, 1, "").unwrap();
        assert!(r.contains("pub struct Root {\n    #[serde(rename = \"2fa\")]\n    pub _2fa: bool,\n    #[serde(rename = \"self\")]\n    pub self_: i64,\n    #[serde(rename = \"\")]\n    pub field_3: i64,\n}"), "{r}");
        assert!(generate("[1, 2]", 0, "Root").is_err());
        assert!(generate("{", 0, "Root").is_err());
        let (nullable_items, _) = generate(r#"{"a": [1, null]}"#, 0, "Root").unwrap();
        assert!(nullable_items.contains("a: (number | null)[];"));
    }
}
