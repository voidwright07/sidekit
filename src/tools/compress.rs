//! Compress or decompress a file, or text pasted as Base64 or hex.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::EditorState;
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task,
    Window, div, prelude::FluentBuilder, px,
};

use super::*;
use super::big;
use crate::logic::compress::{self, CODECS, Codec, LEVELS};
use crate::logic::plural;
use crate::ui::{self, BtnKind, Tone};

const PACK_FORMATS: &[&str] = &["Gzip", "Zlib", "Deflate (raw)", "Brotli", "Zstandard", "Bzip2", "XZ"];
const UNPACK_FORMATS: &[&str] = &["Auto-detect", "Gzip", "Zlib", "Deflate (raw)", "Brotli", "Zstandard", "Bzip2", "XZ"];
const LEVEL_LABELS: &[&str] = &["Fastest", "Balanced", "Smallest"];
const SAMPLE: &str = "{\n  \"name\": \"SideKit\",\n  \"tags\": [\"dev\", \"tools\", \"dev\", \"tools\", \"dev\", \"tools\"],\n  \"note\": \"Repetitive text compresses well. Repetitive text compresses well.\"\n}";

struct SourceFile {
    name: String,
    bytes: Arc<Vec<u8>>,
}

struct Done {
    codec: Codec,
    input_len: usize,
    bytes: Vec<u8>,
    /// Text mode: Base64 of compressed output, or decompressed output as
    /// text. `None` when the decompressed bytes are not UTF-8.
    text: Option<SharedString>,
}

pub struct CompressView {
    decompress: bool,
    text_mode: bool,
    pack_format: usize,
    unpack_format: usize,
    level: usize,
    file: Option<SourceFile>,
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    done: Option<Done>,
    /// The output pane holds only part of the text.
    shortened: bool,
    busy: bool,
    err: Option<String>,
    note: Option<String>,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl CompressView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = code_editor(SAMPLE, "Type or paste text", "plaintext", window, cx);
        let output = code_editor("", "", "plaintext", window, cx);
        for s in [&input, &output] {
            s.update(cx, |s, cx| s.set_soft_wrap(true, window, cx));
        }
        let subs = vec![watch(&input, window, cx, Self::run), watch(&output, window, cx, |_, _, _| {})];
        let mut this = Self {
            decompress: false,
            text_mode: false,
            pack_format: 0,
            unpack_format: 0,
            level: 1,
            file: None,
            input,
            output,
            done: None,
            shortened: false,
            busy: false,
            err: None,
            note: None,
            task: None,
            _subs: subs,
        };
        // TEST-HOOK
        if let Some(p) = std::env::var_os("SIDEKIT_FILE") {
            this.load_file(p.into(), window, cx);
        }
        this
    }

    fn load_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read(&path) {
            Ok(bytes) => {
                // A file that announces a compressed format is most likely here to be opened.
                if compress::detect(&bytes).or(compress::from_name(&name)).is_some() {
                    (self.decompress, self.unpack_format) = (true, 0);
                }
                self.file = Some(SourceFile { name, bytes: Arc::new(bytes) });
                self.run(window, cx);
            }
            Err(e) => {
                (self.file, self.done) = (None, None);
                self.err = Some(format!("Could not read {}: {e}", path.display()));
                cx.notify();
            }
        }
    }

    pub fn set_decompress(&mut self, decompress: bool, window: &mut Window, cx: &mut Context<Self>) {
        // The sample is plain text, so drop it when decompressing and bring it back for compressing.
        let input = text_of(&self.input, cx);
        if decompress && input == SAMPLE {
            set_text(&self.input, "", window, cx);
        } else if !decompress && input.is_empty() {
            set_text(&self.input, SAMPLE, window, cx);
        }
        self.decompress = decompress;
        self.run(window, cx);
    }

    /// "Use output as input": feed the text result back and flip direction.
    fn swap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.done.as_ref().and_then(|d| d.text.clone()) else { return };
        set_text(&self.input, &text, window, cx);
        self.decompress = !self.decompress;
        self.run(window, cx);
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.note = None;
        let (decompress, text_mode) = (self.decompress, self.text_mode);
        let codec = if decompress { self.unpack_format.checked_sub(1).map(|i| CODECS[i]) } else { Some(CODECS[self.pack_format]) };
        let level = LEVELS[self.level];
        let (size, source) = if text_mode {
            (text_len(&self.input, cx), Err(text_of(&self.input, cx)))
        } else {
            match &self.file {
                Some(f) => (f.bytes.len(), Ok(f.bytes.clone())),
                None => {
                    (self.done, self.err, self.busy) = (None, None, false);
                    cx.notify();
                    return;
                }
            }
        };
        let work = move || -> Option<Result<Done, String>> {
            let bytes = match source {
                Ok(bytes) => bytes,
                Err(text) if text.trim().is_empty() => return None,
                Err(text) if decompress => match compress::parse_encoded(&text) {
                    Ok(b) => Arc::new(b),
                    Err(e) => return Some(Err(e)),
                },
                Err(text) => Arc::new(text.into_bytes()),
            };
            let r = match (decompress, codec) {
                (false, Some(c)) => compress::compress(&bytes, c, level).map(|out| (c, out)),
                (true, Some(c)) => compress::decompress(&bytes, c).map(|out| (c, out)),
                (_, None) => compress::decompress_auto(&bytes),
            };
            Some(r.map(|(codec, out)| {
                let text = text_mode.then(|| if decompress { String::from_utf8(out.clone()).ok() } else { Some(compress::to_base64(&out)) });
                Done { codec, input_len: bytes.len(), text: text.flatten().map(Into::into), bytes: out }
            }))
        };
        self.busy = size > big::LARGE;
        big::run(size, self, |t| &mut t.task, window, cx, work, |this, r, window, cx| {
            this.busy = false;
            (this.done, this.err) = match r {
                None => (None, None),
                Some(Ok(d)) => (Some(d), None),
                Some(Err(e)) => (None, Some(e)),
            };
            let text = this.done.as_ref().and_then(|d| d.text.clone()).unwrap_or_default();
            let shown = big::for_display(&text);
            this.shortened = shown.is_some();
            set_text(&this.output, shown.as_deref().unwrap_or(&text), window, cx);
            cx.notify();
        });
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &self.done else { return };
        let name = match (&self.file, self.text_mode, self.decompress) {
            (Some(f), false, false) => compress::compressed_name(&f.name, d.codec),
            (Some(f), false, true) => compress::decompressed_name(&f.name, d.codec),
            (_, _, false) => compress::compressed_name("data.txt", d.codec),
            (_, _, true) => if d.text.is_some() { "data.txt" } else { "data.bin" }.to_string(),
        };
        save_bytes(d.bytes.clone(), name, window, cx, |v: &mut Self, r, cx| {
            v.note = Some(match r {
                Ok(p) => format!("Saved to {}", p.display()),
                Err(e) => format!("Could not save: {e}"),
            });
            cx.notify();
        });
    }

    /// "1,024 bytes → 312 bytes · 70% smaller", or what is happening instead.
    fn summary(&self) -> String {
        match (&self.done, self.busy) {
            (_, true) => if self.decompress { "Decompressing…" } else { "Compressing…" }.into(),
            (Some(d), _) => {
                let change = d.bytes.len() as f64 / d.input_len.max(1) as f64 * 100. - 100.;
                let versus = if change.abs() < 0.5 { "the same size".to_string() } else if change < 0. { format!("{:.0}% smaller", -change) } else { format!("{change:.0}% larger") };
                format!("{} → {} · {versus}", plural(d.input_len, "byte"), plural(d.bytes.len(), "byte"))
            }
            (None, _) => String::new(),
        }
    }
}

