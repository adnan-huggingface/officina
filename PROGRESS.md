# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** C15 — the legacy .xls reader (C0–C14 all done)
**Status:** not started
**Handoff note:**

- Rust 1.97.1 `x86_64-pc-windows-gnu` installed at `~/.cargo/bin` (not on PATH —
  installed with `--no-modify-path`). Links against MSYS2 mingw-w64 GCC 12.1.0,
  so no Visual Studio is needed.
- `ooxml` is complete for C1 and C2: 38 tests, clippy clean.
- `ss-model` is complete for C3: 35 tests, clippy clean. Cell store is 16x16
  chunks in a BTreeMap; `Cell` is pinned at 24 bytes by a test. C4 added a
  per-sheet formula arena and `SheetKind` (chart/dialog sheets keep their slot in
  the sheet list, because `localSheetId` indexes it).
- `ss-xlsx` is complete for C4: 54 tests plus 2 ignored performance checks, and
  since C6 an integration test that recalculates the whole corpus and compares
  against the values Excel cached in it. Run the performance ones with
  `cargo test --release -p ss-xlsx -- --ignored --nocapture`; they are
  meaningless in a debug build.
- Watch item: quick-xml 0.41 does **not** fold entities into text. `&amp;` arrives
  as a separate `Event::GeneralRef`, so any text accumulator must handle it or it
  silently drops every `&`, `<`, and `>`. `xml::push_text` is the one place that
  does this; use it rather than matching `Event::Text` directly.
- `ss-formula` is complete for C5, C6, and C7: 153 tests, of which 57 are the
  conformance suite in `tests/conformance.rs`. Lexer, Pratt parser, dependency
  graph, evaluator, and **199 functions**.
- `ss-model` grew two things in C8 that other crates needed: `datetime` (moved
  out of ss-formula, because a cell value *is* a serial and the grid has to
  render one without the formula engine) and `numfmt`, the number-format
  engine. `format_general` and decimal rounding live there now too, so the
  engine and the grid cannot disagree about what a number looks like.
- `app-calx` is a library plus a thin binary. The split is not cosmetic: the
  grid's geometry, selection model, and frame planner are all testable with no
  window and no GPU, which is the only way the million-row criterion could be
  measured at all.
- `ss-formula` also owns editing (`edit`) and the clipboard (`clip`). Both sit
  above the model rather than in it because every editing operation has to keep
  formula text consistent, and that needs the lexer.
- `ss-xlsx::write` is the writer. It edits the parts it owns rather than
  reprinting them; `write::splice` is the primitive that makes that possible, and
  everything else in the module is built on it. `rfd` is Calx's file dialog, on
  default features so a Linux build needs the XDG portal rather than GTK headers.
