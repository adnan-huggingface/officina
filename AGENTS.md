# AGENTS.md

This file provides guidance to coding agents working in this repository.

Officina is a native office suite in Rust: **Calx** (spreadsheets: xlsx/xls/csv) and
**Scriva** (documents: docx/doc/odt/markdown), built on egui + wgpu. Built on Windows
first; the gate also passes on Ubuntu 24.04.

## Commands

**Start at `MAP.md`.** It is generated from the tree on every `check`: which
target holds a crate's tests, what each file is for, the rules Word, Excel or
LibreOffice was measured to keep and where they are written, and the keys that
reach every menu row. Look there before grepping.

```bash
cargo xtask check      # fmt + clippy (warnings denied) + tests + layout — the gate for any change
cargo xtask check --quick   # the same, clippy and tests only for the crates the working tree changed
cargo xtask author     # rewrite corpus/docx/scriva-authored.docx from its script, renew Word's reading
cargo xtask measure <script>  # author a .docx from a script of Scriva's commands, have Word render it,
                              # print the comparison — "what does Word do here" in one call
cargo xtask map       # rewrite MAP.md (check does it too): crates and their test targets, every
                      # file's purpose, every measured fact with its place, every menu's keys
cargo xtask fidelity   # round-trip harness over corpus/ (untouched save, then save-after-edit)
cargo xtask perf       # stopwatch over the corpus and larger files
cargo xtask compare <file>  # where a page differs from the owning application's rendering,
                            # ranked (needs Word, or LibreOffice for .odt)
cargo xtask compare --check # every corpus document, against LAYOUT.md — fails if any got worse
cargo xtask package    # release zip; regenerates THIRD-PARTY-NOTICES.yml (needs cargo-bundle-licenses)
cargo test -p wp-docx                          # one crate
cargo test -p scriva --test charts_on_paper    # one integration-test file (app crates are packages `calx`/`scriva`)
cargo test -p ss-formula name_of_test          # one test
```

Some tests drive real Word/Excel through COM and skip themselves when Office is absent.

**Driving the applications in a test.** `ui_kit::drive::Driver` runs the frame
the window runs — dialogs, menus, document — with keys, menu letters and text,
and the application's state is read back from the application. Both apps'
test constructors enter `ui_kit::headless`: no file chooser reaches the
desktop, no recent list is written, and no assistant is asked — every
helper but `assist::Scripted` refuses before it connects, and nothing on
the computer (a key, an `ant` login, an Ollama) is looked for. A keyboard sequence is tested at more
than one pace (`settle()` between keys), because a real window paints frames
between keys that the driver does not. `Driver::every_menu` walks every menu
by keyboard; `ui_kit::menu::clashes` reports two rows of one menu sharing a
letter.

What the window *shows* is asked of `Driver::paint`, which returns a
`Painted`: rectangles by fill, texts with the colour each letter was painted
in, rules. Never walk the shapes by hand. The pointer is driven with
`press_at`, `move_to`, `drag`, `double_click`, `right_click` and `hold`, at
points from `Scriva::on_screen(caret)`, not from a page corner and a zoom
worked out by hand. An application runs in the shell's frame. A widget with
tests of its own, such as Calx's grid, implements `ui_kit::drive::Driven` and
runs bare in the driver's window, so its tests keep the widget's own
coordinates and still get the driver's fonts, theme, clock and gestures.
A few lines of chrome with no widget type of their own (a message box, a
menu row, a chart) run bare as `ui_kit::drive::Bare(|ui: &mut egui::Ui| …)`. A
test that only lays type takes its context from `Driver::new()` after
`warm()`, never from a bare `egui::Context`: only the driver makes the process
headless. `Driver::opening()` opens small and grows, as the real window does.
The driver's clock moves a frame's time each frame; `wait(seconds)` sees an
animation out. A headless process reads no font folder, so tests lay type the
same on every machine. `Driver::in_hack()` sets the sans face in Hack, the
second face egui carries, for a check that must not depend on egui's default
metrics. No other face may be used in a test: nothing is bundled, and a test
that uses a machine's copy when there is one passes without checking
anything where there is none.

After meaningful UI work, recreate a real document through the running app —
menus and keystrokes, New through Save As. adr/0002 records why: one afternoon
of it found a crash and two silent data losses a green suite never touched,
and the driver rules that keep the exercise safe.

Layout fidelity is judged by measurement, never by eye: `cargo xtask compare`
lays the document with the application's own shaper, exports the same file from
the application that owns its format, and reports every mark whose pen went down
somewhere else — worst first.
Words and the page's furniture both: a rule, a shading, a border and a picture's
box are compared the same way and counted in their own column, because until
they were, a border could move an inch and no number moved with it. adr/0003
records why it is not a person: the differences that matter are fractions of a
point, well under what a screenshot resolves, and a person can report only one
of them per look.

`LAYOUT.md` records what every corpus document measures, and
`cargo xtask compare --check` fails on any that got worse. **It runs inside
`cargo xtask check`**, so a layout regression fails the gate like any other —
which it has to, because nothing about a layout regression is noticeable: the
tests pass, the document opens, and a line sits a point and a half further down
the page. When the new numbers are the ones to keep, `--record` and commit the
change to `LAYOUT.md` deliberately.

That check needs **neither application**: their readings of the corpus are
committed under `corpus/rendered/`, because an application's answer for a
document cannot change until the document does. One is needed only to renew the
reading of a document that actually changed — `cargo xtask compare --refresh` —
and the file it renews says plainly when it is out of date, and which
application it wants. `crates/wp-compare/tests/without_office.rs` runs the whole
check with nothing on its PATH, so that this is measured rather than merely
designed.
A machine without either can still renew a reading through one that has
them: `tools/probe/service.ps1` on that machine, and `OFFICINA_WORD_SERVICE`
(with `OFFICINA_WORD_TOKEN`) here — see `tools/probe/README.md`.

