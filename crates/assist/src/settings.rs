//! What the user chose, kept in one file both applications read.
//!
//! **One choice serves Calx and Scriva**, so the file is neither's: the
//! applications put it in the suite's own configuration directory
//! (`~/.config/officina/assist.toml`). A key may be in it, so it is written
//! where only its owner can read it — mode 0600 on Unix; on Windows the file
//! is in the user's profile, whose permissions already keep other users out.
//!
//! **A file that cannot be read is the defaults**, and a value that cannot be
//! read is that value's default: a person who mistyped a model's name keeps
//! their key. Saving keeps whatever else the file holds.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use toml::{Table, Value};

use crate::anthropic::ADDRESS;
use crate::models::DEFAULT_CLAUDE_MODEL;

/// The file's name in the suite's configuration directory.
pub const FILE: &str = "assist.toml";

const HEADER: &str = "\
# Assist's settings, shared by Calx and Scriva.
# A key in this file is yours: the file is readable by you alone.

";

/// Which helper answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The helper on this computer.
    Local,
    Claude,
    Ollama,
    /// Another service, by address and key.
    Service,
}

/// Where Claude's credential comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClaudeLogin {
    /// A key given in the settings box.
    #[default]
    Key,
    /// `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`, as this computer has them.
    Environment,
    /// The login the `ant` command keeps.
    Ant,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClaudeSettings {
    pub login: ClaudeLogin,
    pub key: String,
    pub model: String,
    pub address: String,
    /// Whether a request the model declines goes to the server's fallback.
    pub fallback: bool,
}

