//! Scriva — word processor.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!`
// lands somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() -> ui_kit::eframe::Result<()> {
    // `--author <script> <out.docx>` writes a document from a script of the
    // application's own commands and exits without a window — the corpus's
    // way of putting what Scriva writes under the gate; see `scriva::author`.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|first| first == "--author") {
        let (Some(script), Some(out)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: scriva --author <script> <out.docx>");
            std::process::exit(2);
        };
        if let Err(why) = scriva::author::write(script.as_ref(), out.as_ref()) {
            eprintln!("error: {why}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // A path on the command line opens that document, which is what a double
    // click in the file manager becomes.
    let app = match args.into_iter().next() {
        Some(path) => scriva::app::Scriva::opening(std::path::PathBuf::from(path)),
        None => scriva::app::Scriva::new(),
    };
    ui_kit::run(app)
}
