//! Model Context Protocol: check a JSON-RPC transcript against the protocol,
//! list what the server advertises, and discover a live server read-only.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::http::{self, Request};

/// What a transcript can be checked against. The first follows the transcript.
pub const REVISIONS: &[&str] = &["Negotiated", "2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Requested by live discovery; servers answer with the revision they speak.
const LATEST: &str = "2025-06-18";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// 1-based line of the message in the transcript.
    pub line: usize,
    pub level: Level,
    pub text: String,
}

/// A tool, resource or prompt a server advertised.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub facts: Vec<(&'static str, String)>,
}

#[derive(Debug, Default)]
pub struct Inspection {
    pub messages: usize,
    /// The revision the transcript was checked against.
    pub revision: Option<String>,
    pub client: Option<String>,
    pub server: Option<String>,
    pub instructions: Option<String>,
    pub client_caps: Vec<String>,
    pub server_caps: Vec<String>,
    pub tools: Vec<Entry>,
    pub resources: Vec<Entry>,
    pub prompts: Vec<Entry>,
    /// Error responses the peers sent each other.
    pub rpc_errors: Vec<Finding>,
    /// Places where the transcript breaks the protocol.
    pub findings: Vec<Finding>,
    /// Every other method that appears, with how often.
    pub calls: Vec<(String, usize)>,
}

// ---------------------------------------------------------------- reading

struct Message {
    line: usize,
    value: Value,
    /// Part of a JSON-RPC batch (an array of messages).
    batched: bool,
}

/// Log lines often carry the message inside an envelope.
fn unwrap_envelope(v: Value) -> Value {
    if let Value::Object(m) = &v {
        if !m.contains_key("jsonrpc") {
            for key in ["message", "payload", "data", "body"] {
                if m.get(key).is_some_and(|x| x.get("jsonrpc").is_some()) {
                    return m[key].clone();
                }
            }
        }
    }
    v
}

/// Every JSON-RPC message in `src`: one JSON document, JSON Lines, messages
/// run together, or server-sent events. Text that is not JSON is reported and
/// skipped, a line at a time.
fn read(src: &str, findings: &mut Vec<Finding>) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pos = 0;
    let line_of = |at: usize| src[..at].matches('\n').count() + 1;
    let push = |v: Value, line: usize, out: &mut Vec<Message>| match v {
        Value::Array(items) => out.extend(items.into_iter().map(|x| Message { line, value: unwrap_envelope(x), batched: true })),
        other => out.push(Message { line, value: unwrap_envelope(other), batched: false }),
    };
    while pos < src.len() {
        let rest = &src[pos..];
        let skip = rest.len() - rest.trim_start().len();
        if skip == rest.len() {
            break;
        }
        pos += skip;
        let rest = &src[pos..];
        let line_end = rest.find('\n').map(|i| pos + i + 1).unwrap_or(src.len());
        // Server-sent events: `data:` carries the message, the other fields are framing.
        if ["event:", "id:", "retry:", ":"].iter().any(|f| rest.starts_with(f)) {
            pos = line_end;
            continue;
        }
        if let Some(after) = rest.strip_prefix("data:") {
            pos += rest.len() - after.len();
            continue;
        }
        // A whole-file array holds one message per element; keep their own lines.
        if rest.starts_with('[') && out.is_empty() {
            if let Some(items) = array_items(src, pos) {
                for (at, v) in items {
                    push(v, line_of(at), &mut out);
                }
                break;
            }
        }
        let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        match stream.next() {
            Some(Ok(v)) => {
                push(v, line_of(pos), &mut out);
                pos += stream.byte_offset();
            }
            Some(Err(e)) => {
                let shown = super::clip(rest[..line_end - pos].trim(), 48);
                findings.push(Finding { line: line_of(pos), level: Level::Error, text: format!("Not valid JSON ({}): {shown}", e.to_string().split(" at line").next().unwrap_or("")) });
                pos = line_end;
            }
            None => break,
        }
    }
    out
}

/// The elements of the top-level array starting at `start`, with the offset of
/// each, or `None` when it is not a clean array running to the end of the text.
fn array_items(src: &str, start: usize) -> Option<Vec<(usize, Value)>> {
    let mut items = Vec::new();
    let mut pos = start + 1;
    loop {
        pos += src[pos..].len() - src[pos..].trim_start().len();
        match src[pos..].chars().next()? {
            ']' => return src[pos + 1..].trim().is_empty().then_some(items),
            ',' => pos += 1,
            _ => {
                let mut stream = serde_json::Deserializer::from_str(&src[pos..]).into_iter::<Value>();
                let v = stream.next()?.ok()?;
                items.push((pos, v));
                pos += stream.byte_offset();
            }
        }
    }
}

// ---------------------------------------------------------------- the protocol

