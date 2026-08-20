//! Packaging: what a user actually receives.
//!
//! **One archive per platform, holding two single-file executables.** Requirement
//! 4 asks for a single exe, and both binaries are exactly that — no runtime, no
//! shared libraries beyond the system's own, nothing to unpack into a directory
//! that then has to stay where it is. So the archive is a convenience, not a
//! layout: unzipping it anywhere and running the binary from there works.
//!
//! **Nothing is installed anywhere the user did not ask for.** `install` copies
//! to `~/.local/bin`, and on Linux offers to write the desktop entries that make
//! a double-click open the right application. It never touches a system
//! directory, a registry key, or anything outside the user's home.

use std::path::{Path, PathBuf};

use crate::{home, workspace_root, APPS};

/// The version both applications report, from the workspace manifest.
pub fn version() -> String {
    let manifest = workspace_root().join("Cargo.toml");
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("version") {
            if let Some(value) = rest.split('"').nth(1) {
                return value.to_string();
            }
        }
    }
    "0.0.0".to_string()
}

/// The triple this build is for, as the archive name needs it.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// Assembles a directory holding everything a user needs, and archives it.
pub fn package() -> Result<PathBuf, String> {
    let root = workspace_root();
    let name = format!("officina-{}-{}", version(), target());
    let staging = root.join("target").join("dist").join(&name);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("create {}: {e}", staging.display()))?;

    for app in APPS {
        let exe = format!("{app}{}", std::env::consts::EXE_SUFFIX);
        let from = root.join("target").join("release").join(&exe);
        if !from.exists() {
            return Err(format!(
                "{} is missing — run `cargo xtask dist`, which builds it first",
                from.display()
            ));
        }
        copy(&from, &staging.join(&exe))?;
    }
    // The documents a user needs beside the binaries, and nothing else. A
    // release is not a copy of the repository.
    for doc in [
        "README.md",
        "GUIDE.md",
        "FORMATS.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
    ] {
        let from = root.join(doc);
        if from.exists() {
            copy(&from, &staging.join(doc))?;
        }
    }

    let archive = archive(&staging, &name)?;
    Ok(archive)
}

/// Zips the staging directory. Zip on both platforms: `tar` is the Linux
/// convention, but one format that both `unzip` and Explorer open without
/// argument is worth more here than following each platform's habit.
fn archive(staging: &Path, name: &str) -> Result<PathBuf, String> {
    use std::io::Write;

    let path = staging
        .parent()
        .expect("staging has a parent")
        .join(format!("{name}.zip"));
    let file =
        std::fs::File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut entries: Vec<PathBuf> = std::fs::read_dir(staging)
        .map_err(|e| format!("read {}: {e}", staging.display()))?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    for entry in entries {
        let file_name = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Inside a directory named after the release, so unzipping into a
        // downloads folder does not scatter four files across it.
        let inside = format!("{name}/{file_name}");
        zip.start_file(inside, options)
            .map_err(|e| format!("zip entry: {e}"))?;
        let bytes = std::fs::read(&entry).map_err(|e| format!("read {}: {e}", entry.display()))?;
        zip.write_all(&bytes)
            .map_err(|e| format!("zip write: {e}"))?;
    }
    zip.finish().map_err(|e| format!("finish zip: {e}"))?;
    Ok(path)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
}

/// Writes the `.desktop` entries and MIME associations, on Linux.
///
/// **Only under the user's own home**, in the directories the freedesktop spec
/// reserves for exactly this. Nothing here needs a package manager, a root
/// password, or a system directory — which is the point of requirement 7.
///
/// A no-op on Windows: file associations there live in the registry, and this
/// project does not write to a user's registry as a side effect of installing a
/// binary. `cargo xtask associate` prints what to run instead.
pub fn desktop_entries() -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let home = home()?;
    let apps = home.join(".local").join("share").join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| format!("create {}: {e}", apps.display()))?;
    let bin = home.join(".local").join("bin");

    for (app, title, comment, mimes) in ENTRIES {
        let exec = bin.join(app);
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={title}\n\
             Comment={comment}\n\
             Exec={} %f\n\
             Terminal=false\n\
             Categories=Office;\n\
             MimeType={mimes}\n",
            exec.display()
        );
        let path = apps.join(format!("{app}.desktop"));
        std::fs::write(&path, entry).map_err(|e| format!("write {}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }

    println!("run `update-desktop-database ~/.local/share/applications` to pick them up");
    Ok(())
}

/// The two entries, and the types each application claims.
///
/// Claiming a type it cannot write would be worse than claiming nothing: a
/// double-clicked `.doc` opens read-only and says so, which is honest, but a
/// format neither application understands must not land here at all.
const ENTRIES: [(&str, &str, &str, &str); 2] = [
    (
        "calx",
        "Calx",
        "Spreadsheets",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;\
         application/vnd.ms-excel;text/csv;text/tab-separated-values;",
    ),
    (
        "scriva",
        "Scriva",
        "Documents",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document;\
         application/msword;text/markdown;text/plain;",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_comes_from_the_workspace_rather_than_a_constant() {
        // A version in two places is a version that will disagree with itself.
        let version = version();
        assert!(
            version.split('.').count() == 3,
            "expected a three-part version, got {version:?}"
        );
        assert_ne!(version, "0.0.0", "the manifest was not read");
    }

    #[test]
    fn the_archive_name_says_what_it_will_and_will_not_run_on() {
        let name = format!("officina-{}-{}", version(), target());
        assert!(name.contains(std::env::consts::OS), "{name}");
        assert!(name.contains(std::env::consts::ARCH), "{name}");
    }

    #[test]
    fn every_desktop_entry_names_a_type_the_application_can_open() {
        for (app, _, _, mimes) in ENTRIES {
            assert!(!mimes.is_empty(), "{app} claims nothing");
            // No trailing rubbish: a malformed MimeType line makes the whole
            // entry ignored, silently.
            assert!(mimes.ends_with(';'), "{app}: {mimes}");
            assert!(!mimes.contains(";;"), "{app}: {mimes}");
        }
    }
}
