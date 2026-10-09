//! cURL commands → request code. The command is only ever read: nothing is
//! executed and nothing is sent.

use base64::Engine as _;
use serde_json::Value;

use super::http::{self, Request};

pub const TARGETS: &[&str] = &["JavaScript (fetch)", "Python (requests)", "Go (net/http)", "Rust (reqwest)", "PHP (cURL)", "Swift (URLSession)", ".http file"];

/// The editor grammar for each of `TARGETS`.
pub const GRAMMARS: &[&str] = &["javascript", "python", "go", "rust", "php", "swift", "plaintext"];

#[derive(Debug, Default, PartialEq)]
pub struct Curl {
    pub request: Request,
    /// `--max-time`, in seconds.
    pub timeout: Option<f64>,
    pub connect_timeout: Option<f64>,
    /// Options that were dropped or could not be carried over.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- shell words

/// Split a command line the way a POSIX shell would, stopping at the first
/// unquoted `|`, `;`, `>` or `<`. The flag says whether anything was cut off.
fn words(src: &str) -> Result<(Vec<String>, bool), String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            // Line continuations: POSIX `\`, cmd.exe `^` and PowerShell `` ` ``.
            '\\' | '^' | '`' if next == Some('\n') => i += 1,
            '\\' | '^' | '`' if next == Some('\r') && chars.get(i + 2) == Some(&'\n') => i += 2,
            '\\' => {
                if let Some(n) = next {
                    word.get_or_insert_default().push(n);
                    i += 1;
                }
            }
            '\'' => {
                let end = chars[i + 1..].iter().position(|c| *c == '\'').ok_or("A single quote (') is never closed")?;
                word.get_or_insert_default().extend(&chars[i + 1..i + 1 + end]);
                i += end + 1;
            }
            '$' if next == Some('\'') => {
                let w = word.get_or_insert_default();
                i += 2;
                loop {
                    match chars.get(i).copied() {
                        None => return Err("A $'…' string is never closed".into()),
                        Some('\'') => break,
                        Some('\\') => {
                            i += 1;
                            let hex = |from: usize, n: usize| chars.get(from..from + n).and_then(|h| u32::from_str_radix(&h.iter().collect::<String>(), 16).ok());
                            match chars.get(i).copied() {
                                Some('n') => w.push('\n'),
                                Some('t') => w.push('\t'),
                                Some('r') => w.push('\r'),
                                Some('0') => w.push('\0'),
                                Some('x') => {
                                    w.push(hex(i + 1, 2).and_then(char::from_u32).ok_or("Bad \\x escape in a $'…' string")?);
                                    i += 2;
                                }
                                Some('u') => {
                                    w.push(hex(i + 1, 4).and_then(char::from_u32).ok_or("Bad \\u escape in a $'…' string")?);
                                    i += 4;
                                }
                                Some(other) => w.push(other),
                                None => return Err("A $'…' string is never closed".into()),
                            }
                        }
                        Some(other) => w.push(other),
                    }
                    i += 1;
                }
            }
            '"' => {
                let w = word.get_or_insert_default();
                i += 1;
                loop {
                    match chars.get(i).copied() {
                        None => return Err("A double quote (\") is never closed".into()),
                        Some('"') => break,
                        Some('\\') if matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`')) => {
                            w.push(chars[i + 1]);
                            i += 1;
                        }
                        Some('\\') if chars.get(i + 1) == Some(&'\n') => i += 1,
                        Some(other) => w.push(other),
                    }
                    i += 1;
                }
            }
            c if c.is_whitespace() => out.extend(word.take()),
            // Only at the start of a word: URLs are often pasted with a bare `&` or `;` inside.
            '|' | ';' | '>' | '<' | '&' if word.is_none() => {
                return Ok((out, true));
            }
            '#' if word.is_none() => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            c => word.get_or_insert_default().push(c),
        }
        i += 1;
    }
    out.extend(word);
    Ok((out, false))
}

// ---------------------------------------------------------------- options

/// Short options that take a value, which may be glued on (`-XPOST`).
const SHORT_WITH_VALUE: &str = "XHdubAemowxETFcDrUKzC";

fn long_name(short: char) -> Option<&'static str> {
    Some(match short {
        'X' => "request",
        'H' => "header",
        'd' => "data",
        'u' => "user",
        'b' => "cookie",
        'A' => "user-agent",
        'e' => "referer",
        'm' => "max-time",
        'G' => "get",
        'I' => "head",
        'r' => "range",
        'k' => "insecure",
        'F' => "form",
        'x' => "proxy",
        'E' => "cert",
        'T' => "upload-file",
        'U' => "proxy-user",
        'K' => "config",
        'o' => "output",
        'w' => "write-out",
        'c' => "cookie-jar",
        'D' => "dump-header",
        'z' => "time-cond",
        'C' => "continue-at",
        _ => return None,
    })
}

/// Long options that take a value and change nothing about the request itself.
const QUIET_WITH_VALUE: &[&str] = &[
    "output", "write-out", "retry", "retry-delay", "retry-max-time", "max-redirs", "dump-header", "trace", "trace-ascii", "stderr",
    "cookie-jar", "limit-rate", "keepalive-time", "continue-at", "max-filesize", "speed-limit", "speed-time",
];

/// Long options that take a value and cannot be expressed in generated code.
const UNSUPPORTED_WITH_VALUE: &[&str] = &[
    "form", "form-string", "proxy", "cert", "key", "cacert", "capath", "resolve", "connect-to", "interface", "upload-file",
    "proxy-user", "unix-socket", "ciphers", "config", "aws-sigv4", "noproxy", "time-cond", "dns-servers", "pinnedpubkey", "cert-type",
    "key-type", "pass", "proxy-header", "socks5", "socks5-hostname", "preproxy", "local-port", "tls-max", "abstract-unix-socket",
];

/// Percent-encode everything but RFC 3986 unreserved characters, as curl does.
fn curl_escape(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn seconds(v: &str, option: &str) -> Result<f64, String> {
    v.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.).ok_or(format!("--{option} needs a number of seconds, not \"{v}\""))
}

pub fn parse(src: &str) -> Result<Curl, String> {
    let (mut tokens, cut) = words(src)?;
    // A copied prompt (`$ curl …`) is not part of the command.
    if tokens.first().is_some_and(|t| t == "$" || t == "%" || t == ">") {
        tokens.remove(0);
    }
    // Also `/usr/bin/curl`, and `C:\tools\curl.exe` once the shell has eaten its backslashes.
    let is_curl = |t: &str| {
        let name = t.to_lowercase();
        name.ends_with("curl") || name.ends_with("curl.exe")
    };
    if !tokens.first().is_some_and(|t| is_curl(t)) {
        return Err("Paste a command that starts with curl".into());
    }

    let mut c = Curl::default();
    let mut method: Option<String> = None;
    let mut urls: Vec<String> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let (mut json, mut get, mut head) = (false, false, false);
    if cut {
        c.warnings.push("Only the curl command is converted; the pipe or redirect after it was left out".into());
    }

    let mut it = tokens.into_iter().skip(1);
    while let Some(tok) = it.next() {
        // Collect (option name, value glued to it) pairs for this token.
        let mut opts: Vec<(String, Option<String>)> = Vec::new();
        if let Some(long) = tok.strip_prefix("--").filter(|l| !l.is_empty()) {
            match long.split_once('=') {
                Some((n, v)) => opts.push((n.to_string(), Some(v.to_string()))),
                None => opts.push((long.to_string(), None)),
            }
        } else if tok.len() > 1 && tok.starts_with('-') {
            let cluster: Vec<char> = tok[1..].chars().collect();
            for (i, ch) in cluster.iter().enumerate() {
                let name = long_name(*ch).map(str::to_string).unwrap_or_else(|| format!("-{ch}"));
                if SHORT_WITH_VALUE.contains(*ch) {
                    let rest: String = cluster[i + 1..].iter().collect();
                    opts.push((name, Some(rest).filter(|r| !r.is_empty())));
                    break;
                }
                opts.push((name, None));
            }
        } else {
            urls.push(tok);
            continue;
        }

        for (name, glued) in opts {
            let name = name.as_str();
            let mut value = || -> Result<String, String> {
                match glued.clone().or_else(|| it.next()) {
                    Some(v) => Ok(v),
                    None => Err(format!("--{name} needs a value")),
                }
            };
            match name {
                "request" => method = Some(value()?.to_uppercase()),
                "header" => {
                    let h = value()?;
                    match h.split_once(':') {
                        Some((k, v)) if v.trim().is_empty() => {
                            c.warnings.push(format!("-H '{}:' removes one of curl's own headers, which generated code never sends", k.trim()));
                        }
                        Some((k, v)) => c.request.headers.push((k.trim().to_string(), v.trim().to_string())),
                        // `Name;` is how curl spells a header with an empty value.
                        None => match h.strip_suffix(';') {
                            Some(k) => c.request.headers.push((k.trim().to_string(), String::new())),
                            None => c.warnings.push(format!("-H '{h}' is not a header and was left out")),
                        },
                    }
                }
                "data" | "data-ascii" | "data-binary" => {
                    let v = value()?;
                    match v.strip_prefix('@') {
                        Some(file) => {
                            c.warnings.push(format!("--{name} @{file} reads the body from a file; the file name stands in for its contents"));
                            data.push(format!("<contents of {file}>"));
                        }
                        None => data.push(v),
                    }
                }
                "data-raw" => data.push(value()?),
                "json" => {
                    json = true;
                    data.push(value()?);
                }
                "data-urlencode" => {
                    let v = value()?;
                    match v.split_once('=') {
                        Some(("", content)) => data.push(curl_escape(content)),
                        Some((k, content)) => data.push(format!("{k}={}", curl_escape(content))),
                        None if v.contains('@') => c.warnings.push(format!("--data-urlencode {v} reads a file and was left out")),
                        None => data.push(curl_escape(&v)),
                    }
                }
                "user" => {
                    let v = value()?;
                    if !v.contains(':') {
                        c.warnings.push("-u has no password (curl would ask for one); an empty password is used".into());
                    }
                    let pair = if v.contains(':') { v } else { format!("{v}:") };
                    let token = base64::engine::general_purpose::STANDARD.encode(pair);
                    c.request.headers.push(("Authorization".into(), format!("Basic {token}")));
                }
                "oauth2-bearer" => c.request.headers.push(("Authorization".into(), format!("Bearer {}", value()?))),
                "cookie" => {
                    let v = value()?;
                    if v.contains('=') {
                        c.request.headers.push(("Cookie".into(), v));
                    } else {
                        c.warnings.push(format!("-b {v} reads cookies from a file and was left out"));
                    }
                }
                "user-agent" => c.request.headers.push(("User-Agent".into(), value()?)),
                "referer" => c.request.headers.push(("Referer".into(), value()?)),
                "range" => c.request.headers.push(("Range".into(), format!("bytes={}", value()?))),
                "url" => urls.push(value()?),
                "max-time" => c.timeout = Some(seconds(&value()?, name)?),
                "connect-timeout" => c.connect_timeout = Some(seconds(&value()?, name)?),
                "get" => get = true,
                "head" => head = true,
                "insecure" => c.warnings.push("-k / --insecure is not carried over: the generated code still verifies TLS certificates".into()),
                "anyauth" | "digest" | "ntlm" | "negotiate" => c.warnings.push(format!("--{name} authentication is not carried over")),
                _ if QUIET_WITH_VALUE.contains(&name) => {
                    value()?;
                }
                _ if UNSUPPORTED_WITH_VALUE.contains(&name) => {
                    let v = value()?;
                    c.warnings.push(format!("--{name} {v} is not supported and was left out"));
                }
                // Everything else is taken as a flag that only changes what curl prints.
                _ => {}
            }
        }
    }

    let mut urls = urls.into_iter();
    let mut url = urls.next().ok_or("The command has no URL")?;
    let extra = urls.count();
    if extra > 0 {
        c.warnings.push(format!("Only the first URL is converted; {extra} more {} left out", if extra == 1 { "was" } else { "were" }));
    }
    if !url.contains("://") {
        url = format!("http://{url}");
    }

    let body = (!data.is_empty()).then(|| data.join(if json { "" } else { "&" }));
    let has = |c: &Curl, name: &str| c.request.header(name).is_some();
    match body {
        // -G moves the data into the query string.
        Some(b) if get => {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&b);
        }
        Some(b) => {
            // curl adds these itself; code that sets a body has to say so.
            if json {
                if !has(&c, "content-type") {
                    c.request.headers.push(("Content-Type".into(), "application/json".into()));
                }
                if !has(&c, "accept") {
                    c.request.headers.push(("Accept".into(), "application/json".into()));
                }
            } else if !has(&c, "content-type") {
                c.request.headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
            }
            c.request.body = Some(b);
        }
        None => {}
    }
    c.request.method = method.unwrap_or_else(|| {
        if head {
            "HEAD"
        } else if c.request.body.is_some() {
            "POST"
        } else {
            "GET"
        }
        .to_string()
    });
    c.request.url = url;
    Ok(c)
}

// ---------------------------------------------------------------- generators

fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The body as JSON, when the request says it is JSON and it parses.
fn json_body(r: &Request) -> Option<Value> {
    let kind = r.header("content-type")?.to_lowercase();
    if !kind.contains("json") {
        return None;
    }
    serde_json::from_str(r.body.as_deref()?).ok()
}

fn indent(text: &str, by: &str) -> String {
    text.lines().enumerate().map(|(i, l)| if i == 0 || l.is_empty() { l.to_string() } else { format!("{by}{l}") }).collect::<Vec<_>>().join("\n")
}

fn number(secs: f64) -> String {
    if secs.fract() == 0. { format!("{}", secs as u64) } else { format!("{secs}") }
}

fn javascript(c: &Curl) -> String {
    let r = &c.request;
    let mut opts = Vec::new();
    if r.method != "GET" {
        opts.push(format!("method: {}", quoted(&r.method)));
    }
    if !r.headers.is_empty() {
        let repeated = r.headers.iter().enumerate().any(|(i, (k, _))| r.headers[..i].iter().any(|(p, _)| p.eq_ignore_ascii_case(k)));
        // An object cannot hold one name twice; a list of pairs can.
        let rows: Vec<String> = r
            .headers
            .iter()
            .map(|(k, v)| if repeated { format!("    [{}, {}]", quoted(k), quoted(v)) } else { format!("    {}: {}", quoted(k), quoted(v)) })
            .collect();
        let (open, close) = if repeated { ("[", "]") } else { ("{", "}") };
        opts.push(format!("headers: {open}\n{}\n  {close}", rows.join(",\n")));
    }
    if let Some(body) = &r.body {
        opts.push(match json_body(r) {
            Some(v) => format!("body: JSON.stringify({})", indent(&super::to_json(&v, &super::Indent::Spaces(2)), "  ")),
            None => format!("body: {}", quoted(body)),
        });
    }
    if let Some(t) = c.timeout {
        opts.push(format!("signal: AbortSignal.timeout({})", (t * 1000.).round() as u64));
    }
    let call = if opts.is_empty() {
        format!("const response = await fetch({});", quoted(&r.url))
    } else {
        format!("const response = await fetch({}, {{\n  {}\n}});", quoted(&r.url), opts.join(",\n  "))
    };
    format!("{call}\n\nconsole.log(response.status);\nconsole.log(await response.text());")
}

fn py_literal(v: &Value, depth: usize) -> String {
    let pad = "    ".repeat(depth + 1);
    let close = "    ".repeat(depth);
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quoted(s),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Object(m) if m.is_empty() => "{}".into(),
        Value::Array(a) => format!("[\n{}{close}]", a.iter().map(|x| format!("{pad}{},\n", py_literal(x, depth + 1))).collect::<String>()),
        Value::Object(m) => format!("{{\n{}{close}}}", m.iter().map(|(k, x)| format!("{pad}{}: {},\n", quoted(k), py_literal(x, depth + 1))).collect::<String>()),
    }
}

fn python(c: &Curl) -> String {
    let r = &c.request;
    let verb = r.method.to_lowercase();
    let known = ["get", "post", "put", "patch", "delete", "head", "options"].contains(&verb.as_str());
    let mut args = Vec::new();
    if !known {
        args.push(quoted(&r.method));
    }
    args.push(quoted(&r.url));
    if !r.headers.is_empty() {
        // A dict holds each name once: repeats are folded into one comma-separated value.
        let mut merged: Vec<(String, String)> = Vec::new();
        for (k, v) in &r.headers {
            match merged.iter_mut().find(|(p, _)| p.eq_ignore_ascii_case(k)) {
                Some((_, old)) => *old = format!("{old}, {v}"),
                None => merged.push((k.clone(), v.clone())),
            }
        }
        let rows: String = merged.iter().map(|(k, v)| format!("        {}: {},\n", quoted(k), quoted(v))).collect();
        args.push(format!("headers={{\n{rows}    }}"));
    }
    if let Some(body) = &r.body {
        args.push(match json_body(r) {
            Some(v) => format!("json={}", py_literal(&v, 1)),
            None => format!("data={}", quoted(body)),
        });
    }
    match (c.connect_timeout, c.timeout) {
        (Some(a), Some(b)) => args.push(format!("timeout=({}, {})", number(a), number(b))),
        (None, Some(b)) => args.push(format!("timeout={}", number(b))),
        (Some(a), None) => args.push(format!("timeout=({}, None)", number(a))),
        (None, None) => {}
    }
    let func = if known { format!("requests.{verb}") } else { "requests.request".to_string() };
    let call = if args.len() == 1 {
        format!("response = {func}({})", args[0])
    } else {
        format!("response = {func}(\n{})", args.iter().map(|a| format!("    {a},\n")).collect::<String>())
    };
    format!("import requests\n\n{call}\n\nprint(response.status_code)\nprint(response.text)")
}

fn go_string(s: &str) -> String {
    let raw_ok = !s.contains('`') && !s.contains('\r') && (s.contains('\n') || s.contains('"') || s.contains('\\'));
    if raw_ok { format!("`{s}`") } else { quoted(s) }
}

fn go(c: &Curl) -> String {
    let r = &c.request;
    let mut imports = vec!["fmt", "io", "net/http"];
    let mut code = String::new();
    let body = match &r.body {
        Some(b) => {
            imports.push("strings");
            code.push_str(&format!("\tbody := strings.NewReader({})\n", go_string(b)));
            "body"
        }
        None => "nil",
    };
    code.push_str(&format!("\treq, err := http.NewRequest({}, {}, {body})\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n", quoted(&r.method), quoted(&r.url)));
    for (k, v) in &r.headers {
        code.push_str(&format!("\treq.Header.Add({}, {})\n", quoted(k), quoted(v)));
    }
    let client = match c.timeout {
        Some(t) => {
            imports.push("time");
            let d = if t.fract() == 0. { format!("{} * time.Second", t as u64) } else { format!("{} * time.Millisecond", (t * 1000.).round() as u64) };
            code.push_str(&format!("\n\tclient := &http.Client{{Timeout: {d}}}\n"));
            "client"
        }
        None => {
            code.push('\n');
            "http.DefaultClient"
        }
    };
    code.push_str(&format!(
        "\tresp, err := {client}.Do(req)\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n\tdefer resp.Body.Close()\n\n\tdata, err := io.ReadAll(resp.Body)\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n\tfmt.Println(resp.StatusCode, string(data))\n"
    ));
    imports.sort();
    let imports: String = imports.iter().map(|i| format!("\t\"{i}\"\n")).collect();
    format!("package main\n\nimport (\n{imports})\n\nfunc main() {{\n{code}}}")
}

/// A Rust or Swift string literal: both spell other escapes as `\u{…}`.
fn brace_escaped(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn rust_string(s: &str) -> String {
    let plain = !s.contains(['"', '\\', '\n']) || s.chars().any(|c| c.is_control() && c != '\n');
    if plain {
        return brace_escaped(s);
    }
    // A raw string needs more #s than any `"###` run inside it.
    let mut hashes = 1;
    while s.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let h = "#".repeat(hashes);
    format!("r{h}\"{s}\"{h}")
}

fn rust(c: &Curl) -> String {
    let r = &c.request;
    let verb = r.method.to_lowercase();
    let start = if ["get", "post", "put", "patch", "delete", "head"].contains(&verb.as_str()) {
        format!(".{verb}({})", quoted(&r.url))
    } else {
        format!(".request(reqwest::Method::from_bytes(b{})?, {})", quoted(&r.method), quoted(&r.url))
    };
    let mut chain = vec![start];
    chain.extend(r.headers.iter().map(|(k, v)| format!(".header({}, {})", rust_string(k), rust_string(v))));
    if let Some(b) = &r.body {
        chain.push(format!(".body({})", rust_string(b)));
    }
    let duration = |t: f64| if t.fract() == 0. { format!("Duration::from_secs({})", t as u64) } else { format!("Duration::from_millis({})", (t * 1000.).round() as u64) };
    if let Some(t) = c.timeout {
        chain.push(format!(".timeout({})", duration(t)));
    }
    chain.push(".send()?;".into());
    let client = match c.connect_timeout {
        Some(t) => format!("reqwest::blocking::Client::builder().connect_timeout({}).build()?", duration(t)),
        None => "reqwest::blocking::Client::new()".to_string(),
    };
    let uses = if c.timeout.is_some() || c.connect_timeout.is_some() { "use std::time::Duration;\n\n" } else { "" };
    format!(
        "{uses}fn main() -> Result<(), Box<dyn std::error::Error>> {{\n    let client = {client};\n    let response = client\n{}\n\n    println!(\"{{}}\", response.status());\n    println!(\"{{}}\", response.text()?);\n    Ok(())\n}}",
        chain.iter().map(|l| format!("        {l}")).collect::<Vec<_>>().join("\n")
    )
}

fn php_string(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn php(c: &Curl) -> String {
    let r = &c.request;
    let mut opts = vec![format!("CURLOPT_URL => {}", php_string(&r.url)), "CURLOPT_RETURNTRANSFER => true".to_string()];
    match r.method.as_str() {
        "GET" => {}
        "HEAD" => opts.push("CURLOPT_NOBODY => true".into()),
        m => opts.push(format!("CURLOPT_CUSTOMREQUEST => {}", php_string(m))),
    }
    if !r.headers.is_empty() {
        let rows: String = r.headers.iter().map(|(k, v)| format!("        {},\n", php_string(&format!("{k}: {v}")))).collect();
        opts.push(format!("CURLOPT_HTTPHEADER => [\n{rows}    ]"));
    }
    if let Some(b) = &r.body {
        opts.push(format!("CURLOPT_POSTFIELDS => {}", php_string(b)));
    }
    let time = |name: &str, t: f64| if t.fract() == 0. { format!("CURLOPT_{name} => {}", t as u64) } else { format!("CURLOPT_{name}_MS => {}", (t * 1000.).round() as u64) };
    if let Some(t) = c.timeout {
        opts.push(time("TIMEOUT", t));
    }
    if let Some(t) = c.connect_timeout {
        opts.push(time("CONNECTTIMEOUT", t));
    }
    let opts: String = opts.iter().map(|o| format!("    {o},\n")).collect();
    format!("<?php\n\n$ch = curl_init();\ncurl_setopt_array($ch, [\n{opts}]);\n\n$response = curl_exec($ch);\n$status = curl_getinfo($ch, CURLINFO_RESPONSE_CODE);\ncurl_close($ch);\n\necho $status, \"\\n\", $response;")
}

fn swift(c: &Curl) -> String {
    let r = &c.request;
    let mut s = format!("import Foundation\n\nvar request = URLRequest(url: URL(string: {})!)\nrequest.httpMethod = {}\n", brace_escaped(&r.url), brace_escaped(&r.method));
    for (k, v) in &r.headers {
        s.push_str(&format!("request.addValue({}, forHTTPHeaderField: {})\n", brace_escaped(v), brace_escaped(k)));
    }
    if let Some(b) = &r.body {
        s.push_str(&format!("request.httpBody = Data({}.utf8)\n", brace_escaped(b)));
    }
    if let Some(t) = c.timeout {
        s.push_str(&format!("request.timeoutInterval = {}\n", number(t)));
    }
    s.push_str("\nlet (data, response) = try await URLSession.shared.data(for: request)\nif let http = response as? HTTPURLResponse {\n    print(http.statusCode)\n}\nprint(String(decoding: data, as: UTF8.self))");
    s
}

/// Code for `TARGETS[target]`, and what could not be carried over.
pub fn convert(src: &str, target: usize) -> Result<(String, Vec<String>), String> {
    let mut c = parse(src)?;
    let code = match target {
        0 => javascript(&c),
        1 => python(&c),
        2 => go(&c),
        3 => rust(&c),
        4 => php(&c),
        5 => swift(&c),
        _ => http::to_http(&c.request),
    };
    // Languages whose HTTP client has no separate connect timeout.
    if c.connect_timeout.is_some() && [0, 2, 5, 6].contains(&target) {
        c.warnings.push("--connect-timeout has no equivalent here and was left out".into());
    }
    if c.timeout.is_some() && target == 6 {
        c.warnings.push("--max-time has no place in a .http file and was left out".into());
    }
    Ok((code, c.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        words(s).unwrap().0
    }

    #[test]
    fn shell_words() {
        assert_eq!(w("curl -H 'A: b c' \"x \\\"y\\\" $HOME\" a\\ b"), ["curl", "-H", "A: b c", "x \"y\" $HOME", "a b"]);
        assert_eq!(w("curl \\\n  -X POST \\\r\n  url"), ["curl", "-X", "POST", "url"]);
        assert_eq!(w("curl ^\n -X GET `\n url"), ["curl", "-X", "GET", "url"]);
        assert_eq!(w("a $'x\\ty\\n\\'q\\x41\\u00e9' ''"), ["a", "x\ty\n'qAé", ""]);
        assert_eq!(w("a 'it'\\''s' \"a\"'b'c"), ["a", "it's", "abc"]);
        assert_eq!(w("curl http://x/?a=1&b=2 # note\n -s"), ["curl", "http://x/?a=1&b=2", "-s"]);
        assert_eq!(words("curl x | jq .").unwrap(), (vec!["curl".to_string(), "x".to_string()], true));
        assert!(words("curl 'x").is_err());
        assert!(words("curl \"x").is_err());
    }

    const POST: &str = "curl -sS -X POST 'https://api.example.com/users?x=1' \\\n  -H 'Content-Type: application/json' \\\n  -H \"Authorization: Bearer abc\" \\\n  --data-raw '{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}' \\\n  --max-time 30 --connect-timeout 2.5 -k";

    #[test]
    fn parses_requests() {
        let c = parse(POST).unwrap();
        assert_eq!(c.request.method, "POST");
        assert_eq!(c.request.url, "https://api.example.com/users?x=1");
        assert_eq!(c.request.headers, [("Content-Type".to_string(), "application/json".to_string()), ("Authorization".into(), "Bearer abc".into())]);
        assert_eq!(c.request.body.as_deref(), Some("{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}"));
        assert_eq!((c.timeout, c.connect_timeout), (Some(30.), Some(2.5)));
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("--insecure"));
    }

    #[test]
    fn option_forms() {
        // Glued short values, clusters, --name=value, implied POST and its content type.
        let c = parse("curl -XPUT -H'A: 1' -sLd a=1 -d 'b=2' --user-agent=UA --url=example.com/p").unwrap();
        assert_eq!(c.request.method, "PUT");
        assert_eq!(c.request.url, "http://example.com/p");
        assert_eq!(c.request.body.as_deref(), Some("a=1&b=2"));
        assert_eq!(c.request.header("content-type"), Some("application/x-www-form-urlencoded"));
        assert_eq!(c.request.header("user-agent"), Some("UA"));
        assert!(c.warnings.is_empty());

        let c = parse("curl -u ada:s3cret -b 'a=1; b=2' -e https://ref -r 0-99 https://x.test").unwrap();
        assert_eq!(c.request.method, "GET");
        assert_eq!(c.request.header("authorization"), Some("Basic YWRhOnMzY3JldA=="));
        assert_eq!(c.request.header("cookie"), Some("a=1; b=2"));
        assert_eq!(c.request.header("range"), Some("bytes=0-99"));

        let c = parse("curl -G https://x.test/s --data-urlencode 'q=a b&c' -d page=2 -I").unwrap();
        assert_eq!((c.request.method.as_str(), c.request.url.as_str(), &c.request.body), ("HEAD", "https://x.test/s?q=a%20b%26c&page=2", &None));

        let c = parse("curl https://x.test --json '{\"a\":1}'").unwrap();
        assert_eq!(c.request.method, "POST");
        assert_eq!((c.request.header("content-type"), c.request.header("accept")), (Some("application/json"), Some("application/json")));

        let c = parse("$ curl -F file=@a.png --proxy http://p:1 -d @body.json -o out.txt https://x.test https://y.test | jq").unwrap();
        assert_eq!(c.warnings.len(), 5, "{:?}", c.warnings);
        assert_eq!(c.request.body.as_deref(), Some("<contents of body.json>"));

        assert_eq!(parse("wget https://x.test").unwrap_err(), "Paste a command that starts with curl");
        assert_eq!(parse("curl -s").unwrap_err(), "The command has no URL");
        assert_eq!(parse("curl https://x.test -H").unwrap_err(), "--header needs a value");
        assert!(parse("curl https://x.test -m soon").is_err());
        assert!(parse("C:\\tools\\curl.exe https://x.test").is_ok());
    }

    #[test]
    fn javascript_and_python() {
        let (js, warnings) = convert(POST, 0).unwrap();
        assert_eq!(warnings.len(), 2);
        assert_eq!(
            js,
            "const response = await fetch(\"https://api.example.com/users?x=1\", {
  method: \"POST\",
  headers: {
    \"Content-Type\": \"application/json\",
    \"Authorization\": \"Bearer abc\"
  },
  body: JSON.stringify({
    \"name\": \"Ada\",
    \"tags\": [
      \"a\"
    ],
    \"ok\": true
  }),
  signal: AbortSignal.timeout(30000)
});

console.log(response.status);
console.log(await response.text());"
        );
        assert_eq!(convert("curl https://x.test", 0).unwrap().0, "const response = await fetch(\"https://x.test\");\n\nconsole.log(response.status);\nconsole.log(await response.text());");
        assert!(convert("curl https://x.test -H 'A: 1' -H 'a: 2'", 0).unwrap().0.contains("headers: [\n    [\"A\", \"1\"],\n    [\"a\", \"2\"]\n  ]"));

        let (py, warnings) = convert(POST, 1).unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            py,
            "import requests

response = requests.post(
    \"https://api.example.com/users?x=1\",
    headers={
        \"Content-Type\": \"application/json\",
        \"Authorization\": \"Bearer abc\",
    },
    json={
        \"name\": \"Ada\",
        \"tags\": [
            \"a\",
        ],
        \"ok\": True,
    },
    timeout=(2.5, 30),
)

print(response.status_code)
print(response.text)"
        );
        assert!(convert("curl -X PURGE https://x.test -d 'a=1'", 1).unwrap().0.contains("requests.request(\n    \"PURGE\",\n    \"https://x.test\",\n"));
        assert!(convert("curl https://x.test", 1).unwrap().0.contains("response = requests.get(\"https://x.test\")\n"));
        assert!(convert("curl https://x.test -H 'A: 1' -H 'A: 2'", 1).unwrap().0.contains("\"A\": \"1, 2\",\n"));
    }

