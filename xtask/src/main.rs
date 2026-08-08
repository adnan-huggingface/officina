//! Build, package, install, and verification tasks.
//!
//! Run via the cargo alias: `cargo xtask <command>`.
//!
//! Deliberately dependency-free — this is the thing that has to keep working when
//! the rest of the workspace does not build.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const APPS: [&str; 2] = ["calx", "scriva"];

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
        "dist" => dist(),
        "fidelity" => fidelity(rest),
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

  check      fmt --check, clippy -D warnings, and the test suite
  dist       release build of both apps
  install    dist, then copy binaries to ~/.local/bin
  fidelity   run the round-trip fidelity harness over corpus/
  help       this message"
    );
}

fn check() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "--workspace"])?;
    Ok(())
}

fn dist() -> Result<(), String> {
    cargo(&["build", "--release", "-p", "calx", "-p", "scriva"])
}

fn install() -> Result<(), String> {
    dist()?;

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
            eprintln!(
                "        setx PATH \"%PATH%;{}\"",
                bin_dir.display()
            );
        } else {
            eprintln!("      Add this to your shell profile:");
            eprintln!("        export PATH=\"$HOME/.local/bin:$PATH\"");
        }
    }
    Ok(())
}

fn fidelity(_args: &[String]) -> Result<(), String> {
    // Lands with chunk C2. Until then this is honestly unimplemented rather than
    // a green check that means nothing.
    Err("fidelity harness not implemented yet (chunk C2 — see PROGRESS.md)".into())
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

fn workspace_root() -> PathBuf {
    // xtask lives at <root>/xtask, so CARGO_MANIFEST_DIR's parent is the root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest dir always has a parent")
        .to_path_buf()
}

fn home() -> Result<PathBuf, String> {
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
