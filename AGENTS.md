# AGENTS.md

This file provides guidance to coding agents working in this repository.

Officina is a native office suite in Rust: **Calx** (spreadsheets: xlsx/xls/csv) and
**Scriva** (documents: docx/doc/markdown), built on egui + wgpu. Windows-first;
Linux code paths exist but are unverified.

## Commands

```bash
cargo xtask check      # fmt + clippy (warnings denied) + entire test suite — the gate for any change
cargo xtask fidelity   # round-trip harness over corpus/ (untouched save, then save-after-edit)
cargo xtask perf       # stopwatch over the corpus and larger files
cargo xtask compare <file>  # where a page differs from Word's own, ranked (needs Word)
cargo xtask compare --check # every corpus document, against LAYOUT.md — fails if any got worse
cargo xtask package    # release zip; regenerates THIRD-PARTY-NOTICES.yml (needs cargo-bundle-licenses)
cargo test -p wp-docx                          # one crate
cargo test -p scriva --test charts_on_paper    # one integration-test file (app crates are packages `calx`/`scriva`)
cargo test -p ss-formula name_of_test          # one test
```

Some tests drive real Word/Excel through COM and skip themselves when Office is absent.

After meaningful UI work, recreate a real document through the running app —
menus and keystrokes, New through Save As. adr/0002 records why: one afternoon
of it found a crash and two silent data losses a green suite never touched,
and the driver rules that keep the exercise safe.

Layout fidelity is judged by measurement, never by eye: `cargo xtask compare`
lays the document with the application's own shaper, exports the same file from
Word, and reports every word whose pen went down somewhere else — worst first.
adr/0003 records why it is not a person: the differences that matter are
fractions of a point, well under what a screenshot resolves, and a person can
report only one of them per look.

`LAYOUT.md` records what every corpus document measures, and
`cargo xtask compare --check` fails on any that got worse. Run it after any
change to shaping, line breaking or pagination; `--record` when the new numbers
are the ones to keep. It is not part of `cargo xtask check`, which must keep
working on a machine with no Word.

## The invariant everything serves

**Saving never rewrites what wasn't edited.** On open, every OPC package part is
classified (DESIGN.md §3): *modeled* (parsed, re-serialized), *retained* (unknown —
raw bytes written back identically), or *derived* (regenerated each save). Unknown
elements inside modeled parts are kept as opaque nodes and re-emitted in order.
"Unsupported" must mean "survives untouched", never "silently dropped". Any change to
readers or writers must leave `cargo xtask fidelity` at zero failures.

Excel's and Word's observed behavior is the spec, including their bugs (1900
leap-year, coercion order). When a decision needs an oracle, measure the real
application — see adr/0001 and tools/word-probe/.

## Architecture

Data flows through crates in layers; UI never touches file formats directly:

- `ooxml` (OPC packages + the preservation vault) and `cfb` (read-only legacy container)
  are the foundation.
- Spreadsheet stack: `ss-model` (sparse cells, styles) ← `ss-formula` (parser,
  incremental dependency-graph recalc) ← `ss-xlsx` / `ss-xls` / `ss-csv` ← `app-calx`.
- Document stack: `wp-model` (paragraph/run tree, lazy style inheritance) ←
  `wp-layout` (shaping, pagination, floats) ← `wp-docx` / `wp-doc` / `wp-text` ←
  `wp-print` (PDF) ← `app-scriva`.
- `chart` renders DrawingML charts for both apps; `ui-kit` holds shared egui widgets
  (menu bar + toolbar — there is no ribbon), fonts, and theming.

Fonts come from the user's system at runtime; icons are drawn in code. Nothing is
bundled — keep it that way (licensing).

## Provenance rules (licensing)

- Never copy or port code from other office implementations (LibreOffice, POI, …);
  cite specs by section (ECMA-376, [MS-DOC]) but never paste spec text.
- corpus/ files are self-made: generate.ps1 drives the user's own Word/Excel;
  strangers.py hand-writes second-producer OOXML. Never commit downloaded documents
  or anything with unknown redistribution terms. manual_examples/ is gitignored for
  this reason.
- The accent green #1E6F5C is deliberately not Microsoft's brand color; UI strings
  use Microsoft marks only nominatively ("Excel workbook (*.xlsx)").

## Writing style

Comments and docs state constraints and reasons, not mechanics, in full prose — match
the surrounding voice. Commit messages are a single short sentence that tells the
story of the change (see `git log --oneline`). LEARNINGS.md records what a format
taught; PROGRESS.md is the work log.
