//! Run requests written in the `.http` format. Nothing is sent until Send is
//! pressed, and nothing is kept afterwards: no cookies, cache or history.

use std::time::Duration;

use gpui_kit::component::input::EditorState;
use gpui_kit::{
    Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder, px,
};

use super::*;
use super::codegen::notes_el;
use crate::logic::http::{self, HttpFile, Request, Response};
use crate::logic::{plural, thousands};
use crate::theme::MONO_FONT;
use crate::ui::{self, BtnKind, Tone};

const SAMPLE: &str = "@host = https://httpbin.org\n@token = demo-token\n\n### Fetch a resource\nGET {{host}}/get?tool=sidekit\nAccept: application/json\n\n### Create a resource\n# @name create\nPOST {{host}}/post\nContent-Type: application/json\nAuthorization: Bearer {{token}}\nX-Request-Id: {{$uuid}}\n\n{\n  \"name\": \"Ada Lovelace\",\n  \"roles\": [\"admin\"]\n}\n";

const TIMEOUT: Duration = Duration::from_secs(60);
const TABS: &[&str] = &["Body", "Headers", "Console"];

/// One finished exchange: what went out and what came back.
struct Exchange {
    request: Request,
    /// Placeholders nothing defined; they went out as written.
    unresolved: Vec<String>,
    result: Result<Response, String>,
}

pub struct HttpView {
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    file: HttpFile,
    selected: usize,
    tab: usize,
    last: Option<Exchange>,
    sending: bool,
    task: Option<Task<()>>,
    wrap: bool,
    _subs: Vec<Subscription>,
}

impl HttpView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sample = std::env::var("SIDEKIT_HTTP").map(|s| s.replace("\\n", "\n")).unwrap_or_else(|_| SAMPLE.to_string()); // TEST-HOOK
        let input = code_editor(&sample, "GET https://example.com", "plaintext", window, cx);
        let output = code_editor("", "", "json", window, cx);
        let subs = vec![watch(&input, window, cx, |this: &mut Self, _, cx| this.reparse(cx)), watch(&output, window, cx, |_, _, _| {})];
        let mut this = Self { input, output, file: HttpFile::default(), selected: 0, tab: 0, last: None, sending: false, task: None, wrap: false, _subs: subs };
        this.reparse(cx);
        this
    }

    fn reparse(&mut self, cx: &mut Context<Self>) {
        self.file = http::parse_http(&text_of(&self.input, cx));
        self.selected = self.selected.min(self.file.requests.len().saturating_sub(1));
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.file.requests.get(self.selected) else { return };
        let (request, unresolved) = http::resolve(request, &self.file.variables);
        self.sending = true;
        let outgoing = request.clone();
        // Replacing the task drops a request still in flight: its answer is no longer wanted.
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { http::send(&outgoing, TIMEOUT) }).await;
            let _ = this.update_in(cx, |v, w, cx| {
                v.sending = false;
                v.last = Some(Exchange { request, unresolved, result });
                v.show(w, cx);
            });
        }));
        cx.notify();
    }

    /// Put the chosen tab of the last response into the output pane.
    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, lang) = match &self.last {
            None => (String::new(), "plaintext"),
            Some(x) => match (self.tab, &x.result) {
                (0, Ok(r)) => r.display_body(),
                (1, Ok(r)) => (r.headers.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"), "plaintext"),
                (2, _) => (console(x), "plaintext"),
                // A failed request has no body or headers; its error shows above the pane.
                _ => (String::new(), "plaintext"),
            },
        };
        let shown = big::for_display(&text);
        big::fit_language(&self.output, lang, text.len(), cx);
        set_text(&self.output, shown.as_deref().unwrap_or(&text), window, cx);
        cx.notify();
    }
}

/// The exchange as a transcript: `>` lines went out, `<` lines came back.
fn console(x: &Exchange) -> String {
    let r = &x.request;
    let mut out = format!("> {} {}\n", r.method, r.url);
    out.extend(r.headers.iter().map(|(k, v)| format!("> {k}: {v}\n")));
    if let Some(b) = &r.body {
        out.push_str(">\n");
        out.extend(b.lines().map(|l| format!("> {l}\n")));
    }
    out.push('\n');
    match &x.result {
        Ok(resp) => {
            out.push_str(&format!("< {} {} {}\n", resp.version, resp.status, resp.reason));
            out.extend(resp.headers.iter().map(|(k, v)| format!("< {k}: {v}\n")));
            out.push_str(&format!("\nCompleted in {} ms · {}\n", thousands(resp.elapsed.as_millis() as u64), plural(resp.body.len(), "byte")));
            if resp.truncated {
                out.push_str("The body was longer than 16 MiB; the rest was not read.\n");
            }
        }
        Err(e) => out.push_str(&format!("Failed: {e}\n")),
    }
    for name in &x.unresolved {
        out.push_str(&format!("Warning: {{{{{name}}}}} is not defined and was sent as written.\n"));
    }
    out
}

