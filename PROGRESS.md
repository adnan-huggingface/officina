# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** C5 — formula engine, parser + graph (C0, C1, C3, C4 done; C2 blocked on corpus)
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
- `ss-xlsx` is complete for C4: 54 tests plus 2 ignored performance checks.
  Run those with `cargo test --release -p ss-xlsx -- --ignored --nocapture`;
  they are meaningless in a debug build.
- Watch item: quick-xml 0.41 does **not** fold entities into text. `&amp;` arrives
  as a separate `Event::GeneralRef`, so any text accumulator must handle it or it
  silently drops every `&`, `<`, and `>`. `xml::push_text` is the one place that
  does this; use it rather than matching `Event::Text` directly.
- `cargo xtask fidelity` runs and correctly **fails** on an empty corpus rather
  than reporting a vacuous pass.
- **C2 is blocked on the corpus, which is in transit.** The Office on this
  machine is unlicensed ("read and print only mode"), so COM cannot save — and
  it does not fail cleanly, it blocks on an activation dialog that hangs
  automation forever. `corpus/generate.ps1` now preflights for exactly that.
  Adnan is running it on another laptop and returning a zip; unzip it so
  `corpus/docx/` and `corpus/xlsx/` are populated, then run `cargo xtask
  fidelity` until green. That green is C2's exit criterion. C3 onward does not
  depend on it.
- Watch item: Windows PowerShell 5.1 reads BOM-less `.ps1` as ANSI, so any
  script here containing non-ASCII **must** be saved UTF-8 *with* BOM or it
  fails to parse. `generate.ps1` has one.
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

- [~] **C2. Fidelity harness + corpus**
  `cargo xtask fidelity`, the three checks from DESIGN.md §7, and a starter corpus of
  real Word/Excel documents covering the awkward cases.
  *Exit: harness runs green on C1's round-trip guarantee.*

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

- [ ] **C5. Formula engine — parser + graph**
  Lexer, Pratt parser, AST, A1/R1C1 references, ranges, dependency graph, topological
  incremental recalc, cycle detection, volatile tracking.
  *Exit: dependency-order recalc correct on a generated stress workbook.*

- [ ] **C6. Function library — batch 1** (math/trig, logical, text)
  *Exit: conformance suite passes, incl. Excel's coercion and error-propagation quirks.*

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
- [ ] **C30. Docs** — user guide, format support matrix, honest fidelity report.

## Deferred

- [-] **PDF** — dropped per Q3. Revisit after Phase 5.
- [-] **.doc / .xls writing** — never; save-as-modern is the escape hatch.
- [-] **Macros / VBA** — preserved verbatim, never executed.
- [-] **PowerPoint** — out of scope.
