//! Shared application shell for Calx and Scriva.
//!
//! Window setup, config/state directory resolution, and the chrome both apps have
//! in common. The document surfaces themselves are *not* built from egui layout —
//! each app owns a custom widget that lays out and paints directly (see `DESIGN.md` §6).

#![forbid(unsafe_code)]

pub mod dialog;
pub mod fonts;
pub mod paths;
pub mod recent;
pub mod shell;

pub use eframe::{self, egui};
pub use fonts::Family;
pub use recent::Recent;
pub use shell::{run, DocumentApp};

/// Identity of a host application, used for window titles and config paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppId {
    /// Lowercase, filesystem-safe name. Also the config directory name.
    pub slug: &'static str,
    /// Human-readable name shown in the window title.
    pub display: &'static str,
}

pub const CALX: AppId = AppId {
    slug: "calx",
    display: "Calx",
};
pub const SCRIVA: AppId = AppId {
    slug: "scriva",
    display: "Scriva",
};
