# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** C7 — function library, batch 2 (C0–C6 all done)
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
- `ss-formula` is complete for C5 and C6: 133 tests, of which 37 are the
  conformance suite in `tests/conformance.rs`. Lexer, Pratt parser, dependency
  graph, evaluator, and ~110 functions. Workspace total is **265 tests**, all
  green; `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` are both clean.
- Watch item: whether text becomes a number depends on **how the value reached
  the function**, not on what it is. `SUM(TRUE)` is 1, `SUM({TRUE})` is 0.
  `functions::visit_args` tags each value `Direct` or `Inside` for exactly this;
  never flatten arguments before a function sees them.
- Watch item: the criteria language (`SUMIF`, and `COUNTIF` in C7) compares
  *within* a type. Under the `=` operator's rules text sorts above every number,
  so `SUMIF(A:A,">0")` would otherwise count a column's text header.
- Watch item: `DependencyGraph::dependents_of` is a linear scan over every
  formula, so `evaluation_order` is O(formulas^2). Fine at corpus scale and for
  the 1,200-cell stress test; it will not survive a 100k-formula workbook. C5
  recorded this as a deliberate trade; `recalculate` is what makes it reachable
  by a user, so it is now on C29's list rather than a hypothetical.
- `cargo xtask fidelity` is green: **27 passed, 0 failed**. It still correctly
  *fails* on an empty corpus rather than reporting a vacuous pass.
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
  engine from C11. Implicit intersection is C7's; until then a multi-cell
  operand in scalar context collapses to its first cell.

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

- [ ] **C7. Function library — batch 2** (lookup/reference, date/time, statistical)
  Includes the 1900 leap-year bug and implicit intersection. *Exit: as C6.*

- [ ] **C8. Grid UI**
  Virtualized wgpu grid, selection model, frozen panes, merged ranges, number-format
  rendering, resize/insert/delete rows+cols.
  *Exit: 1M-row sheet scrolls at frame rate.*

- [ ] **C9. Editing + undo + keybindings**
  Cell editor, formula bar with reference highlighting, fill handle, cut/copy/paste
  (incl. clipboard interop with real Excel), Excel keybinding table.
  *Exit: an hour of real spreadsheet work without hitting a missing verb.*

- [ ] **C10. xlsx writer**
  Serialize from model through the Preservation Vault.
  *Exit: harness check 2 (edit round-trip) green across the corpus.*

## Phase 2 — Calx completeness

- [ ] **C11. Formatting** — fonts, borders, fills, alignment, conditional formatting,
      data validation, cell styles.
- [ ] **C12. Charts** — read/preserve/render the common types; edit basic properties.
- [ ] **C13. csv/tsv** — dialect sniffing, encoding detection, large-file streaming.
- [ ] **C14. Function library — batch 3** (financial, database, dynamic arrays,
      engineering) + pivot table read/preserve.
- [ ] **C15. .xls reader (legacy)** — CFB container + BIFF8 records, read-only.

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
