# Progress

Resumable work log. **If a session ends mid-chunk, read this file first** — the
"Current state" section below is the handoff note.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done · `[-]` deferred

---

## Current state

**Chunk:** none — C0 through C30 are done, which is every chunk in the plan.
**Status:** complete, with three things open and named below.
**Handoff note:**

- **A real document off a real desk found five faults no corpus file had.** See
  "What one resume found" below. The short version: a `.docx` that Google Docs
  wrote is a different dialect from one Word wrote, and every corpus file was
  Word's.
- **Phases 3 and 4 are done. `cargo xtask fidelity` is 30 of 30 on check 1 and
  30 of 30 on check 2**, documents as well as spreadsheets — which is C22's
  stated exit criterion and the gate that protects users' files. 1363 tests.
- **A `.docx` can now be authored from nothing** (`wp_docx::write::blank`), so a
  new document saves and a `.doc` has somewhere to go. The authored package is
  a skeleton only: the paragraphs go in through the same splice writer that
  edits a file Word wrote, because a second code path for writing paragraphs is
  a second thing that can be wrong.
- Watch item: **Word writes 129, not 1, for direct bold in a `.doc`.** 129 means
  "the opposite of what the style says", because bold *is* a toggle. A reader
  that only understands 1 shows a document with no formatting at all and no
  error to explain it. `wp_doc::sprm` resolves it against the default, and says
  so where it does.
- Open, carried from phase 3: **the fixed-width shaper cannot see a fault that
  only appears with real metrics.** One heading of `RedAndBlackReport.dotx`
  draws past the right margin in the running application and does not reproduce
  under `Fixed`.
- Open: **the Linux build has never been run.** See C28.
- Open: **a large document is laid out in full on every edit.** 8000 paragraphs
  cost about 330ms; 800 cost 35ms. Incremental layout — reusing the lines of
  paragraphs that did not change — is the fix, and it is an architectural change
  rather than an optimisation, so it was named rather than half-built.
- Carried, and still the user's call rather than ours: **AUDIT finding 27**
  (a sheet's `defaultRowHeight` and `defaultColWidth` are unread, so rows are
  about 4% too tall) and the Calx `collect_cell` allocation refactor.
- **`LEARNINGS.md` is the debrief from Calx**, and it earned its keep: three of
  the bugs below are the exact faults it predicted, and one of them (toolbar
  glyphs that render as hollow boxes) was repeated anyway before the screenshot
  caught it.
- **The docx writer edits `document.xml`; it does not reprint it.** Each `<w:p>`
  is compared against the model by *re-reading it*, so "changed" means exactly
  "would read back differently" — the only definition that cannot drift away
  from the reader. Paragraphs are paired by document order **at any depth**: one
  corpus document is nothing but a content control, and a writer that walked only
  the body's children could not save an edit to it.
- Watch item: **quick-xml's `buffer_position` does not count a UTF-8 byte-order
  mark.** Every span is then three bytes short. Copying spans hides it perfectly
  — they still tile the input — and the first *replaced* span cuts three bytes
  early and leaves three bytes of the next element behind. `write::splice`
  adds the offset back.
- Watch item: **a `<w:sdt>` may wrap a `<w:tr>` or a `<w:tc>`.** Word's own
  templates are full of them. A reader that skips the wrapper loses the row
  inside it, the model runs behind the file, and the writer rewrites the wrong
  paragraphs.
- Watch item: **a reader must not add content.** An earlier version repaired
  every table cell whose last block was not a paragraph. A cell ending in a
  content control is ordinary, so the reader invented a paragraph the file did
  not have — and every paragraph after it went into the wrong cell on save.
- Watch item: **`<w:drawing>`, `<m:oMath>` and VML pictures are captured as
  bytes on read and written back verbatim.** Without that, editing a paragraph
  that holds a picture destroys the picture. The Preservation Vault applied
  *inside* a modelled part.
- **The layout engine measures through a trait.** `wp_layout::shape::Shaper` is
  all it knows about fonts, so it lays out headlessly against `Fixed`, whose
  every glyph is half its point size — a test can then say "this line holds
  eleven characters" and mean it. Known limit, and it bit: **the fixed-width
  shaper cannot see a fault that only appears with real metrics.** One heading
  of `RedAndBlackReport.dotx` draws past the right margin in the running
  application and does not reproduce under `Fixed`. Open, and the first thing
  C23 should look at.
- Watch item: **a paragraph's index was shadowed by the line loop's own
  `index`**, so every laid-out line reported paragraph zero and every click
  landed in the first paragraph of the document. Found by
  `view::tests::a_click_below_the_last_line_lands_in_the_last_paragraph`.
- **Scriva runs.** It opens `RedAndBlackReport.dotx`, paginates it to nine
  pages, counts 675 words, draws the themed heading colour and the table rules,
  and centres the page on a grey desk. Verified from a screenshot of the real
  program, not from a test.
- Watch item, and a repeat of Calx's: **toolbar icons are drawn, not typed.**
  `⯇`, `↶` and `≡` came out as hollow boxes in the first build, exactly as
  `LEARNINGS.md` §7 says they would. `app-scriva/src/icons.rs` draws them, and
  `every_icon_puts_ink_on_the_screen` keeps them drawn.

- **`LEARNINGS.md` is the debrief from building Calx** — what generalises,
  organised by preservation, reading, writing, modelling, what tests do not
  catch, performance, UI and process, and ending with what it predicts about
  Scriva. Read it before starting a Scriva chunk. The Excel-specific watch
  items stay in this file; that one holds the rules that will be true again.
- **C16 is done: `wp-model` is the Word document model.** 84 tests, and every
  one of them is about a rule that is invisible until it is wrong. What a
  reader (C17) must know before it starts:
  - **Absent is not false.** Every property is an `Option`, and `None` means
    inherit. A bare `<w:b/>` is *true*; `w:val="0"`, `"false"` and `"off"` are
    false. `prop::on_off` is the one place that decides.
  - **Thirteen properties toggle rather than override.** Bold applied by a
    style on top of a style that is already bold comes out *not* bold — the
    values XOR through the style hierarchy and only direct formatting is
    absolute. That is why Word's Strong character style un-bolds a heading.
    `prop::Toggles` and `prop::Toggle::ALL` are the closed list.
  - **A paragraph with no `<w:pStyle>` is not unstyled** — it takes the style
    marked `w:default="1"`. Skipping that layer loses the document's body font
    and reads as a rendering bug.
  - **A bare `<w:vMerge/>` means *continue*, not restart** — the opposite of
    every other bare on/off element in the format. Read the usual way, every
    vertically merged cell in a document becomes its own merge and the table
    draws empty.
  - **`<w:sectPr>` terminates a section rather than beginning one.** Read as a
    container, every page setup lands one section late.
  - **Word measures in five units**, and `w:sz` is half-points on a run and
    eighths of a point on a border. `units` makes them five types that cannot
    be added to each other.
  - **Footnote ids 0 and −1 are the separators**, not notes.
  - **`w14:paraId` is a paragraph's durable identity**, and it is what will
    make a splice writer possible for `document.xml`. Absent in files from
    producers other than Word, so nothing may depend on it.
  - Recorded divergence: the numbering level's `<w:pPr>` is layered *above* the
    paragraph style and below direct formatting. ECMA §17.7.2 orders it the
    other way; Word plainly does not, or no bulleted list would sit at its
    list's indent. Pinned by `a_list_level_moves_a_paragraph_its_style_did_not`
    and worth re-checking once there is a renderer.
  - Stated limit: `themeTint`/`themeShade` are blended toward white and black
    in sRGB, which is *not* a spreadsheet's HSL `tint`. Checked against the
    cached `w:val` Word writes beside the attributes, not against Word's screen.
