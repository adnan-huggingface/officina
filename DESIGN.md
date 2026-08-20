# Design Debriefing

Two native desktop applications, written in Rust, shipped as standalone static
executables:

- **Calx** — spreadsheet. `.xlsx`, `.xlsm`, `.csv`, `.tsv` read+write; `.xls` read (phase 2).
- **Scriva** — word processor. `.docx`, `.txt`, `.md` read+write; `.doc` read (phase 2).

Priorities, in order: **import fidelity → Excel before Word → authoring depth → polish.**

---

## 1. Why this architecture

The single highest-risk property of a document editor is not what it renders — it is
what it *destroys*. A user opens a 60-page contract produced by real Word, changes one
paragraph, saves, and mails it to a client. If our parser silently dropped the tracked
changes, the custom XML data-bound fields, or the embedded chart, we have not built a
word processor; we have built a shredder with a nice font picker.

So the architecture is organized around one idea:

> **Parse what we understand. Preserve verbatim what we do not. Never write a part we
> did not either author or faithfully retain.**

Everything else — layout, UI, formulas — is downstream of that guarantee.

## 2. Workspace layout

```
crates/
  ooxml/        OPC container: zip, [Content_Types].xml, relationships,
                the Preservation Vault (§3), shared XML reader/writer
  ss-model/     Spreadsheet document model: workbook, sheets, cells, styles,
                defined names, number formats
  ss-formula/   Formula lexer, parser, AST, dependency graph, recalc engine,
                function library
  ss-xlsx/      xlsx/xlsm reader + writer over ooxml + ss-model
  ss-csv/       csv/tsv dialect sniffing, import/export
  wp-model/     Word document model: body, paragraphs, runs, styles, sections,
                numbering, tables
  wp-layout/    Text shaping (cosmic-text/swash), line breaking, pagination,
                table layout, float placement
  wp-docx/      docx reader + writer over ooxml + wp-model
  cfb/          Compound File Binary reader (phase 2, feeds ms-doc / ms-xls)
  ui-kit/       Shared egui widgets: menu bar, dialogs, theming, command palette,
                keybinding engine, undo stack
  app-calx/     -> calx binary
  app-scriva/   -> scriva binary
xtask/          Build, package, install, and the fidelity harness runner
corpus/         Test documents (real Word/Excel output) + expected snapshots
```

Two binaries rather than one: the apps share ~60% of their code through the crates
above, but a spreadsheet and a word processor have genuinely different main loops, and
one fused binary would pay both startup costs on every launch. Requirement 4 said
"prefer single exe *if possible*" — two single-file exes with no installer honors the
spirit (no DLL hell, no runtime, copy-and-run) better than one bloated one.

## 3. The Preservation Vault

The mechanism behind the fidelity guarantee, and the first thing built.

When an OPC package is opened, every part is classified:

- **Modeled** — we parse it into our document model and re-serialize from the model
  on save (`document.xml`, `sheet1.xml`, `styles.xml`, …).
- **Retained** — we do not understand it, so the raw bytes are held and written back
  byte-identically (custom XML, embedded OLE objects, ink annotations, unknown
  extensions, vendor namespaces).
- **Derived** — regenerated from scratch every save (`[Content_Types].xml`, relationship
  files, `app.xml` statistics).

Within a *modeled* part, unknown elements and attributes are captured into the model as
opaque nodes attached to their parent, and re-emitted in document order. This is what
makes "we don't support feature X" mean *X survives the round trip untouched* rather
than *X is gone*.

The invariant is machine-checked (§7), not a promise in a README.

## 4. Calx — the spreadsheet

**Model.** Sparse cell storage (row-major chunked, not a dense 1M×16k array), interned
strings, style indices rather than inline styles. Targets: 1M rows without swapping,
open a 50MB xlsx in under two seconds.

**Formula engine.** Hand-written lexer and Pratt parser to an AST, compiled to a
dependency graph. Recalculation is topologically ordered and *incremental* — editing one
cell recalculates its transitive dependents, not the workbook. Cycles are detected and
surfaced as `#CIRCULAR!` (with optional iterative-calculation mode, matching Excel).
Volatile functions (`NOW`, `RAND`, `OFFSET`, `INDIRECT`) are tracked explicitly so they
do not poison the incremental path.

Function library grows in batches, each batch checked against a generated conformance
suite: math/trig → statistical → lookup/reference → text → date/time → logical →
financial → database → dynamic arrays. Excel's quirks are the spec, including the ones
that are arguably bugs (1900 leap-year, `=1/0` → `#DIV/0!` propagation rules, implicit
intersection, coercion order).

