//! Inspect Apple provisioning profiles: what an app may be signed for.

use std::path::{Path, PathBuf};

use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder, px,
};

use super::codegen::notes_el;
use super::*;
use crate::logic::plist::{self, Profile};
use crate::logic::plural;
use crate::ui::{self, Tone};

/// Where a profile sits inside an app bundle: iOS, then macOS.
const IN_BUNDLE: &[&str] = &["embedded.mobileprovision", "Contents/embedded.provisionprofile"];

/// The profile at `path`: the file itself, or the one embedded in an app bundle.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = if path.is_dir() {
        IN_BUNDLE.iter().map(|rel| path.join(rel)).find(|p| p.is_file()).ok_or("This folder is not an app bundle with an embedded provisioning profile")?
    } else {
        path.to_path_buf()
    };
    std::fs::read(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))
}

pub struct ProfileView {
    name: Option<String>,
    profile: Option<Result<Profile, String>>,
}

impl ProfileView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { name: None, profile: None };
        // TEST-HOOK
        if let Some(p) = std::env::var_os("SIDEKIT_FILE") {
            this.open(p.into(), window, cx);
        }
        this
    }

    fn open(&mut self, path: PathBuf, _: &mut Window, cx: &mut Context<Self>) {
        self.name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.profile = Some(read(&path).and_then(|bytes| plist::inspect_profile(&bytes)));
        cx.notify();
    }
}

impl Render for ProfileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::get(cx);
        let hint: SharedString = match &self.name {
            Some(n) => n.clone().into(),
            None => "or drop a .mobileprovision, .provisionprofile or .app here".into(),
        };
        let root = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(drop_zone("profile-drop", "Choose profile…", hint, true, &pal, cx, Self::open))
            .child(div().text_size(px(12.)).text_color(pal.text3).child("Read on this computer. Profiles list team IDs, devices and entitlements, so they are never uploaded."));
        let p = match &self.profile {
            None => return root,
            Some(Err(e)) => return root.child(ui::err_box(e.clone(), &pal).mt(px(8.))),
            Some(Ok(p)) => p,
        };

        let (status, tone) = if p.days_left < 0 {
            (format!("Expired {} ago", plural(-p.days_left as usize, "day")), Tone::Err)
        } else if p.days_left < 30 {
            (format!("Expires in {}", plural(p.days_left as usize, "day")), Tone::Err)
        } else {
            (format!("Valid · {} left", plural(p.days_left as usize, "day")), Tone::Ok)
        };
        let store = if p.store_ready { ("App Store ready", Tone::Ok) } else { ("Not for the App Store", Tone::Neutral) };

        let mut facts = ui::kv_card(&pal);
        for (i, (k, v)) in p.rows.iter().enumerate() {
            facts = facts.child(ui::kv_copy_row(("profile-row", i), *k, 170., v.clone().into(), true, i + 1 == p.rows.len(), &pal, window, cx));
        }
        let mut entitlements = ui::kv_card(&pal);
        for (i, (k, v)) in p.entitlements.iter().enumerate() {
            entitlements = entitlements.child(ui::kv_copy_row(("profile-ent", i), k.clone(), 330., v.clone().into(), true, i + 1 == p.entitlements.len(), &pal, window, cx));
        }
        let mut certificates = ui::kv_card(&pal);
        for (i, c) in p.certificates.iter().enumerate() {
            let (when, tone) = if c.days_left < 0 {
                ("Expired".to_string(), Tone::Err)
            } else if c.before_profile {
                (format!("{} left · before the profile", plural(c.days_left as usize, "day")), Tone::Err)
            } else {
                (format!("{} left", plural(c.days_left as usize, "day")), Tone::Ok)
            };
            certificates = certificates.child(
                ui::kv_row(("profile-cert", i), i + 1 == p.certificates.len(), &pal)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .flex_1()
                            .min_w_0()
                            .py(px(4.))
                            .child(div().text_size(px(13.)).child(c.name.clone()))
                            .child(div().text_size(px(12.)).text_color(pal.text3).child(format!("Valid until {} · SHA-1 {}", c.expires, c.sha1))),
                    )
                    .child(ui::badge(when, tone, &pal))
                    .child(ui::copy_btn(("profile-cert-copy", i), c.sha1.clone().into(), &pal, window, cx)),
            );
        }
        let devices: SharedString = p.devices.join("\n").into();
        let section = |title: &'static str, count: usize| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .mt(px(16.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(ui::badge(count.to_string(), Tone::Neutral, &pal))
        };

        root.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .mt(px(14.))
                .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(p.name.clone()))
                .child(ui::badge(p.kind, Tone::Info, &pal))
                .child(ui::badge(status, tone, &pal))
                .child(ui::badge(store.0, store.1, &pal)),
        )
        .when(!p.warnings.is_empty(), |d| d.child(notes_el(&p.warnings, &pal)))
        .child(facts.mt(px(8.)))
        .when(!p.entitlements.is_empty(), |d| d.child(section("Entitlements", p.entitlements.len())).child(entitlements))
        .when(!p.certificates.is_empty(), |d| d.child(section("Signing certificates", p.certificates.len())).child(certificates))
        .when(!p.devices.is_empty(), |d| {
            d.child(section("Devices", p.devices.len()).child(div().flex_1()).child(ui::copy_btn_labeled("profile-devices", "Copy all", devices.clone(), &pal, window, cx)))
                .child(
                    ui::kv_card(&pal)
                        .p(px(14.))
                        .font_family(crate::theme::MONO_FONT)
                        .text_size(px(12.5))
                        .line_height(gpui_kit::relative(1.6))
                        .text_color(pal.text2)
                        .children(p.devices.iter().take(200).map(|d| div().child(d.clone())))
                        .when(p.devices.len() > 200, |c| c.child(div().text_color(pal.text3).child(format!("… and {} more (Copy all has every one)", p.devices.len() - 200)))),
                )
        })
    }
}