- Workspace total is now **1203 tests**.
- **Calx is feature-complete for daily use as far as the audit reaches**, with
  one exception recorded on purpose: printing, which is out of scope by
  instruction. `AUDIT.md` holds every claim and every finding; findings 1–26
  and 28–32 are fixed, and 27 (a sheet's own `defaultRowHeight` /
  `defaultColWidth` are never read, so rows draw about 4% too tall) is
  deliberately open and belongs with the layout rather than with any one
  feature.
- Watch item: **finding 28 was a chart drawing every bar upside down**, which
  egui fills with nothing, so no bar chart had ever shown a bar. It survived
  C12, the audit and the insert-a-chart task because every test asked the
  model and none asked the painter. `grid::chart::tests::painted` is the
  answer to that: it runs a frame and reads the shapes back.
- **Ctrl+W closes the workbook** and leaves the window standing, even when it
  is the last workbook — Excel takes the application down with it, and a
  mistyped Ctrl+W should not end a session. It goes through the same unsaved
  prompt as closing the window, and `blank_slate` is now the one place that
  empties a document, so New and Close cannot drift apart. It clears the open
  dialogs too: a Format Cells over a selection that no longer exists is a box
  whose buttons can only do harm.
- **Every dialog is drawn by `ui_kit::dialog`** — frame, gutter, heading face,
  buttons, action row, message box. Finding 33, and the reason it is worth
  knowing: the app theme draws buttons flat and borderless on purpose, which is
  right for the toolbar and makes any dialog built from plain `ui.button` look
  unfinished. Use `dialog::confirm` / `dialog::row` / `dialog::button` for a
  form and `dialog::message` for a box; do not reach for `ui.button` inside a
  modal.
- **There is a menu bar** — File, Edit, View, Insert, Format, Data, Tools —
  drawn by `ui_kit::menu`, and it is where every command in the application
  lives with its keystroke printed beside it. Finding 34. The toolbar under it
  is now icons only; the fifteen bare word-buttons that used to run across the
  top were the whole of the complaint. Two things to know before touching it:
  every command goes through the `Command` enum and `Calx::run`, so the menu
  and the toolbar cannot answer the same command differently; and **nothing
  inside a menu may ask for `ui.available_width()`** — a popup measures itself
  in a pass where that is the width of the screen, which is what made the first
  File menu 700 points wide. `menu::sep` guards it with `ui.is_sizing_pass()`,
  and `menu::tests::a_rule_does_not_get_a_vote_on_how_wide_the_menu_is` keeps
  it that way.
- **The menus answer to the keyboard.** Alt+F, Alt+E, Alt+V, Alt+I, Alt+O,
  Alt+D, Alt+T open them; inside an open menu the underlined letter runs the
  command or opens the submenu. Mark the letter in the label with `&`, Windows
  style — `"Save &As…"`. The underlines show only while Alt is held or a menu
  is down. Two traps, both of which bit: a menu letter has to be *taken* rather
  than read, because `consume_key` removes the key event and leaves the `Text`
  event that anything accepting typing reads (`Marked::taken`); and a submenu
  opened by key needs `MenuState::mark_shown` before its row is recorded as
  open, or the menu forgets a row whose submenu has not been drawn *yet*.
- Workspace total is **1203 tests**, all green; `cargo clippy --workspace
  --all-targets -- -D warnings` and `cargo fmt --all --check` are both clean;
  `cargo xtask fidelity` is check 1, 27 of 27 and check 2, 12 of 12.
- The release binary at `target/release/calx.exe` is current with this state.
- **A save can be refused by Windows and that is not a bug in the save.** A
  workbook open in Excel cannot be written by anything else. Calx now says so
  in a box at the moment it happens *and* when the file is opened, and writes
  beside the target and renames over it so a refusal, or a crash mid-write,
  cannot leave a half-written workbook. Findings 30 and 31.
- Watch item: **`cargo fmt` had drifted across twenty files** before this was
  last checked. Edits made by script rather than by hand are the reason — they
  do not run the formatter. Run `cargo fmt --all` after any scripted edit.
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

### Resizing rows and columns (after the sheets pass)

Dragging a header boundary had worked since the UX pass. Nobody could find it,
and what it produced never reached the file.

- **A grab zone on one side of a line is not a grab zone.** It was four pixels
  wide and all four to the *left* of the boundary, so half of every attempt to
  drag landed on "select this column" instead. `paint::header_edge` now answers
  from either side, names the row or column *before* the line — the one a drag
  resizes — and is the single place the press, the hover cursor and the
  double-click all ask.
- **A boundary that can be dragged has to say so.** Nothing marks one out: the
  line between two columns is drawn whether or not it is draggable. The pointer
  turns into a resize arrow over it, which is the whole of the affordance.
- Watch item: **a drag names its own cursor; hovering cannot answer for it.**
  The icon was only ever chosen from where the pointer *is*, and the pointer
  leaves the boundary the moment it starts dragging it — so the arrow dropped
  back to a plain one halfway through the gesture, which reads as the drag
  having been let go. Every drag that has an icon now states it.
- Watch item: **double-clicking a cell to edit it fell into the same trap as
  double-clicking a boundary**, and for the same reason: the second click lands
  while the first one's selection sweep still owns the pointer, and the drag
  branch of `handle_input` returns before anything after it is looked at. It is
  resolved beside the boundary case now, and only supersedes a plain
  `Drag::Select` — a picture being moved or a scrollbar thumb keeps the pointer.
- Watch item: **a double-click on a boundary has to be resolved before the
  drag.** The first press starts a resize, a drag owns the pointer until it
  ends, and `handle_input` returns early while one is in flight — so the second
  click was never looked at. It is checked ahead of that branch now. The first
  click still opens and closes a resize that moves nothing, which is why
  `Action::Resized` carrying the geometry the sheet already has is dropped
  rather than pushed: without that, every autofit landed on two do-nothing undo
  entries.
- **`<cols>` had a writer for styles and none for widths.** A dragged column
  showed on screen, survived undo, and was gone the moment the file was read
  back. `ColumnLook` carries style and width together, because they are two
  attributes of one element and a run can only be spelled as one `<col>` when
  it agrees about both. `customWidth` rides with the width: a bare `width` is a
  producer's own measurement and Excel may re-fit it, which would quietly undo
  the drag.
- Watch item: **a hidden column is `hidden="1"`, not `width="0"`.** Both hide it
  on screen; only the first is what Excel's Unhide looks for, and the width
  beside it is the size the column comes back at. The model spells hidden as a
  width of zero — the same way a hidden row is already a height of zero — so the
  reader turns `hidden` into a zero and the writer turns a zero back into
  `hidden`, leaving the file's own `width` alone. Reading it any other way
  brought every column Excel had hidden back visible.
- Watch item: **a `<col>` the model needs and the file lacks belongs *in* the
  sequence.** Appending it after the file's own spans is legal and every reader
  copes, but no spreadsheet has ever written `<col>` elements out of numerical
  order. `file_columns` finds the file's spans in one pass ahead of the rewrite
  so the new ones can be threaded between them.
- Excel's exact-size boxes are there too — Column width in characters, Row
  height in points, the file's own units, with Excel's own ceilings of 255 and
  409 — on the right-click menu and under a Format menu on the toolbar, which is
  where Excel keeps them and where anybody who has not found the boundary looks.

### Sorting a real workbook (139,868 rows × 10 columns, 1.33M cells)

Measured against `excel_sort_test1.xlsx`, a 6.4 MB export whose sheet part is
53 MB of XML and whose string table holds 209k distinct values. It took just
under three quarters of a second to sort and left a hundred megabytes on the
undo stack. It is now about 120 ms and half a megabyte.

