# Goal

Create a clone of MS Word and MS Excel.
These should be excellent clones for professional use.

# Requirements

1. MSWord clone must be able to edit .doc, .docx, and any regular text file (ascii/utf8).  If not too difficult then I'd also like to edit .pdf.
2. MSExcel clone must be able to edit .xls, xslx, .csv.
3. Must be able to compile/run easily on Windows 11 or modern Ubuntu Linux.
4. Prefer single exe if possible.
5. Startup should be relatively fast but feature parity is more important.
6. MSWord clone must be able to open and edit any .doc/.docx created by true MSWord.  Likewise for the MSExcel clone.  These should be professional clones.
7. Prefer manual installation into users ~/.local/bin and configuration+state in ~/.config\[app_name_goes_here]/.
8. Think of a short name for these that doesn't conflict.

# Research

Do some initial research online to see if there are any existing open source projects that fulfill the requirements already or perhaps very closely.
I may just opt for that instead of perhaps that could be used as a starting point.

# Questions

Before starting to code, ask any clarifying questions and document them here, and also give me a short design debriefing.
Break down the work into chunks that can be easily resumed in case I run out of my 5-hour Claude limit.
Try to perform work in parallel.

# Tools/compilers

Feel free to download/install any tools or compilers that are commonly used and from known safe sites.

---

# Answers (2026-08-08)

## Research summary

No existing open-source project satisfies all 8 requirements. Surveyed:

| Project | Verdict |
|---|---|
| LibreOffice Writer/Calc | Best fidelity in existence (~10M LOC, 25 yrs). ~400MB install, slow start, not a single exe. |
| ONLYOFFICE Desktop | Closest DOCX rendering to real Word. Huge C++/JS codebase, impractical to fork. |
| [Univer](https://github.com/dream-num/univer) (Apache-2.0, TS) | Real Sheets+Docs engine, 500+ functions. Web-first; Docs weaker; no legacy formats. |
| Gnumeric / AbiWord | Native + light, but effectively maintenance-mode and GTK-bound. |
| [calamine](https://github.com/tafia/calamine), umya-spreadsheet, rust_xlsxwriter | Good Rust xlsx I/O building blocks. Libraries, not apps. calamine is read-only. |
| [litchi](https://github.com/DevExzh/litchi) (Rust, .doc/.xls/.ppt) | Exactly the legacy gap-filler, but v0.0.1 / 37 stars / author says not production ready. |
| [rust-cfb](https://github.com/mdsteele/rust-cfb), antiword, [wvWare](https://wvware.sourceforge.net/) | Legacy plumbing only; C/GPL or very low-level. |

**Conclusion:** build it, in Rust, borrowing the good libraries where they fit.

## Reality check on requirement 6

"Must open and edit *any* .doc/.docx created by true MSWord" at feature parity is not
reachable from scratch — that is the goal LibreOffice has pursued for 25 years and has
not fully closed. What *is* reachable, and what this project commits to instead:

> **Round-trip-safe editing of the ~95% of real-world documents**, with every OOXML part
> we do not understand preserved verbatim, so saving never destroys data we failed to parse.

That is the definition of done. Fidelity is measured, not asserted — see the round-trip
harness in the plan.

## Clarifying questions asked, and the answers given

**Q1. Which architecture?**
→ **Pure Rust native.** Two static binaries, egui + wgpu, custom layout engine.
True single exe, ~100ms startup, zero runtime deps on Windows 11 and modern Ubuntu.
Rejected: Tauri/webview (drags in WebView2 + webkit2gtk), LibreOfficeKit shell (400MB
dependency, and it would be a skin rather than a clone).

**Q2. Legacy binary formats (.doc/.xls)?**
→ **Read-only, phase 2.** Phase 1 ships .docx/.xlsx/.csv/plain text read+write.
Legacy files open and edit, then save as the modern format. We never write .doc/.xls
(~1700 pages of combined spec for a format Microsoft deprecated in 2007).

**Q3. PDF?**
→ **Dropped from the initial roadmap.** Revisit once both clones are solid.

**Q4. What do we optimize for when trading off?**
→ **Import fidelity** first (never corrupt a user's file), and **Excel before Word**
(the spreadsheet is the more tractable of the two; its core proves out the shared
infrastructure the word processor then reuses).

## Names

- **Calx** — the spreadsheet app. `calx`, config in `~/.config/calx/`
- **Scriva** — the word processor. `scriva`, config in `~/.config/scriva/`

(Initially proposed "Tabula" for the spreadsheet and withdrew it: tabula-java/tabula-py
are well known in the document-processing space specifically, which is the worst place
to collide.)

## Design debriefing and work breakdown

See [DESIGN.md](DESIGN.md) for architecture and [PROGRESS.md](PROGRESS.md) for the
resumable chunk list and current state.
