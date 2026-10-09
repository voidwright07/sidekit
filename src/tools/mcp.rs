//! Inspect a Model Context Protocol server without running its tools: check
//! a transcript, or connect read-only and list what the server advertises.

use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, Window, div, prelude::FluentBuilder, px,
};

use super::*;
use crate::logic::mcp::{self, Entry, Finding, Inspection, Level};
use crate::logic::plural;
use crate::theme::MONO_FONT;
use crate::ui::{self, BtnKind, Tone};

const SAMPLE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{"roots":{"listChanged":true}},"clientInfo":{"name":"example-client","version":"1.4.0"}}}
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true},"resources":{}},"serverInfo":{"name":"files","version":"0.3.1"},"instructions":"Paths are relative to the workspace root."}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read_file","description":"Read a text file from the workspace.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"max_bytes":{"type":"integer"}},"required":["path"]},"annotations":{"readOnlyHint":true,"openWorldHint":false}},{"name":"delete_file","description":"Delete a file. This cannot be undone.","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]},"annotations":{"destructiveHint":true,"openWorldHint":false}}]}}
{"jsonrpc":"2.0","id":3,"method":"resources/list"}
{"jsonrpc":"2.0","id":3,"result":{"resources":[{"uri":"file:///workspace/README.md","name":"README","mimeType":"text/markdown","size":2048}]}}
{"jsonrpc":"2.0","id":4,"method":"prompts/list"}
{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"Method not found"}}
"#;

pub struct McpView {
    input: Entity<EditorState>,
    url: Entity<InputState>,
    auth: Entity<InputState>,
    /// Indexes `mcp::REVISIONS`.
    revision: usize,
    /// Connect to a server rather than paste a transcript.
    live: bool,
    report: Option<Result<Inspection, String>>,
    connecting: bool,
    connect_err: Option<String>,
    /// What "Copy sanitized" would put on the clipboard, and how many values it hides.
    sanitized: (SharedString, usize),
    task: Option<Task<()>>,
    wrap: bool,
    _subs: Vec<Subscription>,
}

impl McpView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = code_editor(SAMPLE, "Paste JSON-RPC messages: JSON, JSON Lines, or one after another", "json", window, cx);
        let url = line(&std::env::var("SIDEKIT_MCP_URL").unwrap_or_default(), "https://example.com/mcp", window, cx); // TEST-HOOK
        let auth = line("", "Authorization header, e.g. Bearer …  (optional)", window, cx);
        let subs = vec![watch(&input, window, cx, |this: &mut Self, _, cx| this.inspect(cx)), watch(&url, window, cx, |_, _, cx| cx.notify()), watch(&auth, window, cx, |_, _, cx| cx.notify())];
        let mut this = Self {
            input,
            url,
            auth,
            revision: 0,
            live: false,
            report: None,
            connecting: false,
            connect_err: None,
            sanitized: (SharedString::default(), 0),
            task: None,
            wrap: false,
            _subs: subs,
        };
        this.inspect(cx);
        this
    }

    fn inspect(&mut self, cx: &mut Context<Self>) {
        let src = text_of(&self.input, cx);
        if src.trim().is_empty() {
            (self.report, self.sanitized) = (None, (SharedString::default(), 0));
        } else {
            self.report = Some(mcp::inspect(&src, self.revision));
            self.sanitized = mcp::sanitize(&src).map(|(text, n)| (text.into(), n)).unwrap_or_default();
        }
        cx.notify();
    }

    /// Initialize against the endpoint and list what it advertises. The
    /// exchange lands in the transcript pane, where it is inspected like any other.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = line_text(&self.url, cx);
        let auth = line_text(&self.auth, cx);
        (self.connecting, self.connect_err) = (true, None);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { mcp::discover(&url, Some(auth.as_str())) }).await;
            let _ = this.update_in(cx, |v, w, cx| {
                v.connecting = false;
                match result {
                    Ok(transcript) => {
                        set_text(&v.input, &transcript, w, cx);
                        v.revision = 0;
                        v.inspect(cx);
                    }
                    Err(e) => v.connect_err = Some(e),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn heading(title: &'static str, count: Option<usize>, pal: &Pal) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .mt(px(16.))
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .when_some(count, |d, n| d.child(ui::badge(n.to_string(), Tone::Neutral, pal)))
}

fn findings_el(id: &'static str, findings: &[Finding], pal: &Pal) -> gpui_kit::Div {
    let mut card = ui::kv_card(pal);
    for (i, f) in findings.iter().enumerate() {
        let (label, tone) = match f.level {
            Level::Error => ("Error", Tone::Err),
            Level::Warning => ("Warning", Tone::Info),
        };
        card = card.child(
            ui::kv_row((id, i), i + 1 == findings.len(), pal)
                .child(div().w(px(64.)).flex_none().font_family(MONO_FONT).text_size(px(12.5)).text_color(pal.text3).child(format!("line {}", f.line)))
                .child(div().w(px(78.)).flex_none().flex().child(ui::badge(label, tone, pal)))
                .child(div().flex_1().min_w_0().py(px(4.)).text_size(px(13.)).child(f.text.clone())),
        );
    }
    card
}

/// What a server advertises under one heading: name, description and details.
fn catalog_el(entries: &[Entry], pal: &Pal) -> gpui_kit::Div {
    let p = *pal;
    let mut card = ui::kv_card(pal);
    for (i, e) in entries.iter().enumerate() {
        card = card.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .py(px(10.))
                .px(px(18.))
                .when(i + 1 < entries.len(), |d| d.border_b_1().border_color(p.stroke))
                .child(div().font_family(MONO_FONT).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(e.name.clone()))
                .when(!e.description.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(p.text2).child(e.description.clone())))
                .children(e.facts.iter().map(|(k, v)| {
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(12.5))
                        .child(div().w(px(90.)).flex_none().text_color(p.text3).child(*k))
                        .child(div().flex_1().min_w_0().font_family(MONO_FONT).text_color(p.text2).child(v.clone()))
                })),
        );
    }
    card
}