- **A sort is a permutation of rows, and the undo is the inverse permutation.**
  `Patch::Permute` carries a list of row numbers, not a copy of every cell that
  moved — the same insight as `Patch::Rearrange`, arrived at from the other
  direction. Rows whose formulas are rewritten as they travel are genuinely not
  a permutation of the rows they were, so `sort` looks first (`carries_formulas`,
  short-circuited by an empty formula arena) and falls back to `Patch::Cells`
  for those. 404 ms + 127 ms of apply became 68 ms + 38 ms.
- Watch item: **the inverse is built before anything moves, because building it
  is the check that the list is a permutation at all.** A list naming a row
  twice has no inverse, and applying it would scatter cells that undo could
  never gather up again.
- **Ten columns of a row live in one chunk, so one map lookup answers for all
  of them.** `CellStore::read_band` / `write_band` walk a row by chunk band;
  going through `get` and `set` paid for the lookup ten times over.
- **Case folding belongs in the decoration, not in the comparator.** `compare`
  called `to_lowercase` on both sides — two allocations — and a sort of 140k
  rows asks two million times. Folded once per row it is 182 ms of comparison
  down to about 20. Two spellings of a word now tie and fall through to the row
  number, which is also what makes a case-insensitive sort stable the way
  Excel's is.
- **`auto_row_heights` runs over every cell in the sheet on every edit.** It
  asked three map lookups and a font of each of 1.33M cells to find out that
  none of them wanted a taller row. Whether a *style* can ever want one is
  decided once per style into a table; whether any *string* in the workbook
  holds a line break is asked of the string table, where a status column is
  four strings rather than a million. 130 ms became 40.
- **`looks_like_headers` now answers on what is at the top, not on how it
  compares to what is below.** Text over text was called data, on the grounds
  that guessing wrong would drop a row out of the sort — but that is what most
  exported data looks like, every column a timestamp or an id or a status, and
  it filed the word `topic` in among the topics. Both answers are wrong
  sometimes; Excel takes the other one, and so do we now, because a heading
  sitting in the middle of the data is obvious from the screen and a row
  quietly left out of the sort is not. A number or a date at the top still
  vetoes the guess outright.
- **Saving this workbook took 5 s. It is now about 3.** Measured rather than
  guessed, with `save_a_real_workbook` in `ss-xlsx/tests/large_workbook.rs`:
  point `CALX_BIG` at a file and it prints where the time goes.

  Two of those five seconds were spent asking the *file* what it already says —
  its tab colour, its pane, its autofilter, its protection, its conditional
  formats — and each of those five questions walked all 1.33M cells to reach an
  element that cannot be among them. `sheet_out::without_cells` makes one copy
  of the part with `<sheetData>` emptied, and every one of those questions is
  now asked of a few kilobytes. Finding the two tags costs nothing because of
  where it looks: forwards from the start for the opening one, backwards from
  the end for the closing one.

  Remaining, measured and not yet done: **1.6 s of the rest is `collect_cell`**,
  which allocates twice for every cell in the file — an owned copy of the start
  tag, and a `String` for the value — to compare each against the model and
  throw both away. `Val::Text(String)` is what forces it. A borrowing `Val`, or
  a shared-string cell compared by index instead of by text, is the next move.
  Deflate is about 1 s of the 3 and is not worth trading: level 1 saved 20% of
  the time for 78% more file.


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

### The parity audit (after the sorting pass)

`AUDIT.md` is the whole of it: eighteen areas — selection, navigation, editing,
the fill handle, the clipboard, formatting, sorting, formulas in the UI, undo,
sheets, view, cursors, find, objects, files, the status bar, context menus —
written as concrete claims about **what Excel does**, before looking at what
Calx did. Twenty mismatches came out of the first pass and are logged with the
task that fixed each; seven more were found afterwards and logged the same way.

- **Most of them were about the mouse and the keyboard, not the file format.**
  Right-click collapsed the selection; the active cell was filled instead of
  left white inside its range; a selection sweep did not auto-scroll past the
  edge; Ctrl+A always took the whole sheet; End and Ctrl+End did nothing; there
  was no Go To, no formula point mode, no F4, no Alt+Enter, no marching ants,
  no drag-move of a range by its border, no fill-handle double-click. None of
  that is visible from a test of the model, and all of it is what using a
  spreadsheet *is*.
- **A context menu's Paste pasted the wrong thing.** It replayed the text Calx
  remembered rather than asking the system clipboard, so anything copied in
  another application arrived as whatever had been copied here last.
- **Select all, then sort, and the process died.** The corner box hands the
  sort a selection 1,048,576 rows tall, and the sort tried to allocate the
  rectangle it was given. A selection is now intersected with the used range
  before anything sizes an array from it.
- **The in-cell editor did not look like a cell**, and the window opened with a
  dark bar down its right, then opened maximized every morning once that was
  fixed. Three separate faults in the shell, all recorded in the log; the last
  one is why `~/.config/calx/window` exists.

### Completing the spreadsheet (tasks #39–#50)

Everything the audit found missing that was a *feature* rather than a fix,
taken one at a time. Each is in the toolbar where Excel keeps it, on the
right-click menu where Excel offers it, and under Excel's own key.

- **Find and Replace, Paste Special, Format Cells, the Name Manager**, and the
  data-validation and conditional-formatting dialogs. Find and Replace is one
  window with a row hidden rather than two windows, so Ctrl+H over an open Find
  keeps what has been typed.
- **Group and ungroup rows and columns**, with the outline bar and its
  collapse controls drawn beside the headers, under Excel's Shift+Alt+arrow.
- **Split panes**, which found a real bug on the way in: `<sheetView>` had *no
  writer at all*, so freezing a sheet never survived its own save. Both halves
  are fixed, and a file whose panes nobody touched still goes back byte for
  byte. The split bar drags and can be dropped back off the edge to remove it.
- **Protect Sheet**, read and written — the password attributes are preserved
  verbatim and never interpreted, because a hash we cannot check is not a lock
  we may open — and enforced at `perform`, the one place every change lands.
  Every flag in `<sheetProtection>` states what is *forbidden*, so the model
  stores the inverse and the dialog reads the way Excel's does.
- **Text to Columns and Remove Duplicates.** Both read everything before
  writing anything and rewrite the formulas that move. Remove Duplicates
  widens the selection to the block it sits in, because removing rows from one
  column while its neighbours stay put tears every row of the table in half.
- **Insert a chart, insert a picture** — the first time this crate *authors*
  parts rather than editing ones it was handed. Four things have to line up for
  one object to exist: the object part, a drawing part with the anchor and a
  relationship, a worksheet relationship and a `<drawing>` element, and a
  content type for each new part. Any one missing is a workbook Excel calls
  damaged. An empty part name is what marks an object as new, since anything
  read from a file names where it came from.
- **Notes on cells**, which live in three places at once: the comments part,
  the VML part that draws the box — deprecated in 2007 and still required — and
  the `<legacyDrawing>` naming it. A comments part with no VML beside it opens,
  and then Excel offers to repair the file, which is worse than no note at all.
- **The keys.** Alt+F1 charts the selection where it stands; F9 recalculates,
  which is only for the volatile functions since everything else recalculates
  on every edit; Ctrl+Shift+O selects every cell carrying a note, the only way
  to find one on a sheet bigger than the screen; Ctrl+Shift+7 outlines the
  selection and Ctrl+Shift+minus takes its borders off — that last one has to
  be answered before Ctrl+minus deletes rows, since shift is the only thing
  telling them apart. Split panes, protection and the two data tools get no key,
  because Excel gives them none: they are ribbon paths, and this has no ribbon.
