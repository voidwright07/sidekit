//! One HTTP request model for cURL to Code, the HTTP Client and the MCP
//! Inspector: the `.http` file format, `{{variable}}` substitution, sending.

use std::io::Read;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub name: Option<String>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl Request {
    /// The first value of a header, whatever its capitalisation.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// `GET /users` style label for lists.
    pub fn label(&self) -> String {
        let path = self.url.split_once("://").map(|(_, r)| r.find('/').map(|i| &r[i..]).unwrap_or("/")).unwrap_or(&self.url);
        let path = path.split(['?', '#']).next().unwrap_or(path);
        match &self.name {
            Some(n) => format!("{} {path} · {n}", self.method),
            None => format!("{} {path}", self.method),
        }
    }
}

// ---------------------------------------------------------------- .http files

#[derive(Debug, Default)]
pub struct HttpFile {
    pub requests: Vec<Request>,
    pub variables: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn comment(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("//").or_else(|| t.strip_prefix('#'))
}

fn request_line(line: &str) -> (String, String) {
    let mut parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() > 1 && parts.last().is_some_and(|p| p.starts_with("HTTP/")) {
        parts.pop();
    }
    let verb = parts.first().is_some_and(|p| p.len() <= 16 && p.bytes().all(|b| b.is_ascii_uppercase() || b == b'-'));
    if verb && parts.len() > 1 {
        (parts[0].to_string(), parts[1..].join(" "))
    } else {
        // A bare URL is a GET.
        ("GET".to_string(), parts.join(" "))
    }
}

/// Read every request in a `.http` file. Lenient: odd lines become warnings.
pub fn parse_http(src: &str) -> HttpFile {
    enum At {
        Start,
        Headers,
        Body,
    }
    let mut file = HttpFile::default();
    let mut at = At::Start;
    let mut current: Option<Request> = None;
    let mut name: Option<String> = None;
    let mut body: Vec<&str> = Vec::new();

    fn finish(file: &mut HttpFile, current: &mut Option<Request>, body: &mut Vec<&str>) {
        if let Some(mut r) = current.take() {
            let text = body.join("\n");
            let text = text.trim_matches(['\n', '\r']).to_string();
            if text.trim_start().starts_with("< ") {
                file.warnings.push(format!("{}: bodies read from a file (< path) are not supported", r.label()));
            } else if !text.trim().is_empty() {
                r.body = Some(text);
            }
            file.requests.push(r);
        }
        body.clear();
    }

    for line in src.lines() {
        if let Some(title) = line.strip_prefix("###") {
            finish(&mut file, &mut current, &mut body);
            at = At::Start;
            name = Some(title.trim().to_string()).filter(|t| !t.is_empty());
            continue;
        }
        match at {
            At::Start => {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if let Some(c) = comment(line) {
                    if let Some(n) = c.trim().strip_prefix("@name") {
                        name = Some(n.trim().trim_start_matches('=').trim().to_string()).filter(|n| !n.is_empty());
                    }
                    continue;
                }
                if let Some((k, v)) = t.strip_prefix('@').and_then(|r| r.split_once('=')) {
                    let k = k.trim();
                    if !k.is_empty() && !k.contains(char::is_whitespace) {
                        file.variables.retain(|(n, _)| n != k);
                        file.variables.push((k.to_string(), v.trim().to_string()));
                        continue;
                    }
                }
                let (method, url) = request_line(t);
                current = Some(Request { name: name.take(), method, url, headers: Vec::new(), body: None });
                at = At::Headers;
            }
            At::Headers => {
                let t = line.trim();
                let Some(r) = current.as_mut() else { continue };
                if t.is_empty() {
                    at = At::Body;
                } else if line.starts_with(char::is_whitespace) && (t.starts_with('?') || t.starts_with('&')) {
                    // A long query string may continue on indented lines.
                    r.url.push_str(t);
                } else if comment(line).is_some() {
                } else if let Some((k, v)) = t.split_once(':').filter(|(k, _)| is_token(k.trim())) {
                    r.headers.push((k.trim().to_string(), v.trim().to_string()));
                } else {
                    file.warnings.push(format!("{}: \"{}\" is not a header; a blank line should come before the body", r.label(), super::clip(t, 40)));
                    at = At::Body;
                    body.push(line);
                }
            }
            At::Body => body.push(line),
        }
    }
    finish(&mut file, &mut current, &mut body);
    file
}

/// The request as `.http` text.
pub fn to_http(r: &Request) -> String {
    let mut s = format!("{} {}", r.method, r.url);
    for (k, v) in &r.headers {
        s.push_str(&format!("\n{k}: {v}"));
    }
    if let Some(b) = &r.body {
        s.push_str("\n\n");
        s.push_str(b);
    }
    s
}

fn dynamic(name: &str) -> Option<String> {
    let mut words = name.split_whitespace();
    Some(match words.next()? {
        "$uuid" | "$guid" | "$random.uuid" => super::uuid_v4(),
        "$timestamp" => chrono::Utc::now().timestamp().to_string(),
        "$isoTimestamp" => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "$datetime" => match words.next() {
            Some("rfc1123") => chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
            _ => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        },
        "$randomInt" => {
            let min: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
            let max: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(1000);
            let span = (max - min).max(1) as usize;
            (min + super::rand_below(span) as i64).to_string()
        }
        _ => return None,
    })
}

/// Fill in `{{name}}` placeholders. Names nothing defines are left as typed
/// and collected in `missing`.
fn substitute(text: &str, vars: &[(String, String)], missing: &mut Vec<String>, depth: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(len) = rest[open + 2..].find("}}") else { break };
        let name = rest[open + 2..open + 2 + len].trim();
        out.push_str(&rest[..open]);
        let value = match vars.iter().find(|(k, _)| k == name) {
            // Variables may be built from other variables.
            Some((_, v)) if depth < 8 => Some(substitute(v, vars, missing, depth + 1)),
            Some(_) => None,
            None => dynamic(name),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => {
                if !missing.iter().any(|m| m == name) {
                    missing.push(name.to_string());
                }
                out.push_str(&rest[open..open + len + 4]);
            }
        }
        rest = &rest[open + len + 4..];
    }
    out.push_str(rest);
    out
}

