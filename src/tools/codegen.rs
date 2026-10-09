//! Tools that turn an input into code for a chosen target: cURL to Code and
//! JSON to Code.

use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task,
    Window, div, prelude::FluentBuilder, px,
};

use super::*;
use super::big;
use crate::icons::icon;
use crate::logic::{self, plural};
use crate::registry::ToolId;
use crate::ui::{self, Tone};

/// The code, a status line, and notes about anything left out.
type Run = Result<(String, String, Vec<String>), String>;

struct Spec {
    id: &'static str,
    input_title: &'static str,
    placeholder: &'static str,
    sample: &'static str,
    in_lang: &'static str,
    targets: &'static [&'static str],
    grammars: &'static [&'static str],
    target_hint: &'static str,
    /// A one-line setting beside the target: its label, hint and starting value.
    name: Option<(&'static str, &'static str, &'static str)>,
    run: fn(&str, usize, &str) -> Run,
}

fn spec(id: ToolId) -> Spec {
    match id {
        ToolId::JsonCode => Spec {
            id: "jsoncode",
            input_title: "Sample JSON",
            placeholder: "Paste a JSON object, or an array of them",
            sample: "{\n  \"id\": 1024,\n  \"user_name\": \"ada\",\n  \"email\": \"ada@example.com\",\n  \"active\": true,\n  \"score\": 97.5,\n  \"nickname\": null,\n  \"roles\": [\"admin\", \"editor\"],\n  \"address\": { \"city\": \"London\", \"postcode\": \"N1 9GU\" },\n  \"orders\": [\n    { \"id\": 1, \"total\": 42.5, \"shipped\": true },\n    { \"id\": 2, \"total\": 9.99, \"shipped\": false, \"note\": \"gift\" }\n  ]\n}",
            in_lang: "json",
            targets: logic::jsoncode::LANGS,
            grammars: logic::jsoncode::GRAMMARS,
            target_hint: "Fields missing from some records become optional",
            name: Some(("Root type name", "What the outermost type is called", "Root")),
            run: |src, lang, root| {
                let (code, types) = logic::jsoncode::generate(src, lang, root)?;
                let status = format!("{} · {}", plural(types, "type"), plural(code.lines().count(), "line"));
                Ok((code, status, Vec::new()))
            },
        },
        _ => Spec {
            id: "curl",
            input_title: "cURL command",
            placeholder: "Paste a command that starts with curl",
            sample: "curl -X POST 'https://api.example.com/v1/users?notify=true' \\\n  -H 'Content-Type: application/json' \\\n  -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.example' \\\n  --data-raw '{\"name\": \"Ada Lovelace\", \"roles\": [\"admin\"], \"active\": true}' \\\n  --max-time 30",
            in_lang: "bash",
            targets: logic::curl::TARGETS,
            grammars: logic::curl::GRAMMARS,
            target_hint: "The command is only read: nothing is run and nothing is sent",
            name: None,
            run: |src, target, _| {
                let (code, warnings) = logic::curl::convert(src, target)?;
                let status = plural(code.lines().count(), "line");
                Ok((code, status, warnings))
            },
        },
    }
}