- Workspace total is **563 tests**, all green;
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` are both clean.
- `ss-model` grew four modules across C11–C14: `color` (the four spellings of
  a colour), `cond` (conditional formatting and data validation), `chart`, and
  `pivot`. `ss-formula` grew `cond` (evaluating those rules) and four function
  families. `ss-csv` is now real.
- Watch item: whether text becomes a number depends on **how the value reached
  the function**, not on what it is. `SUM(TRUE)` is 1, `SUM({TRUE})` is 0.
  `functions::visit_args` tags each value `Direct` or `Inside` for exactly this;
  never flatten arguments before a function sees them.
- Watch item: the criteria language (`SUMIF`, `COUNTIF`, and the rest of the
  `*IF`/`*IFS` family) compares *within* a type. Under the `=` operator's rules
  text sorts above every number, so `SUMIF(A:A,">0")` would otherwise count a
  column's text header. All of them go through `criteria::visit_if`/`visit_ifs`.
- Watch item: **a position in a clipped range is not a position in the range the
  user wrote.** `MATCH(x,A:A,0)` must count from row 1 even though only the used
  range is worth reading. C6 hit this in `SUMIF` and C7 hit it again in the
  lookup family; `lookup::Grid` keeps the declared corner and the visitable
  window apart so it cannot happen a third time.
- Watch item: `Context::now()` is **UTC**, and Excel's `NOW` is local. A host
  that knows the user's time zone should override it. Near midnight `TODAY()`
  is otherwise off by a day.
- Watch item: **grid positions are `f64`, screen positions are `f32`.** A
  million rows of twenty pixels is a twenty-million-pixel axis, and above
  sixteen million an `f32` counts in twos — a cell would be painted at one
  place and clicked at another for the bottom nineteen-twentieths of the sheet.
  `Axis` is `f64` throughout and only the viewport-relative difference is cast.
- Watch item: `<cellStyleXfs>` and `<cellXfs>` are two different lists of `<xf>`
  elements in styles.xml, and a cell's `s` attribute indexes only the second.
  Reading both into one list shifts every style by the size of the first and
  formats the whole sheet plausibly wrongly.
- Watch item: **`$` means different things to the two translations.** Moving the
  grid moves anchored references with everything else; copying a formula leaves
  them behind. A single shared "adjust a reference" helper would be wrong for
  one caller or the other.
- Watch item: **every edit currently recalculates the whole workbook.**
  `DependencyGraph::dependents_of` is still a linear scan, so a large sheet will
  feel this. On C29's list, along with the graph.
- Watch item: **`edit::input` and `clip::paste` grow the formula arena even when
  the change is later undone.** The orphaned entries are unreachable and
  harmless, and the writer only emits what cells point at, so nothing reaches a
  file — but the arena still only grows.
- Watch item: **the writer re-reads three parts on every save** — the worksheet
  it is editing, `sharedStrings.xml`, and `styles.xml` — to compare against
  rather than trusting what a reader believed some time earlier. Correct, and it
  makes a save cost a parse. On C29's list with the recalc scan.
- Watch item: **`spans` is a sixteen-row block hint, not a row hint.** Widen it;
  never recompute it. See C10.
- Watch item: **theme index 0 is `lt1`, and `<a:clrScheme>` writes `dk1` first.**
  The first two pairs are swapped when the scheme is read. Read in order, every
  themed heading is painted white on white, which reads as "the text vanished"
  rather than as an off-by-one.
- Watch item: **a solid fill's colour is `fgColor`; a *differential* fill's is
  `bgColor`.** The two are opposite, and reading a `<dxf>` the ordinary way
  leaves every conditional format in the workbook uncoloured.
- Watch item: **`tint` is defined in HSL.** Scaling the RGB channels instead
  gives a colour of the wrong hue — obvious side by side, invisible alone.
- Watch item: **`showDropDown="1"` on a data validation *hides* the arrow.**
  The attribute is inverted in the file.
- Watch item: **a conditional format's formula is written against the region's
  top-left cell**, not against the cell being tested, so every relative
  reference is shifted before evaluation. `ss_formula::cond` does this.
- Watch item: **the style tables are index-addressed and append-only.**
  `StyleTable::restyle` is the only way to change formatting; nothing inserts
  or reorders. A font inserted at index 2 re-letters every cell past it.
- Watch item: **formatting a whole column is one attribute on `<col>`**, not a
  million styled cells. `edit::format` knows the difference,
  `Patch::AxisStyles` makes it undoable, and `write::sheet_out` writes it.
- Watch item: **a row's `s` needs `customFormat="1"` beside it.** Without that
  attribute Excel ignores the style, so a formatted row comes back unformatted.
- Watch item: **every test can pass while the file is still wrong.** The
  `<cols>` gap survived a green workspace and a green fidelity run, because the
  model round-tripped through itself perfectly and nothing asked the *file*.
  Check the bytes with an independent reader — openpyxl — before believing a
  writer.
- Watch item: **bold in the grid is faked by drawing the glyphs twice.** egui
  ships one weight of its default font and no way to ask for another. Italic is
  genuine (epaint slants it). If a real bold face is ever embedded, delete
  `paint_text`'s double draw rather than leaving both.
- Watch item: **`<xdr:cNvPr id="2">` comes before a chart's `r:id` in document
  order.** Taking the first `id` attribute points every chart at part two.
- Watch item: **`<c:pt idx="3">` may skip.** A chart's value cache read as a
  flat list puts every point after a gap against the wrong category.
- Watch item: **a `<c:title>` may hold formatting and no text at all.** Editing
  its runs finds none and silently does nothing, so `chart_out` replaces such a
  title wholesale. `<c:autoTitleDeleted>` has to be cleared at the same time.
- Watch item: **which functions spill is a list, not a rule.** `=A1:A5` is an
  array result too and must stay an implicit intersection in a legacy file.
  `functions::dynamic::spills` is the list; adding a function to the library
  does not add it there.
- Watch item: **`Sheet::spills` is derived at recalculation and never read from
  a file.** It exists so a shrinking result clears what it used to fill.
- Watch item: **a csv delimiter is chosen by consistency, not frequency**, and
  counted outside quotes. Prose separated by semicolons has far more commas
  than semicolons.
- Watch item: **`007` imports as 7**, which is Excel's behaviour and therefore
  the contract. `ss-csv/tests/typed_import.rs` says so by name so nobody
  "fixes" it.
- Watch item: **typing into a pivot table's region leaves the file
  self-contradictory** — Excel discards the edit at the next refresh. Calx
  refuses; `Sheet::pivot_at` is the check.
- Watch item: **adding or removing a sheet has no writer.** `flush` walks the
  `<sheet>` entries in the workbook part and rewrites the parts they name, so a
  sheet the model grew after the package was built has nowhere to be written.
  Nothing in the UI can add one yet; the moment something can, this is the code
  that has to know.
- Watch item: **a `t="s"` cell points at an `<si>`, and an `<si>` can be rich
  text.** New text is matched into the table by its characters, so typing a
  string that already exists in bold gives the new cell the bold entry. Excel
  dedups the same way, but it is a surprise worth remembering when C11 starts
  reading run properties.
- Watch item: `DependencyGraph::dependents_of` is a linear scan over every
  formula, so `evaluation_order` is O(formulas^2). Fine at corpus scale and for
  the 1,200-cell stress test; it will not survive a 100k-formula workbook. C5
  recorded this as a deliberate trade; `recalculate` is what makes it reachable
  by a user, so it is now on C29's list rather than a hypothetical.
- `cargo xtask fidelity` is green on both checks: **check 1, 27 of 27; check 2,
  12 of 12**. It still correctly *fails* on an empty corpus rather than
  reporting a vacuous pass.
- **Watch item — driving Office through COM in-process hangs.** Not a licensing
  problem, though it looks like one: `Documents.Add()` never returns, Word spins
  at 100% CPU, and no error is ever raised. It reproduced on two machines. The
  identical calls in a *child process* work fine, so `corpus/generate.ps1` runs
  every document via `Start-Job` under a timeout. Do not "simplify" that back to
  a single in-process Office instance.
- Watch item: Windows PowerShell 5.1 reads BOM-less `.ps1` as ANSI, so any
  script here containing non-ASCII **must** be saved UTF-8 *with* BOM or it
  fails to parse. `generate.ps1` has one.
- Watch item: in PowerShell `[char]0x41 + [char]0x42` is **131**, not `"AB"` —
  `+` on two chars is numeric. Use `-join`. This silently broke the RTL/CJK
  generator.
- Both apps launch and hold a window (verified, not assumed).
- `crt-static` is confirmed working: `objdump -p` on calx.exe lists only Windows
  system DLLs — no libgcc/libwinpthread/libstdc++. Requirement 4 is genuinely met
  on Windows. Still to verify on Ubuntu.
- Watch item: `eframe` 0.36 replaced `App::update(ctx)` with `App::ui(&mut Ui)`.
  Any eframe example found online will be for the older API.
- Watch item: `cargo build 2>&1 | tail` reports *tail's* exit code, not cargo's.
  Check `${PIPESTATUS[0]}` or the "Finished"/"error" line, not `$?`.

---

## Ground rules for every chunk

1. A chunk is done when it **builds, its tests pass, and the fidelity harness does not
   regress**. Not when the code is written.
2. Never write an OOXML part we did not either author or retain verbatim (DESIGN.md §3).
3. Update the "Current state" block above before ending a session.

---

## Phase 0 — Foundation

- [x] **C0. Toolchain + workspace scaffold**
  Rust stable gnu toolchain, cargo workspace, all crates from DESIGN.md §2 stubbed,
  `cargo xtask` runner, CI-equivalent local check script, empty egui+wgpu window for
  both apps. *Exit: `cargo run -p app-calx` opens a window on Windows.* — **met**

- [x] **C1. OPC container + Preservation Vault**
  Zip read/write, `[Content_Types].xml`, relationship graph, part classification
  (modeled/retained/derived), opaque-node capture inside modeled parts.
  *Exit: any .docx or .xlsx opens and re-saves byte-identically (normalized compare).*
  **This is the load-bearing chunk. Everything downstream trusts it.**

- [x] **C2. Fidelity harness + corpus**
  `cargo xtask fidelity`, the three checks from DESIGN.md §7, and a starter corpus of
  real Word/Excel documents covering the awkward cases.
  *Exit: harness runs green on C1's round-trip guarantee.* — **met: 27 passed, 0 failed.**

  Corpus is 27 documents generated by `corpus/generate.ps1` through real Word and
  Excel: tracked changes, comments, nested tables, floating images, footnotes,
  mixed section orientation, RTL/CJK, content controls, a `.dotx`, and on the
  Excel side pivot tables, charts, array formulas, conditional formatting, data
  validation, and defined names. `corpus/manifest.json` records provenance.

  Still worth adding when they turn up: real-world documents with the accumulated
  oddities of many Word versions, `.xlsm` with a VBA project, and a 100k+ row
  sheet. Generated files are clean by construction, and clean is where
  preservation bugs *don't* live.

## Phase 1 — Calx (spreadsheet) core

- [x] **C3. Spreadsheet model**
  Sparse chunked cell store, interned strings, style table, defined names, number
  formats. *Exit: property tests pin the store's invariants across randomized
  insert/overwrite/erase sequences, checked against a naive reference map.*
  (Revised from "serde round-trip": serde is not used in production here, so
  testing it would exercise a dependency rather than the store.)

- [x] **C4. xlsx reader**
  workbook.xml, sheets, sharedStrings, merged cells, defined names, frozen panes,
  column/row geometry, formulas (normal, array, shared master/follower, data table).
  *Exit: opens a real 50MB Excel file in <2s with correct values and formats.*
  — **met on the reachable reading of "50MB"; see below.**

  Parts are found by walking the relationship graph, never by assuming
  `/xl/workbook.xml`, so Strict-profile and third-party files open too. Reading
  leaves every part `Retained`: the model is a view over the package, not a
  replacement, so open-and-save still reproduces the original bytes.

  Styles are carried as opaque `StyleId` indices only — resolving them into fonts,
  fills, and number formats is C11, so "with correct formats" is *not* yet met.

  **On the 50MB target.** The criterion was ambiguous and the two readings are far
  apart. Measured (`cargo test --release -p ss-xlsx -- --ignored --nocapture`):
  - 55 MB of sheet XML, 1.3M cells → **1.75 s**. Met.
  - quick-xml's raw event scan of the same document, building nothing → 0.74 s.
    That is the floor; the reader runs at ~2.4x it.
  - A 50 MB *.xlsx on disk* is ~500 MB of XML after deflate. The scan alone would
    take ~7 s, so that reading is **not reachable** with a DOM-in-memory design.
    It would need lazy per-sheet parsing, which is not planned. Recorded here
    rather than quietly redefined.

  What made the difference, for the next reader: `<c>`'s attributes were being
  read with three separate lookups, each running quick-xml's default duplicate
  check, which is quadratic and re-walks the tag. One pass with
  `with_checks(false)` took the whole parse from 3.6 s to 2.1 s. Nothing else came
  close — the store, value building, and interning together were under 0.5 s.

- [x] **C5. Formula engine — parser + graph**
  Lexer, Pratt parser, AST, A1/R1C1 references, ranges, dependency graph, topological
  incremental recalc, cycle detection, volatile tracking.
  *Exit: dependency-order recalc correct on a generated stress workbook.* — **met**
  by `workbook::tests::a_stress_workbook_recalculates_correctly`, which checks a
  60x20 grid of cross-referencing formulas against an independent computation of
  the same recurrence.

  Precedents are stored as *areas*, never expanded: `SUM(A:A)` would otherwise be
  a million edges. Cells in a cycle — including cells merely downstream of one —
  are excluded from the order and reported, rather than given an arbitrary value.
  A self-reference needs its own detection: a self-loop contributes no in-degree,
  so Kahn's algorithm hands it back as perfectly sortable.

- [x] **C6. Function library — batch 1** (math/trig, logical, text)
  *Exit: conformance suite passes, incl. Excel's coercion and error-propagation quirks.*
  — **met: 37 conformance tests, all green.**

  Roughly 110 functions across math/trig, logical, text, and the `IS*` family
  (which the logical ones need). Plus `SUMIF`/`SUMIFS` and the criteria
  mini-language they share with C7's `COUNTIF`.

  What took the work was not the functions; it was the three rules underneath
  them, each of which is invisible until it is wrong:

  - **Arguments arrive unevaluated.** `IF` must not compute the branch it does
    not take, and aggregation has to know whether a value was written directly
    or found in a range.
  - **`ROUND` rounds the decimal, not the binary.** `ROUND(1.005,2)` is 1.01 in
    Excel; 1.005 as an f64 is 1.00499999999999989, so scaling and rounding gives
    1.00. `functions::decimal_round` goes through the 15-significant-digit
    decimal that General format would display.
  - **Criteria are not comparisons.** See the watch item above.

  Not implemented, and deliberately: `TEXT`, which needs the number-format
  engine from C11. Implicit intersection was left to C7, which landed it.

  `workbook::recalculate` is the loop that drives all of this over a real
  `Workbook` — parse, build the graph, evaluate in topological order, write
  back. A formula that does not parse keeps the value the file cached for it:
  Excel's own answer is better than one we know we cannot compute.

  **The strongest check is not the hand-written suite.** Every formula cell in
  an xlsx carries the value Excel last computed for it, which makes the corpus a
  conformance suite nobody had to write:
  `ss-xlsx/tests/recalc_against_excel.rs` opens each workbook, recalculates from
  the formula text alone, and compares. **27 of 27 comparable cells match.** The
  5 skipped cells all name a function that is genuinely not written yet —
  AVERAGE, COUNTIF, TEXT, TODAY, TRANSPOSE, VLOOKUP — and the test proves that
  by parsing the formula and checking the registry, so a `#NAME?` from a
  function we *do* have still fails.

