//! The build-script half: writing the icon into a Windows executable.
//!
//! Included by path from both applications' build scripts rather than
//! exported from the library, because it speaks to cargo — `cargo:` lines on
//! stdout — and nothing but a build script may.

use std::path::PathBuf;
use std::process::Command;

/// Writes `app`'s icon and its name as a resource object and asks cargo to
/// link it into the binary named `slug`. A no-op off Windows, and a warning
/// where `windres` is not to be found.
///
/// The name matters as much as the picture: "Open with" and the Task Manager
/// call an executable by the description in its version resource, and one
/// without is called by its file name, "calx.exe".
pub fn stamp(slug: &str, display: &str, app: brand::App) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico = out.join(format!("{slug}.ico"));
    let rc = out.join(format!("{slug}.rc"));
    let obj = out.join(format!("{slug}-icon.o"));
    std::fs::write(&ico, brand::ico(app)).expect("write the icon");
    // Resource 1 is the one Explorer takes for the executable: the lowest id.
    // Forward slashes, because a resource script reads a backslash as an
    // escape.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let script = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "{display}"
      VALUE "ProductName", "Officina {display}"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "{slug}"
      VALUE "OriginalFilename", "{slug}.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 0x4B0
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "/"),
    );
    std::fs::write(&rc, script).expect("write the resource script");

    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
    let status = Command::new(&windres)
        .arg("-O")
        .arg("coff")
        .arg("-i")
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status();
    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bins={}", obj.display());
        }
        Ok(status) => {
            println!("cargo:warning={windres} failed ({status}); {slug}.exe gets no icon");
        }
        Err(err) => {
            println!("cargo:warning={windres} not found ({err}); {slug}.exe gets no icon");
        }
    }
}