- Known limit, stated rather than hidden: **Excel itself has not opened the
  files with authored charts, pictures or notes.** The evidence is our own
  reader agreeing with the writer, plus the schema order checked against the
  spec. That is real, and it is not the same as Excel's verdict.

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

## Phase 3 — Scriva (word processor) core — **complete**

- [x] **C16. Word model** — paragraph/run tree, style inheritance resolution, sections,
      numbering, tables, revision + comment layers. **84 tests.**

  `wp-model` is nine modules: `units`, `color`, `prop`, `style`, `numbering`,
  `section`, `table`, `revision`, `doc`. Nothing in it is resolved formatting —
  a paragraph stores what its own `<w:pPr>` said and no more, because the file
  records the difference between "this run is 12pt" and "this run inherits
  12pt", and a user editing the style expects the second kind to move.

  **The four-layer resolution is the heart of it.** Document defaults, then the
  paragraph style and everything it is `basedOn` *root first*, then the
  numbering level, then the character style chain, then the paragraph's own
  properties, then the run's. Walking `basedOn` outward from the named style
  and taking the first value found gives the right answer for ordinary
  properties and the wrong one for every toggle. A `basedOn` cycle resolves
  rather than hanging, because documents contain them and Word opens them.

  **Numbering is two tables and a walk.** `<w:num>` is an instance and
  `<w:abstractNum>` is a definition; collapsing them makes the second list in a
  document continue the first. A number is a function of every numbered
  paragraph before it, so `Counters` is walked forward once and rebuilt from
  scratch after an edit rather than patched in the middle. Word repeats a
  letter rather than carrying — 27 is `aa` and 28 is `bb`, not `ab` — which is
  invisible until a list passes twenty-six items.

  **The revision and comment layers are here rather than in C24** on purpose. A
  reader that flattens a tracked deletion has destroyed it, and no later chunk
  can bring the author back. `<w:delText>` is kept apart from `<w:t>` so a word
  count, a search and the flowed length all skip what has been deleted while
  the revision view still draws it.

  Deliberately not modelled and carried whole: equations (`<m:oMath>`, with
  their text extracted so search is not blind), VML and OLE objects, and every
  compatibility flag in `settings.xml`.
- [x] **C17. docx reader** — document.xml, styles, numbering, settings, headers/footers,
      footnotes, endnotes, comments, people. **62 tests**, plus the 41 templates
      Office ships as an independent corpus.
- [x] **C18. Layout engine — inline** — itemization, measurement through a trait,
      UAX #14-ish line breaking, tabs, alignment, justification.

  Shaping is not cosmic-text: measurement is a trait and the application answers
  it with the same epaint faces the spreadsheet draws with. That is what lets
  the engine be laid out headlessly against a shaper whose every glyph is half
  its point size — and a layout engine tested against a real face is tested
  against a moving target.
- [x] **C19. Layout engine — block** — pagination with the keep rules, tables,
      headers and footers, sections.

  Two passes: flow into items, then paginate and *pull the break back* to honour
  keep-with-next, keep-lines and widow control. Doing it in one pass works right
  up to the moment a paragraph says "keep with next", by which time the decision
  is made. Stated limits: no text wrap around an anchored drawing, no column
  balancing, and a table row taller than a page overflows rather than splitting.
- [x] **C20. Document UI** — the page surface, caret and selection, scrolling,
      zoom, menus with mnemonics, a drawn-icon toolbar and a status bar.
- [x] **C21. Editing + undo + keybindings** — typing, deletion, split and merge,
      run and paragraph formatting, Word's keys, undo coalesced at word
      boundaries.

  Undo is a value: applying a change returns the change that undoes it, so redo
  is the undo of the undo. Cost is bounded by what changed — typing remembers
  one paragraph, splitting remembers where, merging remembers the two. Nothing
  clones the body.
- [x] **C22. docx writer** — through the Preservation Vault.
      *Exit: harness check 2 green across the Word corpus.* — **met: 27 of 27.**

## Phase 4 — Scriva completeness

- [x] **C23. Styles UI, TOC, fields, bookmarks, hyperlinks.**

  Fields are two-pass: `{ PAGE }` cannot be laid out until the pages exist, and
  the pages depend on how wide the number is. So the document is laid out once
  to count pages, the values are evaluated, and it is laid out again.
