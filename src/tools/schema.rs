//! JSON Schema: infer one from samples, or check a document against one.

use gpui_kit::component::input::EditorState;
use gpui_kit::{
    Context, Entity, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::*;
use super::codegen::notes_el;
use crate::logic::schema::{self, InferOpts, Issue};
use crate::logic::{self, Indent, plural};
use crate::theme::MONO_FONT;
use crate::ui::{self, Tone};

const SAMPLE: &str = "[\n  { \"id\": 1, \"email\": \"ada@example.com\", \"role\": \"admin\", \"joined\": \"2026-01-12T09:30:00Z\", \"tags\": [\"core\"] },\n  { \"id\": 2, \"email\": \"alan@example.com\", \"role\": \"member\", \"joined\": \"2026-02-03T14:05:00Z\", \"tags\": [], \"manager\": 1 },\n  { \"id\": 3, \"email\": \"grace@example.com\", \"role\": \"member\", \"joined\": null, \"tags\": [\"ops\", \"oncall\"] },\n  { \"id\": 4, \"email\": \"edsger@example.com\", \"role\": \"admin\", \"joined\": \"2026-03-21T08:00:00Z\", \"tags\": [] }\n]";

pub struct SchemaView {
    doc: Entity<EditorState>,
    /// Generated (read-only) while inferring; edited by hand while validating.
    schema: Entity<EditorState>,
    validate: bool,
    hints: bool,
    closed: bool,
    out: SharedString,
    status: (String, Option<Tone>),
    err: Option<String>,
    issues: Vec<Issue>,
    skipped: Vec<String>,
    wrap: bool,
    _subs: Vec<Subscription>,
}

impl SchemaView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let doc = code_editor(SAMPLE, "Paste JSON: one document, or several one after another", "json", window, cx);
        let schema = code_editor("", "Paste a JSON Schema", "json", window, cx);
        let subs = vec![watch(&doc, window, cx, Self::recompute), watch(&schema, window, cx, |this: &mut Self, w, cx| {
            // The generated schema changing is our own doing, not an edit.
            if this.validate {
                this.recompute(w, cx);
            }
        })];
        let mut this = Self {
            doc,
            schema,
            validate: false,
            hints: true,
            closed: false,
            out: SharedString::default(),
            status: (String::new(), None),
            err: None,
            issues: Vec::new(),
            skipped: Vec::new(),
            wrap: false,
            _subs: subs,
        };
        this.recompute(window, cx);
        this
    }

    fn recompute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = text_of(&self.doc, cx);
        (self.err, self.issues, self.skipped) = (None, Vec::new(), Vec::new());
        if self.validate {
            let schema = text_of(&self.schema, cx);
            self.status = if doc.trim().is_empty() || schema.trim().is_empty() {
                ("Waiting for a document and a schema".into(), None)
            } else {
                match schema::validate(&doc, &schema) {
                    Ok(report) => {
                        let n = report.issues.len();
                        (self.issues, self.skipped) = (report.issues, report.skipped);
                        if n == 0 { ("The document matches the schema".into(), Some(Tone::Ok)) } else { (plural(n, "problem"), Some(Tone::Err)) }
                    }
                    Err(e) => {
                        self.err = Some(e);
                        ("Error".into(), Some(Tone::Err))
                    }
                }
            };
        } else {
            let opts = InferOpts { hints: self.hints, closed: self.closed };
            let (text, status) = if doc.trim().is_empty() {
                (String::new(), ("Waiting for sample JSON".into(), None))
            } else {
                match schema::infer(&doc, opts) {
                    Ok((v, samples)) => {
                        let from = if samples == 1 { "1 sample".to_string() } else { format!("{samples} samples merged") };
                        (logic::to_json(&v, &Indent::Spaces(2)), (format!("Draft 2020-12 · {from}"), Some(Tone::Ok)))
                    }
                    Err(e) => {
                        self.err = Some(e);
                        (String::new(), ("Invalid JSON".into(), Some(Tone::Err)))
                    }
                }
            };
            set_text(&self.schema, &text, window, cx);
            self.out = text.into();
            self.status = status;
        }
        cx.notify();
    }

    fn set_validate(&mut self, validate: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Going to Validate keeps the schema that was just generated, ready to edit.
        self.validate = validate;
        self.recompute(window, cx);
    }
}

