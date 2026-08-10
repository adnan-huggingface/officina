# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** C16 — the Word model (C0–C15 done, plus a UX pass and a sheets /
sort / filter pass)
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
- Workspace total is **734 tests**, all green;
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
- `cfb-reader` and `ss-xls` are C15: the legacy `.xls` reader, read-only. There
  is **no `.xls` in this repository** — the corpus is generated locally rather
  than downloaded — so the unit fixtures are laid out byte by byte from the
  record layouts by `cfb_reader::fixture` (behind the `test-support` feature)
  and the crate's own test module. Independent confirmation comes from
  `C:\Program Files\Microsoft Office\root\Office16\SAMPLES\SOLVSAMP.XLS`, which
  Office installs: seven sheets, 160 defined names, 129 formulas, all of which
  recompute to the values Excel cached. Re-run it after touching either crate.
- Watch item: **`FONT` index 4 does not exist.** The records are numbered
  0, 1, 2, 3, 5, 6, … so an `XF` saying `ifnt = 7` means the sixth record. Read
  as a plain index every cell in a workbook with five or more fonts is drawn in
  the one before the right one, which reads as a rendering bug.
- Watch item: **a shared formula's column offset is eight bits and its row
  offset is sixteen.** `FF` in the column field is one column *left*;
  sign-extended from the fourteen bits the field nominally has it is 255
  columns *right*, which lands in empty space — so the formula comes back
  well-formed and pointing at nothing. This is the one thing the hand-built
  fixtures did not catch and the real file did.
- Watch item: **the shared string table's `CONTINUE` boundary carries a fresh
  encoding byte.** A string may be cut in half, and its second half may be
  wide where its first half was compressed. Join the payloads and parse the
  result as one buffer and every string past the first split is mojibake, with
  no error anywhere.
- Watch item: **an unreadable formula keeps its cached value and loses only its
  text.** `func::VAR` marks every function whose fixed arity is not certain, and
  a `ptgFunc` naming one abandons the formula rather than popping a guessed
  number of operands. A cell showing the right number with nothing behind it is
  a limitation; a cell showing a formula that is subtly not the one in the file
  is not.
### The UX pass (after C14, before C15)

The application was rebuilt around the complaint that it was unusable on a real
workbook. The reference was `190A1210.xlsx` — fifteen sheets, 378,608 cells,
frozen panes, ninety rotated column headings, sixty-five merges per sheet — next
to a screenshot of the same file in Excel.

- **The shell owns three panels now, not one rectangle.** `ui-kit::shell` puts
  the toolbar in a top panel, the tabs and status line in a bottom panel, and
  hands the app what is left. The old arrangement subtracted a hard-coded 48
  pixels for the tabs and drew them by hand, which is why *they were not on the
  screen at all*: a fifteen-sheet workbook with no way to reach sheet two.
  `shell::tests` now asserts the centre cannot overlap the status panel.
- Watch item: **a window can open bigger than the screen.**
  `ViewportBuilder::with_maximized` is dropped when an explicit inner size is
  given beside it, and "1440 x 900" is *logical* — 2160 x 1350 at 150% scaling.
  The window then hides its own bottom edge behind the taskbar, which is where
  the sheet tabs live. The shell sends `ViewportCommand::Maximized` for the
  first few frames instead, because the very first frame is too early.
- **Type is loaded from the system.** `ui-kit::fonts` registers twelve families
  — sans, serif, mono × regular, bold, italic, bold-italic — from the machine's
  own font files, and the grid asks for the one the cell's style names. The
  synthetic bold (the same glyphs stamped twice, half a pixel apart) is gone.
  Watch item: **epaint panics rather than substituting for a family that was
  never registered**, so anything driving the grid without the shell has to call
  `fonts::register` first; the grid's own tests do.
- Watch item: **a row with no stored height is not a default-height row.** Excel
  measures it at paint time, so a cell holding three lines has no `ht` in the
  file at all. Read as the default, all three lines draw in the space of one and
  spill over the rows above and below — legible, if you squint, as two different
  rows of the spreadsheet at once. `axis::auto_row_heights` fits them.
- **The sheet view is part of the document.** `SheetView` carries the zoom, the
  gridline and heading switches, the selection, the scrolled-to cell and the tab
  colour; `Workbook::active_sheet` carries which tab was showing. A workbook now
  opens where it was closed. Watch item: **a frozen sheet writes one
  `<selection>` per pane** and only the one matching `activePane` is the real
  one — the first is usually the frozen headings. Watch item: **`topLeftCell` is
  measured from A1 while the scroll offset is measured from the frozen split**,
  so it has to be subtracted or the sheet opens scrolled past everything the
  freeze existed to keep on screen.
