//! The window icon, as the windowing system takes it.
//!
//! The picture itself is drawn in the `brand` crate, which has no
//! dependencies so that the build scripts can draw the same picture into the
//! executables; this is the wrapper that hands it to the viewport.

use crate::AppId;

/// The icon for `id`, at a size the taskbar will scale down rather than up.
pub fn icon(id: AppId) -> crate::egui::IconData {
    const SIZE: usize = 128;
    let app = brand::App::from_slug(id.slug).unwrap_or(brand::App::Calx);
    crate::egui::IconData {
        rgba: brand::rgba(app, SIZE),
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_application_gets_its_own_icon() {
        let calx = icon(crate::CALX);
        let scriva = icon(crate::SCRIVA);
        assert_eq!((calx.width, calx.height), (128, 128));
        assert_ne!(calx.rgba, scriva.rgba);
    }
}