/// (method, capability the receiving side must have declared, first revision)
const CLIENT_REQUESTS: &[(&str, Option<&str>, &str)] = &[
    ("initialize", None, "2024-11-05"),
    ("ping", None, "2024-11-05"),
    ("tools/list", Some("tools"), "2024-11-05"),
    ("tools/call", Some("tools"), "2024-11-05"),
    ("resources/list", Some("resources"), "2024-11-05"),
    ("resources/templates/list", Some("resources"), "2024-11-05"),
    ("resources/read", Some("resources"), "2024-11-05"),
    ("resources/subscribe", Some("resources"), "2024-11-05"),
    ("resources/unsubscribe", Some("resources"), "2024-11-05"),
    ("prompts/list", Some("prompts"), "2024-11-05"),
    ("prompts/get", Some("prompts"), "2024-11-05"),
    ("logging/setLevel", Some("logging"), "2024-11-05"),
    ("completion/complete", None, "2024-11-05"),
    ("tasks/get", None, "2025-11-25"),
    ("tasks/result", None, "2025-11-25"),
    ("tasks/list", None, "2025-11-25"),
    ("tasks/cancel", None, "2025-11-25"),
];

const SERVER_REQUESTS: &[(&str, Option<&str>, &str)] = &[
    ("sampling/createMessage", Some("sampling"), "2024-11-05"),
    ("roots/list", Some("roots"), "2024-11-05"),
    ("elicitation/create", Some("elicitation"), "2025-06-18"),
];

const NOTIFICATIONS: &[(&str, &str)] = &[
    ("notifications/initialized", "2024-11-05"),
    ("notifications/cancelled", "2024-11-05"),
    ("notifications/progress", "2024-11-05"),
    ("notifications/message", "2024-11-05"),
    ("notifications/resources/updated", "2024-11-05"),
    ("notifications/resources/list_changed", "2024-11-05"),
    ("notifications/tools/list_changed", "2024-11-05"),
    ("notifications/prompts/list_changed", "2024-11-05"),
    ("notifications/roots/list_changed", "2024-11-05"),
    ("notifications/tasks/status", "2025-11-25"),
    ("notifications/elicitation/complete", "2025-11-25"),
];

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn party(info: Option<&Value>) -> Option<String> {
    let info = info?;
    let name = text(info, "title").or_else(|| text(info, "name"))?;
    Some(match text(info, "version") {
        Some(v) => format!("{name} {v}"),
        None => name,
    })
}

/// `tools (listChanged)`, `resources (subscribe, listChanged)`, `logging`.
fn capabilities(caps: Option<&Value>) -> Vec<String> {
    let Some(Value::Object(m)) = caps else { return Vec::new() };
    m.iter()
        .map(|(k, v)| {
            let flags: Vec<&str> = v.as_object().map(|o| o.iter().filter(|(_, on)| on.as_bool() == Some(true)).map(|(f, _)| f.as_str()).collect()).unwrap_or_default();
            if flags.is_empty() { k.clone() } else { format!("{k} ({})", flags.join(", ")) }
        })
        .collect()
}

fn tool_entry(t: &Value) -> Result<Entry, String> {
    let name = text(t, "name").ok_or("a tool has no name")?;
    let schema = t.get("inputSchema").and_then(Value::as_object).ok_or(format!("tool \"{name}\" has no inputSchema object"))?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(format!("the inputSchema of tool \"{name}\" is not of type \"object\""));
    }
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let params: Vec<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(k, p)| {
                    let ty = match p.get("type") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" | "),
                        _ if p.get("enum").is_some() => "enum".into(),
                        _ => "any".into(),
                    };
                    format!("{k}: {ty}{}", if required.contains(&k.as_str()) { ", required" } else { "" })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut facts = Vec::new();
    if let Some(title) = text(t, "title").or_else(|| t.get("annotations").and_then(|a| text(a, "title"))) {
        facts.push(("Title", title));
    }
    facts.push(("Parameters", if params.is_empty() { "none".into() } else { params.join(" · ") }));
    if let Some(Value::Object(a)) = t.get("annotations") {
        let hint = |k: &str| a.get(k).and_then(Value::as_bool);
        let mut hints = Vec::new();
        match hint("readOnlyHint") {
            Some(true) => hints.push("read-only"),
            _ => {
                // Unless a tool says otherwise it may change things, destructively.
                if hint("destructiveHint") != Some(false) {
                    hints.push("may be destructive");
                }
            }
        }
        if hint("idempotentHint") == Some(true) {
            hints.push("idempotent");
        }
        if hint("openWorldHint") != Some(false) {
            hints.push("reaches outside systems");
        }
        if !hints.is_empty() {
            facts.push(("Hints", hints.join(" · ")));
        }
    }
    if t.get("outputSchema").is_some() {
        facts.push(("Output", "structured (has an outputSchema)".into()));
    }
    Ok(Entry { name, description: text(t, "description").unwrap_or_default(), facts })
}