- [x] **C7. Function library — batch 2** (lookup/reference, date/time, statistical)
  Includes the 1900 leap-year bug and implicit intersection. *Exit: as C6.*
  — **met: 57 conformance tests, all green.**

  93 more functions, taking the library to 199. `src/datetime.rs` holds the
  calendar; `functions/{date,lookup,stats}.rs` hold the rest.

  **1900 was not a leap year and Excel thinks it was.** Serial 60 is 29 February
  1900, a day that never existed — Lotus 1-2-3 had the bug, Excel copied it in
  1985 for file compatibility, and removing it now would move every date in
  every existing file. So serials 1–59 and 61-onwards use *different* epochs,
  and Excel believes 1 January 1900 was a Sunday when it was a Monday. Getting
  this wrong is invisible: every date after February 1900 shifts by one day,
  which reads as an off-by-one rather than a calendar bug.

  **Implicit intersection** is in: a range used where one value is expected
  collapses to the cell in line with the formula, so `=A1:A10` is A5 in row 5
  and `#VALUE!` in row 11. Operators deliberately do *not* intersect — they
  broadcast, which is modern Excel's rule, and the pre-2019 behaviour is
  recorded as a divergence rather than emulated.

  `recalculate` now spills a CSE array formula across the range it covers.
  Before this, every cell but the anchor kept whatever the file had cached.

  Corpus recalculation: **30 of 30 comparable cells match Excel**, 2 skipped —
  one naming `TEXT` (C11) and one that relies on legacy implicit intersection
  for a non-array-entered `TRANSPOSE`. Both are printed by name, and the skip is
  a stated rule rather than a hard-coded cell.

  Known gaps, deliberate: shared-formula followers are not translated from their
  master (no corpus file exercises it yet, and an untranslated follower keeps
  its cached value rather than being wrong); `INDIRECT` and `ADDRESS` are A1-only
  because the parser is; a cell fed by an array formula's spill is not a node in
  the dependency graph, so a formula reading one may be ordered before it.