/// `r` with its placeholders filled in, and the names that stayed unresolved.
pub fn resolve(r: &Request, vars: &[(String, String)]) -> (Request, Vec<String>) {
    let mut missing = Vec::new();
    let mut sub = |s: &str| substitute(s, vars, &mut missing, 0);
    let out = Request {
        name: r.name.clone(),
        method: r.method.clone(),
        url: sub(&r.url),
        headers: r.headers.iter().map(|(k, v)| (sub(k), sub(v))).collect(),
        body: r.body.as_deref().map(&mut sub),
    };
    (out, missing)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The request as a cURL command.
pub fn to_curl(r: &Request) -> String {
    let mut parts = vec![match r.method.as_str() {
        "GET" => format!("curl {}", sh_quote(&r.url)),
        "HEAD" => format!("curl -I {}", sh_quote(&r.url)),
        m => format!("curl -X {m} {}", sh_quote(&r.url)),
    }];
    parts.extend(r.headers.iter().map(|(k, v)| format!("-H {}", sh_quote(&format!("{k}: {v}")))));
    if let Some(b) = &r.body {
        parts.push(format!("--data-raw {}", sh_quote(b)));
    }
    parts.join(" \\\n  ")
}

// ---------------------------------------------------------------- sending

/// The most of a response body that is kept.
const BODY_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The body was longer than [`BODY_LIMIT`].
    pub truncated: bool,
    pub elapsed: Duration,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The body for display and the grammar to show it in. JSON is
    /// pretty-printed; anything else is returned as received.
    pub fn display_body(&self) -> (String, &'static str) {
        let kind = self.header("content-type").unwrap_or("").to_lowercase();
        let Ok(text) = std::str::from_utf8(&self.body) else {
            return (format!("Binary response · {}", super::plural(self.body.len(), "byte")), "plaintext");
        };
        let trimmed = text.trim_start();
        if kind.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                return (super::to_json(&v, &super::Indent::Spaces(2)), "json");
            }
        }
        let lang = if kind.contains("html") || kind.contains("xml") { "html" } else { "plaintext" };
        (text.to_string(), lang)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        // A 404 is a response to show, not a failure.
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_global(Some(timeout))
        .user_agent(concat!("SideKit/", env!("CARGO_PKG_VERSION")))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