fn resource_entry(r: &Value) -> Result<Entry, String> {
    let name = text(r, "name").ok_or("a resource has no name")?;
    let (label, uri) = match (text(r, "uri"), text(r, "uriTemplate")) {
        (Some(u), _) => ("URI", u),
        (None, Some(t)) => ("URI template", t),
        _ => return Err(format!("resource \"{name}\" has no uri")),
    };
    let mut facts = vec![(label, uri)];
    if let Some(m) = text(r, "mimeType") {
        facts.push(("Type", m));
    }
    if let Some(n) = r.get("size").and_then(Value::as_u64) {
        facts.push(("Size", super::plural(n as usize, "byte")));
    }
    Ok(Entry { name, description: text(r, "description").unwrap_or_default(), facts })
}

fn prompt_entry(p: &Value) -> Result<Entry, String> {
    let name = text(p, "name").ok_or("a prompt has no name")?;
    let args: Vec<String> = p
        .get("arguments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| Some(format!("{}{}", text(x, "name")?, if x.get("required").and_then(Value::as_bool) == Some(true) { ", required" } else { "" })))
                .collect()
        })
        .unwrap_or_default();
    let facts = vec![("Arguments", if args.is_empty() { "none".into() } else { args.join(" · ") })];
    Ok(Entry { name, description: text(p, "description").unwrap_or_default(), facts })
}

fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