impl Render for McpView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        sync_wrap(&[&self.input], &mut self.wrap, window, cx);
        let live = self.live;
        let on_source = on_index(cx, |this: &mut Self, i, _, cx| {
            this.live = i == 1;
            cx.notify();
        });
        let on_revision = on_index(cx, |this: &mut Self, i, _, cx| {
            this.revision = i;
            this.inspect(cx);
        });
        let [paste, clear] = paste_clear("mcp", &self.input, &pal, cx, |this: &mut Self, _, cx| this.inspect(cx));
        let zoom = ui::PaneZoom::new("mcp-transcript", window, cx);
        let (sanitized, hidden) = self.sanitized.clone();

        let mut results: Vec<AnyElement> = Vec::new();
        match &self.report {
            None => {}
            Some(Err(e)) => results.push(ui::err_box(e.clone(), &pal).mt(px(10.)).into_any_element()),
            Some(Ok(r)) => {
                let errors = r.findings.iter().filter(|f| f.level == Level::Error).count();
                let warnings = r.findings.len() - errors;
                let (verdict, tone) = match (errors, warnings) {
                    (0, 0) => ("Follows the protocol".to_string(), Tone::Ok),
                    (0, w) => (plural(w, "warning"), Tone::Info),
                    (e, _) => (plural(e, "protocol error"), Tone::Err),
                };
                let mut rows: Vec<(&str, String)> = vec![("Messages", r.messages.to_string())];
                rows.extend(r.revision.clone().map(|v| ("Revision", v)));
                rows.extend(r.server.clone().map(|v| ("Server", v)));
                rows.push(("Server capabilities", if r.server_caps.is_empty() { "none declared".into() } else { r.server_caps.join(" · ") }));
                rows.extend(r.client.clone().map(|v| ("Client", v)));
                rows.push(("Client capabilities", if r.client_caps.is_empty() { "none declared".into() } else { r.client_caps.join(" · ") }));
                rows.extend(r.instructions.clone().map(|v| ("Instructions", v)));
                if !r.calls.is_empty() {
                    rows.push(("Methods seen", r.calls.iter().map(|(m, n)| if *n == 1 { m.clone() } else { format!("{m} ×{n}") }).collect::<Vec<_>>().join(" · ")));
                }
                let mut summary = ui::kv_card(&pal);
                let n = rows.len();
                for (i, (k, v)) in rows.into_iter().enumerate() {
                    summary = summary.child(ui::kv_row(("mcp-sum", i), i + 1 == n, &pal).child(ui::kv_label(k, 160., &pal)).child(ui::kv_val(v, true)));
                }
                results.push(heading("Session", None, &pal).child(ui::badge(verdict, tone, &pal)).into_any_element());
                results.push(summary.into_any_element());
                if !r.findings.is_empty() {
                    results.push(heading("Protocol problems", Some(r.findings.len()), &pal).into_any_element());
                    results.push(findings_el("mcp-find", &r.findings, &pal).into_any_element());
                }
                for (title, entries) in [("Tools", &r.tools), ("Resources", &r.resources), ("Prompts", &r.prompts)] {
                    if !entries.is_empty() {
                        results.push(heading(title, Some(entries.len()), &pal).into_any_element());
                        results.push(catalog_el(entries, &pal).into_any_element());
                    }
                }
                if !r.rpc_errors.is_empty() {
                    results.push(heading("Errors the peers reported", Some(r.rpc_errors.len()), &pal).into_any_element());
                    results.push(findings_el("mcp-rpc", &r.rpc_errors, &pal).into_any_element());
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(ui::section_label("Configuration", &pal))
            .child(ui::setting(
                "mcp-source",
                ui::setting_icon("plug", &pal),
                "Source",
                Some("Check a recorded transcript, or ask a running server what it offers".into()),
                ui::seg("mcp-source-seg", &["Transcript", "Live connection"], live as usize, &pal, on_source),
                &pal,
            ))
            .when(!live, |d| {
                d.child(ui::setting(
                    "mcp-rev",
                    ui::setting_icon("version", &pal),
                    "Protocol revision",
                    Some("Negotiated uses the revision the server answered initialize with".into()),
                    ui::dropdown("mcp-rev-dd", mcp::REVISIONS, self.revision, &pal, window, cx, on_revision),
                    &pal,
                ))
            })
            .when(live, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(14.))
                        .rounded(px(7.))
                        .bg(pal.card)
                        .border_1()
                        .border_color(pal.stroke)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(field_el(&self.url, true, 36., 13., window, cx).flex_1())
                                .child(ui::btn(
                                    "mcp-connect",
                                    Some("plug"),
                                    if self.connecting { "Connecting…" } else { "Connect" },
                                    BtnKind::Accent,
                                    &pal,
                                    cx.listener(|this, _, w, cx| this.connect(w, cx)),
                                )),
                        )
                        .child(field_el(&self.auth, true, 32., 13., window, cx))
                        .child(div().text_size(px(12.)).text_color(pal.text3).child(
                            "Read-only: connects over Streamable HTTP, initializes, and lists the tools, resources and prompts the server advertises. No tools will be executed, no resources read and no prompts fetched. Nothing is contacted until you press Connect.",
                        )),
                )
                .when_some(self.connect_err.clone(), |d, e| d.child(ui::err_box(e, &pal)))
            })
            .child(zoom.wrap(
                ui::pane(is_focused(&self.input, window, cx), &pal)
                    .h(px(260.))
                    .mt(px(8.))
                    .child(
                        ui::pane_head("Transcript", None, &pal)
                            .when(!sanitized.is_empty(), |d| d.child(ui::copy_btn_labeled("mcp-clean", "Copy sanitized", sanitized.clone(), &pal, window, cx)))
                            .child(paste)
                            .child(clear)
                            .child(zoom.button(&pal)),
                    )
                    .child(code_editor_el(&self.input, false, cx))
                    .child(ui::pane_foot(&pal).child(match hidden {
                        0 => "Copy sanitized gives the transcript as JSON Lines; nothing in it looks like a credential".to_string(),
                        n => format!("Copy sanitized replaces {} that look like credentials", plural(n, "value")),
                    })),
                &pal,
                window,
            ))
            .children(results)
    }
}
