//! IPv4 / IPv6 subnet calculator, and the address converter that shares its layout.

use gpui_kit::component::input::InputState;
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};

use super::*;
use crate::logic;
use crate::registry::ToolId;
use crate::ui;

pub struct SubnetView {
    input: Entity<InputState>,
    compute: fn(&str) -> Result<Vec<(&'static str, String)>, String>,
    _subs: Vec<Subscription>,
}

impl SubnetView {
    pub fn new(id: ToolId, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sample, hint, compute): (_, _, fn(&str) -> Result<Vec<(&'static str, String)>, String>) = match id {
            ToolId::IpConv => ("192.168.1.1", "e.g. 192.168.1.1, 3232235777, 0xC0A80101, 2001:db8::1 or 10.0.0.1/24", logic::ip_convert),
            _ => ("192.168.1.130/26", "e.g. 10.0.0.1/24, 10.0.0.1 255.255.255.0 or 2001:db8::1/64", logic::subnet),
        };
        let input = line(sample, hint, window, cx);
        let subs = vec![watch(&input, window, cx, |_, _, cx| cx.notify())];
        Self { input, compute, _subs: subs }
    }

    pub fn set_input(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        set_line(&self.input, text, window, cx);
        cx.notify();
    }
}

impl Render for SubnetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let raw = line_text(&self.input, cx);
        let (rows, err) = match (self.compute)(&raw) {
            Ok(r) => (r, None),
            Err(_) if raw.trim().is_empty() => (Vec::new(), None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let n = rows.len();
        let mut card = ui::kv_card(&pal);
        for (i, (label, value)) in rows.into_iter().enumerate() {
            card = card.child(ui::kv_copy_row(("sn", i), label, 150., value.into(), false, i + 1 == n, &pal, window, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(ui::section_label("Address", &pal))
            .child(field_el(&self.input, true, 44., 16., window, cx))
            .when_some(err, |d, e| d.child(ui::err_box(e, &pal).mt(px(4.))))
            .when(n > 0, |d| d.child(ui::section_label("Output", &pal).mt(px(14.))).child(card))
    }
}
