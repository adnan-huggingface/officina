//! Config, state and cache directory resolution.
//!
//! Requirement 7 asks for configuration and state under `~/.config/<app>/` — the
//! dotfile convention — on *both* platforms. That is deliberately not the Windows
//! norm (`%APPDATA%`), so it is implemented explicitly here rather than via a
//! platform-conventions crate that would "helpfully" do the Windows thing.
//!
//! `XDG_CONFIG_HOME` is honored when set, since a user who has set it has said
//! something more specific than the default.
//!
//! What is downloaded — Assist's helper, a gigabyte of it — is not
//! configuration, and goes under `~/.cache/<app>/` by the same rule, with
//! `XDG_CACHE_HOME` in place of `XDG_CONFIG_HOME`: a person who clears their
//! cache loses a download, never a setting.

use std::ffi::OsString;
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
    // A test's saves and opens are not the user's recent files.
    if crate::headless::active() {
        return Ok(crate::headless::config_base().join(app.slug));
    }
    users_dir(Kind::Config, app)
}

/// Returns the cache directory for `app`, creating it if absent.
///
/// Resolution order:
/// 1. `$XDG_CACHE_HOME/<slug>` if `XDG_CACHE_HOME` is set and absolute
/// 2. `<home>/.cache/<slug>`
pub fn cache_dir(app: AppId) -> std::io::Result<PathBuf> {
    let dir = cache_dir_path(app)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Computes the cache directory without touching the filesystem.
pub fn cache_dir_path(app: AppId) -> std::io::Result<PathBuf> {
    // A test downloads nothing, and finds nothing the user downloaded.
    if crate::headless::active() {
        return Ok(crate::headless::cache_base().join(app.slug));
    }
    users_dir(Kind::Cache, app)
}

/// Assist's settings: one file both applications read, in the suite's own
/// directory, since one choice of helper serves them both.
pub fn assist_settings() -> std::io::Result<PathBuf> {
    Ok(config_dir_path(crate::OFFICINA)?.join(assist::settings::FILE))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Config,
    Cache,
}

impl Kind {
    fn variable(self) -> &'static str {
        match self {
            Kind::Config => "XDG_CONFIG_HOME",
            Kind::Cache => "XDG_CACHE_HOME",
        }
    }

    fn dot_dir(self) -> &'static str {
        match self {
            Kind::Config => ".config",
            Kind::Cache => ".cache",
        }
    }
}

/// Where the user's own directory of `kind` for `app` is, by the platform's
/// rule.
fn users_dir(kind: Kind, app: AppId) -> std::io::Result<PathBuf> {
    by_rule(kind, std::env::var_os(kind.variable()), home_dir, app)
}

/// The rule itself, given what the environment says: the same on every
/// platform, and the same for both kinds bar the names.
fn by_rule(
    kind: Kind,
    set: Option<OsString>,
    home: impl FnOnce() -> std::io::Result<PathBuf>,
    app: AppId,
) -> std::io::Result<PathBuf> {
    let base = match set {
        Some(v) if !v.is_empty() && PathBuf::from(&v).is_absolute() => PathBuf::from(v),
        _ => home()?.join(kind.dot_dir()),
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
    use std::path::Path;

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
        // The rule itself, since the driver's tests in this binary make the
        // process headless and move the directory under the temporary one.
        let p = users_dir(Kind::Config, crate::CALX).unwrap();
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

    #[test]
    fn the_cache_directory_follows_the_config_directorys_rule_on_every_platform() {
        let home = PathBuf::from("/home/someone");
        let at_home = || Ok(PathBuf::from("/home/someone"));
        let absolute = std::env::temp_dir().join("elsewhere");
        let officina = crate::OFFICINA;
        assert_eq!(Kind::Cache.variable(), "XDG_CACHE_HOME");
        assert_eq!(
            by_rule(Kind::Cache, None, at_home, officina).unwrap(),
            home.join(".cache").join("officina")
        );
        assert_eq!(
            by_rule(
                Kind::Cache,
                Some(absolute.clone().into()),
                at_home,
                officina
            )
            .unwrap(),
            absolute.join("officina"),
            "a variable that names a directory is honored"
        );
        for ignored in ["", "relative/dir"] {
            assert_eq!(
                by_rule(Kind::Cache, Some(ignored.into()), at_home, officina).unwrap(),
                home.join(".cache").join("officina"),
                "a variable set to {ignored:?} says nothing"
            );
        }
        // Both kinds answer alike to the same environment.
        for set in [None, Some(absolute.clone().into()), Some("relative".into())] {
            let config = by_rule(Kind::Config, set.clone(), at_home, officina).unwrap();
            let cache = by_rule(Kind::Cache, set.clone(), at_home, officina).unwrap();
            match set {
                Some(dir) if Path::new(&dir).is_absolute() => assert_eq!(config, cache),
                _ => {
                    assert_eq!(config, home.join(".config").join("officina"));
                    assert_eq!(cache, home.join(".cache").join("officina"));
                }
            }
        }
        let homeless = || Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no home"));
        assert!(by_rule(Kind::Cache, None, homeless, officina).is_err());

        // Under a test, the cache is the process's own, apart from its
        // configuration.
        crate::headless::enter();
        let cache = cache_dir_path(officina).unwrap();
        assert!(cache.starts_with(std::env::temp_dir()), "{cache:?}");
        assert!(cache.ends_with("officina"));
        assert_ne!(cache, config_dir_path(officina).unwrap());
        assert_eq!(cache_dir(officina).unwrap(), cache);
        assert!(cache.is_dir());
    }

    #[test]
    fn assists_settings_are_one_file_for_both_applications() {
        crate::headless::enter();
        let path = assist_settings().unwrap();
        assert!(
            path.ends_with(Path::new("officina").join("assist.toml")),
            "{path:?}"
        );
        assert!(
            path.starts_with(std::env::temp_dir()),
            "a test's settings are not the user's"
        );
        let suite = config_dir_path(crate::OFFICINA).unwrap();
        assert_eq!(path.parent(), Some(suite.as_path()));
        for app in [crate::CALX, crate::SCRIVA] {
            assert_ne!(
                config_dir_path(app).unwrap(),
                suite,
                "{} has its own",
                app.display
            );
        }

        // What one application saves there, the other reads.
        let chosen = assist::Settings {
            helper: Some(assist::Choice::Ollama),
            ..assist::Settings::default()
        };
        chosen.save(&path).unwrap();
        assert_eq!(assist::Settings::load(&assist_settings().unwrap()), chosen);
        let _ = std::fs::remove_file(&path);
    }
}