- **The grid has scrollbars, and they are sized to the used range** rather than
  to the sheet. A thumb proportional to 1,048,576 rows is a two-pixel sliver
  representing a document that ends on row 354.
- Watch item: **the cell canvas is white whatever the chrome does.** A cell with
  no fill is paper. The workbook chose black on white; tinting the canvas to
  match a dark application theme shows the user a document they did not make,
  and a themed heading resolved against Excel's light scheme vanishes on it.
- Watch item: **toolbar icons are drawn, not typed.** `⏴`, `⏎`, `↶` and the rest
  live in Unicode blocks that Arial, Segoe UI, and the fonts egui ships all
  decline to cover, so they render as hollow boxes — silently, with the code
  compiling and the tests passing. `icons.rs` draws them from lines and
  rectangles instead. The four that genuinely are letters are drawn in the real
  bold and italic faces, so the toolbar is its own preview.
- The status bar reports Excel's own set — average, sum, count, min, max — over
  the selection, computed from the sheet's *stored* cells rather than from the
  selected addresses, because selecting a column selects a million of them.
- Also landed: a name box that navigates (an address, a range, or a defined
  name), merge, freeze, hide/unhide, fit-to-contents, autosum, a right-click
  menu over the grid, Ctrl+PageUp/PageDown between sheets, and a font-family
  control.
- Watch item: **`recalc_against_excel` skips volatile functions.** A cached
  `TODAY()` agrees with the engine on the day the corpus was written and
  disagrees every day after, which is a fault in the comparison.
- **Anchored pictures are drawn.** A worksheet points at a drawing, and that
  drawing holds pictures as readily as charts — `read_drawing` now follows both
  branches and settles which is which by the content type of the part at the
  end. Watch item: **a heading is not always a cell.** The masthead of the
  reference workbook's first sheet is a PNG anchored over four empty rows, so a
  reader that skips images shows a form with nothing at the top of it, which
  reads as a fault in the file rather than in the reader.
  - The bytes are shared (`Arc<[u8]>`) because a `Workbook` is cloned for undo,
    and decoded once per *part* rather than per anchor — a logo repeated across
    fifteen sheets is one texture upload. A part that fails to decode is
    remembered as a failure, so nothing retries a broken PNG sixty times a
    second, and a box is drawn where it would have been.
  - The image fills its anchor rather than being fitted inside it, which is
    what Excel does and what the handles promise: pull the middle handle out
    and the picture has to stretch, not float at its own proportions in a wider
    box.
- **A picture is a thing you can take hold of.** Click it and it wears Excel's
  own chrome — a thin outline and eight round handles — and can be dragged,
  stretched from any edge or corner, and deleted. The cell cursor and the header
  highlight go away while it is selected, because two selections on screen at
  once leaves the user guessing which one Delete is about to act on.
  - Geometry is done in **sheet space**: pixels from the top-left of A1, before
    scrolling. It is the only frame of reference a drag can be measured in that
    does not move underneath it, and it is what `Drag::ResizePicture` stores so
    the edges a handle *does not* move stay exactly where they were.
  - `write::drawing_out` splices the anchor in place, the way cells are spliced.
    Anchors are matched **by position in the file**, counted over every anchor
    including the ones holding charts, because geometry cannot be the identity
    when changing the geometry is the whole point of the edit. Moving the logo
    in the fifteen-sheet reference workbook and saving changes `drawing1.xml`
    by one byte and loses nothing: the `a16:creationId`, the `cstate="print"`,
    the `prstDash` all come back untouched.
  - Watch item: **a picture also carries a cached `<a:xfrm>` inside its
    `spPr`, and the writer leaves it alone.** The anchor is what Excel positions
    from — it has to be, or a picture would not move when a column is widened —
    so the cache is stale in the same way it is stale in Excel's own files
    between edits. Rewriting it would mean converting our column measurements
    into absolute EMUs and asserting they match Excel's, which is a much larger
    claim than this edit needs to make.
  - Deleting a picture removes its anchor and **leaves the image part and its
    relationship in place**. An orphaned part is untidy; a dangling relationship
    is a file Excel refuses to open, and pruning the graph is a much bigger
    claim than "the user deleted a picture" justifies.
  - Undo is one patch, `Patch::Pictures`, which replaces a sheet's whole list.
    A sheet holds a handful of them, not a million, and moving one and deleting
    another are then the same operation with the same inverse. A per-picture
    patch would need indices that stay valid across a deletion.
