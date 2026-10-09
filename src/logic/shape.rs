//! The shape of JSON documents: which fields exist, what type each holds, and
//! what is optional or nullable. Shared by JSON Schema and JSON to Code.

use serde_json::Value;

/// Past this many distinct strings a position is clearly free text.
const VOCABULARY: usize = 8;

/// The strings seen at one position, kept while they could still be an enum.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strings {
    pub count: usize,
    /// Distinct values in first-seen order; cleared once there are too many.
    pub values: Vec<String>,
    pub overflow: bool,
    /// The format every value had, or `None` once two disagree.
    pub format: Option<&'static str>,
}

impl Strings {
    fn one(s: &str) -> Self {
        Self { count: 1, values: vec![s.to_string()], overflow: false, format: string_format(s) }
    }

    fn merge(&mut self, other: Strings) {
        if self.format != other.format {
            self.format = None;
        }
        self.count += other.count;
        self.overflow |= other.overflow;
        for v in other.values {
            if !self.overflow && !self.values.contains(&v) {
                self.values.push(v);
                self.overflow = self.values.len() > VOCABULARY;
            }
        }
        if self.overflow {
            self.values.clear();
        }
    }

    /// The values, when they repeat enough to read as a fixed vocabulary.
    pub fn vocabulary(&self) -> Option<&[String]> {
        let n = self.values.len();
        (!self.overflow && self.format.is_none() && self.count >= 4 && n <= 5 && n * 2 <= self.count).then_some(&self.values[..])
    }
}

/// The JSON Schema `format` a string plainly has.
pub fn string_format(s: &str) -> Option<&'static str> {
    if s.len() < 3 || s.len() > 2048 {
        return None;
    }
    let b = s.as_bytes();
    if b[0].is_ascii_digit() && s.len() >= 10 && b[4] == b'-' {
        if s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
            return Some("date");
        }
        if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
            return Some("date-time");
        }
    }
    if s.len() == 36 && [8, 13, 18, 23].iter().all(|i| b[*i] == b'-') && s.chars().all(|c| c == '-' || c.is_ascii_hexdigit()) {
        return Some("uuid");
    }
    if s.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some("ipv4");
    }
    if s.contains(':') && s.parse::<std::net::Ipv6Addr>().is_ok() {
        return Some("ipv6");
    }
    if let Some((scheme, rest)) = s.split_once("://") {
        let ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        if ok && !rest.is_empty() && !s.contains(char::is_whitespace) {
            return Some("uri");
        }
    }
    if let Some((user, host)) = s.split_once('@') {
        let plain = |p: &str| !p.is_empty() && !p.contains(|c: char| c.is_whitespace() || c == '@');
        if plain(user) && plain(host) && host.contains('.') && !host.starts_with('.') && !host.ends_with('.') {
            return Some("email");
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Str(Strings),
    Array(Box<Shape>),
    Object(Vec<Field>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub shape: Shape,
    /// Missing from at least one of the objects merged here.
    pub optional: bool,
}

/// Everything seen at one position. No kinds and not nullable means nothing
/// was seen at all, as for the items of an empty array.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub kinds: Vec<Kind>,
    pub nullable: bool,
}

impl Shape {
    pub fn of(v: &Value) -> Shape {
        let kind = match v {
            Value::Null => return Shape { kinds: Vec::new(), nullable: true },
            Value::Bool(_) => Kind::Bool,
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    Kind::Int
                } else {
                    Kind::Float
                }
            }
            Value::String(s) => Kind::Str(Strings::one(s)),
            Value::Array(a) => {
                // Every element counts, not just the first.
                let mut items = Shape::default();
                for x in a {
                    items.merge(Shape::of(x));
                }
                Kind::Array(Box::new(items))
            }
            Value::Object(m) => Kind::Object(m.iter().map(|(k, x)| Field { name: k.clone(), shape: Shape::of(x), optional: false }).collect()),
        };
        Shape { kinds: vec![kind], nullable: false }
    }

    pub fn merge(&mut self, other: Shape) {
        self.nullable |= other.nullable;
        for kind in other.kinds {
            self.merge_kind(kind);
        }
    }

    fn merge_kind(&mut self, kind: Kind) {
        let same = |k: &Kind| std::mem::discriminant(k) == std::mem::discriminant(&kind);
        match self.kinds.iter().position(same) {
            Some(i) => match (&mut self.kinds[i], kind) {
                (Kind::Str(a), Kind::Str(b)) => a.merge(b),
                (Kind::Array(a), Kind::Array(b)) => a.merge(*b),
                (Kind::Object(a), Kind::Object(b)) => merge_fields(a, b),
                _ => {}
            },
            // An integer among floats is just a number.
            None => match kind {
                Kind::Int if self.kinds.contains(&Kind::Float) => {}
                Kind::Float => match self.kinds.iter().position(|k| *k == Kind::Int) {
                    Some(i) => self.kinds[i] = Kind::Float,
                    None => self.kinds.push(Kind::Float),
                },
                other => self.kinds.push(other),
            },
        }
    }
}

