//! Build, package, install, and verification tasks.
//!
//! Run via the cargo alias: `cargo xtask <command>`.
//!
//! Deliberately dependency-free — this is the thing that has to keep working when
//! the rest of the workspace does not build.

mod dist;
mod eval;
mod fidelity;
mod map;
mod perf;
mod spike;

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
        "author" => author(),
        "measure" => measure(rest),
        "map" => map::write().map(|path| println!("{}", path.display())),
        "assist-spike" => spike::run(rest),
        "assist-eval" => eval::run(rest),
        "check" => check(rest),
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

  check      fmt, clippy -D warnings, the test suite, and the layout of
             every corpus document against LAYOUT.md (--quick: clippy and
             the tests of the crates the working tree has changed only)
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
  author     write corpus/docx/scriva-authored.docx from corpus/scriva-authored.txt
             through Scriva's own commands, and renew Word's reading of it if
             the document changed
  measure    <script> [compare options]: author a .docx from a script of
             Scriva's commands (the corpus script's language) into
             target/measure/, have Word render it, and print where Scriva
             and Word disagree — one command for \"what does Word do here\"
  map        rewrite MAP.md: crates, how to run their tests, every file with
             its module doc's first sentence, and every comment that states
             what an application was measured to do (check does it too)
  help       this message"
    );
}