- [x] **C8. Grid UI**
  Virtualized wgpu grid, selection model, frozen panes, merged ranges, number-format
  rendering, resize/insert/delete rows+cols.
  *Exit: 1M-row sheet scrolls at frame rate.* — **met**, by
  `grid::tests::scrolling_a_million_rows_stays_flat`: a frame's cell planning at
  the bottom of a million rows costs the same as at the top, and both fit inside
  a 16 ms frame. It is measured on the pure planning step, which is the part
  that scales with the sheet; the GPU cannot be tested headlessly.

  Nothing may ever be O(rows). `Axis` stores a default size plus a sorted list
  of exceptions with a running total beside it, so "where does row 900,000
  start?" is one binary search rather than 900,000 additions. That is also how
  the file stores sizes, so the two shapes match.

  Frozen panes are why painting is not one loop: a sheet frozen on both axes is
  drawn as four independent views of itself, each the same `plan()` call with a
  different scroll offset and clip rectangle.

  **Number formats came with this chunk, not C11.** A grid cannot render
  `45352` without knowing it is a date, so `ss-model::numfmt` parses and applies
  Excel's format language — four sections split by sign, `m` meaning month or
  minute depending on its neighbours, a comma that groups between digits and
  divides by a thousand after them, `[h]` for elapsed time, fractions,
  conditions, and colours. `ss-xlsx` now reads `<numFmts>` and `<cellXfs>` from
  styles.xml to drive it. Fonts, fills, borders, and alignment are still C11.

  **Moved to C9:** insert and delete rows/columns. They are editing operations
  that are unusable without undo, and doing them properly needs formula
  reference translation (a reference into a deleted range becomes `#REF!`),
  which in turn needs token spans in the lexer. All three belong together in
  the editing chunk. Column and row *resizing* is done here.