    #[test]
    fn go_rust_php_swift_and_http() {
        let go = convert(POST, 2).unwrap().0;
        assert!(go.starts_with("package main\n\nimport (\n\t\"fmt\"\n\t\"io\"\n\t\"net/http\"\n\t\"strings\"\n\t\"time\"\n)\n\nfunc main() {\n\tbody := strings.NewReader(`{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}`)\n\treq, err := http.NewRequest(\"POST\", \"https://api.example.com/users?x=1\", body)\n"), "{go}");
        assert!(go.contains("\treq.Header.Add(\"Authorization\", \"Bearer abc\")\n\n\tclient := &http.Client{Timeout: 30 * time.Second}\n\tresp, err := client.Do(req)\n"));
        let plain = convert("curl https://x.test", 2).unwrap().0;
        assert!(plain.contains("http.NewRequest(\"GET\", \"https://x.test\", nil)") && plain.contains("\n\n\tresp, err := http.DefaultClient.Do(req)") && !plain.contains("strings"));

        let rs = convert(POST, 3).unwrap().0;
        assert!(rs.starts_with("use std::time::Duration;\n\nfn main() -> Result<(), Box<dyn std::error::Error>> {\n    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_millis(2500)).build()?;\n    let response = client\n        .post(\"https://api.example.com/users?x=1\")\n        .header(\"Content-Type\", \"application/json\")\n"), "{rs}");
        assert!(rs.contains("        .body(r#\"{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}\"#)\n        .timeout(Duration::from_secs(30))\n        .send()?;\n"));
        assert_eq!(rust_string("a\"#b"), "r##\"a\"#b\"##");
        assert_eq!(rust_string("tab\tand\u{1}"), "\"tab\\tand\\u{1}\"");

        let php = convert(POST, 4).unwrap().0;
        assert!(php.contains("    CURLOPT_CUSTOMREQUEST => 'POST',\n    CURLOPT_HTTPHEADER => [\n        'Content-Type: application/json',\n        'Authorization: Bearer abc',\n    ],\n    CURLOPT_POSTFIELDS => '{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}',\n    CURLOPT_TIMEOUT => 30,\n    CURLOPT_CONNECTTIMEOUT_MS => 2500,\n]);"), "{php}");
        assert_eq!(php_string("it's a \\ path"), "'it\\'s a \\\\ path'");

        let swift = convert(POST, 5).unwrap().0;
        assert!(swift.contains("request.httpMethod = \"POST\"\nrequest.addValue(\"application/json\", forHTTPHeaderField: \"Content-Type\")\n"), "{swift}");
        assert!(swift.contains("request.httpBody = Data(\"{\\\"name\\\": \\\"Ada\\\", \\\"tags\\\": [\\\"a\\\"], \\\"ok\\\": true}\".utf8)\nrequest.timeoutInterval = 30\n"));

        let (http, warnings) = convert(POST, 6).unwrap();
        assert_eq!(http, "POST https://api.example.com/users?x=1\nContent-Type: application/json\nAuthorization: Bearer abc\n\n{\"name\": \"Ada\", \"tags\": [\"a\"], \"ok\": true}");
        assert_eq!(warnings.len(), 3);
        // What it writes, the HTTP Client reads back as the same request.
        assert_eq!(http::parse_http(&http).requests[0], parse(POST).unwrap().request);
    }
}
