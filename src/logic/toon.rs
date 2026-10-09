//! JSON → TOON (Token-Oriented Object Notation): indentation instead of
//! braces, and one header row for arrays of uniform records.

use serde_json::{Map, Value};

pub const DELIMITERS: &[&str] = &["Comma", "Tab", "Pipe"];

struct Encoder {
    delim: char,
    lines: Vec<String>,
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn number(n: &serde_json::Number) -> String {
    match n.as_f64() {
        // No exponents, and no fraction on whole numbers.
        Some(f) if n.is_f64() => {
            if f == 0. {
                "0".into()
            } else {
                format!("{f}")
            }
        }
        _ => n.to_string(),
    }
}

fn looks_numeric(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = match mantissa.split_once('.') {
        Some((a, b)) => digits(a) && digits(b),
        None => digits(mantissa),
    };
    mantissa_ok && exp.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)))
}

impl Encoder {
    /// A string as a value: bare when nothing else could be read into it.
    fn string(&self, s: &str) -> String {
        let bare = !s.is_empty()
            && s.trim() == s
            && !matches!(s, "true" | "false" | "null")
            && !looks_numeric(s)
            && !s.starts_with('-')
            && !s.contains(|c: char| c == self.delim || c.is_control() || ":\"\\[]{}".contains(c));
        if bare { s.to_string() } else { quote(s) }
    }

    fn primitive(&self, v: &Value) -> Option<String> {
        Some(match v {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => number(n),
            Value::String(s) => self.string(s),
            _ => return None,
        })
    }

    fn join(&self, cells: Vec<String>) -> String {
        cells.join(&self.delim.to_string())
    }

    /// `key: value`, a nested object or an array, starting on `start`.
    fn field(&mut self, k: &str, v: &Value, depth: usize, start: String) {
        match v {
            Value::Object(m) => {
                self.lines.push(format!("{start}{k}:"));
                self.object(m, depth + 1);
            }
            Value::Array(a) => self.array(k, a, depth, start),
            _ => {
                let text = self.primitive(v).unwrap_or_default();
                self.lines.push(format!("{start}{k}: {text}"));
            }
        }
    }

    fn object(&mut self, m: &Map<String, Value>, depth: usize) {
        for (k, v) in m {
            self.field(&key(k), v, depth, indent(depth));
        }
    }

    fn array(&mut self, k: &str, a: &[Value], depth: usize, start: String) {
        // A delimiter other than the comma is announced in the header.
        let mark = if self.delim == ',' { String::new() } else { self.delim.to_string() };
        let head = format!("{start}{k}[{}{mark}]", a.len());
        if let Some(cells) = a.iter().map(|x| self.primitive(x)).collect::<Option<Vec<_>>>() {
            let row = self.join(cells);
            self.lines.push(if row.is_empty() { format!("{head}:") } else { format!("{head}: {row}") });
        } else if let Some(columns) = self.columns(a) {
            let names = self.join(columns.iter().map(|c| key(c)).collect());
            self.lines.push(format!("{head}{{{names}}}:"));
            for row in a {
                let cells = columns.iter().map(|c| self.primitive(&row[c.as_str()]).unwrap_or_default()).collect();
                let line = format!("{}{}", indent(depth + 1), self.join(cells));
                self.lines.push(line);
            }
        } else {
            self.lines.push(format!("{head}:"));
            for item in a {
                self.item(item, depth + 1);
            }
        }
    }