- Watch item: **a merge is drawn from its anchor, and its anchor may be
  off-screen.** `plan` walks the visible rows and columns and draws a merge at
  its top-left cell, so a banner merged from column A disappeared the moment
  column A scrolled away — and on a sheet frozen at F4 it disappeared always:
  the anchor sits in the frozen pane, the fill is clipped to five columns, and
  the text, centred over ninety-five, lands outside that clip entirely. Row 4 of
  the reference workbook's Message Summary is exactly this, and it read as an
  empty grey band. `plan` now makes a second pass over `sheet.merges` for the
  ones that *reach into* the pane without starting in it.
- **Table styles are read and drawn.** A table is the one place where a cell's
  appearance is not in `styles.xml` at all: `<tableStyleInfo
  name="TableStyleMedium15"/>` names a style that lives *in Excel*, and the
  cells it covers usually carry no style of their own. The CRC calculator sheet
  of the reference workbook is entirely this — a black header row with white
  bold headings over a grey data row — and we drew bare text on white.
  - What comes from the file is exact: the range, the header and totals row
    counts, the four emphases, and the `dxf` overrides. What is *not* in the
    file is the built-in style's palette. Excel's definitions are not published
    in the package, and Excel on this machine is unlicensed so they could not be
    measured through COM either — so `ss_model::table` is our rendering of the
    gallery rather than a copy of it, in the same class of approximation as
    drawing a 3-D chart flat. `TableStyleMedium15` is the one checked against
    Excel pixel for pixel, off the user's own screenshot: solid black header,
    white bold text, stripes in `#D9D9D9`, which is black lightened by 0.85.
    Colours come back as *theme* colours, so a workbook with its own scheme
    gets its own palette rather than ours.
  - Watch item: **`<color auto="1"/>` in a dxf is present and says nothing.**
    Excel writes exactly that as a table's `headerRowDxfId`. Taken as an
    override it repaints the white headings of a black header row in
    "automatic" — black on black, which reads as the text having vanished. A
    dxf attribute now only overrides when it resolves to something.
  - The table sits *under* the cell's own style, which is what makes it visible
    at all: a cell that has chosen a fill has chosen to differ from its table.
    Bold is the exception and is a union — a heading's own font is very often
    plain Arial 10, and letting `bold = false` win would erase the header row.
- Known limits, stated rather than hidden: icon sets are still not drawn, 3-D
  charts are drawn flat, stacked text (rotation 255) is drawn upright,
  `Group 0`-style outline grouping is read but its collapse controls are not
  drawn, and a picture's own cropping, rotation and effects are ignored — the
  image is drawn whole and upright, custom table styles defined in the workbook
  are not read (only the built-in names are recognised) and column stripes are
  not drawn. A picture cannot yet be *added*, only moved,
  resized and removed: authoring a drawing part, a media part and two
  relationships from nothing is the writer work that has not been done.
- Known limits of the `.xls` reader, stated rather than hidden: charts,
  drawings, pictures, comments, conditional formatting, data validation,
  autofilters, pivot tables, hyperlinks and print settings are all in the
  format and none of them are read — each is a feature `ss-xlsx` reads from a
  completely different encoding, and each is its own body of work. A formula
  containing an array constant (`ptgArray`) or a function outside the built-in
  table keeps its cached value and loses its text. A reference into another
  workbook does the same, because the linked file's own name list is not read.
  Nothing writes BIFF and nothing will: `DESIGN.md` §9 makes save-as-modern the
  way out, so a legacy file opens with no path and Ctrl+S is Save As.

### Sheets, sort and filter (after C15)

Two things a spreadsheet obviously has and this one did not: managing sheets,
and sorting and filtering data. Both came from screenshots of Excel put next to
Calx — the sheet tab's right-click menu, and the ribbon's Sort & Filter group.

- **A sheet is named by its *name* in every formula and by its *index* in every
  sheet-scoped defined name**, and both change under an operation that looks
  local. `ss_formula::sheets` does all four at once: rename respells every
  qualifier (`translate::rename_sheet`), delete turns every reference into
  `#REF!` (`translate::drop_sheet`) and drops names scoped to it, and insert,
  move and delete all re-point `localSheetId` on the names after them. Doing
  any one without the others leaves a workbook that opens and is quietly wrong.
- Watch item: **deleting a sheet leaves `#REF!` where Excel leaves `#REF!A1`.**
  Excel keeps the address so you can see what was lost, but `#REF!A1` is not a
  formula any engine can evaluate — `#REF!` is an error literal and an address
  cannot follow one. Ours drops the whole reference so the cell evaluates, and
  evaluating to `#REF!` is what it should say.