fn check(args: &[String]) -> Result<(), String> {
    let quick = args.iter().any(|a| a == "--quick");
    if let Some(other) = args.iter().find(|a| *a != "--quick") {
        return Err(format!("unknown option `{other}` for check; only --quick"));
    }
    // Formatted rather than checked. A gate that fails on formatting is a
    // gate run twice for every change, the second time to learn nothing —
    // rustfmt has one answer and this applies it, and what it changed is in
    // the diff for the commit to carry.
    cargo(&["fmt", "--all"])?;
    // The quick tier is for the middle of the work, when the question is
    // whether the last edit broke the crate it touched; the whole workspace
    // is for the commit, because a change in `wp-model` breaks `scriva`
    // without touching a line of it, and only the whole run sees that.
    let packages = match quick {
        true => changed_packages()?,
        false => Vec::new(),
    };
    let mut clippy = vec!["clippy"];
    let mut test = vec!["test"];
    match packages.as_slice() {
        [] => {
            clippy.push("--workspace");
            test.push("--workspace");
        }
        changed => {
            println!("check --quick: {}", changed.join(", "));
            for package in changed {
                clippy.extend(["-p", package]);
                test.extend(["-p", package]);
            }
        }
    }
    clippy.extend(["--all-targets", "--", "-D", "warnings"]);
    cargo(&clippy)?;
    cargo(&test)?;
    // The map is rewritten here for the reason the formatting is: a map one
    // step behind the code is worse than none, and renewing it is not a
    // thing anybody should have to remember. After the tests, whose build
    // its menu walk reuses.
    map::write()?;
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

/// The packages whose files the working tree has changed since the last
/// commit — or, with nothing changed, the ones the last commit touched.
///
/// A change outside any crate — the workspace manifest, the corpus, a
/// script — is a change to everything, and answers with an empty list, which
/// the caller reads as the whole workspace. Read from `git` and the crates'
/// own manifests rather than from cargo metadata, since this crate stays free
/// of dependencies and a package's name is one line of its `Cargo.toml`.
fn changed_packages() -> Result<Vec<String>, String> {
    let root = workspace_root();
    let git = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .map_err(|e| format!("failed to run git: {e}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let mut files: Vec<String> = git(&["status", "--porcelain", "--untracked-files=all"])?
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| path.rsplit(" -> ").next().unwrap_or(path).to_owned())
        .collect();
    if files.is_empty() {
        files = git(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"])?
            .lines()
            .map(str::to_owned)
            .collect();
    }
    let mut packages = Vec::new();
    for file in files {
        let Some(rest) = file.strip_prefix("crates/") else {
            return Ok(Vec::new());
        };
        let Some((dir, _)) = rest.split_once('/') else {
            return Ok(Vec::new());
        };
        let manifest = root.join("crates").join(dir).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let name = text
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("name = ")?
                    .trim()
                    .strip_prefix('"')?
                    .strip_suffix('"')
            })
            .ok_or_else(|| format!("{}: no package name", manifest.display()))?
            .to_owned();
        if !packages.contains(&name) {
            packages.push(name);
        }
    }
    Ok(packages)
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

/// The corpus document the application writes itself, written again.
///
/// Every other corpus document was written by Word and measures how Scriva
/// reads. This one is authored by Scriva from a script of its own commands,
/// and Word's reading of it is committed beside it like the others', so what
/// Scriva *writes* is held to Word by the same check. The test in
/// `scriva::author` fails when the script no longer authors the committed
/// bytes, and this is the way through: the document is rewritten, and if it
/// changed, its reading is renewed — which needs Word, or the service that
/// stands in for it — and both are left for a commit.
fn author() -> Result<(), String> {
    let root = workspace_root();
    let script = root.join("corpus").join("scriva-authored.txt");
    let out = root
        .join("corpus")
        .join("docx")
        .join("scriva-authored.docx");
    let before = std::fs::read(&out).ok();
    authored(&script, &out)?;
    let after = std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let name = out
        .strip_prefix(&root)
        .unwrap_or(&out)
        .display()
        .to_string();
    if before.as_deref() == Some(after.as_slice()) {
        println!("{name} is unchanged; its reading still answers for it");
        return Ok(());
    }
    println!("wrote {name}; renewing Word's reading of it");
    compare(&[name.clone(), "--refresh".to_owned()])?;
    println!(
        "Now `cargo xtask compare --record` if LAYOUT.md should hold what it measures, \
         and commit {name}, corpus/rendered/scriva-authored.docx.tsv and LAYOUT.md together."
    );
    Ok(())
}

/// Runs `scriva --author`: the script at `script`, through the application's
/// own commands, saved as `out`.
fn authored(script: &Path, out: &Path) -> Result<(), String> {
    let root = workspace_root();
    // A configuration directory of its own: a save remembers its path in the
    // user's recent list, and an authored document is not something the user
    // opened.
    let config = root.join("target").join("author");
    std::fs::create_dir_all(&config).map_err(|e| format!("{}: {e}", config.display()))?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(&cargo)
        .args(["run", "-q", "-p", "scriva", "--", "--author"])
        .arg(script)
        .arg(out)
        .env("XDG_CONFIG_HOME", &config)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("failed to run scriva --author: {e}"))?;
    if !status.success() {
        return Err(format!("`scriva --author` failed with {status}"));
    }
    Ok(())
}

/// "What does Word do here", as one command.
///
/// Every measurement used to be a probe written by hand: a document built in
/// PowerShell or Python on the laptop, rendered, read, compared by eye. This
/// takes the same small script `cargo xtask author` reads — type, key, table,
/// style, page-break — authors the document through Scriva's own commands,
/// has Word render it (through `OFFICINA_WORD_SERVICE` on a machine without
/// Word), and prints the comparison. The document and its reading stay under
/// `target/measure/` and `target/compare/`: a measurement is a question, and
/// only a question that should be asked again belongs in the corpus.
fn measure(args: &[String]) -> Result<(), String> {
    let (script, rest) = args
        .split_first()
        .ok_or("measure wants a script: cargo xtask measure <script> [compare options]")?;
    let script = PathBuf::from(script);
    let stem = script
        .file_stem()
        .ok_or_else(|| format!("{} names no file", script.display()))?
        .to_string_lossy()
        .into_owned();
    let out = workspace_root()
        .join("target")
        .join("measure")
        .join(format!("{stem}.docx"));
    authored(
        &std::path::absolute(&script).unwrap_or(script.clone()),
        &out,
    )?;
    println!("authored {}", out.display());
    let mut argv = vec![out.display().to_string(), "--refresh".to_owned()];
    argv.extend(rest.iter().cloned());
    compare(&argv)
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
