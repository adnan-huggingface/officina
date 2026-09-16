//! Scroll bars in the suite's palette.
//!
//! egui's default bar floats over the content and is painted in the
//! widget's *foreground* colour, which under this theme is the ink: a
//! black stripe down the desk's edge, and another along its foot. The bar
//! here is solid — it takes its ten points beside the content, as Word's
//! does — on a chrome track, with a soft grey handle that darkens under the
//! pointer.
//!
//! The colours a scroll bar is painted with are the same visuals a checkbox
//! and a text field are painted with, so they cannot be set once for the
//! window: a grey `bg_fill` would fill every unticked box. [`show`] sets
//! them on a scope of its own and hands the content back the style it came
//! in with, so only the bar changes.

use crate::theme;
use eframe::egui;

/// The bar's style, for a [`egui::Style`] that will paint one.
pub fn bars(style: &mut egui::Style) {
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 10.0,
        handle_min_length: 24.0,
        bar_inner_margin: 2.0,
        ..egui::style::ScrollStyle::solid()
    };
    // The window's style turned egui's fade at a scroll area's foot off —
    // on the desk it was a white blur along the bottom of every frame —
    // and a style built from `solid()` would turn it back on.
    style.spacing.scroll.fade.strength = 0.0;
    let v = &mut style.visuals;
    v.extreme_bg_color = theme::CHROME;
    v.widgets.inactive.bg_fill = theme::INK_FAINT;
    v.widgets.hovered.bg_fill = theme::FIELD_EDGE_HOT;
    v.widgets.active.bg_fill = theme::INK_SOFT;
}

/// The width a bar takes from the content beside it — for a desk that
/// sizes its content to the space it has, so that the vertical bar's own
/// width does not put the content over by that much and call up a
/// horizontal bar for it.
pub fn takes() -> f32 {
    let mut style = egui::Style::default();
    bars(&mut style);
    let scroll = &style.spacing.scroll;
    scroll.bar_width + scroll.bar_inner_margin + scroll.bar_outer_margin
}

/// `area.show(ui, add)`, with the bars in the suite's palette and the
/// content in the style `ui` had.
pub fn show<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let content = ui.style().clone();
    ui.scope(|ui| {
        bars(ui.style_mut());
        area.show(ui, |ui| {
            ui.set_style(content);
            add(ui)
        })
    })
    .inner
}