fn merge_fields(mine: &mut Vec<Field>, theirs: Vec<Field>) {
    for f in mine.iter_mut() {
        if !theirs.iter().any(|t| t.name == f.name) {
            f.optional = true;
        }
    }
    for t in theirs {
        match mine.iter_mut().find(|f| f.name == t.name) {
            Some(f) => {
                f.optional |= t.optional;
                f.shape.merge(t.shape);
            }
            None => mine.push(Field { optional: true, ..t }),
        }
    }
}

/// One document, or several one after another (JSON Lines included), as a
/// single merged shape and the number of documents read.
pub fn of_documents(src: &str) -> Result<(Shape, usize), String> {
    let mut shape = Shape::default();
    let mut n = 0;
    for doc in serde_json::Deserializer::from_str(src).into_iter::<Value>() {
        shape.merge(Shape::of(&doc.map_err(|e| super::json_err(&e))?));
        n += 1;
    }
    if n == 0 {
        return Err("Paste a JSON document".into());
    }
    Ok((shape, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field<'a>(s: &'a Shape, name: &str) -> &'a Field {
        let Kind::Object(fields) = &s.kinds[0] else { panic!("not an object") };
        fields.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn arrays_merge_every_element() {
        let s = Shape::of(&json!([{"id": 1, "tag": "a"}, {"id": 2.5, "note": null}, {"id": 3, "tag": null}]));
        let Kind::Array(items) = &s.kinds[0] else { panic!() };
        assert_eq!(field(items, "id").shape.kinds, [Kind::Float]);
        assert!(!field(items, "id").optional);
        let tag = field(items, "tag");
        assert!(tag.optional && tag.shape.nullable);
        let note = field(items, "note");
        assert!(note.optional && note.shape.nullable && note.shape.kinds.is_empty());
        assert!(Shape::of(&json!([])).kinds[0] == Kind::Array(Box::new(Shape::default())));
    }

    #[test]
    fn mixed_kinds_are_kept_apart() {
        let s = Shape::of(&json!([1, "a", [true], {"x": 1}, {"y": 2}]));
        let Kind::Array(items) = &s.kinds[0] else { panic!() };
        assert_eq!(items.kinds.len(), 4);
        assert!(matches!(&items.kinds[3], Kind::Object(f) if f.len() == 2 && f.iter().all(|f| f.optional)));
    }

    #[test]
    fn vocabularies_and_formats() {
        let s = Shape::of(&json!(["open", "closed", "open", "open", "closed"]));
        let Kind::Array(items) = &s.kinds[0] else { panic!() };
        let Kind::Str(strings) = &items.kinds[0] else { panic!() };
        assert_eq!(strings.vocabulary().unwrap(), ["open", "closed"]);
        let names = Shape::of(&json!(["Ada", "Alan", "Grace", "Edsger"]));
        let Kind::Array(items) = &names.kinds[0] else { panic!() };
        let Kind::Str(strings) = &items.kinds[0] else { panic!() };
        assert!(strings.vocabulary().is_none());

        assert_eq!(string_format("2026-10-01"), Some("date"));
        assert_eq!(string_format("2026-10-01T12:30:00Z"), Some("date-time"));
        assert_eq!(string_format("2026-13-01"), None);
        assert_eq!(string_format("3f2504e0-4f89-11d3-9a0c-0305e82c3301"), Some("uuid"));
        assert_eq!(string_format("ada@example.com"), Some("email"));
        assert_eq!(string_format("https://example.com/a"), Some("uri"));
        assert_eq!(string_format("10.0.0.1"), Some("ipv4"));
        assert_eq!(string_format("::1"), Some("ipv6"));
        assert_eq!(string_format("hello world"), None);
        assert_eq!(string_format("a@b"), None);
    }

    #[test]
    fn several_documents() {
        let (s, n) = of_documents("{\"a\": 1}\n{\"a\": 2, \"b\": true}").unwrap();
        assert_eq!(n, 2);
        assert!(field(&s, "b").optional && !field(&s, "a").optional);
        assert!(of_documents("  ").is_err());
        assert!(of_documents("{\"a\": }").is_err());
    }
}
