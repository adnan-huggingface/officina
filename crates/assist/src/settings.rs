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
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::anthropic::ADDRESS;
use crate::http::host_of;
use crate::models::{claude_model, DEFAULT_CLAUDE_MODEL};

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
    /// The chosen helper in words, as the pane's header names it: "Claude —
    /// Opus 5", "Ollama — qwen3:1.7b".
    pub fn helper_name(&self) -> String {
        let named = |helper: &str, model: &str| match model.trim().is_empty() {
            true => helper.to_owned(),
            false => format!("{helper} — {}", model.trim()),
        };
        match self.helper {
            None => "No helper chosen".to_owned(),
            Some(Choice::Local) => "Helper on this computer".to_owned(),
            Some(Choice::Claude) => match claude_model(&self.claude.model) {
                Some(model) => named("Claude", model.name()),
                None => named("Claude", &self.claude.model),
            },
            Some(Choice::Ollama) => named("Ollama", &self.ollama.model),
            Some(Choice::Service) => named(&host_of(&self.service.address), &self.service.model),
        }
    }

    /// What saving these settings should leave in a file that holds
    /// `current` now, when they were edited from `opened`: each value the edit
    /// changed, and every other value as the file has it now. A box left open
    /// in one application then does not put back what the other application
    /// has saved since — a key replaced there, say.
    ///
    /// **Values that mean something only together are taken together**, all
    /// from the edit when it changed any of them: a key, the way of signing in
    /// it is for and the address it goes to; a service's address, key and
    /// model; Ollama's address and the model chosen from it. A key typed while
    /// the box showed one address is not sent to another.
    pub fn onto(&self, opened: &Settings, current: &Settings) -> Settings {
        fn pick<T: Clone + PartialEq>(edited: &T, opened: &T, current: &T) -> T {
            match edited == opened {
                true => current.clone(),
                false => edited.clone(),
            }
        }
        let sign_in =
            |claude: &ClaudeSettings| (claude.login, claude.key.clone(), claude.address.clone());
        let (claude, was, now) = (&self.claude, &opened.claude, &current.claude);
        let (login, mut key, address) = pick(&sign_in(claude), &sign_in(was), &sign_in(now));
        if login != ClaudeLogin::Key {
            // A key no sign-in of this one uses is not the edit's to change.
            key = pick(&claude.key, &was.key, &now.key);
        }
        Settings {
            helper: pick(&self.helper, &opened.helper, &current.helper),
            claude: ClaudeSettings {
                login,
                key,
                address,
                model: pick(&claude.model, &was.model, &now.model),
                fallback: pick(&claude.fallback, &was.fallback, &now.fallback),
            },
            ollama: pick(&self.ollama, &opened.ollama, &current.ollama),
            service: pick(&self.service, &opened.service, &current.service),
        }
    }

    /// Whether `other` names the same helper, asked the same way: the same
    /// service at the same address, the same model, the same sign-in. A key or
    /// a fallback may differ; the conversation can carry on across them.
    pub fn same_helper(&self, other: &Settings) -> bool {
        if self.helper != other.helper {
            return false;
        }
        match self.helper {
            Some(Choice::Claude) => {
                let (a, b) = (&self.claude, &other.claude);
                a.login == b.login && a.model == b.model && a.address == b.address
            }
            Some(Choice::Ollama) => self.ollama == other.ollama,
            Some(Choice::Service) => {
                let (a, b) = (&self.service, &other.service);
                a.address == b.address && a.model == b.model
            }
            Some(Choice::Local) | None => true,
        }
    }

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
    /// A file that is there and cannot be understood — not TOML, or not text
    /// at all, as one an editor saved in UTF-16 is not — is not written over,
    /// since a key may be in it. It is moved aside first, to
    /// `assist.toml.unreadable` or the first of `assist.toml.unreadable-2`,
    /// `-3`… that is free, so that a second such file does not replace the
    /// first, and the name it was given is returned. A file that cannot be
    /// read at all is left alone, and the save fails.
    pub fn save(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let mut aside = None;
        let mut table = match fs::read(path) {
            Ok(bytes) => {
                let parsed = std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|text| text.parse::<Table>().ok());
                match parsed {
                    Some(table) => table,
                    None => {
                        let to = free_aside(path);
                        fs::rename(path, &to)?;
                        aside = Some(to);
                        Table::new()
                    }
                }
            }
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
        write_private(path, &format!("{HEADER}{body}"))?;
        Ok(aside)
    }
}

