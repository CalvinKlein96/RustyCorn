//! RustyCorn design system: colour tokens, fonts, egui style and a few small
//! building blocks (cards, chips, section titles).
//!
//! Colour tokens and style setup adapted from the 'Studio' theme of PhotoCraft
//! (<https://github.com/storytold/photocraft>, MIT OR Apache-2.0).
//!
//! Near-black surfaces in dark mode (airy light surfaces in light mode),
//! rounded cards, Inter for text and JetBrains Mono for numbers, a soft violet
//! accent.
//! Trace and flow-path colours keep one hue per channel and part in both
//! modes; the light palette's trace colours are also those of the PDF export
//! (`rc_plotdata::Channel::light_color`), so screen and paper match.
//!
//! Every widget reads [`palette`] instead of hard-coding colours, so light and
//! dark mode differ only here.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Stroke,
    TextStyle, Visuals,
};

/// Type scale.
pub const FS_SMALL: f32 = 11.0;
pub const FS_BODY: f32 = 12.5;
pub const FS_TITLE: f32 = 10.5;
pub const FS_HEADING: f32 = 15.0;
pub const FS_READING: f32 = 22.0;

/// Colour tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    // ── surfaces ──
    /// window chrome: top bar, toolbars, status bar (egui panel fill)
    pub chrome: Color32,
    /// page background behind cards
    pub dock: Color32,
    /// cards, plot areas
    pub card: Color32,
    pub card_border: Color32,
    /// inputs, fields, dropdowns
    pub field: Color32,
    pub field_border: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub separator: Color32,
    pub shadow: Color32,
    // ── text ──
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub icon: Color32,
    // ── accent ──
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_border: Color32,
    pub accent_text: Color32,
    /// primary (call-to-action) button: fill and text
    pub primary_bg: Color32,
    pub primary_text: Color32,
    // ── states ──
    /// connected, live
    pub ok: Color32,
    /// waiting, viewing the past
    pub warn: Color32,
    /// port missing, unknown valve state, errors
    pub err: Color32,
    /// demo mode
    pub demo: Color32,
    /// recording
    pub rec: Color32,
    // ── plots ──
    pub plot_bg: Color32,
    pub axis: Color32,
    pub grid: Color32,
    pub uv: Color32,
    pub pctb: Color32,
    pub cond: Color32,
    pub ph: Color32,
    pub pres: Color32,
    pub flow: Color32,
    /// event marks: their plot line and run log entry (CSS darkorange)
    pub mark: Color32,
    /// fraction span fills, alternating from tube to tube (a light and a
    /// slightly darker grey, translucent); paper uses rc-plotdata `FRAC_COLORS`
    pub frac_fill: [Color32; 2],
    /// idle pipes, inactive tracks
    pub faint: Color32,
    // ── flow path schematic ──
    pub buf_a: Color32,
    pub buf_port: Color32,
    pub sample: Color32,
    pub buf_b: Color32,
    pub pipe: Color32,
    pub frac: Color32,
    pub node: Color32,
    pub node_text: Color32,
    // ── geometry ──
    pub radius_sm: f32,
    pub radius: f32,
    pub radius_lg: f32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// An opaque colour from 0–1 float components.
