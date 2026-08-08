//! Config and state directory resolution.
//!
//! Requirement 7 asks for configuration and state under `~/.config/<app>/` — the
//! dotfile convention — on *both* platforms. That is deliberately not the Windows
//! norm (`%APPDATA%`), so it is implemented explicitly here rather than via a
//! platform-conventions crate that would "helpfully" do the Windows thing.
//!
//! `XDG_CONFIG_HOME` is honored when set, since a user who has set it has said
//! something more specific than the default.

use std::path::PathBuf;

use crate::AppId;

/// Returns the config/state directory for `app`, creating it if absent.
///
/// Resolution order:
/// 1. `$XDG_CONFIG_HOME/<slug>` if `XDG_CONFIG_HOME` is set and absolute
/// 2. `<home>/.config/<slug>`
pub fn config_dir(app: AppId) -> std::io::Result<PathBuf> {
    let dir = config_dir_path(app)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Computes the config directory without touching the filesystem.
pub fn config_dir_path(app: AppId) -> std::io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() && PathBuf::from(&v).is_absolute() => PathBuf::from(v),
        _ => home_dir()?.join(".config"),
    };
    Ok(base.join(app.slug))
}

fn home_dir() -> std::io::Result<PathBuf> {
    // `HOME` first: it is correct on Linux, and on Windows it is set by MSYS2,
    // Git Bash, and WSL-adjacent shells, where honoring it is what the user means.
    if let Some(h) = std::env::var_os("HOME") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    if let Some(p) = std::env::var_os("USERPROFILE") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "could not determine home directory: neither HOME nor USERPROFILE is set",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_ends_with_app_slug() {
        let p = config_dir_path(crate::CALX).expect("home dir should resolve in tests");
        assert!(
            p.ends_with("calx"),
            "expected path ending in `calx`, got {p:?}"
        );
    }

    #[test]
    fn defaults_under_dot_config_when_xdg_is_unset() {
        // Only meaningful when the environment has not overridden the base, so
        // the assertion is skipped rather than made flaky on machines that set it.
        if std::env::var_os("XDG_CONFIG_HOME").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let p = config_dir_path(crate::CALX).unwrap();
        assert!(
            p.parent().is_some_and(|parent| parent.ends_with(".config")),
            "expected parent `.config`, got {:?}",
            p.parent()
        );
    }

    #[test]
    fn apps_do_not_share_a_config_dir() {
        let a = config_dir_path(crate::CALX).unwrap();
        let b = config_dir_path(crate::SCRIVA).unwrap();
        assert_ne!(a, b);
    }
}
