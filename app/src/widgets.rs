//! Small Studio-style building blocks for the app's own pages (Settings): page
//! header, setting rows with an explanation, a toggle switch.

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};
use rc_theme::{FS_BODY, FS_SMALL, medium, palette, semibold};

/// Widest a form page gets; wider windows centre it.
pub const PAGE_MAX_WIDTH: f32 = 860.0;
pub const FS_PAGE_TITLE: f32 = 22.0;
pub const FS_LABEL: f32 = 13.0;

/// A scrolling page on the dock background with a centred column.
pub fn page<R>(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let full = ui.available_width();
            let width = (full - 64.0).clamp(320.0, PAGE_MAX_WIDTH);
            let side = ((full - width) / 2.0).max(32.0);
            ui.add_space(28.0);
            let inner = ui
                .horizontal_top(|ui| {
                    ui.add_space(side - ui.spacing().item_spacing.x);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.set_max_width(width);
                        add_contents(ui)
                    })
                    .inner
                })
                .inner;
            ui.add_space(40.0);
            inner
        })
        .inner
}

/// Page title and the line explaining it.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    let t = palette(ui.ctx());
    ui.label(
        RichText::new(title)
            .font(semibold(FS_PAGE_TITLE))
            .color(t.text),
    );
    if !subtitle.is_empty() {
        ui.add_space(2.0);
        ui.label(
            RichText::new(subtitle)
                .font(egui::FontId::proportional(FS_BODY))
                .color(t.text_dim),
        );
    }
}

/// A form card: like `rc_theme::card` with roomier padding.
pub fn form_card<R>(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let t = palette(ui.ctx());
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .inner_margin(Margin {
            left: 20,
            right: 20,
            top: 16,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                rc_theme::section_title(ui, title);
                ui.add_space(6.0);
            }
            add_contents(ui)
        })
        .inner
}

/// A thin line between the rows of a card.
pub fn row_separator(ui: &mut Ui) {
    let t = palette(ui.ctx());
    ui.add_space(2.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, t.separator),
    );
    ui.add_space(2.0);
}

/// One setting: name and explanation on the left, the control right-aligned.
/// `control_width` is the room kept for the control.
pub fn setting_row<R>(
    ui: &mut Ui,
    label: &str,
    note: &str,
    control_width: f32,
    add_control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let t = palette(ui.ctx());
    let gap = 24.0;
    let text_width = (ui.available_width() - control_width - gap).max(160.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(text_width, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(text_width);
                ui.add_space(4.0);
                ui.label(RichText::new(label).font(medium(FS_LABEL)).color(t.text));
                if !note.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(note)
                                .font(egui::FontId::proportional(FS_SMALL + 0.5))
                                .color(t.text_dim),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(4.0);
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), add_control)
            .inner
    })
    .inner
}

/// An iOS-style switch in the accent colour.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let t = palette(ui.ctx());
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let off_fill = if response.hovered() {
            t.pressed
        } else {
            t.hover
        };
        let fill = lerp_color(off_fill, t.accent, how_on);
        let radius = rect.height() / 2.0;
        ui.painter().rect(
            rect,
            CornerRadius::same(radius as u8),
            fill,
            Stroke::new(1.0, if *on { t.accent } else { t.field_border }),
            egui::StrokeKind::Inside,
        );
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let knob = if t.dark || *on {
            Color32::WHITE
        } else {
            t.card
        };
        ui.painter().circle(
            egui::pos2(x, rect.center().y),
            radius - 3.0,
            knob,
            Stroke::new(0.5, t.shadow),
        );
    }
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

/// A green confirmation line under a form, or nothing when `text` is empty.
pub fn feedback(ui: &mut Ui, text: &str, color: Color32) {
    if text.is_empty() {
        return;
    }
    ui.add(egui::Label::new(RichText::new(text).font(medium(FS_BODY)).color(color)).wrap());
}