/// Validate a transcript and collect what the server advertised. `revision`
/// indexes [`REVISIONS`].
pub fn inspect(src: &str, revision: usize) -> Result<Inspection, String> {
    let mut out = Inspection::default();
    let messages = read(src, &mut out.findings);
    if messages.is_empty() && out.findings.is_empty() {
        return Err("Paste a transcript: JSON-RPC messages as JSON, JSON Lines or one after another".into());
    }
    out.messages = messages.len();

    // The revision the peers agreed on is whatever the initialize result says.
    let init_id = messages.iter().find(|m| m.value.get("method").and_then(Value::as_str) == Some("initialize")).and_then(|m| m.value.get("id").map(id_key));
    let negotiated = init_id.as_ref().and_then(|id| {
        messages.iter().find(|m| m.value.get("result").is_some() && m.value.get("id").map(id_key).as_ref() == Some(id)).and_then(|m| text(&m.value["result"], "protocolVersion"))
    });
    let chosen = REVISIONS.get(revision).filter(|_| revision > 0).map(|r| r.to_string());
    let known = |r: &str| REVISIONS[1..].contains(&r);
    let rev = chosen.clone().or_else(|| negotiated.clone().filter(|r| known(r))).unwrap_or_else(|| LATEST.to_string());
    out.revision = Some(rev.clone());
    let since = |first: &str| rev.as_str() >= first;

    // Requests waiting for their response, keyed by who sent them and the id.
    let mut open: HashMap<(bool, String), (String, usize)> = HashMap::new();
    let mut counts: Vec<(String, usize)> = Vec::new();
    // 0 nothing yet · 1 initialize sent · 2 initialize answered · 3 initialized
    let mut phase = 0;
    let mut server_caps: Option<Value> = None;
    let mut client_caps: Option<Value> = None;
    let mut findings = std::mem::take(&mut out.findings);
    let mut flag = |line: usize, level: Level, text: String| findings.push(Finding { line, level, text });

    for m in &messages {
        let line = m.line;
        let Value::Object(msg) = &m.value else {
            flag(line, Level::Error, "A JSON-RPC message must be an object".into());
            continue;
        };
        if m.batched && since("2025-06-18") {
            flag(line, Level::Error, format!("JSON-RPC batches (arrays of messages) were removed in {rev}"));
        }
        if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            flag(line, Level::Error, "Missing \"jsonrpc\": \"2.0\"".into());
        }
        let id = msg.get("id");
        if let Some(id) = id {
            if !(id.is_string() || id.is_number()) {
                flag(line, Level::Error, "The id must be a string or a number".into());
            }
        }
        match msg.get("method") {
            Some(Value::String(method)) => {
                let method = method.as_str();
                if msg.contains_key("result") || msg.contains_key("error") {
                    flag(line, Level::Error, format!("{method}: a request cannot also carry a result or an error"));
                }
                if msg.get("params").is_some_and(|p| !p.is_object()) {
                    flag(line, Level::Error, format!("{method}: params must be an object"));
                }
                match counts.iter_mut().find(|(n, _)| n.as_str() == method) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((method.to_string(), 1)),
                }
                let Some(id) = id else {
                    // A notification.
                    match NOTIFICATIONS.iter().find(|(n, _)| *n == method) {
                        Some((_, first)) if !since(first) => flag(line, Level::Warning, format!("{method} was added in {first}, after {rev}")),
                        Some(_) => {}
                        None if CLIENT_REQUESTS.iter().chain(SERVER_REQUESTS).any(|(n, _, _)| *n == method) => {
                            flag(line, Level::Error, format!("{method} is a request and needs an id"));
                        }
                        None => flag(line, Level::Warning, format!("{method} is not a notification of MCP {rev}")),
                    }
                    if method == "notifications/initialized" {
                        if phase < 2 {
                            flag(line, Level::Error, "notifications/initialized came before the initialize response".into());
                        }
                        phase = phase.max(3);
                    }
                    continue;
                };
                let from_server = SERVER_REQUESTS.iter().any(|(n, _, _)| *n == method);
                let spec = CLIENT_REQUESTS.iter().chain(SERVER_REQUESTS).find(|(n, _, _)| *n == method);
                match spec {
                    Some((_, _, first)) if !since(first) => flag(line, Level::Warning, format!("{method} was added in {first}, after {rev}")),
                    Some((_, Some(cap), _)) => {
                        let (declared, who) = if from_server { (&client_caps, "client") } else { (&server_caps, "server") };
                        // Only once the capabilities are known can their absence mean something.
                        if declared.as_ref().is_some_and(|c| c.get(*cap).is_none()) {
                            flag(line, Level::Warning, format!("{method} was sent although the {who} did not declare the {cap} capability"));
                        }
                    }
                    Some(_) => {}
                    None if NOTIFICATIONS.iter().any(|(n, _)| *n == method) => flag(line, Level::Error, format!("{method} is a notification and must not have an id")),
                    None => flag(line, Level::Warning, format!("{method} is not a method of MCP {rev}")),
                }
                if method == "initialize" {
                    if phase > 0 {
                        flag(line, Level::Error, "initialize was sent more than once".into());
                    }
                    phase = phase.max(1);
                    let params = msg.get("params").cloned().unwrap_or(Value::Null);
                    for field in ["protocolVersion", "capabilities", "clientInfo"] {
                        if params.get(field).is_none() {
                            flag(line, Level::Error, format!("initialize: params.{field} is missing"));
                        }
                    }
                    out.client = party(params.get("clientInfo"));
                    out.client_caps = capabilities(params.get("capabilities"));
                    client_caps = params.get("capabilities").cloned();
                } else if method != "ping" && init_id.is_some() {
                    match phase {
                        0 | 1 => flag(line, Level::Error, format!("{method} was sent before initialization finished")),
                        2 if !from_server => flag(line, Level::Warning, format!("{method} was sent before notifications/initialized")),
                        _ => {}
                    }
                }
                if open.insert((from_server, id_key(id)), (method.to_string(), line)).is_some() {
                    flag(line, Level::Error, format!("{method} reuses id {} while an earlier request with it is still open", id_key(id)));
                }
            }
            Some(_) => flag(line, Level::Error, "The method must be a string".into()),
            None => {
                // A response.
                let (result, error) = (msg.get("result"), msg.get("error"));
                match (result, error) {
                    (Some(_), Some(_)) => flag(line, Level::Error, "A response carries either a result or an error, not both".into()),
                    (None, None) => {
                        flag(line, Level::Error, "Neither a request (no method) nor a response (no result or error)".into());
                        continue;
                    }
                    _ => {}
                }
                let key = id.map(id_key).unwrap_or_default();
                let request = open.remove(&(false, key.clone())).or_else(|| open.remove(&(true, key.clone())));
                let Some((method, _)) = request else {
                    flag(line, Level::Error, format!("Response to id {key}, which no open request has"));
                    continue;
                };
                if let Some(e) = error {
                    let code = e.get("code").and_then(Value::as_i64);
                    let message = text(e, "message").unwrap_or_default();
                    if code.is_none() || e.get("message").and_then(Value::as_str).is_none() {
                        flag(line, Level::Error, format!("{method}: an error needs an integer code and a string message"));
                    }
                    out.rpc_errors.push(Finding { line, level: Level::Error, text: format!("{method} failed with {}: {message}", code.map(|c| c.to_string()).unwrap_or_else(|| "no code".into())) });
                    if method == "initialize" {
                        phase = phase.max(2);
                    }
                    continue;
                }
                let result = result.unwrap_or(&Value::Null);
                let catalog = |key: &str, into: &mut Vec<Entry>, entry: fn(&Value) -> Result<Entry, String>, flag: &mut dyn FnMut(usize, Level, String)| match result.get(key).and_then(Value::as_array) {
                    None => flag(line, Level::Error, format!("{method}: the result has no {key} array")),
                    Some(items) => {
                        for item in items {
                            match entry(item) {
                                // A later page or a re-list replaces what was known.
                                Ok(e) => match into.iter_mut().find(|old| old.name == e.name) {
                                    Some(old) => *old = e,
                                    None => into.push(e),
                                },
                                Err(why) => flag(line, Level::Error, format!("{method}: {why}")),
                            }
                        }
                    }
                };
                match method.as_str() {
                    "initialize" => {
                        phase = phase.max(2);
                        for field in ["protocolVersion", "capabilities", "serverInfo"] {
                            if result.get(field).is_none() {
                                flag(line, Level::Error, format!("initialize: result.{field} is missing"));
                            }
                        }
                        out.server = party(result.get("serverInfo"));
                        out.instructions = text(result, "instructions").filter(|i| !i.trim().is_empty());
                        out.server_caps = capabilities(result.get("capabilities"));
                        server_caps = result.get("capabilities").cloned();
                    }
                    "tools/list" => catalog("tools", &mut out.tools, tool_entry, &mut flag),
                    "resources/list" => catalog("resources", &mut out.resources, resource_entry, &mut flag),
                    "resources/templates/list" => catalog("resourceTemplates", &mut out.resources, resource_entry, &mut flag),
                    "prompts/list" => catalog("prompts", &mut out.prompts, prompt_entry, &mut flag),
                    _ => {}
                }
            }
        }
    }

    let mut unanswered: Vec<(String, usize)> = open.into_values().collect();
    unanswered.sort_by_key(|(_, line)| *line);
    for (method, line) in unanswered {
        flag(line, Level::Warning, format!("{method} never got a response"));
    }
    if init_id.is_none() {
        flag(1, Level::Warning, format!("There is no initialize request, so the transcript was checked as MCP {rev}"));
    } else if phase == 2 {
        flag(1, Level::Warning, "The client never sent notifications/initialized".into());
    }
    if let Some(n) = &negotiated {
        if !known(n) {
            flag(1, Level::Error, format!("The server answered with protocol revision {n}, which is not a known MCP revision"));
        } else if chosen.as_ref().is_some_and(|c| c != n) {
            flag(1, Level::Warning, format!("The transcript negotiated {n} but was checked against {rev}"));
        }
    }
    findings.sort_by_key(|f| f.line);
    out.findings = findings;
    out.calls = counts.into_iter().filter(|(m, _)| m != "initialize" && m != "notifications/initialized").collect();
    Ok(out)
}