impl std::fmt::Debug for ClaudeSettings {
    // A key is never written anywhere but the file, a log included.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeSettings")
            .field("login", &self.login)
            .field("key", &if self.key.is_empty() { "" } else { "…" })
            .field("model", &self.model)
            .field("address", &self.address)
            .field("fallback", &self.fallback)
            .finish()
    }
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        ClaudeSettings {
            login: ClaudeLogin::Key,
            key: String::new(),
            model: DEFAULT_CLAUDE_MODEL.to_owned(),
            address: ADDRESS.to_owned(),
            fallback: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaSettings {
    pub address: String,
    pub model: String,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        OllamaSettings {
            // Not `localhost`, which may be tried as ::1 first, where Ollama
            // does not listen by default.
            address: "http://127.0.0.1:11434".to_owned(),
            model: String::new(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ServiceSettings {
    pub address: String,
    pub key: String,
    pub model: String,
}

impl std::fmt::Debug for ServiceSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceSettings")
            .field("address", &self.address)
            .field("key", &if self.key.is_empty() { "" } else { "…" })
            .field("model", &self.model)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// `None` until the user has chosen, which is what brings up the first-run
    /// card.
    pub helper: Option<Choice>,
    pub claude: ClaudeSettings,
    pub ollama: OllamaSettings,
    pub service: ServiceSettings,
}

impl Settings {
    /// The settings in `path`, or the defaults where they cannot be read.
    pub fn load(path: &Path) -> Settings {
        Settings::read(path).unwrap_or_default()
    }

    /// The settings in `path`: the defaults when there is no file yet, and why
    /// not when there is one that cannot be read at all.
    pub fn read(path: &Path) -> Result<Settings, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(error) => return Err(error.to_string()),
        };
        let table: Table = text
            .parse()
            .map_err(|error: toml::de::Error| error.to_string())?;
        Ok(Settings::from_table(&table))
    }

    fn from_table(table: &Table) -> Settings {
        let defaults = Settings::default();
        let section = |name: &str| table.get(name).and_then(Value::as_table);
        let text = |name: &str, key: &str, default: &str| {
            section(name)
                .and_then(|section| section.get(key))
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_owned()
        };
        let helper = table
            .get("helper")
            .and_then(Value::as_str)
            .and_then(|word| match word {
                "local" => Some(Choice::Local),
                "claude" => Some(Choice::Claude),
                "ollama" => Some(Choice::Ollama),
                "service" => Some(Choice::Service),
                _ => None,
            });
        let login = match text("claude", "login", "key").as_str() {
            "environment" => ClaudeLogin::Environment,
            "ant" => ClaudeLogin::Ant,
            _ => ClaudeLogin::Key,
        };
        let fallback = section("claude")
            .and_then(|claude| claude.get("fallback"))
            .and_then(Value::as_bool)
            .unwrap_or(defaults.claude.fallback);
        Settings {
            helper,
            claude: ClaudeSettings {
                login,
                key: text("claude", "key", ""),
                model: text("claude", "model", &defaults.claude.model),
                address: text("claude", "address", &defaults.claude.address),
                fallback,
            },
            ollama: OllamaSettings {
                address: text("ollama", "address", &defaults.ollama.address),
                model: text("ollama", "model", ""),
            },
            service: ServiceSettings {
                address: text("service", "address", ""),
                key: text("service", "key", ""),
                model: text("service", "model", ""),
            },
        }
    }

    /// Writes the settings to `path`, where only the file's owner can read
    /// them, keeping whatever else the file already held.
    ///
    /// A file that is there and cannot be understood is not written over, since
    /// a key may be in it: it is moved aside to `assist.toml.unreadable` first.
    /// One that cannot be read at all is left alone, and the save fails.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut table = match fs::read_to_string(path) {
            Ok(text) => match text.parse::<Table>() {
                Ok(table) => table,
                Err(_) => {
                    fs::rename(path, path.with_extension("toml.unreadable"))?;
                    Table::new()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Table::new(),
            Err(error) => return Err(error),
        };
        let helper = self.helper.map(|choice| match choice {
            Choice::Local => "local",
            Choice::Claude => "claude",
            Choice::Ollama => "ollama",
            Choice::Service => "service",
        });
        match helper {
            Some(word) => table.insert("helper".into(), word.into()),
            None => table.remove("helper"),
        };
        let login = match self.claude.login {
            ClaudeLogin::Key => "key",
            ClaudeLogin::Environment => "environment",
            ClaudeLogin::Ant => "ant",
        };
        let set = |table: &mut Table, name: &str, values: Vec<(&str, Value)>| {
            let section = table
                .entry(name)
                .or_insert_with(|| Value::Table(Table::new()));
            if !section.is_table() {
                *section = Value::Table(Table::new());
            }
            if let Some(section) = section.as_table_mut() {
                for (key, value) in values {
                    section.insert(key.into(), value);
                }
            }
        };
        let claude = &self.claude;
        set(
            &mut table,
            "claude",
            vec![
                ("login", login.into()),
                ("key", claude.key.clone().into()),
                ("model", claude.model.clone().into()),
                ("address", claude.address.clone().into()),
                ("fallback", claude.fallback.into()),
            ],
        );
        set(
            &mut table,
            "ollama",
            vec![
                ("address", self.ollama.address.clone().into()),
                ("model", self.ollama.model.clone().into()),
            ],
        );
        let service = &self.service;
        set(
            &mut table,
            "service",
            vec![
                ("address", service.address.clone().into()),
                ("key", service.key.clone().into()),
                ("model", service.model.clone().into()),
            ],
        );
        let body = toml::to_string(&table).map_err(io::Error::other)?;
        write_private(path, &format!("{HEADER}{body}"))
    }
}

/// Writes `text` to `path` through a file only its owner can read, replacing
/// whatever was there in one step, so that a power cut leaves the old
/// settings or the new ones and never half of either.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Of this process: Calx and Scriva may save at the same moment, and each
    // must replace the file with a whole one of its own.
    let fresh = path.with_extension(format!("toml.{}.new", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&fresh)?;
    // A file left over from an interrupted save keeps the mode it was made
    // with, which may not be this one.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&fresh, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A directory of the test's own, removed when the test is over.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("officina-assist-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_key_is_written_to_a_file_only_its_owner_can_read() {
        let scratch = Scratch::new("key");
        let path = scratch.0.join("officina").join(FILE);
        let mut settings = Settings {
            helper: Some(Choice::Claude),
            ..Settings::default()
        };
        settings.claude.key = "sk-ant-api03-secret".into();
        settings.claude.model = "claude-sonnet-5".into();
        settings.save(&path).unwrap();
        assert_eq!(Settings::read(&path).unwrap(), settings);
        assert!(
            !format!("{settings:?}").contains("secret"),
            "a key is not written into a log"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            // A file someone made readable by everyone is private again once
            // a key is saved into it.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            settings.save(&path).unwrap();
            assert_eq!(mode(&path), 0o600);
        }
        let left: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, [FILE], "nothing is left beside the file");
    }

    #[test]
    fn settings_that_cannot_be_read_are_the_defaults() {
        let scratch = Scratch::new("unreadable");
        let path = scratch.0.join(FILE);
        assert_eq!(
            Settings::read(&path),
            Ok(Settings::default()),
            "no file yet"
        );
        fs::write(&path, "helper = [unterminated").unwrap();
        assert!(Settings::read(&path).is_err());
        assert_eq!(Settings::load(&path), Settings::default());

        // One value that cannot be read costs that value, not the key.
        fs::write(
            &path,
            "helper = \"banana\"\ntheme = \"mine\"\n\n[claude]\nkey = \"sk-kept\"\nfallback = \"yes\"\n\
             model = 5\n",
        )
        .unwrap();
        let settings = Settings::load(&path);
        assert_eq!(settings.helper, None);
        assert_eq!(settings.claude.key, "sk-kept");
        assert_eq!(settings.claude.model, DEFAULT_CLAUDE_MODEL);
        assert!(settings.claude.fallback);
        // And what the file held that is not Assist's survives a save.
        settings.save(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("theme = \"mine\""), "{saved}");
        assert_eq!(Settings::load(&path), settings);

        // A file that cannot be understood is moved aside, not written over.
        fs::write(
            &path,
            "[claude]\nkey = \"sk-precious\"\nmodel = \"a\"\nmodel = \"b\"\n",
        )
        .unwrap();
        assert!(Settings::read(&path).is_err(), "a key given twice");
        let chosen = Settings {
            helper: Some(Choice::Local),
            ..Settings::default()
        };
        chosen.save(&path).unwrap();
        assert_eq!(Settings::load(&path), chosen);
        let aside = fs::read_to_string(path.with_extension("toml.unreadable")).unwrap();
        assert!(aside.contains("sk-precious"), "{aside}");

        // One that cannot be read at all is left alone.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            // Whoever can read it anyway (root) has nothing to prove here.
            if fs::read_to_string(&path).is_err() {
                assert!(Settings::default().save(&path).is_err());
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                assert_eq!(Settings::load(&path), chosen, "the file was not replaced");
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}