- [x] **C9. Editing + undo + keybindings**
  Cell editor, formula bar with reference highlighting, fill handle, cut/copy/paste
  (incl. clipboard interop with real Excel), Excel keybinding table.
  Plus insert/delete rows+cols carried over from C8, with the formula reference
  translation they need.
  *Exit: an hour of real spreadsheet work without hitting a missing verb.* — **met**
  as far as a test can state it, by `ss-formula/tests/editing_session.rs`: type
  numbers, total them, fill the total sideways, insert a row in the middle, undo
  it, cut the block and paste it elsewhere, with a recalculation after every
  step. The keyboard table itself is tested headlessly in
  `grid::paint::tests`, driving real pointer and key events through
  `GridView::show`.

  **Undo is a value, not a second implementation.** Applying a `Change` returns
  the `Change` that undoes it, so redo is just the undo of the undo and the two
  directions cannot drift apart. Cost is bounded by what actually changed: the
  cells a deletion destroyed, the formulas whose text was rewritten, and one
  snapshot of merges, sizes, and the freeze. Nothing clones a sheet.

  **Two different translations, and `$` distinguishes them.**
  `translate::translate` rewrites references when the grid moves under a formula
  — inserting a row moves the data that was in `$A$1` down to `$A$2`, so an
  anchored reference moves too. `translate::offset` rewrites them when the
  formula itself is copied, and *there* the dollar signs pin. Both work on the
  token stream rather than the parse tree, so `=sum( A1 , A2 )` keeps its
  spacing, its lowercase, and every byte the user typed that is not a reference.

  **A structural edit is workbook-wide.** A formula on Sheet2 naming
  `Sheet1!A1` is just as wrong after Sheet1 gains a row, so `edit::structural`
  walks every sheet's formula arena, not only the one that moved.

  Typing needed a writable style table: a date is a serial *plus* a format, and
  without the second half the user sees `45306`. `StyleTable::style_for_format`
  reuses an existing style, resolves a built-in `numFmtId` when the code is one
  of Excel's, and only then allocates a custom id — the ids a C10 writer has to
  put back.