pub fn rgbf(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

/// `[r, g, b]` floats (rc-plotdata colours) as a colour.
pub fn rgb3(c: [f32; 3]) -> Color32 {
    rgbf(c[0], c[1], c[2])
}

impl Palette {
    /// Studio (dark): near-black surfaces, soft violet accent.
    pub fn studio_dark() -> Palette {
        Palette {
            dark: true,
            chrome: rgb(20, 20, 21),
            dock: rgb(14, 14, 15),
            card: rgb(26, 26, 28),
            card_border: rgb(40, 40, 44),
            field: rgb(35, 35, 38),
            field_border: rgb(52, 52, 57),
            hover: rgb(44, 44, 48),
            pressed: rgb(56, 56, 62),
            separator: rgb(38, 38, 42),
            shadow: Color32::from_black_alpha(140),
            text: rgb(236, 236, 240),
            text_dim: rgb(150, 150, 158),
            text_faint: rgb(96, 96, 104),
            icon: rgb(196, 196, 204),
            accent: rgb(139, 124, 246),
            accent_soft: Color32::from_rgba_unmultiplied(139, 124, 246, 46),
            accent_border: Color32::from_rgba_unmultiplied(160, 148, 255, 110),
            accent_text: rgb(214, 208, 255),
            primary_bg: rgb(246, 246, 248),
            primary_text: rgb(12, 12, 14),
            ok: rgbf(0.30, 0.76, 0.46),
            warn: rgb(240, 190, 90),
            err: rgb(240, 96, 96),
            demo: rgbf(0.55, 0.57, 1.0),
            rec: rgbf(0.97, 0.30, 0.30),
            plot_bg: rgb(26, 26, 28),
            axis: rgb(64, 64, 70),
            grid: rgb(34, 34, 38),
            uv: rgbf(0.40, 0.62, 1.0),
            pctb: rgbf(0.35, 0.80, 0.45),
            cond: rgbf(0.98, 0.45, 0.35),
            ph: rgbf(0.78, 0.55, 0.95),
            pres: rgbf(0.98, 0.70, 0.25),
            flow: rgbf(0.30, 0.78, 0.85),
            mark: rgb(255, 140, 0),
            // over plot_bg 26: about 34 and 40
            frac_fill: [
                Color32::from_rgba_unmultiplied(200, 204, 212, 12),
                Color32::from_rgba_unmultiplied(200, 204, 212, 20),
            ],
            faint: rgb(58, 58, 64),
            buf_a: rgbf(0.92, 0.38, 0.38),
            buf_port: rgbf(0.98, 0.65, 0.30),
            sample: rgbf(0.78, 0.52, 0.95),
            buf_b: rgbf(0.95, 0.82, 0.30),
            pipe: rgbf(0.42, 0.63, 1.0),
            frac: rgbf(0.35, 0.80, 0.50),
            node: rgb(35, 35, 38),
            node_text: rgb(226, 226, 232),
            radius_sm: 6.0,
            radius: 8.0,
            radius_lg: 12.0,
        }
    }

    /// Studio Light: the same system on light surfaces.
    pub fn studio_light() -> Palette {
        Palette {
            dark: false,
            chrome: rgb(246, 246, 248),
            dock: rgb(240, 240, 243),
            card: rgb(252, 252, 253),
            card_border: rgb(222, 222, 228),
            field: rgb(242, 242, 245),
            field_border: rgb(214, 214, 220),
            hover: rgb(232, 232, 237),
            pressed: rgb(220, 220, 226),
            separator: rgb(226, 226, 232),
            shadow: Color32::from_black_alpha(50),
            text: rgb(24, 24, 28),
            text_dim: rgb(96, 96, 106),
            text_faint: rgb(150, 150, 160),
            icon: rgb(60, 60, 68),
            accent: rgb(108, 92, 231),
            accent_soft: Color32::from_rgba_unmultiplied(108, 92, 231, 36),
            accent_border: Color32::from_rgba_unmultiplied(108, 92, 231, 120),
            accent_text: rgb(80, 64, 200),
            primary_bg: rgb(20, 20, 24),
            primary_text: rgb(250, 250, 252),
            ok: rgbf(0.12, 0.58, 0.32),
            warn: rgbf(0.86, 0.55, 0.08),
            err: rgb(210, 60, 60),
            demo: rgbf(0.38, 0.40, 0.88),
            rec: rgbf(0.86, 0.15, 0.15),
            plot_bg: rgb(252, 252, 253),
            axis: rgb(200, 200, 208),
            grid: rgb(238, 238, 242),
            uv: rgbf(0.15, 0.35, 0.70),
            pctb: rgbf(0.10, 0.55, 0.25),
            cond: rgbf(0.80, 0.20, 0.10),
            ph: rgbf(0.55, 0.25, 0.65),
            pres: rgbf(0.75, 0.45, 0.00),
            flow: rgbf(0.10, 0.45, 0.55),
            // a little deeper than darkorange, for text on light cards
            mark: rgb(226, 116, 0),
            // over plot_bg 252: about 242 and 235
            frac_fill: [
                Color32::from_rgba_unmultiplied(179, 184, 191, 36),
                Color32::from_rgba_unmultiplied(128, 133, 143, 36),
            ],
            faint: rgbf(0.80, 0.82, 0.85),
            buf_a: rgbf(0.85, 0.27, 0.27),
            buf_port: rgbf(0.95, 0.58, 0.25),
            sample: rgbf(0.68, 0.38, 0.82),
            buf_b: rgbf(0.93, 0.78, 0.15),
            pipe: rgbf(0.20, 0.50, 0.85),
            frac: rgbf(0.20, 0.65, 0.40),
            node: rgb(242, 242, 245),
            node_text: rgbf(0.20, 0.22, 0.26),
            radius_sm: 6.0,
            radius: 8.0,
            radius_lg: 12.0,
        }
    }

    pub fn for_mode(dark: bool) -> Palette {
        if dark {
            Palette::studio_dark()
        } else {
            Palette::studio_light()
        }
    }

    /// Soft fill of a state colour blended onto the card, so it stays opaque:
    /// pipes ending under a node must not show through its label.
    pub fn tint(&self, c: Color32, a: f32) -> Color32 {
        let mix = |x: u8, y: u8| (a * x as f32 + (1.0 - a) * y as f32).round() as u8;
        Color32::from_rgb(
            mix(c.r(), self.card.r()),
            mix(c.g(), self.card.g()),
            mix(c.b(), self.card.b()),
        )
    }

    /// Colour of a connection status text, from its wording.
    pub fn status_color(&self, txt: &str) -> Color32 {
        if txt.starts_with("Demo") {
            self.demo
        } else if txt.starts_with('●') {
            self.ok
        } else if txt.contains("not available") || txt.starts_with("Stopped") {
            self.err
        } else {
            self.warn
        }
    }
}

fn palette_id() -> egui::Id {
    egui::Id::unique("rustycorn-palette")
}

/// The active palette (Studio dark if `apply` has not run yet).
pub fn palette(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palette>(palette_id()))
        .unwrap_or_else(Palette::studio_dark)
}

static INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
static JETBRAINS_MONO: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

/// Register Inter (UI), JetBrains Mono (numbers) and the named weights
/// "medium" and "semibold". egui's default fonts stay last in every stack,
/// for symbols Inter lacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("Inter", INTER_REGULAR);
    add("Inter-Medium", INTER_MEDIUM);
    add("Inter-SemiBold", INTER_SEMIBOLD);
    add("JetBrainsMono", JETBRAINS_MONO);
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, font) in [("medium", "Inter-Medium"), ("semibold", "Inter-SemiBold")] {
        let mut stack = vec![font.to_owned()];
        stack.extend(fallback.iter().filter(|f| f.as_str() != "Inter").cloned());
        fonts
            .families
            .insert(FontFamily::Name(family.into()), stack);
    }
    ctx.set_fonts(fonts);
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Apply the palette to egui (visuals, spacing, text styles). Call once at
/// start (after `install_fonts`) and whenever dark mode is switched.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let t = Palette::for_mode(dark);
    ctx.data_mut(|d| d.insert_temp(palette_id(), t));
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });

    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = t.chrome;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0, t.card_border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.card;
    v.code_bg_color = t.field;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.err;
    v.window_corner_radius = CornerRadius::same(t.radius_lg as u8);
    v.menu_corner_radius = CornerRadius::same(t.radius as u8);
    v.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 32,
        spread: 0,
        color: t.shadow,
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: t.shadow,
    };
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent_text);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    let r = CornerRadius::same(t.radius_sm as u8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.card;
    w.noninteractive.weak_bg_fill = t.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text_dim);
    w.noninteractive.corner_radius = r;
    for (wv, bg, stroke) in [
        (&mut w.inactive, t.field, t.field_border),
        (&mut w.hovered, t.hover, t.field_border),
        (&mut w.active, t.pressed, t.accent_border),
        (&mut w.open, t.hover, t.field_border),
    ] {
        wv.bg_fill = bg;
        wv.weak_bg_fill = bg;
        wv.bg_stroke = Stroke::new(1.0, stroke);
        wv.fg_stroke = Stroke::new(1.0, t.text);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }

    let mut style = (*ctx.global_style()).clone();
    style.visuals = v;
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(FS_SMALL)),
        (TextStyle::Body, FontId::proportional(FS_BODY)),
        (TextStyle::Button, FontId::proportional(FS_BODY)),
        (TextStyle::Heading, semibold(FS_HEADING)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.interact_size = egui::vec2(24.0, 24.0);
    style.spacing.slider_width = 150.0;
    style.spacing.combo_width = 120.0;
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.icon_width = 14.0;
    style.spacing.tooltip_width = 280.0;
    style.visuals.indent_has_left_vline = false;
    style.interaction.tooltip_delay = 0.35;
    let mut scroll = egui::style::ScrollStyle::thin();
    scroll.interact_handle_opacity = 1.0;
    scroll.interact_background_opacity = 0.4;
    scroll.floating_width = 4.0;
    style.spacing.scroll = scroll;
    // Same style whatever the OS appearance: RustyCorn has its own dark mode setting.
    let style = Arc::new(style);
    ctx.set_style_of(egui::Theme::Light, (*style).clone());
    ctx.set_style_of(egui::Theme::Dark, (*style).clone());
}