// ---------------------------------------------------------------- sanitizing

const SECRET_KEYS: &[&str] = &["authorization", "password", "passwd", "secret", "token", "apikey", "api_key", "api-key", "cookie", "credential", "private_key", "privatekey", "access_key", "accesskey", "session"];
const SECRET_PREFIXES: &[&str] = &["sk-", "sk_live_", "sk_test_", "ghp_", "gho_", "ghs_", "github_pat_", "xoxb-", "xoxp-", "xoxa-", "AKIA", "AIza", "glpat-", "-----BEGIN"];
const REDACTED: &str = "[REDACTED]";

/// Whether a string by itself reads as a credential.
fn looks_secret(s: &str) -> bool {
    let t = s.trim();
    let lower = t.to_lowercase();
    let jwt = t.starts_with("eyJ") && t.split('.').count() == 3 && !t.contains(' ');
    lower.starts_with("bearer ") || lower.starts_with("basic ") || jwt || SECRET_PREFIXES.iter().any(|p| t.starts_with(p))
}

/// Hide `user:password@` and secret-looking query parameters inside a URL.
fn redact_url(s: &str, n: &mut usize) -> Option<String> {
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty() || rest.contains(char::is_whitespace) {
        return None;
    }
    let mut out = rest.to_string();
    let authority_end = out.find(['/', '?', '#']).unwrap_or(out.len());
    if let Some(at) = out[..authority_end].rfind('@') {
        out.replace_range(..at, REDACTED);
        *n += 1;
    }
    if let Some(q) = out.find('?') {
        let end = out.find('#').unwrap_or(out.len());
        let query: Vec<String> = out[q + 1..end]
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) if !v.is_empty() && SECRET_KEYS.iter().any(|s| k.to_lowercase().contains(s)) => {
                    *n += 1;
                    format!("{k}={REDACTED}")
                }
                _ => pair.to_string(),
            })
            .collect();
        out.replace_range(q + 1..end, &query.join("&"));
    }
    (out != rest).then(|| format!("{scheme}://{out}"))
}

fn redact(v: &mut Value, n: &mut usize) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                let key = k.to_lowercase();
                let secret_key = SECRET_KEYS.iter().any(|s| key.contains(s));
                // Only strings: a schema names a password field without holding one.
                if secret_key && x.is_string() {
                    *x = REDACTED.into();
                    *n += 1;
                } else {
                    redact(x, n);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| redact(x, n)),
        Value::String(s) => {
            if looks_secret(s) {
                *s = REDACTED.into();
                *n += 1;
            } else if let Some(clean) = redact_url(s, n) {
                *s = clean;
            }
        }
        _ => {}
    }
}

/// The transcript as JSON Lines with likely credentials replaced, and how
/// many values were replaced.
pub fn sanitize(src: &str) -> Result<(String, usize), String> {
    let mut ignored = Vec::new();
    let messages = read(src, &mut ignored);
    if messages.is_empty() {
        return Err("There are no JSON-RPC messages to export".into());
    }
    let mut n = 0;
    let lines: Vec<String> = messages
        .into_iter()
        .map(|mut m| {
            redact(&mut m.value, &mut n);
            m.value.to_string()
        })
        .collect();
    Ok((lines.join("\n"), n))
}

// ---------------------------------------------------------------- live discovery