/// The first name beside `path` that an unreadable file can be moved to
/// without replacing one moved there before.
fn free_aside(path: &Path) -> PathBuf {
    let first = path.with_extension("toml.unreadable");
    std::iter::once(first.clone())
        .chain((2..1000).map(|n| path.with_extension(format!("toml.unreadable-{n}"))))
        .find(|candidate| candidate.symlink_metadata().is_err())
        .unwrap_or(first)
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
    // Made new, never opened through whatever stands at that name: a file an
    // interrupted save left there is removed first, and a link someone put
    // there is removed rather than followed.
    match fs::remove_file(&fresh) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&fresh)?;
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

        // A link standing where the new file is made is not followed.
        #[cfg(unix)]
        {
            let elsewhere = scratch.0.join("elsewhere");
            fs::write(&elsewhere, "untouched").unwrap();
            let fresh = path.with_extension(format!("toml.{}.new", std::process::id()));
            std::os::unix::fs::symlink(&elsewhere, &fresh).unwrap();
            settings.save(&path).unwrap();
            assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "untouched");
            assert_eq!(Settings::read(&path).unwrap(), settings);
            assert!(fresh.symlink_metadata().is_err(), "and the link is gone");
        }
    }

    /// Saving an edit keeps what the file gained meanwhile, and a key or a
    /// fallback changed is still the same helper.
    #[test]
    fn an_edit_is_saved_onto_the_file_as_it_is_now() {
        let opened = Settings {
            helper: Some(Choice::Claude),
            ..Settings::default()
        };
        let mut current = opened.clone();
        current.claude.key = "sk-ant-new-from-the-other-window".into();
        current.ollama.model = "qwen3:1.7b".into();
        let mut edited = opened.clone();
        edited.claude.fallback = false;
        let saved = edited.onto(&opened, &current);
        assert_eq!(saved.claude.key, "sk-ant-new-from-the-other-window");
        assert_eq!(saved.ollama.model, "qwen3:1.7b");
        assert!(!saved.claude.fallback, "and the edit");
        let mut chose = opened.clone();
        chose.helper = Some(Choice::Ollama);
        assert_eq!(chose.onto(&opened, &current).helper, Some(Choice::Ollama));

        assert!(
            saved.same_helper(&opened),
            "a key and a fallback are not a helper"
        );
        let mut model = opened.clone();
        model.claude.model = "claude-haiku-4-5".into();
        assert!(!model.same_helper(&opened));
        let mut login = opened.clone();
        login.claude.login = ClaudeLogin::Ant;
        assert!(!login.same_helper(&opened));
        assert!(!chose.same_helper(&opened));
        let mut service = Settings {
            helper: Some(Choice::Service),
            ..Settings::default()
        };
        let before = service.clone();
        service.service.key = "sk-other".into();
        assert!(service.same_helper(&before));
        service.service.address = "https://elsewhere.example.com/v1".into();
        assert!(!service.same_helper(&before));
    }

    /// A key goes where the box that took it said, signed in as it said: a
    /// key typed while the box showed one address or one way of signing in is
    /// saved with those, whatever the other window saved meanwhile. So is a
    /// model with the server it was chosen from. Values that stand alone —
    /// Claude's model, the fallback — are still taken one by one.
    #[test]
    fn a_key_comes_with_its_sign_in_and_its_address_and_a_model_with_its_server() {
        let mut opened = Settings {
            helper: Some(Choice::Claude),
            ..Settings::default()
        };
        opened.claude.key = "sk-ant-old".into();
        opened.service.address = "https://shown.example.com/v1".into();
        opened.service.key = "sk-shown".into();
        opened.service.model = "shown-model".into();
        opened.ollama.model = "qwen3:1.7b".into();

        // The other window signs in with ant, and moves the service and
        // Ollama elsewhere.
        let mut current = opened.clone();
        current.claude.login = ClaudeLogin::Ant;
        current.claude.model = "claude-haiku-4-5".into();
        current.service.address = "https://other.example.com/v1".into();
        current.ollama.address = "http://gpu.example.com:11434".into();

        // This box pastes a key, and gives the service a new one.
        let mut edited = opened.clone();
        edited.claude.key = "sk-ant-typed".into();
        edited.service.key = "sk-typed".into();
        edited.ollama.model = "llama3.2".into();
        let saved = edited.onto(&opened, &current);
        assert_eq!(
            (saved.claude.login, saved.claude.key.as_str()),
            (ClaudeLogin::Key, "sk-ant-typed"),
            "the key, with the sign-in it was typed for"
        );
        assert_eq!(
            saved.claude.model, "claude-haiku-4-5",
            "a model is not part of a sign-in"
        );
        assert_eq!(
            saved.service, edited.service,
            "the service's key, with the address it was typed for"
        );
        assert_eq!(
            saved.ollama, edited.ollama,
            "a model, with the server it was chosen from"
        );

        // A box that left the sign-in alone takes the other window's, key and
        // all; one that chose ant keeps the file's key beside it.
        let mut untouched = opened.clone();
        untouched.claude.fallback = false;
        let saved = untouched.onto(&opened, &current);
        assert_eq!(saved.claude, {
            let mut claude = current.claude.clone();
            claude.fallback = false;
            claude
        });
        assert_eq!(saved.service, current.service);
        assert_eq!(saved.ollama, current.ollama);
        let mut newer = current.clone();
        newer.claude.login = ClaudeLogin::Key;
        newer.claude.key = "sk-ant-newer".into();
        let mut ant = opened.clone();
        ant.claude.login = ClaudeLogin::Ant;
        let saved = ant.onto(&opened, &newer);
        assert_eq!(saved.claude.login, ClaudeLogin::Ant);
        assert_eq!(saved.claude.key, "sk-ant-newer", "a key ant does not use");
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
        let set_aside = chosen.save(&path).unwrap();
        assert_eq!(set_aside, Some(path.with_extension("toml.unreadable")));
        assert_eq!(Settings::load(&path), chosen);
        let aside = fs::read_to_string(path.with_extension("toml.unreadable")).unwrap();
        assert!(aside.contains("sk-precious"), "{aside}");

        // Nor is one that is not text at all — Notepad's old "Unicode" — and a
        // second such file is kept beside the first, not over it.
        let utf16: Vec<u8> = "\u{feff}helper = \"claude\"\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(&path, &utf16).unwrap();
        assert!(Settings::read(&path).is_err());
        let aside = chosen.save(&path).unwrap().expect("set aside");
        assert!(aside.ends_with("assist.toml.unreadable-2"), "{aside:?}");
        assert_eq!(fs::read(&aside).unwrap(), utf16);
        let first = fs::read_to_string(path.with_extension("toml.unreadable")).unwrap();
        assert!(first.contains("sk-precious"), "the first is still there");
        assert_eq!(
            chosen.save(&path).unwrap(),
            None,
            "nothing to set aside now"
        );

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