// ── Building blocks ──────────────────────────────────────────────────────────

/// A card: rounded panel with a small upper-case muted title.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = palette(ui.ctx());
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius as u8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(
                    RichText::new(title.to_uppercase())
                        .font(semibold(FS_TITLE))
                        .color(t.text_faint),
                );
                ui.add_space(2.0);
            }
            add_contents(ui)
        })
        .inner
}

/// A chip: small rounded badge (connection, REC, …).
pub fn chip(ui: &mut egui::Ui, text: &str, fill: Color32, text_color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(semibold(FS_SMALL))
                    .color(text_color),
            )
        })
        .response
}

/// A soft chip: the colour as text on a tinted background (Studio style).
pub fn soft_chip(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let t = palette(ui.ctx());
    chip(ui, text, t.tint(color, 0.18), color)
}

/// Upper-case muted section title inside a page.
pub fn section_title(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = palette(ui.ctx());
    ui.label(
        RichText::new(text.to_uppercase())
            .font(semibold(FS_TITLE))
            .color(t.text_faint),
    )
}

/// A primary (call-to-action) button: solid high-contrast fill.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = palette(ui.ctx());
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .font(medium(FS_BODY))
                .color(t.primary_text),
        )
        .fill(t.primary_bg)
        .corner_radius(CornerRadius::same(t.radius_sm as u8)),
    )
}

/// A flat tab: text on the chrome, the active one in accent with an underline.
pub fn tab(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    let t = palette(ui.ctx());
    let color = if active { t.text } else { t.text_dim };
    let font = if active {
        semibold(FS_BODY)
    } else {
        medium(FS_BODY)
    };
    let resp = ui.add(egui::Button::new(RichText::new(text).font(font).color(color)).frame(false));
    if active {
        let r = resp.rect;
        ui.painter().line_segment(
            [
                egui::pos2(r.left() + 2.0, r.bottom() + 3.0),
                egui::pos2(r.right() - 2.0, r.bottom() + 3.0),
            ],
            Stroke::new(2.0, t.accent),
        );
    }
    resp
}

/// A large reading: value in mono, name in its colour (readings card).
pub fn reading(ui: &mut egui::Ui, name: &str, value: &str, color: Color32) {
    let t = palette(ui.ctx());
    ui.vertical(|ui| {
        ui.label(RichText::new(name).font(medium(FS_SMALL)).color(color));
        ui.label(RichText::new(value).font(mono(FS_READING)).color(t.text));
    });
}