**Rendering.** Virtualized grid on wgpu — only visible cells are laid out and drawn, so
scroll cost is independent of sheet size. Frozen panes, merged ranges, and per-cell
number-format-driven text are resolved during the visible-range pass.

## 5. Scriva — the word processor

The hard part, and the reason it goes second.

**Layout engine.** This is what we are actually building; everything else in a word
processor is chrome. The pipeline:

```
runs -> itemize (script/direction/font) -> shape (harfbuzz via swash)
     -> break into lines (Unicode UAX #14 + Knuth-Plass-ish justification)
     -> flow into columns -> paginate -> place floats, tables, footnotes
```

Text shaping uses `cosmic-text`/`swash` (harfbuzz-quality shaping, pure Rust) so complex
scripts, ligatures, and kerning are correct rather than approximated.

**Where fidelity will and will not match Word.** Word's line-breaking and justification
are undocumented and subtly version-dependent. We will match Word on line and page
*breaks* for mainstream documents; we will not always match sub-pixel glyph positions
within a justified line. Documents will look right and paginate right; a pixel diff
against Word will not be empty. This is stated up front because it is the one place the
goal of professional-grade Word compatibility has an irreducible gap.

**Model.** Paragraph/run tree with style inheritance resolved lazily
(document defaults → style → numbering → direct formatting), sections, headers/footers,
numbering definitions, and a comment/revision layer that is preserved even before it is
editable.

## 6. Shared UI

egui in "retained-ish" mode over wgpu: immediate-mode for the menus, toolbars, dialogs,
and panels; a custom retained widget for the document/grid surface where we control
layout and paint directly.

Word/Excel keybinding compatibility is a first-class feature with its own table-driven
engine, including the multi-key sequences (`Ctrl+K, Ctrl+C`) and the Excel navigation
verbs (`Ctrl+Arrow`, `Ctrl+Shift+Arrow`, F2/F4, `Alt+=`) that muscle memory depends on.

Undo is a shared command-stack crate with coalescing (typing collapses into one undo
entry per word-ish boundary, as Word does).

## 7. The fidelity harness

Runs in CI and via `cargo xtask fidelity`. Three checks, in increasing strength:

1. **Byte round-trip.** Open → save with no edits → the output must be semantically
   identical to the input (zip-order-normalized, whitespace-normalized XML compare).
   Any diff is a preservation bug. This is the gate that protects users' files.
2. **Edit round-trip.** Open → make a scripted edit → save → reopen → assert the model
   matches expectation *and* every untouched part is still byte-identical.
3. **Render snapshot.** Rasterize page 1..N and compare against a stored snapshot to
   catch layout regressions. (Compared against our own previous output, not Word's —
   see §5 on why a Word pixel-diff is not a realistic gate.)

The corpus is real documents produced by real Word/Excel, covering the awkward cases:
tracked changes, nested tables, floating images with text wrap, footnotes and endnotes,
multi-section documents with different page setups, RTL and CJK text, pivot tables,
conditional formatting, array formulas, charts, defined names, external references.

## 8. Platform and packaging

- **Toolchain:** Rust stable, `x86_64-pc-windows-gnu` on Windows (MSYS2 mingw-w64 GCC is
  already present here, which avoids a multi-gigabyte Visual Studio install),
  `x86_64-unknown-linux-gnu` on Ubuntu.
- **Output:** one self-contained executable per app. No installer, no runtime, no DLLs
  beyond the OS's own.
- **Install:** `cargo xtask install` copies to `~/.local/bin/` (both platforms — Windows
  gets `%USERPROFILE%\.local\bin` and a PATH check with instructions rather than a
  silent registry edit).
- **Config and state:** `~/.config/calx/` and `~/.config/scriva/` on both platforms, per
  requirement 7 — deliberately *not* `%APPDATA%` on Windows, since the requirement asked
  for the dotfile convention.

## 9. Honest risk register

| Risk | Mitigation |
|---|---|
| Word layout fidelity is a bottomless pit | Ship measurable line/page-break parity on a real corpus; state the sub-pixel gap openly rather than chase it |
| Formula function count is a long tail | Batch by category with generated conformance tests; publish a live coverage table |
| Legacy .doc/.xls parsing is ~1700pp of spec | Read-only, phase 2, best-effort, and always with the "save as modern format" escape hatch |
| egui is not designed for document-grade text | We do not use egui's text layout for documents — only for chrome. The document surface is our own widget. |
| Scope is genuinely enormous | Chunked plan in PROGRESS.md, each chunk independently shippable and resumable |
