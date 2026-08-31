//! Build, package, install, and verification tasks.
//!
//! Run via the cargo alias: `cargo xtask <command>`.
//!
//! Deliberately dependency-free — this is the thing that has to keep working when
//! the rest of the workspace does not build.

mod dist;
mod fidelity;
mod perf;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub const APPS: [&str; 2] = ["calx", "scriva"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => {
            usage();
            return ExitCode::FAILURE;
        }
    };

    let result = match cmd {
        "install" => install(),
        "dist" => build_dist(),
        "package" => package(),
        "associate" => associate(),
        "fidelity" => fidelity(rest),
        "perf" => perf(rest),
        "compare" => compare(rest),
        "check" => check(),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`; try `cargo xtask help`")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "\
cargo xtask <command>

  check      fmt --check, clippy -D warnings, the test suite, and the
             layout of every corpus document against LAYOUT.md
  dist       release build of both apps
  package    dist, then a versioned zip in target/dist/
  install    dist, then copy binaries to ~/.local/bin
  associate  make the desktop open .docx and .xlsx with these
  fidelity   run the round-trip fidelity harness over corpus/
             (--report also writes FIDELITY.md)
  perf       time reading and laying out every file in corpus/
  compare    where a document differs from the rendering the application
             that owns its format makes of it, ranked — Word for .docx and
             .doc, LibreOffice for .odt (--check runs inside `check` and
             needs neither; only --refresh does, and only for a document
             that changed)
  help       this message"
    );
}

fn check() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    // Where the document lands on the page, against Word's own rendering of the
    // same file — held to `LAYOUT.md`. It belongs here rather than beside it
    // because a layout regression is not a thing anybody notices: the tests all
    // pass, the document opens, and a line is a point and a half further down
    // the page than it was. Word is *not* needed for this — its readings of the
    // corpus are committed — so this still runs on a machine that has never had
    // Office on it. Eight seconds, in debug, on the build the tests just made.
    cargo(&["run", "-q", "-p", "wp-compare", "--", "--check"])?;
    Ok(())
}

fn build_dist() -> Result<(), String> {
    cargo(&["build", "--release", "-p", "calx", "-p", "scriva"])
}

fn package() -> Result<(), String> {
    build_dist()?;
    let archive = dist::package()?;
    println!("packaged {}", archive.display());
    Ok(())
}

/// Desktop integration, asked for rather than done as a side effect.
///
/// Installing a binary should not rearrange a user's desktop. This is the
/// separate step, and on Windows it prints the commands rather than running
/// them: file associations there are registry keys, and a build tool has no
/// business writing to a user's registry.
fn associate() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        return dist::desktop_entries();
    }
    let bin = home()?.join(".local").join("bin");
    println!("On Windows, associate the file types yourself — this does not edit");
    println!("your registry. In an elevated PowerShell, for each extension:");
    println!();
    for (ext, app) in [
        (".xlsx", "calx"),
        (".xls", "calx"),
        (".csv", "calx"),
        (".docx", "scriva"),
        (".doc", "scriva"),
        (".odt", "scriva"),
        (".md", "scriva"),
    ] {
        println!(
            "  cmd /c assoc {ext}=CalxScriva{app} ; cmd /c ftype CalxScriva{app}={:?} %1",
            bin.join(format!("{app}.exe")).display()
        );
    }
    println!();
    println!("Or right-click a file, Open with, Choose another app, and browse to");
    println!("{}.", bin.display());
    Ok(())
}

fn install() -> Result<(), String> {
    build_dist()?;

    let bin_dir = home()?.join(".local").join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("could not create {}: {e}", bin_dir.display()))?;

    for app in APPS {
        let exe = format!("{app}{}", std::env::consts::EXE_SUFFIX);
        let src = workspace_root().join("target").join("release").join(&exe);
        let dst = bin_dir.join(&exe);
        std::fs::copy(&src, &dst)
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
        println!("installed {}", dst.display());
    }

    if !path_contains(&bin_dir) {
        eprintln!();
        eprintln!("note: {} is not on your PATH.", bin_dir.display());
        if cfg!(windows) {
            eprintln!("      Add it with:");
            eprintln!("        setx PATH \"%PATH%;{}\"", bin_dir.display());
        } else {
            eprintln!("      Add this to your shell profile:");
            eprintln!("        export PATH=\"$HOME/.local/bin:$PATH\"");
        }
    }
    Ok(())
}

fn perf(args: &[String]) -> Result<(), String> {
    let corpus = match args.first() {
        Some(p) => PathBuf::from(p),
        None => workspace_root().join("corpus"),
    };
    perf::run(&corpus)
}

fn fidelity(args: &[String]) -> Result<(), String> {
    let corpus = match args.first().filter(|a| !a.starts_with("--")) {
        Some(p) => PathBuf::from(p),
        None => workspace_root().join("corpus"),
    };

    println!("fidelity: no-op round trip over {}", corpus.display());
    let report = fidelity::run(&corpus)?;
    if args.iter().any(|a| a == "--report") {
        let path = workspace_root().join("FIDELITY.md");
        std::fs::write(&path, fidelity::markdown(&report, &corpus))
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if fidelity::print(&report) {
        Ok(())
    } else if report.total() == 0 {
        Err(format!(
            "corpus at {} is empty — add real .docx/.xlsx files produced by Word and Excel",
            corpus.display()
        ))
    } else {
        Err("fidelity check failed: the rewrite is not faithful to the original".into())
    }
}

/// Where a document differs from Word's own rendering of it.
///
/// Shelled out rather than linked: the comparator measures a page with the
/// application's own shaper, so it depends on the application, on egui and on
/// wgpu — and this crate stays the one thing that still runs when the rest of
/// the workspace does not build.
fn compare(args: &[String]) -> Result<(), String> {
    let mut argv = vec!["run", "--release", "-q", "-p", "wp-compare", "--"];
    argv.extend(args.iter().map(String::as_str));
    cargo(&argv)
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(&cargo)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to run `cargo {}`: {e}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed with {status}", args.join(" ")))
    }
}

pub fn workspace_root() -> PathBuf {
    // xtask lives at <root>/xtask, so CARGO_MANIFEST_DIR's parent is the root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest dir always has a parent")
        .to_path_buf()
}

pub fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "could not determine home directory (HOME/USERPROFILE unset)".into())
}

fn path_contains(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|e| e == dir))
        .unwrap_or(false)
}
