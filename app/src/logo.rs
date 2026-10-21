//! The RustyCorn logo (assets/RustyCorn.png): the window and taskbar icon, and
//! the mark at the left of the top bar. Both use pre-scaled copies so the
//! executable stays small and the small mark is sharp.

use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};

static ICON_PNG: &[u8] = include_bytes!("../../assets/icon-256.png");
static MARK_PNG: &[u8] = include_bytes!("../../assets/logo-64.png");

/// Window icon (taskbar, Alt-Tab). None only if the embedded PNG were broken.
pub fn icon() -> Option<Arc<egui::IconData>> {
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => Some(Arc::new(icon)),
        Err(err) => {
            log::warn!("Could not decode the window icon: {err}");
            None
        }
    }
}

/// The top-bar mark as a texture, loaded once per context.
pub fn mark(ctx: &egui::Context) -> Option<TextureHandle> {
    let id = egui::Id::unique("rustycorn-logo-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(tex);
    }
    let icon = eframe::icon_data::from_png_bytes(MARK_PNG).ok()?;
    let image =
        ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    let tex = ctx.load_texture("rustycorn-logo", image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}