const TIMEOUT: Duration = Duration::from_secs(20);
/// Listing stops after this many pages of one catalog.
const MAX_PAGES: usize = 25;

/// The JSON-RPC response to `id` in an HTTP body: plain JSON, or the `data:`
/// lines of an event stream.
fn response_to(body: &[u8], id: u64) -> Option<Value> {
    let text = std::str::from_utf8(body).ok()?;
    let is_ours = |v: &Value| v.get("id").and_then(Value::as_u64) == Some(id) && (v.get("result").is_some() || v.get("error").is_some());
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return match v {
            Value::Array(a) => a.into_iter().find(|x| is_ours(x)),
            v => is_ours(&v).then_some(v),
        };
    }
    text.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok()).find(|v| is_ours(v))
}

struct Session {
    url: String,
    authorization: Option<String>,
    id: Option<String>,
    revision: Option<String>,
    next: u64,
    /// Everything sent and received, in order.
    transcript: Vec<String>,
}

impl Session {
    fn request(&self, method: &str, body: Option<String>) -> Request {
        let mut headers = vec![("Accept".to_string(), "application/json, text/event-stream".to_string())];
        if body.is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        if let Some(a) = &self.authorization {
            headers.push(("Authorization".into(), a.clone()));
        }
        if let Some(s) = &self.id {
            headers.push(("Mcp-Session-Id".into(), s.clone()));
        }
        if let Some(r) = &self.revision {
            headers.push(("MCP-Protocol-Version".into(), r.clone()));
        }
        Request { name: None, method: method.into(), url: self.url.clone(), headers, body }
    }

    /// Send one request and wait for its response.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        let mut message = Map::new();
        message.insert("jsonrpc".into(), "2.0".into());
        message.insert("id".into(), id.into());
        message.insert("method".into(), method.into());
        if !params.is_null() {
            message.insert("params".into(), params);
        }
        let body = Value::Object(message).to_string();
        self.transcript.push(body.clone());
        let resp = http::send_until(&self.request("POST", Some(body)), TIMEOUT, &mut |seen| response_to(seen, id).is_some())?;
        if self.id.is_none() {
            self.id = resp.header("mcp-session-id").map(str::to_string);
        }
        if !(200..300).contains(&resp.status) {
            let hint = match resp.status {
                401 | 403 => " — the server wants credentials; add an Authorization header",
                404 | 405 => " — is this the server's MCP endpoint? Only the Streamable HTTP transport is supported",
                _ => "",
            };
            return Err(format!("{method}: the server answered HTTP {} {}{hint}", resp.status, resp.reason));
        }
        let answer = response_to(&resp.body, id).ok_or(format!("{method}: the server's reply held no JSON-RPC response"))?;
        self.transcript.push(answer.to_string());
        Ok(answer)
    }
}

/// Initialize against a Streamable HTTP endpoint and list what it advertises.
/// Read-only: no tool is called, no resource read and no prompt fetched.
/// Returns the exchange as a JSON Lines transcript.
pub fn discover(url: &str, authorization: Option<&str>) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Type the server's http:// or https:// endpoint".into());
    }
    let mut s = Session { url: url.to_string(), authorization: authorization.map(str::to_string).filter(|a| !a.trim().is_empty()), id: None, revision: None, next: 1, transcript: Vec::new() };
    let init = s.call(
        "initialize",
        json!({"protocolVersion": LATEST, "capabilities": {}, "clientInfo": {"name": "SideKit", "version": env!("CARGO_PKG_VERSION")}}),
    )?;
    if let Some(result) = init.get("result") {
        s.revision = text(result, "protocolVersion");
        let done = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
        s.transcript.push(done.clone());
        http::send(&s.request("POST", Some(done)), TIMEOUT)?;

        // Only what the server says it has is asked for.
        let caps = result.get("capabilities").cloned().unwrap_or(Value::Null);
        let lists = [("tools", "tools/list"), ("resources", "resources/list"), ("resources", "resources/templates/list"), ("prompts", "prompts/list")];
        for (_, method) in lists.iter().filter(|(cap, _)| caps.get(*cap).is_some()) {
            let mut cursor: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let params = cursor.take().map(|c| json!({"cursor": c})).unwrap_or(Value::Null);
                let page = s.call(method, params)?;
                cursor = page.get("result").and_then(|r| text(r, "nextCursor")).filter(|c| !c.is_empty());
                if cursor.is_none() {
                    break;
                }
            }
        }
    }
    // Let the server drop the session; it may not support that, which is fine.
    if s.id.is_some() {
        let _ = http::send(&s.request("DELETE", None), Duration::from_secs(5));
    }
    Ok(s.transcript.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{"roots":{"listChanged":true}},"clientInfo":{"name":"inspector","version":"1.0"}}}
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true},"logging":{}},"serverInfo":{"name":"files","version":"0.3.1"},"instructions":"Use absolute paths."}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read_file","description":"Read a file","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"integer"}},"required":["path"]},"annotations":{"readOnlyHint":true,"openWorldHint":false}},{"name":"delete_file","inputSchema":{"type":"object","properties":{"path":{"type":"string"}}},"annotations":{}}]}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/etc/hosts"}}}
{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"Unknown path"}}"#;

    #[test]
    fn inspects_a_clean_session() {
        let i = inspect(TRANSCRIPT, 0).unwrap();
        assert_eq!(i.messages, 7);
        assert!(i.findings.is_empty(), "{:?}", i.findings);
        assert_eq!(i.revision.as_deref(), Some("2025-06-18"));
        assert_eq!((i.client.as_deref(), i.server.as_deref()), (Some("inspector 1.0"), Some("files 0.3.1")));
        assert_eq!(i.instructions.as_deref(), Some("Use absolute paths."));
        assert_eq!(i.client_caps, ["roots (listChanged)"]);
        assert_eq!(i.server_caps, ["tools (listChanged)", "logging"]);
        assert_eq!(i.tools.len(), 2);
        assert_eq!(i.tools[0].facts, [("Parameters", "path: string, required · limit: integer".to_string()), ("Hints", "read-only".to_string())]);
        assert_eq!(i.tools[1].facts[1], ("Hints", "may be destructive · reaches outside systems".to_string()));
        assert_eq!(i.rpc_errors, [Finding { line: 7, level: Level::Error, text: "tools/call failed with -32602: Unknown path".into() }]);
        assert_eq!(i.calls, [("tools/list".to_string(), 1), ("tools/call".to_string(), 1)]);
    }

    fn problems(src: &str, revision: usize) -> Vec<(usize, String)> {
        inspect(src, revision).unwrap().findings.into_iter().map(|f| (f.line, f.text)).collect()
    }

    #[test]
    fn ties_problems_to_lines() {
        let src = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"c"}}}
