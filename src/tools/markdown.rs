//! Write Markdown or HTML on the left, see it rendered on the right.

use gpui_kit::component::input::EditorState;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::*;
use crate::logic::plural;
use crate::ui;

/// The preview renders at most this much; laying out megabytes of rich text
/// takes minutes and gigabytes.
const PREVIEW_LIMIT: usize = 128 * 1024;

const SAMPLE: &str = "# Release notes\n\nSideKit **0.2** adds a few tools:\n\n- A *Markdown* preview (this one)\n- A cron expression reader\n- JSON ↔ CSV\n\n> Everything runs offline.\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n| Tool | Category |\n| --- | --- |\n| Cron | Testers |\n| QR code | Generators |\n\n[Read more](https://example.com)\n";

const HTML_SAMPLE: &str = "<h1>Release notes</h1>\n<p>SideKit <strong>0.2</strong> adds <em>fourteen</em> tools, among them:</p>\n<ul>\n  <li>An HTML preview (this one)</li>\n  <li>A formatter for <code>HTML</code>, <code>CSS</code> and <code>JS</code></li>\n  <li>An HTTP client</li>\n</ul>\n<blockquote>Everything is rendered on this computer.</blockquote>\n<table>\n  <tr><th>Tool</th><th>Category</th></tr>\n  <tr><td>HTML Preview</td><td>Text</td></tr>\n  <tr><td>HTTP Client</td><td>Network</td></tr>\n</table>\n<p><a href=\"https://example.com\">Read more</a></p>\n<script>document.title = \"scripts never run\";</script>\n";

pub struct MarkdownView {
    input: Entity<EditorState>,
    /// Preview HTML rather than Markdown.
    html: bool,
    /// Bumped per edit so the preview re-parses.
    rev: usize,
    _subs: Vec<Subscription>,
}

impl MarkdownView {
    pub fn new(html: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = match html {
            true => code_editor(HTML_SAMPLE, "Write HTML", "html", window, cx),
            false => code_editor(SAMPLE, "Write Markdown", "markdown", window, cx),
        };
        input.update(cx, |s, cx| s.set_soft_wrap(true, window, cx));
        let subs = vec![watch(&input, window, cx, |this: &mut Self, _, cx| {
            this.rev += 1;
            cx.notify();
        })];
        Self { input, html, rev: 0, _subs: subs }
    }
}

impl Render for MarkdownView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let text = text_of(&self.input, cx);
        let words = text.split_whitespace().count();
        let [paste, clear] = paste_clear("md", &self.input, &pal, cx, |this: &mut Self, _, cx| {
            this.rev += 1;
            cx.notify();
        });
        let html_ish: SharedString = text.clone().into();
        let cut = text.len() > PREVIEW_LIMIT;
        let preview_text = if cut {
            let end = text.floor_char_boundary(PREVIEW_LIMIT);
            text[..text[..end].rfind('\n').unwrap_or(end)].to_string()
        } else {
            text.clone()
        };
        let zoom = ui::PaneZoom::new("md-preview", window, cx);
        let preview = match self.html {
            true => TextView::html(crate::id!("html-view-{}", self.rev), preview_text),
            false => TextView::markdown(crate::id!("md-view-{}", self.rev), preview_text),
        };

        div()
            .flex()
            .gap(px(12.))
            .flex_1()
            .min_h(px(460.))
            .child(
                ui::pane(is_focused(&self.input, window, cx), &pal)
                    .flex_1()
                    .child(ui::pane_head(if self.html { "HTML" } else { "Markdown" }, None, &pal).child(paste).child(clear))
                    .child(code_editor_el(&self.input, false, cx))
                    .child(ui::pane_foot(&pal).child(format!("{} · {}", plural(words, "word"), plural(text.lines().count(), "line")))),
            )
            .child(zoom.wrap(
                ui::pane(false, &pal)
                    .flex_1()
                    .child(ui::pane_head("Preview", None, &pal).child(ui::copy_btn("md-copy", html_ish, &pal, window, cx)).child(zoom.button(&pal)))
                    .child(
                        div()
                            .id("md-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .px(px(20.))
                            .py(px(16.))
                            .child(preview.selectable(true))
                            // The renderer is not a browser, and is not meant to be one.
                            .when(self.html, |d| {
                                d.child(div().mt(px(16.)).text_size(px(12.)).text_color(pal.text3).child("Shows structure and text formatting. Styles, scripts and remote content are never loaded."))
                            })
                            .when(cut, |d| {
                                d.child(div().mt(px(16.)).text_size(px(12.)).text_color(pal.text3).child("The preview shows the first 128 KB."))
                            }),
                    ),
                &pal,
                window,
            ))
    }
}