- [x] **C10. xlsx writer** — plus a file dialog and Ctrl+S.
  Serialize from model through the Preservation Vault.
  *Exit: harness check 2 (edit round-trip) green across the corpus.* — **met:
  12 of 12 spreadsheets, and check 1 still 27 of 27.**

  **The writer edits the file; it does not reprint it.** A real worksheet holds
  autofilters, conditional formatting, data validation, hyperlinks, sheet
  protection, the anchor tying a chart to the grid, page setup, and whatever the
  current version of Excel puts in `extLst`. Reprinting the part from our model
  would delete every one of them, so instead the original bytes *are* the
  document and `write::sheet_out` walks them replacing only `<sheetData>` — and
  inside it, only the cells whose content actually differs from what the file
  says. A cell nobody touched goes back byte for byte; a cell that was edited
  keeps its own start tag, so `cm`, `vm`, and `ca="1"` survive an edit rather
  than being dropped as things we never modeled.

  `write::splice` is what makes that possible: an XML reader that hands back each
  event *and the span of source bytes it was parsed from*. Copying the span
  rather than re-serializing the event keeps the producer's whitespace, its
  choice of `<c/>` over `<c></c>`, and its entity escaping.

  **The harness now asks two questions instead of one.** A no-edit save proves
  the bytes we copied survived being copied — which, for a writer built on
  copying, is nearly a tautology. So spreadsheets also go through
  `flush_regenerating`, which writes *every* cell out of the model, and the
  corpus is compared against that too. No save does this; it exists so that a
  divergence between our idea of a worksheet and Excel's is reported by the
  harness rather than by the one user whose edit lands on that row. It found
  three the ordinary pass could not: `ca="1"`, `ref="D1"` rather than `D1:D1` on
  a single-cell array formula, and `spans`.

  **`spans` is a block hint, not a row hint.** Excel writes the span of the
  sixteen-row block a row belongs to, so a row holding A and B alone says `1:4`
  if another row in its block reaches D. Recomputing it would rewrite rows nobody
  edited for no gain, since the attribute's only requirement is that it *covers*
  the cells — so an edited row widens the file's value instead of replacing it.

  **A new workbook is authored, not templated.** `write::blank` builds the
  smallest package Excel will open — and deliberately builds it *empty*:
  `<sheetData/>`, no shared strings, one style. Everything typed then arrives
  through the same splice writer that edits a real Excel file, so there is one
  code path for cells rather than two that can disagree.

  Calx can now open, save, and save-as through a file dialog, with Ctrl+S,
  Ctrl+Shift+S, Ctrl+O and Ctrl+N, a dot in the title bar for unsaved changes,
  and a prompt before anything that would discard them.

