//! SVG markup → data URIs, CSS, HTML and JSX, with a preview.

use std::sync::Arc;

use gpui_kit::component::input::EditorState;
use gpui_kit::{
    Context, Entity, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Render, SharedString,
    Styled, StyledImage, Subscription, Window, div, img, prelude::FluentBuilder, px,
};

use super::*;
use crate::logic::plural;
use crate::logic::svg::{self, Svg};
use crate::ui::{self, Tone};

const SAMPLE: &str = "<svg width=\"96\" height=\"96\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#005fb8\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n  <!-- A shield with a tick -->\n  <path d=\"M12 3l8 3v6c0 4.5-3.4 8-8 9-4.6-1-8-4.5-8-9V6z\" fill=\"#005fb8\" fill-opacity=\"0.12\"/>\n  <path d=\"M9 12l2 2 4-4\"/>\n</svg>";

/// One label per entry of `Svg::outputs`, for the picker.
const OUTPUTS: &[&str] = &["Data URI", "Data URI (Base64)", "CSS", "HTML image", "Inline SVG", "React (JSX)"];
const GRAMMARS: &[&str] = &["plaintext", "plaintext", "css", "html", "html", "javascript"];

pub struct SvgView {
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    pick: usize,
    svg: Option<(Svg, Arc<Image>)>,
    err: Option<String>,
    _subs: Vec<Subscription>,
}

impl SvgView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = code_editor(SAMPLE, "Paste SVG markup", "html", window, cx);
        let output = code_editor("", "", GRAMMARS[0], window, cx);
        // A data URI is one long line.
        output.update(cx, |s, cx| s.set_soft_wrap(true, window, cx));
        let subs = vec![watch(&input, window, cx, Self::recompute), watch(&output, window, cx, |_, _, _| {})];
        let mut this = Self { input, output, pick: 0, svg: None, err: None, _subs: subs };
        this.recompute(window, cx);
        this
    }

    pub fn set_input(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        set_text(&self.input, text, window, cx);
        self.recompute(window, cx);
    }

    fn recompute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let src = text_of(&self.input, cx);
        if src.trim().is_empty() {
            (self.svg, self.err) = (None, None);
        } else {
            match svg::convert(&src) {
                Ok(s) => {
                    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, s.image.clone().into_bytes()));
                    (self.svg, self.err) = (Some((s, image)), None);
                }
                Err(e) => (self.svg, self.err) = (None, Some(e)),
            }
        }
        self.show(window, cx);
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.svg.as_ref().map(|(s, _)| s.outputs[self.pick].1.as_str()).unwrap_or("");
        self.output.update(cx, |s, cx| s.set_highlighter(GRAMMARS[self.pick], cx));
        set_text(&self.output, text, window, cx);
        cx.notify();
    }
}

impl Render for SvgView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let [paste, clear] = paste_clear("svg", &self.input, &pal, cx, Self::recompute);
        let on_pick = on_index(cx, |this: &mut Self, i, w, cx| {
            this.pick = i;
            this.show(w, cx);
        });
        let zoom = ui::PaneZoom::new("svg-output", window, cx);
        let out: SharedString = self.svg.as_ref().map(|(s, _)| s.outputs[self.pick].1.clone()).unwrap_or_default().into();
        let tone = if self.err.is_some() { Some(Tone::Err) } else if self.svg.is_some() { Some(Tone::Ok) } else { None };
        let status = match (&self.err, &self.svg) {
            (Some(_), _) => "Not valid SVG".to_string(),
            (None, Some(_)) => plural(out.len(), "byte"),
            (None, None) => "Waiting for SVG".to_string(),
        };
        let image = self.svg.as_ref().map(|(_, i)| i.clone());
        let rows = self.svg.as_ref().map(|(s, _)| s.rows.clone()).unwrap_or_default();
        let n = rows.len();
        let mut card = ui::kv_card(&pal);
        for (i, (k, v)) in rows.into_iter().enumerate() {
            card = card.child(ui::kv_row(("svg-i", i), i + 1 == n, &pal).child(ui::kv_label(k, 90., &pal)).child(ui::kv_val(v, false)));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        ui::pane(is_focused(&self.input, window, cx), &pal)
                            .flex_1()
                            .h(px(300.))
                            .child(ui::pane_head("SVG", None, &pal).child(paste).child(clear))
                            .child(code_editor_el(&self.input, false, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .w(px(300.))
                            .flex_none()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .h(px(150.))
                                    .p(px(16.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(pal.stroke)
                                    .bg(pal.editor)
                                    .map(|d| match image {
                                        Some(i) => d.child(img(i).size_full().object_fit(ObjectFit::Contain)),
                                        None => d.child(div().text_size(px(13.)).text_color(pal.text3).child("The drawing appears here")),
                                    }),
                            )
                            .when(n > 0, |d| d.child(card)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mt(px(12.))
                    .child(ui::section_label("Output", &pal).flex_1())
                    .child(ui::dropdown("svg-pick", OUTPUTS, self.pick, &pal, window, cx, on_pick)),
            )
            .child(zoom.wrap(
                ui::pane(is_focused(&self.output, window, cx), &pal)
                    .flex_1()
                    .min_h(px(220.))
                    .child(ui::pane_head(OUTPUTS[self.pick], None, &pal).child(ui::copy_btn("svg-copy", out, &pal, window, cx)).child(zoom.button(&pal)))
                    .when_some(self.err.clone(), |d, e| d.child(ui::err_box(e, &pal).m(px(12.))))
                    .child(code_editor_el(&self.output, true, cx))
                    .child(ui::pane_foot(&pal).child(ui::dot(tone, &pal)).child(status)),
                &pal,
                window,
            ))
    }
}