not json at all
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"s"}}}
{"id":5,"method":"prompts/list"}
{"jsonrpc":"2.0","id":9,"result":{}}
{"jsonrpc":"2.0","id":null,"method":"ping"}
{"jsonrpc":"2.0","method":"tools/call"}
{"jsonrpc":"2.0","id":6,"method":"vendor/custom","params":[1]}"#;
        let got = problems(src, 0);
        let at = |line: usize| got.iter().filter(|f| f.0 == line).map(|f| f.1.as_str()).collect::<Vec<_>>();
        assert_eq!(at(1), ["tools/list was sent before initialization finished", "The client never sent notifications/initialized"]);
        assert_eq!(at(2), ["initialize: params.protocolVersion is missing", "initialize reuses id 1 while an earlier request with it is still open"]);
        assert_eq!(at(3), ["Not valid JSON (expected ident): not json at all"]);
        assert!(at(4).is_empty());
        assert_eq!(at(5), ["Missing \"jsonrpc\": \"2.0\"", "prompts/list was sent although the server did not declare the prompts capability", "prompts/list was sent before notifications/initialized", "prompts/list never got a response"]);
        assert_eq!(at(6), ["Response to id 9, which no open request has"]);
        assert_eq!(at(7), ["The id must be a string or a number", "ping never got a response"]);
        assert_eq!(at(8), ["tools/call is a request and needs an id"]);
        assert_eq!(at(9), ["vendor/custom: params must be an object", "vendor/custom is not a method of MCP 2025-06-18", "vendor/custom was sent before notifications/initialized", "vendor/custom never got a response"]);
    }

    #[test]
    fn revisions_and_catalog_shape() {
        // Checked against an older revision than the one negotiated.
        let got = problems(TRANSCRIPT, 4);
        assert_eq!(got, [(1, "The transcript negotiated 2025-06-18 but was checked against 2024-11-05".to_string())]);
        let batch = r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","id":2,"method":"elicitation/create"}]
[{"jsonrpc":"2.0","id":1,"result":{}},{"jsonrpc":"2.0","id":2,"result":{}}]"#;
        let old = problems(batch, 3);
        assert_eq!(old[0], (1, "elicitation/create was added in 2025-06-18, after 2025-03-26".to_string()));
        assert_eq!(old[1], (1, "There is no initialize request, so the transcript was checked as MCP 2025-03-26".to_string()));
        assert_eq!(old.len(), 2);
        assert!(problems(batch, 2).iter().filter(|f| f.1.starts_with("JSON-RPC batches")).count() == 4);

        let bad = r#"[
  {"jsonrpc":"2.0","id":1,"method":"tools/list"},
  {"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"ok","inputSchema":{"type":"object"}},{"description":"nameless"},{"name":"x"}]}},
  {"jsonrpc":"2.0","id":2,"method":"resources/list"},
  {"message": {"jsonrpc":"2.0","id":2,"result":{"resources":[{"name":"readme","uri":"file:///readme.md","mimeType":"text/markdown","size":2048}]}}},
  {"jsonrpc":"2.0","id":3,"method":"prompts/list"},
  {"jsonrpc":"2.0","id":3,"result":{"nope":[]}}
]"#;
        let i = inspect(bad, 2).unwrap();
        // A file that is one array keeps each message's own line and is not a batch.
        let texts: Vec<(usize, &str)> = i.findings.iter().map(|f| (f.line, f.text.as_str())).collect();
        assert_eq!(texts[1..], [(3, "tools/list: a tool has no name"), (3, "tools/list: tool \"x\" has no inputSchema object"), (7, "prompts/list: the result has no prompts array")]);
        assert_eq!(i.tools.len(), 1);
        assert_eq!(i.resources[0].facts, [("URI", "file:///readme.md".to_string()), ("Type", "text/markdown".to_string()), ("Size", "2,048 bytes".to_string())]);
        assert!(inspect("   ", 0).is_err());
    }

    #[test]
    fn sanitizes_credentials() {
        let src = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fetch","arguments":{"url":"https://ada:hunter2@api.example.com/v1?api_key=abc123&page=2","headers":{"Authorization":"Bearer eyJhbGciOi.payload.sig","X-Trace":"t-1"},"note":"sk-live-1234567890"}}}
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}],"session_token":"s3cr3t","inputSchema":{"properties":{"password":{"type":"string"}}}}}"#;
        let (clean, n) = sanitize(src).unwrap();
        assert_eq!(n, 5);
        let lines: Vec<&str> = clean.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"url\":\"https://[REDACTED]@api.example.com/v1?api_key=[REDACTED]&page=2\""), "{clean}");
        assert!(lines[0].contains("\"Authorization\":\"[REDACTED]\"") && lines[0].contains("\"X-Trace\":\"t-1\"") && lines[0].contains("\"note\":\"[REDACTED]\""));
        // A schema that merely names a password field is left alone.
        assert!(lines[1].contains("\"session_token\":\"[REDACTED]\"") && lines[1].contains("\"password\":{\"type\":\"string\"}"));
        assert!(!clean.contains("hunter2") && !clean.contains("abc123") && !clean.contains("s3cr3t"));
        // What comes out is still a transcript.
        assert_eq!(inspect(&clean, 0).unwrap().messages, 2);
    }

    #[test]
    fn reads_event_streams() {
        assert_eq!(response_to(b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{}}\n\n", 4).unwrap()["id"], 4);
        assert!(response_to(b"data: {\"jsonrpc\":\"2.0\",\"id\":4,\"res", 4).is_none());
        assert!(response_to(b"{\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{}}", 4).is_none());
        let sse = "event: message\nid: 7\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n: keep-alive\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        let i = inspect(sse, 2).unwrap();
        assert_eq!(i.messages, 2);
        assert_eq!(i.findings.len(), 1);
    }

    /// A fake Streamable HTTP server: answers `replies.len()` requests in
    /// order and returns what it was sent.
    fn serve(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut head, mut length) = (String::new(), 0);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(format!("{head}\n{}", String::from_utf8(body).unwrap()));
                stream.write_all(reply.as_bytes()).unwrap();
            }
            seen
        });
        (url, handle)
    }

    fn reply(kind: &str, extra: &str, body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    #[test]
    fn discovers_only_what_is_advertised() {
        let (url, server) = serve(vec![
            reply("application/json", "Mcp-Session-Id: abc\r\n", r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"demo","version":"1"}}}"#),
            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            // The first page arrives as an event stream.
            reply("text/event-stream", "", "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"a\",\"inputSchema\":{\"type\":\"object\"}}],\"nextCursor\":\"p2\"}}\n\n"),
            reply("application/json", "", r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"b","inputSchema":{"type":"object"}}]}}"#),
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
        ]);
        let transcript = discover(&url, Some("Bearer t0ken")).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 5);
        let lower: Vec<String> = seen.iter().map(|s| s.to_lowercase()).collect();
        assert!(seen[0].starts_with("POST /mcp HTTP/1.1") && lower[0].contains("authorization: bearer t0ken") && !lower[0].contains("mcp-session-id"));
        assert!(lower[1].contains("mcp-session-id: abc") && lower[1].contains("mcp-protocol-version: 2025-03-26") && seen[1].contains("notifications/initialized"));
        assert!(seen[2].contains("\"method\":\"tools/list\"") && !seen[2].contains("cursor"));
        assert!(seen[3].contains("\"params\":{\"cursor\":\"p2\"}"));
        assert!(seen[4].starts_with("DELETE /mcp"));
        // Nothing but initialize and the one advertised catalog was requested.
        assert!(!seen.iter().any(|s| s.contains("resources/") || s.contains("prompts/") || s.contains("tools/call")));

        let i = inspect(&transcript, 0).unwrap();
        assert!(i.findings.is_empty(), "{:?}", i.findings);
        assert_eq!(i.server.as_deref(), Some("demo 1"));
        assert_eq!(i.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);

        let (url, server) = serve(vec!["HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()]);
        assert!(discover(&url, None).unwrap_err().contains("HTTP 401"));
        server.join().unwrap();
        assert!(discover("stdio://server", None).is_err());
    }
}