## Phase 2 — Calx completeness

- [x] **C11. Formatting** — fonts, borders, fills, alignment, conditional
      formatting, data validation, cell styles, plus `TEXT()`.

  **A cell's colour is almost never an RGB triple.** `theme="4" tint="-0.25"` is
  what the whole modern palette uses, so `ss-model::color` resolves all four
  spellings — rgb, theme, indexed, auto — and `ss-xlsx::theme` reads the scheme
  out of `theme1.xml`. Two traps live in that alone, both recorded above.

  **The style tables are index-addressed, and that governs every mutation.**
  `StyleTable::restyle` is the whole formatting API: look a style up, change a
  field, ask for the style that *has* that look. Nothing is edited in place and
  nothing is inserted. `write::styles_out` appends fonts, fills, borders and
  xfs the same way it already appended `<numFmt>`, and re-reads the file's own
  counts rather than trusting the reader.

  Conditional formatting is evaluated in `ss-formula::cond`, because half the
  rule kinds are a formula. Colour scales and data bars need the whole region,
  so the rules are prepared once per sheet rather than asked per cell.

  Data validation is read, enforced on entry, and offered as a dropdown. A rule
  is about the *value* rather than the characters, so `edit::typed_value`
  decides what an entry would become before anything is stored.

  **A whole-column format is one attribute on `<col>`**, and `write::sheet_out`
  grew a writer for it — found by checking the output with openpyxl rather than
  by a test, because every test we had was satisfied by the model alone. A
  `<col>` span whose columns stop agreeing is *split*, each piece a retag of
  the original, so `outlineLevel`, `customWidth`, and everything else nobody
  models rides along. A row style needs `customFormat="1"` beside its `s` or
  Excel ignores it entirely.

  Known limits, stated rather than hidden: bold is synthetic (see the watch
  item), icon sets are not drawn though `showValue="0"` is honoured, and
  stacked text (rotation 255) is drawn upright.

- [x] **C12. Charts** — read, preserve, render; the title is editable.

  A chart is three parts and two relationship hops: worksheet → drawing →
  chart. The numbers are re-read from the cells every frame, so editing B7
  redraws the bar above it; the file's cache is the fallback for a series whose
  reference cannot be resolved.

  Drawn: bar and column (clustered and stacked), line, area, scatter, pie,
  doughnut, with gridlines, value labels, thinned category labels, and a legend.
  The value axis always includes zero, because an axis starting at the smallest
  value exaggerates every difference on it. Not drawn: 3-D perspective (a
  `bar3DChart` is drawn flat), trendlines, gradients, per-point formatting —
  none of which costs anything, because the parts go back byte for byte.

  `tests/charts.rs` runs against the corpus rather than a fixture, and checks
  that a retitle changes one chart part and nothing else.

