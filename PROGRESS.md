# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** C2 — fidelity harness (C0 and C1 done)
**Status:** harness built; **blocked on corpus**
**Handoff note:**

- Rust 1.97.1 `x86_64-pc-windows-gnu` installed at `~/.cargo/bin` (not on PATH —
  installed with `--no-modify-path`). Links against MSYS2 mingw-w64 GCC 12.1.0,
  so no Visual Studio is needed.
- `ooxml` is complete for C1 and C2: 38 tests, clippy clean.
- `cargo xtask fidelity` runs and correctly **fails** on an empty corpus rather
  than reporting a vacuous pass.
- **Next action:** put real Word/Excel documents under `corpus/` (see
  `corpus/README.md`), then `cargo xtask fidelity` until green. That green is
  C2's exit criterion and the gate for starting C3.
- Watch item: `eframe` 0.36 replaced `App::update(ctx)` with `App::ui(&mut Ui)`.
  Any eframe example found online will be for the older API.
- Watch item: the `crt-static` rustflag in `.cargo/config.toml` is there to make
  the exe standalone (requirement 4). It has not yet been verified against the
  wgpu stack — if linking fails, that flag is the first suspect.

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
  both apps. *Exit: `cargo run -p app-calx` opens a window on Windows.*

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

- [ ] **C3. Spreadsheet model**
  Sparse chunked cell store, interned strings, style table, defined names, number
  formats. *Exit: model round-trips through its own serde with property tests.*

- [ ] **C4. xlsx reader**
  workbook.xml, sheets, sharedStrings, styles.xml, merged cells, defined names.
  *Exit: opens a real 50MB Excel file in <2s with correct values and formats.*

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
