//! A copyable summary of this computer, for bug reports and support requests.

use gpui_kit::{
    Context, IntoElement, ParentElement, Render, SharedString, Styled, Task, Window, div,
    prelude::FluentBuilder, px,
};

use super::*;
use crate::logic::sysinfo::{self, Row, Section};
use crate::ui::{self, BtnKind};

pub struct SysInfoView {
    sections: Vec<Section>,
    /// Leave out host name, user name, addresses and paths.
    hide: bool,
    /// Indexes `sysinfo::EXPORTS`.
    format: usize,
    loading: bool,
    task: Option<Task<()>>,
}

impl SysInfoView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { sections: Vec::new(), hide: false, format: 0, loading: false, task: None };
        this.refresh(window, cx);
        this
    }

    /// Asking the OS about disks and network interfaces takes a moment, so it
    /// happens off the UI thread.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Screens are only known to the UI thread.
        let displays: Vec<Row> = cx
            .displays()
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let size = d.bounds().size;
                Row { label: format!("Display {}", i + 1), value: format!("{} × {} points", f32::from(size.width).round(), f32::from(size.height).round()), identifying: false }
            })
            .collect();
        let scale = format!("{}%", (window.scale_factor() * 100.).round());
        self.loading = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut sections = cx.background_executor().spawn(async move { sysinfo::collect() }).await;
            let mut rows = displays;
            rows.push(Row { label: "Scaling".into(), value: scale, identifying: false });
            sections.insert(2, Section { title: "Display", rows });
            let _ = this.update(cx, |v, cx| {
                (v.sections, v.loading) = (sections, false);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for SysInfoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let hide = self.hide;
        let on_format = on_index(cx, |this: &mut Self, i, _, cx| {
            this.format = i;
            cx.notify();
        });
        let report: SharedString = sysinfo::export(&self.sections, hide, self.format).into();

        let mut body = div().flex().flex_col().gap(px(6.));
        for (si, section) in self.sections.iter().enumerate() {
            let rows: Vec<&Row> = section.rows.iter().filter(|r| !(hide && r.identifying)).collect();
            if rows.is_empty() {
                continue;
            }
            let mut card = ui::kv_card(&pal);
            let n = rows.len();
            for (i, r) in rows.into_iter().enumerate() {
                card = card.child(ui::kv_copy_row(crate::id!("sys-{si}-{i}"), r.label.clone(), 210., r.value.clone().into(), true, i + 1 == n, &pal, window, cx));
            }
            body = body.child(ui::section_label(section.title, &pal).mt(px(12.))).child(card);
        }

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(ui::section_label("Report", &pal))
            .child(ui::setting(
                "sys-hide",
                ui::setting_icon("eye", &pal),
                "Hide identifying fields",
                Some("Leaves out the host name, user name, network addresses and file paths".into()),
                ui::toggle_labeled("sys-hide-tg", hide, &pal, cx.listener(|this, _, _, cx| {
                    this.hide = !this.hide;
                    cx.notify();
                })),
                &pal,
            ))
            .child(ui::setting(
                "sys-copy",
                ui::setting_icon("copy", &pal),
                "Copy the report",
                Some("Everything shown below, ready to paste into a bug report".into()),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(ui::seg("sys-format", sysinfo::EXPORTS, self.format, &pal, on_format))
                    .child(ui::copy_btn_labeled("sys-copy-btn", "Copy", report, &pal, window, cx))
                    .child(ui::btn("sys-refresh", Some("refresh"), "Refresh", BtnKind::Normal, &pal, cx.listener(|this, _, w, cx| this.refresh(w, cx)))),
                &pal,
            ))
            .when(self.loading && self.sections.is_empty(), |d| d.child(div().mt(px(12.)).text_size(px(13.)).text_color(pal.text3).child("Reading system details…")))
            .child(body)
            .child(div().mt(px(8.)).text_size(px(12.)).text_color(pal.text3).child("Collected on this computer when the page opens. Nothing is sent anywhere."))
    }
}