- [x] **C13. csv/tsv** — dialect sniffing, encoding detection, streaming.

  Nothing holds a whole file: `Reader` pulls one record at a time into a buffer
  it reuses, and a record is not a line because a quoted field may contain
  newlines. Encoding is a byte-order mark, then unmarked UTF-16 by where its
  NULs fall, then UTF-8, then Windows-1252 — the fallback precisely because it
  cannot fail.

  Fields are *interpreted* rather than stored as text, through the same
  `edit::typed_cell` typing uses. Calx opens and exports csv, tsv, tab and txt;
  an import does not adopt the path as the document's own, because a csv holds
  one sheet of values and Ctrl+S over the original would discard everything
  added since.

- [x] **C14. Function library — batch 3** (financial, database, engineering,
      dynamic arrays) + pivot table read/preserve. **271 functions.**

  The financial family turns on two conventions — money out is negative, and
  `type` says when in the period the payment falls. `RATE`, `IRR` and `XIRR`
  use Newton with a bisection fallback, because Newton alone diverges on a
  series that changes sign more than once.

  **Dynamic arrays spill**, and three things had to be true for that to be
  safe: which functions spill is a list rather than a rule, the sheet remembers
  where each spill went so a shrinking result clears its tail, and a cell in
  the way is reported as `#SPILL!` with the obstruction untouched.

  Pivot tables are read for their *rectangle* — the cells are already in the
  worksheet, so the grid drew them correctly all along; what it needed was to
  know not to let anyone type into them.
- [ ] **C15. .xls reader (legacy)** — CFB container + BIFF8 records, read-only.
      `cfb-reader` is still a stub; this is the whole of it plus the BIFF record
      stream. Read-only by design (see Deferred).

## Phase 3 — Scriva (word processor) core

- [ ] **C16. Word model** — paragraph/run tree, style inheritance resolution, sections,
      numbering, tables, revision + comment layers.
- [ ] **C17. docx reader** — document.xml, styles, numbering, settings, headers/footers.
- [ ] **C18. Layout engine — inline** — itemization, shaping via cosmic-text/swash,
      UAX #14 line breaking, justification.
- [ ] **C19. Layout engine — block** — pagination, tables, floats with text wrap,
      footnotes/endnotes, columns.
- [ ] **C20. Document UI** — custom render surface, caret/selection model, scrolling,
      zoom, page view.
- [ ] **C21. Editing + undo + keybindings** — typing, formatting commands, Word
      keybinding table, coalesced undo.
- [ ] **C22. docx writer** — through the Preservation Vault.
      *Exit: harness check 2 green across the Word corpus.*

## Phase 4 — Scriva completeness

- [ ] **C23. Styles UI, TOC, fields, bookmarks, hyperlinks.**
- [ ] **C24. Track changes + comments — editable**, not just preserved.
- [ ] **C25. Images, shapes, text boxes** — placement, wrap, basic editing.
- [ ] **C26. Plain text + Markdown** read/write; encoding and line-ending handling.
- [ ] **C27. .doc reader (legacy)** — CFB + MS-DOC piece table, read-only.

## Phase 5 — Ship

- [ ] **C28. Packaging + install** — `cargo xtask install` to `~/.local/bin`,
      config/state in `~/.config/{calx,scriva}/`, Linux build verified on Ubuntu.
- [ ] **C29. Performance pass** — startup, large-file open, scroll, recalc.
      Known item from C6: `DependencyGraph::dependents_of` is a linear scan, so
      building the evaluation order is quadratic in the formula count.
- [ ] **C30. Docs** — user guide, format support matrix, honest fidelity report.

## Deferred

- [-] **PDF** — dropped per Q3. Revisit after Phase 5.
- [-] **.doc / .xls writing** — never; save-as-modern is the escape hatch.
- [-] **Macros / VBA** — preserved verbatim, never executed.
- [-] **PowerPoint** — out of scope.