fn explain(e: ureq::Error) -> String {
    match e {
        ureq::Error::HostNotFound => "The host could not be found".into(),
        ureq::Error::Timeout(_) => "The request timed out".into(),
        ureq::Error::ConnectionFailed => "Could not connect to the server".into(),
        ureq::Error::BadUri(u) => format!("Not a valid URL: {u}"),
        ureq::Error::Io(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused => "The connection was refused".into(),
            _ => e.to_string(),
        },
        other => other.to_string(),
    }
}

/// Send `r` as written. Blocks: call it off the UI thread. Nothing is kept
/// between calls: no cookies, no cache, no history.
pub fn send(r: &Request, timeout: Duration) -> Result<Response, String> {
    send_until(r, timeout, &mut |_| false)
}

/// [`send`], but stop reading the body as soon as `enough` says so. Event
/// streams never end by themselves, so their reader has to decide.
pub fn send_until(r: &Request, timeout: Duration, enough: &mut dyn FnMut(&[u8]) -> bool) -> Result<Response, String> {
    let url = r.url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("The URL must start with http:// or https://".into());
    }
    let mut b = ureq::http::Request::builder().method(r.method.as_str()).uri(url);
    for (k, v) in &r.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let started = Instant::now();
    let agent = agent(timeout);
    let result = match &r.body {
        Some(body) => b.body(body.clone()).map_err(|e| e.to_string()).and_then(|req| agent.run(req).map_err(explain)),
        None => b.body(()).map_err(|e| e.to_string()).and_then(|req| agent.run(req).map_err(explain)),
    };
    let mut resp = result?;
    let mut body = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut reader = resp.body_mut().as_reader();
    while body.len() <= BODY_LIMIT && !enough(&body) {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(format!("The response body could not be read: {e}")),
        }
    }
    drop(reader);
    let truncated = body.len() > BODY_LIMIT;
    body.truncate(BODY_LIMIT);
    Ok(Response {
        status: resp.status().as_u16(),
        reason: resp.status().canonical_reason().unwrap_or("").to_string(),
        version: format!("{:?}", resp.version()),
        headers: resp.headers().iter().map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect(),
        body,
        truncated,
        elapsed: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    const FILE: &str = "@host = https://api.example.com
@token=abc123
@auth = Bearer {{token}}

### List users
# a comment
GET {{host}}/users
    ?page=2
    &size=10
Accept: application/json
Authorization: {{auth}}

###
# @name createUser
POST {{host}}/users HTTP/1.1
Content-Type: application/json
X-Request-Id: {{$uuid}}

{
  \"name\": \"{{name}}\"
}

###

https://example.com/health
";

    #[test]
    fn reads_http_files() {
        let f = parse_http(FILE);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(f.variables.len(), 3);
        assert_eq!(f.requests.len(), 3);
        let list = &f.requests[0];
        assert_eq!((list.method.as_str(), list.url.as_str()), ("GET", "{{host}}/users?page=2&size=10"));
        assert_eq!(list.name.as_deref(), Some("List users"));
        assert_eq!(list.headers.len(), 2);
        assert_eq!(list.body, None);
        let create = &f.requests[1];
        assert_eq!(create.name.as_deref(), Some("createUser"));
        assert_eq!(create.url, "{{host}}/users");
        assert_eq!(create.body.as_deref(), Some("{\n  \"name\": \"{{name}}\"\n}"));
        assert_eq!(create.label(), "POST {{host}}/users · createUser");
        assert_eq!((f.requests[2].method.as_str(), f.requests[2].url.as_str()), ("GET", "https://example.com/health"));
        assert_eq!(f.requests[2].label(), "GET /health");
    }

    #[test]
    fn fills_placeholders_and_reports_the_rest() {
        let f = parse_http(FILE);
        let (r, missing) = resolve(&f.requests[0], &f.variables);
        assert_eq!(r.url, "https://api.example.com/users?page=2&size=10");
        assert_eq!(r.header("authorization"), Some("Bearer abc123"));
        assert!(missing.is_empty());
        let (r, missing) = resolve(&f.requests[1], &f.variables);
        assert_eq!(missing, ["name"]);
        // Unresolved placeholders stay visible rather than vanishing.
        assert!(r.body.as_deref().unwrap().contains("{{name}}"));
        assert_eq!(r.header("X-Request-Id").unwrap().len(), 36);
        // A variable that refers to itself must not loop forever.
        let vars = vec![("a".to_string(), "{{a}}".to_string())];
        let (r, missing) = resolve(&Request { url: "{{a}}".into(), ..Default::default() }, &vars);
        assert_eq!((r.url.as_str(), missing), ("{{a}}", vec!["a".to_string()]));
    }

    #[test]
    fn odd_files_and_export() {
        let f = parse_http("POST https://example.com/a\nContent-Type: text/plain\nthis is not a header\n");
        assert_eq!(f.warnings.len(), 1);
        assert_eq!(f.requests[0].body.as_deref(), Some("this is not a header"));
        let f = parse_http("PUT https://example.com/a\n\n< ./body.json\n");
        assert!(f.warnings[0].contains("not supported") && f.requests[0].body.is_none());
        assert!(parse_http("").requests.is_empty());

        let r = Request { method: "POST".into(), url: "https://example.com/it's".into(), headers: vec![("A".into(), "b".into())], body: Some("x=1".into()), name: None };
        assert_eq!(to_curl(&r), "curl -X POST 'https://example.com/it'\\''s' \\\n  -H 'A: b' \\\n  --data-raw 'x=1'");
        assert_eq!(to_http(&r), "POST https://example.com/it's\nA: b\n\nx=1");
        assert_eq!(parse_http(&to_http(&r)).requests[0], r);
    }

    /// A one-shot server on a local port that answers with `reply` and hands
    /// back what it was sent.
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut seen = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                seen.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            seen.push_str(&String::from_utf8(body).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
            seen
        });
        (url, handle)
    }

    #[test]
    fn sends_requests_and_reads_responses() {
        let (url, server) = serve("HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nX-Trace: t1\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"error\":\"nope\"}");
        let r = Request {
            name: None,
            method: "POST".into(),
            url: format!("{url}/users?x=1"),
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Custom".into(), "yes".into())],
            body: Some("{\"name\":\"Ada\"}".into()),
        };
        let resp = send(&r, Duration::from_secs(10)).unwrap();
        // An error status is still a response.
        assert_eq!((resp.status, resp.reason.as_str()), (404, "Not Found"));
        assert_eq!(resp.header("x-trace"), Some("t1"));
        assert_eq!(resp.display_body(), ("{\n  \"error\": \"nope\"\n}".to_string(), "json"));
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /users?x=1 HTTP/1.1\r\n"), "{seen}");
        assert!(seen.to_lowercase().contains("x-custom: yes\r\n") && seen.ends_with("{\"name\":\"Ada\"}"));

        assert!(send(&Request { method: "GET".into(), url: "ftp://example.com".into(), ..Default::default() }, Duration::from_secs(1)).is_err());
        // Nothing listens on port 9 of localhost.
        assert!(send(&Request { method: "GET".into(), url: "http://127.0.0.1:9/".into(), ..Default::default() }, Duration::from_secs(5)).is_err());
    }
}