- **`Sheet::part` is a sheet's durable identity.** The writer pairs model
  sheets to file parts by it, never by position, because a dragged tab is the
  same sheet somewhere else and pairing by index would write its cells into its
  neighbour. `None` means "never written", which is what tells the writer to
  author a part; `blank::package_for` fills it in for a new workbook so the
  first save is not mistaken for a workbook full of unknown sheets.
- **`write::workbook_out`** is the writer that was missing — `<sheets>` and
  `<definedNames>` spliced, everything else in `workbook.xml` byte for byte —
  together with `reconcile_sheets` in `write::mod`, which authors a worksheet
  part for a new sheet, removes the part, relationship and content-type
  override for a deleted one, and **does nothing at all when nothing
  structural differs**. That last part is what keeps the no-edit fidelity check
  honest rather than merely passing.
- Watch item: **`Relationships::next_id` counts past the highest id in use, not
  past the count.** Otherwise removing two relationships makes a removed id
  available again, and a stale `r:id` elsewhere in the package resolves to
  whatever took its place instead of to nothing.
- Watch item: **removing a part must take its `.rels` companion and its
  content-type override with it.** An override naming a part that is not there
  is an invalid package, and Excel reports it as damage rather than as a
  missing sheet. `Package::remove_part` does the three together and
  deliberately does *not* follow the removed part's own relationships — an
  orphaned drawing is untidy and opens; a part another sheet still needs is not.
- **Sorting orders kinds before values.** Excel puts every number before every
  piece of text, then `FALSE`, `TRUE`, errors, and blanks — and blanks last *in
  both directions*, because a blank is the absence of a value rather than a
  small one. Coercing text to number would file `"10"` next to `10`; coercing
  the other way would sort 2 after 10.
- Watch item: **a formula that moves in a sort is rewritten like a copied one.**
  `=B5*2` landing in row 8 becomes `=B8*2`, via `translate::offset`, dollar
  signs and all. The new text goes into a *new* arena entry rather than over the
  old one, because an entry can be shared and overwriting a shared master would
  rewrite a formula that never moved.
- **A filter is a rule and a result, and they are stored separately.** The rule
  is `<autoFilter>`; the result is nothing more than the ordinary hidden-row
  state, so a workbook filtered here shows the right rows in a program that has
  never heard of filters. `ss_formula::filter::apply` turns one into the other.
- Watch item: **the filter matches displayed text, not the stored value** —
  that is what `<filter val="…">` holds and what the user ticked. A number
  comparison is the exception and compares numerically, or `>10` would exclude
  everything from 2 to 9 as text.
- Watch item: **`ss_xlsx::autofilter` is parsed by the writer as well as the
  reader.** A `<filterColumn>` can hold a `<top10>`, a `<dynamicFilter>`, a
  colour or icon filter or a date grouping, none of which this crate models, so
  the writer reads the file's own element back and returns the *original bytes*
  when the model agrees with it. Only a filter the user changed is
  rebuilt. The same shape governs `<tabColor>`, so a themed tab colour is not
  flattened to the rgb it happens to resolve to.
- Known limits, stated rather than hidden: a tab cannot be dragged (Move or
  Copy does it), "Select All Sheets" reports rather than groups — grouped
  editing is a mode with real consequences and nothing else in Calx has the
  concept — and the filter offers a value list and Excel's two-comparison
  custom filter, but not top-10, above-average, colour or date-group filters.
  Those are read and preserved, and shown as an unconstrained column.


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
- [x] **C15. .xls reader (legacy)** — CFB container + BIFF8 records, read-only.

  `cfb-reader` is real: header, DIFAT chain, FAT, the *mini* FAT for streams
  below 4096 bytes, and the directory tree. `ss-xls` is the BIFF8 reader over
  it — record stream with `CONTINUE` rejoining, the workbook globals, one
  substream per sheet, and a `Ptg` decompiler that turns the RPN token stream
  back into formula text.

  Read-only, and permanently: a legacy file opens with no path, so Ctrl+S is
  Save As and the format changes on the way out. That is `DESIGN.md` §9's
  save-as-modern escape hatch, not a gap.

  Verified against `SOLVSAMP.XLS`, which Microsoft ships with Office: **all 129
  of its formulas recompute to exactly the values Excel cached in the file.**
  That is the same check the xlsx corpus gets, on a file nobody here wrote.

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
