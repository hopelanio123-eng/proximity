//! Small, anti-aliased UI artwork.
//!
//! FLTK's line and circle primitives are not anti-aliased, which is very
//! visible on 20-ish pixel navigation icons. The shell therefore draws its
//! icons through nanosvg (via `SvgImage`), tinting a single glyph per state.
//! Images are cached per (glyph, color) in a thread-local map, so redrawing at
//! 60 Hz never re-parses the SVG.

use std::cell::RefCell;
use std::collections::HashMap;

use fltk::image::SvgImage;
use fltk::prelude::ImageExt;

use crate::theme::Rgb24;

pub const LOGO_SVG: &str = include_str!("../assets/logo.svg");
pub const LOGO_GLYPH_SVG: &str = include_str!("../assets/logo-glyph.svg");
pub const ICON_HOME_SVG: &str = include_str!("../assets/icons/home.svg");
pub const ICON_SETTINGS_SVG: &str = include_str!("../assets/icons/settings.svg");
/// Inline line icons for the address row (24px grid, 1.8px stroke). Statics,
/// not consts, so their address - the glyph cache key - is stable.
pub static ICON_COPY_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="__COLOR__" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2.5"/><path d="M15.5 8.5V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7.5a2 2 0 0 0 2 2h2.5"/></svg>"#;
pub static ICON_CHECK_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="__COLOR__" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>"#;

thread_local! {
    static GLYPHS: RefCell<HashMap<(usize, u32, i32, i32), SvgImage>> = RefCell::new(HashMap::new());
}

/// Draw a monochrome SVG glyph, replacing every `__COLOR__` with `color`.
pub fn draw_glyph(svg: &'static str, color: Rgb24, x: i32, y: i32, w: i32, h: i32) {
    let key = (svg.as_ptr() as usize, color_key(color), w, h);
    GLYPHS.with(|glyphs| {
        let mut glyphs = glyphs.borrow_mut();
        if !glyphs.contains_key(&key) {
            let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
            let data = svg.replace("__COLOR__", &hex);
            if let Ok(mut image) = SvgImage::from_data(&data) {
                // Pre-size the SVG so FLTK draws it 1:1 instead of clipping the
                // native 64 px raster into the small widget rectangle.
                image.scale(w, h, false, true);
                glyphs.insert(key, image);
            }
        }
        if let Some(image) = glyphs.get_mut(&key) {
            image.draw(x, y, w, h);
        }
    });
}

/// The full-colour app mark used for the OS window/taskbar icon.
pub fn app_icon() -> Option<SvgImage> {
    SvgImage::from_data(LOGO_SVG).ok()
}

fn color_key(color: Rgb24) -> u32 {
    ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | color.b() as u32
}