impl Render for CompressView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let on_mode = on_index(cx, |this: &mut Self, i, w, cx| this.set_decompress(i == 1, w, cx));
        let on_source = on_index(cx, |this: &mut Self, i, w, cx| {
            this.text_mode = i == 1;
            this.run(w, cx);
        });
        let on_format = on_index(cx, |this: &mut Self, i, w, cx| {
            if this.decompress { this.unpack_format = i } else { this.pack_format = i }
            this.run(w, cx);
        });
        let on_level = on_index(cx, |this: &mut Self, i, w, cx| {
            this.level = i;
            this.run(w, cx);
        });
        let (formats, format) = if self.decompress { (UNPACK_FORMATS, self.unpack_format) } else { (PACK_FORMATS, self.pack_format) };
        let detected = self.done.as_ref().filter(|_| self.decompress && self.unpack_format == 0).map(|d| format!("Found {}", d.codec.label()));
        let has_result = self.done.is_some();
        let save = ui::btn("compress-save", Some("install"), "Save…", BtnKind::Accent, &pal, cx.listener(|this, _, w, cx| this.save(w, cx)));
        let summary = self.summary();

        let config = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(ui::section_label("Configuration", &pal))
            .child(ui::setting(
                "compress-mode",
                ui::setting_icon("conv", &pal),
                "Direction",
                None,
                ui::seg("compress-mode-seg", &["Compress", "Decompress"], self.decompress as usize, &pal, on_mode),
                &pal,
            ))
            .child(ui::setting(
                "compress-format",
                ui::setting_icon("archive", &pal),
                "Format",
                Some(detected.unwrap_or_else(|| if self.decompress { "Auto-detect reads the format from the data".into() } else { "Gzip and Zlib suit the web; Zstandard and XZ make the smallest files".into() }).into()),
                ui::dropdown("compress-format-dd", formats, format, &pal, window, cx, on_format),
                &pal,
            ))
            .when(!self.decompress, |d| {
                d.child(ui::setting(
                    "compress-level",
                    ui::setting_icon("sparkle", &pal),
                    "Level",
                    Some("Smaller output takes longer to make".into()),
                    ui::dropdown("compress-level-dd", LEVEL_LABELS, self.level, &pal, window, cx, on_level),
                    &pal,
                ))
            })
            .child(ui::setting(
                "compress-source",
                ui::setting_icon("folder", &pal),
                "Input",
                Some(if self.decompress { "Text input takes Base64 or hexadecimal" } else { "Text input gives Base64 output" }.into()),
                ui::seg("compress-source-seg", &["File", "Text"], self.text_mode as usize, &pal, on_source),
                &pal,
            ));

        if !self.text_mode {
            let hint: SharedString = match &self.file {
                Some(f) => format!("{} · {}", f.name, plural(f.bytes.len(), "byte")).into(),
                None => "or drop a file here".into(),
            };
            return config
                .child(ui::section_label("File", &pal).mt(px(8.)))
                .child(drop_zone("compress-drop", "Choose file…", hint, false, &pal, cx, Self::load_file))
                .when_some(self.err.clone(), |d, e| d.child(ui::err_box(e, &pal).mt(px(4.))))
                .when(has_result || self.busy, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .mt(px(8.))
                            .when(has_result, |d| d.child(save))
                            .child(div().text_size(px(13.)).text_color(pal.text2).child(summary)),
                    )
                })
                .when(self.file.is_none(), |d| d.child(div().mt(px(6.)).text_size(px(12.)).text_color(pal.text3).child("Files are processed on this computer and never uploaded.")))
                .when_some(self.note.clone(), |d, n| d.child(div().text_size(px(12.)).text_color(pal.text3).child(n)));
        }

        let input = text_of(&self.input, cx);
        let in_stat = plural(input.len(), "byte");
        let binary = self.decompress && self.done.as_ref().is_some_and(|d| d.text.is_none());
        let out_text = self.done.as_ref().and_then(|d| d.text.clone()).unwrap_or_default();
        let out_stat = if self.err.is_some() {
            "Error".to_string()
        } else if self.shortened {
            format!("{summary} · {}", big::SHORTENED)
        } else {
            summary
        };
        let tone = if self.err.is_some() { Some(Tone::Err) } else if has_result { Some(Tone::Ok) } else { None };
        let [paste, clear] = paste_clear("compress", &self.input, &pal, cx, Self::run);
        let zoom = ui::PaneZoom::new("compress-output", window, cx);
        config.child(
            div()
                .flex()
                .gap(px(12.))
                .flex_1()
                .min_h(px(340.))
                .mt(px(8.))
                .child(
                    ui::pane(is_focused(&self.input, window, cx), &pal)
                        .flex_1()
                        .child(ui::pane_head(if self.decompress { "Compressed (Base64 or hex)" } else { "Text" }, None, &pal).child(paste).child(clear))
                        .child(code_editor_el(&self.input, false, cx))
                        .child(ui::pane_foot(&pal).child(in_stat)),
                )
                .child(zoom.wrap(
                    ui::pane(is_focused(&self.output, window, cx), &pal)
                        .flex_1()
                        .child(
                            ui::pane_head(if self.decompress { "Text" } else { "Compressed (Base64)" }, None, &pal)
                                .when(has_result, |d| d.child(ui::icon_btn("compress-file", "install", &pal, cx.listener(|this, _, w, cx| this.save(w, cx)))))
                                .child(ui::icon_btn("compress-swap", "swap", &pal, cx.listener(|this, _, w, cx| this.swap(w, cx))))
                                .child(ui::copy_btn("compress-copy", out_text, &pal, window, cx))
                                .child(zoom.button(&pal)),
                        )
                        .when_some(self.err.clone(), |d, e| d.child(ui::err_box(e, &pal).m(px(12.))))
                        .when(binary, |d| d.child(ui::err_box("The output is binary, not text. Save it to a file instead.", &pal).m(px(12.))))
                        .child(code_editor_el(&self.output, true, cx))
                        .child(ui::pane_foot(&pal).child(ui::dot(tone, &pal)).child(out_stat))
                        .when_some(self.note.clone(), |d, n| d.child(ui::pane_foot(&pal).child(n))),
                    &pal,
                    window,
                )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_lists_follow_codecs() {
        let labels: Vec<_> = CODECS.iter().map(|c| c.label()).collect();
        assert_eq!(PACK_FORMATS, labels);
        assert_eq!(UNPACK_FORMATS[1..], labels);
        assert_eq!(LEVEL_LABELS.len(), LEVELS.len());
    }
}