impl Render for HttpView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        sync_wrap(&[&self.input, &self.output], &mut self.wrap, window, cx);
        let [paste, clear] = paste_clear("http", &self.input, &pal, cx, |this: &mut Self, _, cx| this.reparse(cx));
        let zoom = ui::PaneZoom::new("http-output", window, cx);
        let on_tab = on_index(cx, |this: &mut Self, i, w, cx| {
            this.tab = i;
            this.show(w, cx);
        });

        let chips = self.file.requests.iter().enumerate().map(|(i, r)| {
            let on = i == self.selected;
            div()
                .id(("http-req", i))
                .flex()
                .items_center()
                .h(px(28.))
                .px(px(12.))
                .rounded(px(14.))
                .border_1()
                .cursor_pointer()
                .font_family(MONO_FONT)
                .text_size(px(12.))
                .when(on, |d| d.bg(pal.accent_soft).border_color(pal.accent).text_color(pal.accent).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |d| d.border_color(pal.stroke).text_color(pal.text2).hover(move |s| s.bg(pal.subtle)))
                .child(crate::logic::clip(&r.label(), 48))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = i;
                    cx.notify();
                }))
        }).collect::<Vec<_>>();

        // What would go out right now, for the cURL copy and the placeholder warnings.
        let resolved = self.file.requests.get(self.selected).map(|r| http::resolve(r, &self.file.variables));
        let curl: SharedString = resolved.as_ref().map(|(r, _)| http::to_curl(r)).unwrap_or_default().into();
        let mut notes = self.file.warnings.clone();
        if let Some((_, missing)) = &resolved {
            notes.extend(missing.iter().map(|m| format!("{{{{{m}}}}} is not defined. It will be sent as written.")));
        }

        let (badge, summary) = match &self.last {
            _ if self.sending => (Some(("Sending…".to_string(), Tone::Info)), String::new()),
            Some(Exchange { result: Ok(r), .. }) => {
                let tone = if r.status < 400 { Tone::Ok } else { Tone::Err };
                (Some((format!("{} {}", r.status, r.reason).trim().to_string(), tone)), format!("{} ms · {}", thousands(r.elapsed.as_millis() as u64), plural(r.body.len(), "byte")))
            }
            Some(Exchange { result: Err(_), .. }) => (Some(("Failed".to_string(), Tone::Err)), String::new()),
            None => (None, "Nothing has been sent yet".to_string()),
        };
        let failure = self.last.as_ref().and_then(|x| x.result.as_ref().err().cloned()).filter(|_| !self.sending && self.tab != 2);
        let copy_text: SharedString = text_of(&self.output, cx).into();
        let has_request = resolved.is_some();

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(ui::section_label("Requests", &pal).flex_1())
                    .when(has_request, |d| {
                        d.child(ui::copy_btn_labeled("http-curl", "Copy as cURL", curl, &pal, window, cx)).child(ui::btn(
                            "http-send",
                            Some("send"),
                            if self.sending { "Sending…" } else { "Send" },
                            BtnKind::Accent,
                            &pal,
                            cx.listener(|this, _, w, cx| this.send(w, cx)),
                        ))
                    }),
            )
            .child(match chips.is_empty() {
                true => div().text_size(px(13.)).text_color(pal.text3).child("Write a request: a URL, or a method and a URL, then its headers."),
                false => div().flex().flex_wrap().gap(px(6.)).children(chips),
            })
            .when(!notes.is_empty(), |d| d.child(notes_el(&notes, &pal)))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .flex_1()
                    .min_h(px(400.))
                    .mt(px(8.))
                    .child(
                        ui::pane(is_focused(&self.input, window, cx), &pal)
                            .flex_1()
                            .child(ui::pane_head(".http", None, &pal).child(paste).child(clear))
                            .child(code_editor_el(&self.input, false, cx))
                            .child(ui::pane_foot(&pal).child("### separates requests · @name = value defines {{name}}")),
                    )
                    .child(zoom.wrap(
                        ui::pane(is_focused(&self.output, window, cx), &pal)
                            .flex_1()
                            .child(
                                ui::pane_head("Response", None, &pal)
                                    .child(ui::seg("http-tabs", TABS, self.tab, &pal, on_tab).mr(px(6.)))
                                    .child(ui::copy_btn("http-copy", copy_text, &pal, window, cx))
                                    .child(zoom.button(&pal)),
                            )
                            .when_some(failure, |d, e| d.child(ui::err_box(e, &pal).m(px(12.))))
                            .child(code_editor_el(&self.output, true, cx))
                            .child(
                                ui::pane_foot(&pal)
                                    .when_some(badge, |d, (text, tone)| d.child(ui::badge(text, tone, &pal)))
                                    .child(summary),
                            ),
                        &pal,
                        window,
                    )),
            )
            .child(div().text_size(px(12.)).text_color(pal.text3).child("Requests leave this computer only when you press Send. Cookies, responses and history are not kept."))
    }
}