pub struct CodeGenView {
    spec: Spec,
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    name: Option<Entity<InputState>>,
    target: usize,
    out: SharedString,
    status: String,
    err: Option<String>,
    notes: Vec<String>,
    wrap: bool,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl CodeGenView {
    pub fn new(id: ToolId, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let spec = spec(id);
        let input = code_editor(spec.sample, spec.placeholder, spec.in_lang, window, cx);
        let output = code_editor("", "", spec.grammars[0], window, cx);
        let name = spec.name.map(|(_, _, start)| line(start, start, window, cx));
        let mut subs = vec![watch(&input, window, cx, Self::recompute), watch(&output, window, cx, |_, _, _| {})];
        if let Some(n) = &name {
            subs.push(watch(n, window, cx, Self::recompute));
        }
        let mut this = Self {
            spec,
            input,
            output,
            name,
            target: 0,
            out: SharedString::default(),
            status: String::new(),
            err: None,
            notes: Vec::new(),
            wrap: false,
            task: None,
            _subs: subs,
        };
        this.recompute(window, cx);
        this
    }

    pub fn set_input(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        set_text(&self.input, text, window, cx);
        self.recompute(window, cx);
    }

    fn recompute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let src = text_of(&self.input, cx);
        let name = self.name.as_ref().map(|n| line_text(n, cx)).unwrap_or_default();
        let (run, target) = (self.spec.run, self.target);
        let work = move || {
            if src.trim().is_empty() {
                return (String::new(), "Waiting for input".to_string(), None, Vec::new());
            }
            match run(&src, target, &name) {
                Ok((code, status, notes)) => (code, status, None, notes),
                Err(e) => (String::new(), "Error".to_string(), Some(e), Vec::new()),
            }
        };
        let size = text_len(&self.input, cx);
        big::run(size, self, |t| &mut t.task, window, cx, work, |this, (out, mut status, err, notes), window, cx| {
            let shown = big::for_display(&out);
            if shown.is_some() {
                status = format!("{status} · {}", big::SHORTENED);
            }
            big::fit_language(&this.output, this.spec.grammars[this.target], out.len(), cx);
            set_text(&this.output, &tabs_shown(shown.as_deref().unwrap_or(&out)), window, cx);
            this.out = out.into();
            (this.status, this.err, this.notes) = (status, err, notes);
            cx.notify();
        });
    }
}

/// A list of things the user should know about, each on its own line.
pub fn notes_el(notes: &[String], pal: &Pal) -> gpui_kit::Div {
    let p = *pal;
    div().flex().flex_col().gap(px(4.)).mt(px(6.)).children(notes.iter().map(|n| {
        div()
            .flex()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(6.))
            .bg(p.subtle)
            .text_size(px(12.5))
            .text_color(p.text2)
            .child(div().pt(px(1.)).child(icon("warn", 14., p.warn)))
            .child(div().flex_1().min_w_0().child(n.clone()))
    }))
}

impl Render for CodeGenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        sync_wrap(&[&self.input, &self.output], &mut self.wrap, window, cx);
        let id = self.spec.id;
        let on_target = on_index(cx, |this: &mut Self, i, w, cx| {
            this.target = i;
            this.recompute(w, cx);
        });
        let [paste, clear] = paste_clear(id, &self.input, &pal, cx, Self::recompute);
        let zoom = ui::PaneZoom::new(crate::id!("{id}-output"), window, cx);
        let tone = if self.err.is_some() { Some(Tone::Err) } else if self.out.is_empty() { None } else { Some(Tone::Ok) };
        let name_row = self.spec.name.zip(self.name.clone()).map(|((label, hint, _), state)| {
            ui::setting(crate::id!("{id}-name"), ui::setting_icon("tag", &pal), label, Some(hint.into()), field_el(&state, true, 32., 13., window, cx).w(px(220.)), &pal)
        });

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .child(ui::section_label("Configuration", &pal))
            .child(ui::setting(
                crate::id!("{id}-target"),
                ui::setting_icon("conv", &pal),
                "Target",
                Some(self.spec.target_hint.into()),
                ui::dropdown(crate::id!("{id}-target-dd"), self.spec.targets, self.target, &pal, window, cx, on_target),
                &pal,
            ))
            .children(name_row)
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .flex_1()
                    .min_h(px(360.))
                    .mt(px(8.))
                    .child(
                        ui::pane(is_focused(&self.input, window, cx), &pal)
                            .flex_1()
                            .child(ui::pane_head(self.spec.input_title, None, &pal).child(paste).child(clear))
                            .child(code_editor_el(&self.input, false, cx)),
                    )
                    .child(zoom.wrap(
                        ui::pane(is_focused(&self.output, window, cx), &pal)
                            .flex_1()
                            .child(
                                ui::pane_head(self.spec.targets[self.target], None, &pal)
                                    .child(ui::copy_btn(crate::id!("{id}-copy"), self.out.clone(), &pal, window, cx))
                                    .child(zoom.button(&pal)),
                            )
                            .when_some(self.err.clone(), |d, e| d.child(ui::err_box(e, &pal).m(px(12.))))
                            .child(code_editor_el(&self.output, true, cx))
                            .child(ui::pane_foot(&pal).child(ui::dot(tone, &pal)).child(self.status.clone())),
                        &pal,
                        window,
                    )),
            )
            .when(!self.notes.is_empty(), |d| d.child(notes_el(&self.notes, &pal)))
    }
}