    /// The shared keys, when every element is an object with exactly the same
    /// keys and only primitive values: those arrays become tables.
    fn columns<'a>(&self, a: &'a [Value]) -> Option<Vec<&'a String>> {
        let first = a.first()?.as_object()?;
        let columns: Vec<&String> = first.keys().collect();
        let uniform = !columns.is_empty()
            && a.iter().all(|x| {
                x.as_object().is_some_and(|m| m.len() == columns.len() && columns.iter().all(|c| m.get(*c).is_some_and(|v| !v.is_object() && !v.is_array())))
            });
        uniform.then_some(columns)
    }

    fn item(&mut self, v: &Value, depth: usize) {
        let dash = format!("{}- ", indent(depth));
        match v {
            Value::Array(a) => self.array("", a, depth, dash),
            Value::Object(m) => {
                let mut fields = m.iter();
                match fields.next() {
                    None => self.lines.push(format!("{}-", indent(depth))),
                    // The first field shares the dash's line; the rest line up under it.
                    Some((k, x)) => {
                        self.field(&key(k), x, depth + 1, dash);
                        for (k, x) in fields {
                            self.field(&key(k), x, depth + 1, indent(depth + 1));
                        }
                    }
                }
            }
            _ => {
                let text = self.primitive(v).unwrap_or_default();
                self.lines.push(format!("{dash}{text}"));
            }
        }
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn key(k: &str) -> String {
    let bare = k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if bare { k.to_string() } else { quote(k) }
}

/// `delimiter` indexes `DELIMITERS`.
pub fn encode(v: &Value, delimiter: usize) -> String {
    let mut e = Encoder { delim: [',', '\t', '|'][delimiter.min(2)], lines: Vec::new() };
    match v {
        Value::Object(m) => e.object(m, 0),
        Value::Array(a) => e.array("", a, 0, String::new()),
        _ => return e.primitive(v).unwrap_or_default(),
    }
    e.lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_and_primitive_arrays() {
        let v = json!({"id": 123, "name": "Ada", "active": true, "note": null, "user": {"id": 1, "empty": {}}, "tags": ["admin", "ops"], "none": []});
        assert_eq!(encode(&v, 0), "id: 123\nname: Ada\nactive: true\nnote: null\nuser:\n  id: 1\n  empty:\ntags[2]: admin,ops\nnone[0]:");
    }

    #[test]
    fn uniform_objects_become_tables() {
        let v = json!({"items": [{"sku": "A1", "qty": 2, "price": 9.99}, {"sku": "B2", "qty": 1, "price": 14.5}]});
        assert_eq!(encode(&v, 0), "items[2]{sku,qty,price}:\n  A1,2,9.99\n  B2,1,14.5");
        assert_eq!(encode(&v, 2), "items[2|]{sku|qty|price}:\n  A1|2|9.99\n  B2|1|14.5");
        assert_eq!(encode(&v, 1), "items[2\t]{sku\tqty\tprice}:\n  A1\t2\t9.99\n  B2\t1\t14.5");
        assert_eq!(encode(&json!([{"a": 1}, {"a": 2}]), 0), "[2]{a}:\n  1\n  2");
    }

    #[test]
    fn mixed_arrays_become_lists() {
        let v = json!({"items": [1, {"a": 1, "b": {"c": true}, "d": [1, 2]}, "text", [1, 2], {}, [{"x": 1}, {"x": 2}]]});
        assert_eq!(
            encode(&v, 0),
            "items[6]:\n  - 1\n  - a: 1\n    b:\n      c: true\n    d[2]: 1,2\n  - text\n  - [2]: 1,2\n  -\n  - [2]{x}:\n    1\n    2"
        );
        // Records that differ in their keys cannot share a header row.
        let ragged = json!([{"id": 1, "name": "a"}, {"id": 2}]);
        assert_eq!(encode(&ragged, 0), "[2]:\n  - id: 1\n    name: a\n  - id: 2");
        let nested_first = json!([{"rows": [{"a": 1}, {"a": 2}], "n": 2}, 1]);
        assert_eq!(encode(&nested_first, 0), "[2]:\n  - rows[2]{a}:\n      1\n      2\n    n: 2\n  - 1");
    }

    #[test]
    fn quoting() {
        let v = json!({
            "a": "", "b": " pad", "c": "true", "d": "42", "e": "-x", "f": "a,b", "g": "a:b", "h": "say \"hi\"\n", "i": "plain text",
            "j": "1e5", "k": "05", "l": "[x]", "m": "a|b", "bad key": 1, "ok.key_2": 2
        });
        assert_eq!(
            encode(&v, 0),
            "a: \"\"\nb: \" pad\"\nc: \"true\"\nd: \"42\"\ne: \"-x\"\nf: \"a,b\"\ng: \"a:b\"\nh: \"say \\\"hi\\\"\\n\"\ni: plain text\nj: \"1e5\"\nk: \"05\"\nl: \"[x]\"\nm: a|b\n\"bad key\": 1\nok.key_2: 2"
        );
        // Only the delimiter in use forces quotes.
        assert_eq!(encode(&json!(["a,b", "a|b"]), 2), "[2|]: a,b|\"a|b\"");
    }

    #[test]
    fn numbers_and_roots() {
        assert_eq!(encode(&json!({"a": 1.0, "b": 1e21, "c": -0.0, "d": 1.5e-7, "e": 18446744073709551615u64}), 0), "a: 1\nb: 1000000000000000000000\nc: 0\nd: 0.00000015\ne: 18446744073709551615");
        assert_eq!(encode(&json!("x"), 0), "x");
        assert_eq!(encode(&json!({}), 0), "");
        assert_eq!(encode(&json!([]), 0), "[0]:");
    }
}