- [x] **C24. Track changes + comments — editable**, not just preserved.
- [x] **C25. Images, shapes, text boxes** — decoded and drawn, anchored ones
      placed by what the file says they are relative to, and selectable as
      objects: drag to move, corners to resize keeping the shape, Delete to
      remove, one undo entry per drag.

  This is what made the writer splice *inside* a drawing. A paragraph holding a
  picture could not be rewritten at all, so a move would have been shown and
  then thrown away on save. Now the `cx`/`cy` that state the size and the
  `posOffset` values that state the position are overwritten in the drawing's
  own bytes, and everything else in it — effects, crops, the VML fallback, the
  SmartArt — is copied. A drawing nobody touched still comes back byte for byte.

  Stated limits: shapes and text boxes are preserved whole but not editable, and
  text does not wrap around an anchored drawing (C19's limit, unchanged).
- [x] **C26. Plain text + Markdown** read/write; encoding and line-ending
      handling. The encoding and the line ending a file arrived with are the
      ones it is written back with.
- [x] **C27. .doc reader (legacy)** — CFB + MS-DOC piece table, read-only, with
      save-as-`.docx` as the escape hatch.

  What is read: the text through the piece table (which is the whole game — a
  `.doc` is a memory image with a fast-save log on the end, and reading it front
  to back gives several drafts interleaved), the parts told apart by the FIB's
  character counts, paragraphs, tables from the cell and row marks, direct
  character and paragraph formatting from the bin tables of property exceptions,
  and the style *names* from the stylesheet.

  What is not: pictures, drawings, fields, revision marks, and the stylesheet's
  own definitions. Writing a `.doc` is not attempted and will not be: every byte
  offset in the file would have to be rebuilt, and one wrong offset makes a file
  Word opens as something else.

  Corpus: six documents written by Word itself with known content, plus the two
  legacy `.doc` files Office ships (`PROTTPLN.DOC`, `PROTTPLV.DOC`) — the same
  move as the Excel sample workbook, and the same payoff.

  A `.docx` package can now also be authored from nothing (`wp_docx::write::blank`),
  which is what a `.doc` is saved through and what finally lets a new document be
  saved at all.

## Phase 5 — Ship

- [x] **C28. Packaging + install** — `cargo xtask install` to `~/.local/bin`
      (verified: both applications installed and run), `cargo xtask package` for
      a versioned zip, `cargo xtask associate` for the freedesktop `.desktop`
      entries. Config and state in `~/.config/{calx,scriva}/`.

  Associating is a separate command from installing, because installing a binary
  should not rearrange somebody's desktop; on Windows it prints the `assoc` and
  `ftype` commands rather than writing to a user's registry.

  **Not met: the Linux build is unverified.** Only `x86_64-pc-windows-gnu` is
  installed here, and cross-linking a GUI application needs a Linux toolchain
  this machine does not have. The code has Linux branches everywhere it needs
  them — config directory, font search, path comparison — and the only Windows
  paths in the repository are in tests that skip when Office is absent. But that
  is an argument, not a run, and `README.md` says so in those words.
- [x] **C29. Performance pass.**

  The known item was real and worse than recorded. `dependents_of` scanned every
  formula in the workbook, and `order_over` asked it once per formula: 8000
  formulas took 138ms, and the ready-set sort inside Kahn's loop added an
  n² log n of its own. Areas are now registered in a coarse 64-cell grid with the
  area stored beside the node, so a candidate costs a containment test rather
  than a tree descent; areas too broad to bucket (`A:A`) go on a scanned list.
  8000 formulas now take 8ms, 16,000 take 51ms, and four times the formulas costs
  2.3 times the work.

  Two things fell out of measuring. `a_long_chain_sorts_correctly` rebuilt the
  graph inside a doubly-nested loop — 117 seconds of every test run, now 1.2.
  And Scriva laid out twice on every keystroke to arrive at a page number it
  already had; the second pass now runs only when a field value changed.

  New: `cargo xtask perf`, a stopwatch over the corpus and then over sizes the
  corpus does not have.
- [x] **C30. Docs** — `README.md`, `GUIDE.md`, `FORMATS.md`, and `FIDELITY.md`
      generated by the harness rather than written by hand.

  `FORMATS.md` has three columns rather than two, because "preserved" is a
  different question from "read" and it is the one that matters: a feature this
  does not understand is *copied*, not dropped. It ends with the six known gaps
  stated plainly.

## What one resume found

A `.docx` exported from Google Docs, opened in Scriva beside the same file in
Word. It did not look nearly right; it looked *broken*. Five faults, in the
order they were peeled back.

1. **Every table collapsed to one character per line.** Google Docs writes
   `<w:tblW w:w="10397.0"/>` — a decimal where the schema says integer. The
   parser refused it, the table's declared width became zero, and the columns
   were scaled by zero. `parse_i32` now rounds a fraction rather than refusing
   it: every attribute that carries one is a measurement whose fractional twip
   is below the resolution of anything done with it, and *catastrophe* is a poor
   answer to a rounding question. Eighty attributes in that one file.

2. **Three-column tables were read as six columns, half as wide.**
   `<w:tblGridChange>` holds a complete second `<w:tblGrid>` — the grid as it
   stood before the last tracked revision — and the reader descended into it and
   counted its columns as more columns. It now skips any child that is not a
   `gridCol`.

3. **No bullet or number has ever been drawn.** `Content::Label` carried no
   text, and the painter skips every fragment that is not `Content::Text`. The
   layout had always measured the label, reserved its width and indented the
   text correctly — so a bulleted list looked like an indented list and nothing
   pointed at the hole. The variant now carries its own text, because the label
   is not in the document's runs and a renderer handed only the paragraph has
   nothing to draw it from. The tab that follows it now stops at the paragraph's
   own indent, which is where Word puts it.

4. **`{ PAGE }` in a footer showed nothing, twice over.** Google Docs writes the
   field with an empty cached result — begin, instruction, separate, end — so
   nothing was drawn and there was no fragment for the second pass to fill in; a
   zero-width placeholder now carries the mark. And a footer is laid out again
   for every page it appears on, so one field mark answered every page with the
   number of the last: `FieldMark` gained a band, `None` in the body and
   `Some(page)` in a header or footer.

5. **The footer floated two inches above where it belonged.** Its height was the
   sum of every placement, and a footer holding one row of three cells counted
   as three rows. `band` now returns the height of the stack it built.

And one that was a stated limit rather than a bug: **a table row now splits
across a page break.** Word does, this did not, and a resume laid out in tables
lost three inches at the foot of every page and paginated differently from Word
from page one. A row is flowed into bands — a band being a run of lines between
two heights at which *no* cell has a line in progress — and the bands paginate
like anything else. Where no such height exists the row still moves whole, which
is the honest answer: Word would cut each cell at its own line boundary and
leave the two halves of one row at different heights.

Two things fell out of it. Cell vertical alignment was computed and never
applied — `cell_offset` had tests and no caller. And a row's side edges, now
arriving one band at a time, drew a dotted line instead of a ruled one until
each was made to overlap its neighbour by half its own thickness.

**The lesson is about the corpus, not about tables.** Twenty-seven files, every
one produced by Word or Excel, and all twenty-seven passed while a resume that
had been through Google Docs was unreadable. A corpus that is all one producer
tests one producer's dialect.

## Printing and PDF (2026-08-15, after ship)

Both landed in one stroke, because they are one thing: a second and third
renderer over the same laid-out pages the screen paints. The new `wp-print`
crate flattens a page into device-independent draw operations that mirror the
screen painter decision for decision — same baseline arithmetic, same underline
offsets, same border overlaps — and two backends put ink to them.

- **PDF export** (`File ▸ Export as PDF…`): fonts embedded whole as Identity-H
  CID fonts, resolved through the *same* table the screen resolves names
  through, so the file wears the type the user approved. Every character is
  pinned to the layout's advance with `TJ` corrections — a viewer cannot
  re-break a line. A `ToUnicode` map makes the text copyable; JPEGs pass
  through as their own bytes; PNG transparency becomes an `SMask`. The TTF
  reading (`cmap`, `hmtx`, `head`/`hhea`, `OS/2`/`post`) is our own, in the
  house tradition.
- **Printing** (`File ▸ Print…`, Ctrl+P): `PrintDlgW`, then GDI against the
  printer DC. Text is pinned per character with `lpDx` so the printed line
  breaks where the screen's did, whatever GDI thinks the string measures.
  Orientation follows each page through `ResetDCW`. A dialog-free
  `print_to_file` drives a named driver straight to a file — the ignored
  `print_smoke` test spools the corpus through *Microsoft Print to PDF* and
  reads the result back.
- Verified live on the resume: five pages exported, page-for-page with the
  screen and with Word, split-row borders closed at the cuts, `PAGE` fields
  right on every page, rasterised through Windows' own PDF engine to look.

Stated limits: run `w:shd` shading and tab leaders are not painted — the
screen does not paint them either, and print mirrors the screen; Calx still
does not print (a spreadsheet's print model — areas, scaling, repeat rows — is
its own project).

## The half-point dance (2026-08-15, after printing)

A printed page 4 laid over Word's was *very close but not pixel-perfect* —
the tables the right size, the gaps between them a shade off. Chasing that
to zero uncovered how Word actually spaces single lines, measured through
COM probes (synthetic documents, thirty to fifty-five identical lines,
positions read to the twip):

- **Word does not lay lines at the font's design height.** Each line is laid
  at a quantized, hinted *base pitch* (Verdana 10pt: 12.085pt, not the
  12.153pt the hhea table says) while an accumulator tracks the exact ideal;
  whenever the two drift half a point apart, one line is laid half a point
  taller or shorter to pay the debt. Thirty lines of Verdana measure as
  12.085pt pitches with a 12.585pt line every seventh, averaging the design
  height to the third decimal.
- The accumulator **resets at every page top** (identical jump patterns down
  pages one, two, three of an unbroken run), **is shared across font sizes**
  in a flow, and **starts at a quarter point inside a table cell**.
- **A table's horizontal rules occupy their thickness**: content starts below
  the rule above it and every row is taller by it — a 2pt-bordered probe
  shifts its first line down exactly 2pt.
- A document with no `docDefaults` falls back to **Times New Roman**, Word's
  ancient default — not Calibri, which is only ever the default because
  modern files say so.

Implemented as `Shaper::pitch` (base + ideal per face and size; the measured
bases for the resume's faces are a table in the app shaper, anything else
rounds the ideal to a twenty-fourth of a point and lets the accumulator bound
the difference below half a point), a drift accumulator in `Flow`, and a
second flow pass with page-top resets — piggybacking on the same two-pass
structure the PAGE field already needed. The fixed test shaper answers
`base == ideal`, so the dance is a no-op in every arithmetic test.

Verified against Word over COM: resume page 3 line tops within 0.1–0.3pt of
Word's throughout, the half-point payments landing on the *same lines*; page
4 gaps within 0.15pt. Stated limits: a row resumed after a page break sits
about a point higher than Word resumes it (the continuation's headroom is
not modelled); probed pitches cover Verdana 8/10/12/14, Arial 10/10.5 and
Times New Roman 10 — other faces ride the ±0.5pt bound; layout now costs up
to two flow passes when a half-point was ever paid.

## Charts in documents (2026-08-16, after the sample corpus)

The three downloaded samples all draw a clustered bar chart on page one, and
Scriva drew a correctly-sized hole where it should be. The inline drawing is
not a picture at all — it is `<c:chart r:id="rId3">` — and the reader only
ever looked for a picture's `r:embed`.

Charts now have a crate of their own. A workbook and a document carry the
*same* `<c:chartSpace>` part, so reading it twice would be reading it twice
differently: the model and the reader moved out of `ss-xlsx` into `chart`,
with `Plot` (what is plotted — the half both formats share) split from
`Chart` (that, plus the cells a sheet anchors it to). `ss-model` re-exports
it, so a sheet's chart is still `ss_model::chart::Chart` where it has always
been.

Where the ink goes is `chart::draw`, in plain numbers: rectangles, polylines,
polygons and strings positioned by their top-left corner. Three renderers
consume it — `ui_kit::chart` turns a primitive into an egui shape, the PDF
writer into a content stream, GDI into a brush and a `Polygon` — because
three renderers each doing their own arithmetic is three pictures. The one
question the geometry cannot answer itself is how wide a string is, so it
asks: `Measure` on the screen is an egui galley, on paper the *page's own
shaper*, which is what makes a printed label sit where the screen's did.

- `Op::Chart` carries the box and the relationship through the flattening;
  `ops::draw_charts` expands it into ink. A backend that never calls it
  leaves the box empty, which is what both of them did before.
- `Op::Poly` arrived with charts and is the only op they needed: a pie slice
  is a fan of quads and a marker is a twelve-sided ring, so no backend grew a
  circle or an arc.
- Translucency is *blended*, not carried — an area chart's 45% band is mixed
  against the chart's own background here rather than becoming a PDF graphics
  state and a GDI alpha blend that would disagree.
- Verified in the running application and, through the export, rasterised out
  of the PDF: three series, four categories, legend, gridlines, axis labels,
  where Word puts them.

Stated limits: a chart's own text is set in one sans face rather than in the
faces the chart part names; no gradients, no 3-D, no trendlines, no
per-point formatting — and none of that matters to the file, because both
applications put back the bytes they opened.

## What the visual pass demanded (2026-08-20, after the recreations)

Rendering the by-hand recreations and their originals through real Word side
by side (adr/0002's step 4, now stated properly) showed the text diffs had
been grading the wrong thing: identical words in the wrong face, the wrong
colour, no bullets, and a title living in a header part no `document.xml`
comparison would ever count. This round closed the gaps that caused it:

- **Scriva Format menu**: a Font submenu of the faces `ui-kit` can actually
  draw (scrolled, because twenty-seven entries lost their tail below a
  laptop's screen edge until they did), Word's standard text-colour palette
  with an any-hex dialog behind Other…, and the marker-pen highlight
  gallery.
- **Scriva Table menu**: borders all-or-none (an explicit `none`, because an
  absent border is an inherited one), cell shading from the same palette,
  and a column-width box that keeps the grid and the cells agreeing.
- **Lists**: Bullets and Numbering on the Paragraph menu, backed by Word's
  own gallery definitions, with `numbering.xml` authored whole for a fresh
  document and *appended to* — never regenerated — for a file that already
  has one, so the vocabulary the model does not keep survives.
- **Headers and footers**: an Insert dialog editing the default header's
  text, undo restoring bodies and references together, and the writer
  assigning part names and relationship ids at save — which is why
  `wp_docx::save` now takes the document by `&mut`.
- **Calx**: Format ▸ Fill Colour on the menu bar (the toolbar palette,
  column widths, row heights and drag-resize already existed), and writer
  tests pinning fills, widths and heights through the untouched-stays-
  byte-identical bargain.

The smoke test that gates it: drive the rebuilt binary through every new
menu by mouse and keyboard, save, and open the result in real Word. Word
refusing the first attempt is what found the unbound `r:id` (see
LEARNINGS.md) — the suite, reading with a namespace-blind parser, had
passed it without comment. Still open, and stated rather than implied:
inserting charts, and editing a header in place rather than through a box.

## The recreations, recreated (2026-08-20, the same evening)

Both Word documents were then redone through the running app with the new
features — this time in about forty minutes rather than an afternoon,
because the bulk text went in through Ctrl+V (paste walks the same
`text::insert` path typing does, so pending formatting carries) while every
menu, dialog and toggle stayed by hand. The resume came out word-for-word
(1,057 of 1,057) with its grids to the twip, its header and footer in real
parts, and its eighteen bullets as real numbering; the sample matched at
881 words on exact A4 with its styles, list and photo. The run also closed
the morning's one unexplained defect: the "bold-italic spray" was the
recreation harness reading `<w:b w:val="0"/>` — an explicit *off* — as on.
The application was never at fault, which is its own lesson about
validating the validator. What a keyboard still cannot reach, stated
plainly: charts, cell merges, and page-number fields in a footer.

## The chart crosses over (2026-08-21)

The one gap the recreations could not close — Scriva has no chart authoring —
is now closed the way the two applications were built to close it: the chart is
made in Calx, where the numbers live, and travels to Scriva on the clipboard as
the `<c:chartSpace>` part itself, under a registered format of our own
(`chart::clipboard`, one statement of the wire format for both sides). Calx's
copy branches to the selected chart and packs the part its own writer already
authored, with the size measured off the sheet in EMUs; Scriva's paste embeds
the bytes as `word/charts/chartN.xml` with relationship and content type
(`media::embed_chart`), authors the inline graphicFrame around it — every
prefix declared on the element, per the namespace lesson — and the render,
resize, move and delete paths needed nothing at all, because a pasted chart is
indistinguishable from one read out of a file. What Scriva deliberately cannot
do is edit the plot: changing the data means going back to Calx, the same
stated-limits rule as no-crop on pictures.

Word, measured rather than assumed, accepts the result: opens clean, counts one
chart, renders every bar from the caches — no embedded workbook required (see
LEARNINGS on `externalData`). The hands-on pass earned its keep again before
the feature was an hour old: with a chart selected, Ctrl+C read "Copied" where
"Chart copied" belonged, because the key press deselected the chart before its
own `Event::Copy` arrived, handing the copy to the cells underneath. A held
command modifier now leaves the selection alone, and a regression test reads
the two events in the order the platform sends them.

## The gallery grows up (2026-08-21, after the crossing)

Advanced charts, scoped to what lives in the classic `<c:chartSpace>` (the
exceljet.net catalogue was the checklist): the painter now honours what it
already read but silently mis-drew, and Calx's Insert ▸ Chart menu grew from
four entries to the families. Four painter debts paid for files that already
exist: `barDir val="bar"` drew columns (now a transposed layout, first
category at the bottom, first series nearest the axis — both confirmed
against Excel's render of our own parts); `percentStacked` drew raw sizes
(now shares, axis pinned 0–100% by tens); stacked line and area drew every
series from the baseline (now each rides the running total, areas as opaque
bands); radar fell through to the bar renderer and scatter threw its X values
away (each has its own renderer now — the scatter one plots real pairs, with
Excel's own KB 211119 rule for when an axis may leave zero out). A scatter's
`<c:xVal>` rides the reader's existing category slot as text and is parsed at
the edges: `Plotted.xs` for painting — Calx resolves it live, so editing an X
cell moves its point — and a `numCache` when writing.

The writer authors `scatterChart` (xVal/yVal pairs, two value axes — the
bottom axis being a *value* axis is what makes it a scatter) and `radarChart`,
and the whole gallery was pushed through the real applications: every variant
inserted through the menus of the running Calx, saved, and read back by
Excel's own object model — six for six on the exact `ChartType` asked for,
after one measured correction (see LEARNINGS: a scatter series must state
`noFill` on its line or Excel reads scatter-with-lines). A scatter copied
from Calx pasted into Scriva, saved, opened clean in Word and rendered
markers-only at the right pairs. The hands-on pass also caught what no test
asked: a doughnut legended its one series' name instead of its slices — a
pie's legend now names its categories, and pie/doughnut inserts get the
legend Excel always gives them. Out of scope, stated: waterfall, histogram,
treemap, sunburst and funnel are *chartex*, a different part format —
preserved verbatim, drawn as placeholders; pie-of-pie draws as plain pie,
bubble as scatter, a combo as its first plot.

Building the demo gallery afterwards (`C:\Adnan\test\chart_gallery.xlsx`, every
variant inserted through the menus, read back by Excel fourteen for fourteen)
found one more: typing a chart title kept only its first letter. The grid
reads keys raw, so the keystroke that went into the title box also reached
the grid, which took it as "deselect the chart" — and the rest of the title
went into a cell. A text box that is not the cell's own editor now owns the
keyboard (`keys_belong_elsewhere`), which fixes the name box the same way.

## The gallery measured against Excel (2026-08-22)

Opened the fourteen-chart gallery in Excel beside Calx and compared every
one, chart by chart, with Excel's own PNG export (`Chart.Export`, which needs a
visible window scrolled to the chart — a hidden instance exports empty files
for charts it has not painted). Most of them were different, some beyond
recognition, and every difference had one of two causes.

The first was the painter drawing what the part did not say: gridlines on
every chart, straight lines without markers, translucent outlined areas set
between the ticks, a pie built from triangles with visible seams, a doughnut
of one ring. The painter now gates gridlines on `<c:majorGridlines>`, bends a
line series Excel would bend and marks its points in Excel's rotation (a
diamond, a square, a triangle, an x, a star…), paints areas solid and edge to
edge, paints a pie's slices without seams and nearly to the plot's edge, draws
every series of a doughnut as a ring of its own at the stated hole, varies
colour and marker per point on a one-series chart that asks to, orders a
stack's legend top down and a flat bar's bottom up, and divides a radar's
scale the way Excel divides a short axis. The type was also a quarter too
small in Calx alone: the painter sizes its type in points and the grid's unit
is a 96-dpi pixel, and the grid had been handing it its own zoom. The same
slip sat in the EMU conversion for anchor offsets, which put a chart pinned
half an inch into a column a quarter inch short.

The second was the writer leaving out what Excel fills in with its own
defaults — the subject of today's LEARNINGS entry. The writer now states
everything the model says, and `excel_defaults` sets on an inserted chart what
Excel's own Insert would have written, so the chart on screen and the chart
in the file are one chart.

Rebuilding the gallery through the GUI found a regression of yesterday's focus
fix: the cell editor let egui walk the focus on Tab, which the grid's new
guard then found sitting on a toolbar button and went deaf, and the Enter
meant to finish the next cell pressed Save — the run left a `Mar.xlsx`
behind. The editor now holds its focus and hands the Tab to the grid
(`tab_commits_and_moves_right_and_the_focus_stays_with_the_grid`).

The one I missed: Excel drew the stacked columns with the two series side
by side, each bar's foot on the other's top, and I passed it. The part said
no `<c:overlap>`, and Excel's default for that is nought even on a stack. The
model carries `overlap` now, the painter lays out every bar group by it — a
stack is a group whose pitch is nought, not a special case — the writer
states it, and `excel_defaults` sets 100 on a stack
(`a_stack_whose_part_states_no_overlap_sets_its_series_side_by_side`).

The next one the user held up was the area chart: Excel's Sales dipped to 6
beneath Costs at Feb, Calx's ran straight across the valley. Not the
geometry — the screen backend filled every `Prim::Poly` with egui's
`convex_polygon`, whose fan from the first corner covers every dip of a
concave shape, and an area chart is concave at each one. The PDF had been
right all along. The chart crate now takes a polygon apart by ear clipping
(`triangles`) and the egui backend draws the mesh with a hairline of the
fill for its edge; the stacked area's Mar peak, which stood at 13 for 15,
came right with it
(`an_area_chart_is_filled_as_its_own_triangles_and_keeps_its_dips`).

"Is this really a doughnut?" It was: two series are two rings in Excel
too. What it lacked was the hairline of background Excel leaves between
the rings, without which Feb's orange inside ran into Feb's orange outside
and the pair read as one ragged shape. The painter draws that seam now.

## The chart inspector (2026-08-22)

"How do I change the properties of the charts?" Until now: in Excel. The
model read every property and the painter drew it, but nothing in Calx
could set one beyond the title. Now a panel stands at the right of the grid
for as long as a chart is selected: its kind (any the Insert menu offers),
title, legend, each series' colour and marker, a bar's gap and overlap, a
doughnut's hole, a line's markers and smoothing, each axis shown or not and
ruled or not, colours varied by point, and Delete.

No dialog and no Apply: every control changes the chart the moment it is
touched, so the chart is the preview, and the undo is one entry per
*gesture* — the plot as it stood when the slider was pressed is held in the
inspector and pushed when the slider is released
(`a_gesture_of_many_frames_is_one_undo_entry_and_undo_puts_the_plot_back`).
A colour picked, a kind chosen, a box ticked: each one entry.

On the file side the invariant held. A chart from Excel is still its own
bytes: `chart_out::restyle` splices each property at the one place the
schema fixes for it — a `val` rewritten where the element exists, the
element authored where a second producer left it out, the legend or the
gridlines dropped whole with their formatting when the model has none — and
everything else, data labels, extension lists, axis text, goes back as it
was (`formatting_a_chart_survives_a_save_and_changes_nothing_else` over the
corpus). A chart given a new *kind* is the exception, and an honest one:
every element moves, so the part is written afresh from the model, as an
inserted chart's is. A chart Calx made gets its colours from the same writer.

Found on the way: the reader took the first `srgbClr` in any `spPr` inside
a series as the series' colour, a data label's or a single point's
included — which is how a pie with a coloured slice read as a pie of one
colour. A label's ink and a point's fill are now neither.

"I don't see pie, doughnut, radar listed." They were there, thirteen rows
down a list that egui had cut at its default popup height and given a
scrollbar nobody notices; and raising the height does not help, because a
popup's first frame is sized against `default_area_size` before its content
is measured. The list was the wrong shape anyway: a chart's family and the
way its series stack are two choices, and the inspector now asks them as
two — Type over seven families, and a Stacking row that appears only for
columns, bars and areas. A stack stays a stack when a column turns into a
bar; any other family is entered the way Excel's Insert enters it.

"When I scroll to the bottom of the sheet I'm not able to fully see the
last chart." The scrollbars' travel was measured against the used cell
range, and a chart that hangs below the data it plots — the usual place for
one — ended past where the sheet would go. A chart and a picture are
content: the extent now reaches the row and column after each drawing's
far corner (`a_chart_below_the_data_can_be_scrolled_into_view`).

"Styling is quite poor, and the same poor styling exists in other places
too." It did, and for one reason: the shared theme draws every control flat
so that the toolbar is not a wall of boxes, and the menus and the toolbar
then set that flatness again for themselves — so the only things the global
flatness reached were the form controls, which it stripped of their boxes.
An unticked checkbox was a word; a drop-down was a word with an arrow.
`dialog::form` now draws a control as a control — a white field with an
edge, darker under the pointer, the accent while open — and every Calx
modal, every Scriva modal, the toolbar's fields and the chart inspector go
through it. Sliders take their rail grey and accent fill in a scope of their
own (`dialog::slider_style`), because egui paints the rail in the checkbox's
fill and the travelled part in the selected row's, and the first attempt
greyed every box and blacked out every selected tab. The inspector got its
groups parted by hairlines, its labels quietened, its axes in a grid, and
Scriva's dialogs the body margin Calx's always had (`dialog::body`).

"In the taskbar, both Scriva and Calx have the same icon." Neither set one,
so both wore eframe's. Nothing in the suite is bundled, so the icon is
rasterised at start-up (`ui_kit::brand`): a rounded tile with a white glyph,
supersampled, handed to the viewport. Calx is the suite's green with a grid
of cells; Scriva is a blue of the same weight with a page — colour first,
glyph second, because a taskbar button is seen at a glance.

"What about this icon?" — the one Explorer and "Open with" show, which is
the executable's own, and the executable had none. The picture moved into a
crate of its own with no dependencies (`brand`), so that each application's
build script can afford to draw it too: it writes the icon as an `.ico` in
every size Windows asks for (an ICO writer of forty lines — a directory, a
bitmap header that states twice the height, BGRA rows bottom-up, an empty
mask), wraps it in a one-line resource script, compiles it with the
`windres` the GNU toolchain carries, and asks cargo to link the object into
the binary. No new dependency; off Windows nothing happens, and without
`windres` the build warns and goes on, because an icon is not worth failing
a build over.

"The text should be simply Calx." Explorer calls an executable by the
`FileDescription` in its version resource, and one without is called by its
file name. The same resource script now carries a `VERSIONINFO` block —
description, product name, the crate's version — so "Open with", the Task
Manager and a file's Properties say Calx and Scriva.

And found at the keyboard, as adr/0002 says such things are: the first
title typed into the panel deselected the chart on Enter. The text box
gives its focus up on Enter *while the panel is drawn*; the grid, drawn
after it, asked whether anything was focused, found nothing, took the Enter
as its own and cleared the selection. The grid now asks before the panel
draws as well as after
(`enter_in_the_title_box_sets_the_title_and_leaves_the_chart_selected`).

## The resume, recreated the third time (2026-08-23)

The resume redone in August was a `.docx` in structure but not in look: its
first page stood taller than the original's, every paragraph loosened by the
8-point-after, 1.08-line default a new Scriva document inherits from Word,
where the resume's own Google-Docs default is nought and single. A page
built on empty spacer paragraphs for its gaps — which this one is — doubles
those gaps when the default adds its own. Selecting the whole document, one
line spacing and nought before and after, brought the page back to the
original's height, word for word and grid for grid, header and footer in real
parts (`RESUME / CV`; `ADNAN KHAN`, then `Page [PAGE] of [NUMPAGES]` after a
centre tab). Verified by exporting both through Word to PDF and setting page
one beside page one: the same lines break at the same words.

Closing it needed the three things a keyboard still could not reach, named as
gaps the evening the recreations were first done:

- **Merge Cells** (Table menu): the cells a selection crosses in one row become
  one, spanning their columns, holding their paragraphs in order, a blank cell
  contributing no blank line — which is how the resume's banner row (name,
  title, an empty third) sits above a single merged cell of prose.
- **Border Colour** (Table menu): every rule of the caret's table in a chosen
  colour, cells' own rules included, and Word's half-point lines drawn where a
  table states none — a colour on no line being a menu choice that did nothing.
  The resume's grids are its `#C0C0C0`.
- **Paragraph…** (Paragraph menu): spacing before and after in points, the four
  indents in inches, a blank field leaving the value to the style and a typed
  one stating it — applied to a selection, only the fields that changed since
  the box opened, so a box opened on one paragraph and applied to many states
  one thing and leaves the rest of each alone.

And the header/footer box grew a voice: a font, a size, a hex colour, an
alignment, and a "Page N of M" tick that lays `PAGE` and `NUMPAGES` fields
after a centre tab at the middle of the text column — Word's own way of
putting a name on the left and a number in the middle of one line.

Driving it taught the harness two things worth keeping. The window's client
came back 982×703 on a fresh launch but 1182×853 once `-Place` was passed a
second time mid-session, so a run that re-placed between stages missed every
dialog it had calibrated; place once, then drive without `-Place`. And a
common Save dialog's "replace?" confirm is a separate top window the
process-matched driver will not touch — the surer path was to remove the file
the recreation was replacing (its own output) so no confirm is raised.

## The resume renders as Word renders it (2026-08-23)

Opening the recreated resume in Word beside Scriva showed the same words on
the same lines but Word's page an eighth of an inch shorter, and the user
named it: the cell padding. Measured, both were true and one caused the
other. Scriva inset every cell's text 5.4pt from the left; Word set it hard
against the cell edge. That is because **Word's familiar 0.08in side padding
lives in the built-in Table Normal style, not in the table** — and this file,
authored by Scriva, has no Table Normal at all, so Word pads its cells with
nothing. Scriva was resolving a bare table's margins to Word's default 108
twips regardless; it now starts from nothing and takes its padding from the
document's *default table style* when there is one, matching Word whether the
style is present (the corpus, where Table Normal always is) or absent (a file
Scriva wrote). `resolve_cell_margins` reads the default table style;
`CellMargins::zero` is the new floor.

The 5.4pt inset was not cosmetic: it narrowed every cell's text column by
10.8pt, which tipped one Accomplishments bullet — "Designed Wi-Tronix's… unit
and functional tests." — from two lines onto three, and that one extra line
pushed everything below it down by a line, the whole of the height the user
saw. With the padding corrected the bullet wraps in two lines as Word wraps
it, and the two renderings now agree the length of the page: exported through
Word to PDF and set line for line against Scriva's own PDF of the same file,
every baseline lands within a point and a half, drifting to that only by the
foot of the page from a hair of line-height rounding.

Verified headlessly along the way by laying the real file out with the screen
shaper and reading the line boxes, then confirmed by the two PDFs — the same
method the chart work used, and the reason the diagnosis landed on one bullet
rather than on "padding, roughly."

## The footer's page number sits where Word sits it (2026-08-23)

The same recreated resume carries a footer of the kind Word's galleries write:
a name on the left, "Page N of M" in the middle of the line, the two parted by
a centre tab at the middle of the text column. Word drew the number centred on
that tab; Scriva drew it *beginning* there, a half-width to the right — because
layout honoured a tab's position but not its kind, advancing the pen to every
stop as though it were a left tab. A centre or right tab is different in that it
positions the text that *follows* it, so it has to look ahead: the fix measures
the run from the tab to the next tab or the line's end and lays it so its middle
(centre) or its right edge (right) falls on the stop. The look-ahead counts the
spaces *between* those pieces — "Page ", the page field, " of ", the count —
which are drawn; only a run's last trailing space hangs and is left out, the
same rule the line's own width already follows. Measured headlessly against
Word's PDF the number's left edge now lands at 283.3pt where Word's is 283.25,
its centre on 306 where Word's is 306.1 — the same page, to a fifth of a point.

## Deferred

- [x] **PDF** — was dropped per Q3; built after ship as `wp-print`. See above.
- [-] **.doc / .xls writing** — never; save-as-modern is the escape hatch.
- [-] **Macros / VBA** — preserved verbatim, never executed.
- [-] **PowerPoint** — out of scope.