impl Render for SchemaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        sync_wrap(&[&self.doc, &self.schema], &mut self.wrap, window, cx);
        let validate = self.validate;
        let on_mode = on_index(cx, |this: &mut Self, i, w, cx| this.set_validate(i == 1, w, cx));
        let [paste, clear] = paste_clear("schema-doc", &self.doc, &pal, cx, Self::recompute);
        let [schema_paste, schema_clear] = paste_clear("schema-schema", &self.schema, &pal, cx, Self::recompute);
        let zoom = ui::PaneZoom::new("schema-output", window, cx);
        let schema_text: SharedString = if validate { text_of(&self.schema, cx).into() } else { self.out.clone() };
        let (hints, closed) = (self.hints, self.closed);

        let mut issues = ui::kv_card(&pal);
        let n = self.issues.len();
        for (i, issue) in self.issues.iter().enumerate() {
            issues = issues.child(
                ui::kv_row(("schema-issue", i), i + 1 == n, &pal)
                    .child(div().w(px(260.)).flex_none().font_family(MONO_FONT).text_size(px(12.5)).text_color(pal.danger).child(issue.path.clone()))
                    .child(div().flex_1().min_w_0().text_size(px(13.)).child(issue.message.clone()))
                    .child(ui::copy_btn(("schema-issue-copy", i), issue.path.clone().into(), &pal, window, cx)),
            );
        }
        let skipped: Vec<String> = match self.skipped.as_slice() {
            [] => Vec::new(),
            list => vec![format!("Not checked, so the result says nothing about {}: {}", if list.len() == 1 { "it" } else { "them" }, list.join(", "))],
        };

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .child(ui::section_label("Configuration", &pal))
            .child(ui::setting(
                "schema-mode",
                ui::setting_icon("conv", &pal),
                "Mode",
                Some("Infer a schema from samples, or check a document against a schema".into()),
                ui::seg("schema-mode-seg", &["Generate", "Validate"], validate as usize, &pal, on_mode),
                &pal,
            ))
            .when(!validate, |d| {
                d.child(ui::setting(
                    "schema-hints",
                    ui::setting_icon("sparkle", &pal),
                    "Recognize formats and vocabularies",
                    Some("Dates, emails, URLs and UUIDs get a format; a few repeated values become an enum".into()),
                    ui::toggle_labeled("schema-hints-tg", hints, &pal, cx.listener(|this, _, w, cx| {
                        this.hints = !this.hints;
                        this.recompute(w, cx);
                    })),
                    &pal,
                ))
                .child(ui::setting(
                    "schema-closed",
                    ui::setting_icon("lock", &pal),
                    "Forbid other properties",
                    Some("Adds additionalProperties: false to every object".into()),
                    ui::toggle_labeled("schema-closed-tg", closed, &pal, cx.listener(|this, _, w, cx| {
                        this.closed = !this.closed;
                        this.recompute(w, cx);
                    })),
                    &pal,
                ))
            })
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .flex_1()
                    .min_h(px(360.))
                    .mt(px(8.))
                    .child(
                        ui::pane(is_focused(&self.doc, window, cx), &pal)
                            .flex_1()
                            .child(ui::pane_head(if validate { "Document" } else { "Sample JSON" }, None, &pal).child(paste).child(clear))
                            .child(code_editor_el(&self.doc, false, cx)),
                    )
                    .child(zoom.wrap(
                        ui::pane(is_focused(&self.schema, window, cx), &pal)
                            .flex_1()
                            .child(
                                ui::pane_head("Schema", None, &pal)
                                    .when(validate, |d| d.child(schema_paste).child(schema_clear))
                                    .child(ui::copy_btn("schema-copy", schema_text, &pal, window, cx))
                                    .child(zoom.button(&pal)),
                            )
                            .when_some(self.err.clone(), |d, e| d.child(ui::err_box(e, &pal).m(px(12.))))
                            .child(code_editor_el(&self.schema, !validate, cx))
                            .child(ui::pane_foot(&pal).child(ui::dot(self.status.1, &pal)).child(self.status.0.clone())),
                        &pal,
                        window,
                    )),
            )
            .when(n > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .mt(px(14.))
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Problems"))
                        .child(ui::badge(n.to_string(), Tone::Err, &pal)),
                )
                .child(issues)
            })
            .when(!skipped.is_empty(), |d| d.child(notes_el(&skipped, &pal)))
    }
}