Which application answers for a document is decided by the document: Word for
`.docx` and `.doc`, LibreOffice for `.odt`. Word reads ODF through a converter
it wrote for a format it does not own, and is not the standard for it.

## When the work is finished

**"Done" is a command here, not a paragraph.** `python .claude/hooks/gate.py`
exits zero when every gate passes and every item of `PLAN.md` — if there is
one — proves itself, and non-zero with the reason when it does not. Prose loses
to fatigue; an exit code does not.

`PLAN.md` is **not a checklist and is immutable while the work runs**. Each item
carries its own `verify:` command and is done exactly when that exits zero; there
is nothing to tick, so nothing doing the work can finish it by editing a file.
`--items` lists them. Two traps that were walked into rather than reasoned about,
both worth remembering when writing a `verify:` line: **a `cargo test` filter
that matches no test exits zero**, so `.claude/hooks/proved.py` insists a test
actually ran; and a harness can be green because it is not looking, which is why
the item about fidelity covering `.odt` checks that the skip is gone as well.

`.claude/hooks/stop_gate.py` is a `Stop` hook that runs it and refuses to let a
session end while it says no, handing the reason back. **It is off by default,
and should be**: a hook that will not let a session end changes every session in
the project, including the ones that were only ever a question. Switch it on in
`.claude/settings.json`:

```json
{ "hooks": { "Stop": [ { "matcher": "*", "hooks": [
  { "type": "command", "command": "python .claude/hooks/stop_gate.py", "timeout": 3600 }
] } ] } }
```

Two refusals it deliberately does *not* make, each of which would be worse than
the problem it solves. It stands down when `stop_hook_active` says it has
already refused once, because a gate that can never pass must reach a person
rather than spend the budget in a circle. And it stands down when the gate
cannot be *asked* at all — a missing toolchain is not unfinished work, and the
first version of it reported one as the other and would have held every session
in this project open.

## The invariant everything serves

**Saving never rewrites what wasn't edited.** On open, every OPC package part is
classified (DESIGN.md §3): *modeled* (parsed, re-serialized), *retained* (unknown —
raw bytes written back identically), or *derived* (regenerated each save). Unknown
elements inside modeled parts are kept as opaque nodes and re-emitted in order.
"Unsupported" must mean "survives untouched", never "silently dropped". Any change to
readers or writers must leave `cargo xtask fidelity` at zero failures.

Excel's and Word's observed behavior is the spec, including their bugs (1900
leap-year, coercion order). When a decision needs an oracle, measure the real
application — see adr/0001 and tools/probe/.

## Architecture

Data flows through crates in layers; UI never touches file formats directly:

- `ooxml` (OPC packages + the preservation vault) and `cfb` (read-only legacy container)
  are the foundation.
- Spreadsheet stack: `ss-model` (sparse cells, styles) ← `ss-formula` (parser,
  incremental dependency-graph recalc) ← `ss-xlsx` / `ss-xls` / `ss-csv` ← `app-calx`.
- Document stack: `wp-model` (paragraph/run tree, lazy style inheritance) ←
  `wp-layout` (shaping, pagination, floats) ← `wp-docx` / `wp-doc` / `wp-odf` /
  `wp-text` ← `wp-print` (PDF) ← `app-scriva`. `wp-odf` has a container of its
  own rather than `ooxml`'s: an ODF package puts `mimetype` first and uncompressed
  and lists its parts in a manifest, where OPC has content types and relationships.
- `chart` renders DrawingML charts for both apps; `ui-kit` holds shared egui widgets
  (menu bar + toolbar — there is no ribbon), fonts, and theming, and the Assist pane
  both apps host (`ui_kit::assist`): the transcript, the composer, the first-run card
  and the settings box. A request runs on a thread of its own; the helper's tool
  calls come back to the application one a frame, and it runs them on its own
  document.
- `assist` is the assistant's other half, below `ui-kit` and knowing neither egui
  nor documents: the helpers that answer (Claude over the Messages API, Ollama or
  any chat-completions service, a script for tests), the conversation, the tool
  loop, and the settings both apps share. A helper proposes edits as tool calls;
  the application runs them through its own editing functions.

Fonts come from the user's system at runtime; icons are drawn in code. Nothing is
bundled — keep it that way (licensing).

## Provenance rules (licensing)

- Never copy or port code from other office implementations (LibreOffice, POI, …);
  cite specs by section (ECMA-376, [MS-DOC], OpenDocument v1.4) but never paste spec
  text. LibreOffice is used as a *measuring instrument* for `.odt` — asked to render,
  never read — which is the standing Word has and no more.
- corpus/ files are self-made: generate.ps1 drives the user's own Word/Excel;
  strangers.py hand-writes second-producer OOXML and odf.py does the same for ODF;
  scrub-odt.py makes a structural rubbing of a real `.odt` that keeps its shape and
  none of its words. Never commit downloaded documents
  or anything with unknown redistribution terms. manual_examples/ is gitignored for
  this reason.
- The accent green #1E6F5C is deliberately not Microsoft's brand color; UI strings
  use Microsoft marks only nominatively ("Excel workbook (*.xlsx)").

## Writing style

Comments and docs state constraints and reasons, not mechanics, in full prose — match
the surrounding voice. Commit messages are a single short sentence that tells the
story of the change (see `git log --oneline`). LEARNINGS.md records what a format
taught; PROGRESS.md is the work log.
