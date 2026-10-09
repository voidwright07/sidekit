//! Beautify or minify HTML, CSS, JavaScript and TypeScript.

use gpui_kit::component::input::EditorState;
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task,
    Window, div, prelude::FluentBuilder, px,
};

use super::*;
use super::big;
use super::codegen::notes_el;
use crate::logic::plural;
use crate::logic::webfmt::{self, Opts};
use crate::ui::{self, Tone};

const SAMPLES: [&str; 4] = [
    "<!doctype html><html><head><title>Orders</title><style>body{font-family:system-ui;margin:0}.total{font-weight:600}</style></head><body><main><h1>Orders</h1><p>Hello <b>Ada</b>, you have <a href=\"/orders\">3 open orders</a>.</p><ul><li>Keyboard<li>Monitor <i>(back-ordered)</i></ul><script>document.querySelector('h1').addEventListener('click',function(){console.log('clicked')})</script></main></body></html>",
    ":root{--accent:#005fb8;--radius:6px}.card{display:flex;gap:12px;padding:12px 16px;border:1px solid rgba(0,0,0,.08);border-radius:var(--radius)}.card:hover,.card:focus-within{border-color:var(--accent)}@media (max-width:600px){.card{flex-direction:column;padding:8px}}",
    "const debounce=(fn,ms=150)=>{let t;return(...args)=>{clearTimeout(t);t=setTimeout(()=>fn(...args),ms)}};async function load(url,{retries=3}={}){for(let i=0;i<retries;i++){try{const r=await fetch(url);if(r.ok)return r.json()}catch(e){console.warn(`attempt ${i+1} failed`,e)}}throw new Error('gave up')}",
    "interface User{id:number;name:string;roles?:Role[]}type Role='admin'|'editor'|'viewer';export async function findUser<T extends User>(id:number,cache:Map<number,T>):Promise<T|undefined>{if(cache.has(id))return cache.get(id);const res=await fetch(`/api/users/${id}`);return res.ok?(await res.json()) as T:undefined}",
];

pub struct WebFmtView {
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    lang: usize,
    minify: bool,
    indent: usize,
    out: SharedString,
    status: String,
    err: Option<String>,
    notes: Vec<String>,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl WebFmtView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = code_editor(SAMPLES[0], "Paste code", webfmt::GRAMMARS[0], window, cx);
        // Minified code is the usual input: one very long line.
        input.update(cx, |s, cx| s.set_soft_wrap(true, window, cx));
        let output = code_editor("", "", webfmt::GRAMMARS[0], window, cx);
        let subs = vec![watch(&input, window, cx, Self::recompute), watch(&output, window, cx, |_, _, _| {})];
        let mut this = Self {
            input,
            output,
            lang: 0,
            minify: false,
            indent: 0,
            out: SharedString::default(),
            status: String::new(),
            err: None,
            notes: Vec::new(),
            task: None,
            _subs: subs,
        };
        this.recompute(window, cx);
        this
    }

    fn set_lang(&mut self, lang: usize, window: &mut Window, cx: &mut Context<Self>) {
        // An untouched sample follows the language; real input stays.
        let current = text_of(&self.input, cx);
        if current.is_empty() || current == SAMPLES[self.lang] {
            set_text(&self.input, SAMPLES[lang], window, cx);
        }
        self.lang = lang;
        self.recompute(window, cx);
    }

    fn recompute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let src = text_of(&self.input, cx);
        let (lang, opts) = (self.lang, Opts { minify: self.minify, indent: self.indent });
        big::fit_language(&self.input, webfmt::GRAMMARS[lang], src.len(), cx);
        // Output wraps when it is one long line, and scrolls sideways when it is laid out.
        let wrap = self.minify;
        self.output.update(cx, |s, cx| s.set_soft_wrap(wrap, window, cx));
        let work = move || {
            if src.trim().is_empty() {
                return (String::new(), "Waiting for input".to_string(), None, Vec::new());
            }
            match webfmt::format(&src, lang, opts) {
                Ok((out, notes)) => {
                    let status = if opts.minify {
                        let saved = 100 - (out.len() * 100 / src.len().max(1)) as i64;
                        format!("{} · {saved}% smaller", plural(out.len(), "byte"))
                    } else {
                        plural(out.lines().count(), "line")
                    };
                    (out, status, None, notes)
                }
                Err(e) => (String::new(), format!("Not valid {}", webfmt::LANGS[lang]), Some(e), Vec::new()),
            }
        };
        let size = text_len(&self.input, cx);
        if size > big::LARGE {
            self.status = "Working…".into();
            cx.notify();
        }
        big::run(size, self, |t| &mut t.task, window, cx, work, |this, (out, mut status, err, notes), window, cx| {
            let shown = big::for_display(&out);
            if shown.is_some() {
                status = format!("{status} · {}", big::SHORTENED);
            }
            big::fit_language(&this.output, webfmt::GRAMMARS[this.lang], out.len(), cx);
            set_text(&this.output, &tabs_shown(shown.as_deref().unwrap_or(&out)), window, cx);
            this.out = out.into();
            (this.status, this.err, this.notes) = (status, err, notes);
            cx.notify();
        });
    }
}

impl Render for WebFmtView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let on_lang = on_index(cx, |this: &mut Self, i, w, cx| this.set_lang(i, w, cx));
        let on_style = on_index(cx, |this: &mut Self, i, w, cx| {
            this.minify = i == 1;
            this.recompute(w, cx);
        });
        let on_indent = on_index(cx, |this: &mut Self, i, w, cx| {
            this.indent = i;
            this.recompute(w, cx);
        });
        let [paste, clear] = paste_clear("webfmt", &self.input, &pal, cx, Self::recompute);
        let zoom = ui::PaneZoom::new("webfmt-output", window, cx);
        let tone = if self.err.is_some() { Some(Tone::Err) } else if self.out.is_empty() { None } else { Some(Tone::Ok) };

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .child(ui::section_label("Configuration", &pal))
            .child(ui::setting(
                "webfmt-lang",
                ui::setting_icon("html", &pal),
                "Language",
                Some("JSX is understood in JavaScript and TypeScript".into()),
                ui::seg("webfmt-lang-seg", webfmt::LANGS, self.lang, &pal, on_lang),
                &pal,
            ))
            .child(ui::setting(
                "webfmt-style",
                ui::setting_icon("conv", &pal),
                "Output",
                Some("Lay the code out to read, or squeeze it onto one line".into()),
                ui::seg("webfmt-style-seg", &["Beautify", "Minify"], self.minify as usize, &pal, on_style),
                &pal,
            ))
            .when(!self.minify, |d| {
                d.child(ui::setting(
                    "webfmt-indent",
                    ui::setting_icon("indent", &pal),
                    "Indentation",
                    None,
                    ui::dropdown("webfmt-indent-dd", webfmt::INDENTS, self.indent, &pal, window, cx, on_indent),
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
                        ui::pane(is_focused(&self.input, window, cx), &pal)
                            .flex_1()
                            .child(ui::pane_head("Input", None, &pal).child(paste).child(clear))
                            .child(code_editor_el(&self.input, false, cx)),
                    )
                    .child(zoom.wrap(
                        ui::pane(is_focused(&self.output, window, cx), &pal)
                            .flex_1()
                            .child(
                                ui::pane_head("Output", None, &pal)
                                    .child(ui::copy_btn("webfmt-copy", self.out.clone(), &pal, window, cx))
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
