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
- ~~Open: **a large document is laid out in full on every edit.**~~ *Settled —
  see "A keystroke lays one paragraph, not eight thousand" below.* It was the
  architectural change it was called, and it was made: 8000 paragraphs cost
  255ms to open and 26ms to type a letter into.
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

## The resume, recreated the fourth time (2026-08-23)

The third recreation rendered the same in Scriva and in Word, which proved only
that it agreed with itself: set against the *original* it was a different page.
Measuring the two — both laid out by Scriva, so the comparison is of the files
and not of two renderers — named four causes, and three of them were things the
app could not say.

The original's tables are padded 5.75pt either side and not at all above or
below, from a table style based on a Table Normal it carries. Scriva could read
that and never state it, so a table it wrote had no padding: every line began
5.4pt to the left and every cell's text column ran 11.5pt wider, which wrapped
the long Accomplishments bullet in two lines where the original wraps it in
three. **Table ▸ Cell Margins** states it now — four sides in points, blank
leaving a side to the style.

A header Scriva wrote took its paragraph spacing from the *body's* defaults,
because a bare paragraph inherits `docDefaults`. Word's Header and Footer
styles hold the line to single and take the space off both ends, so a one-line
header in a document spaced 8pt after every paragraph pushed the whole page
down by nearly ten. `apply_chrome` writes that spacing itself.

Setting a whole document in one face left every blank line in the face the
document had started in, because a selection formatted runs and never the
paragraph mark — and a blank line has no run, only a mark. Word counts the mark
as selected once the selection reaches the end of the paragraph; `format_runs`
does too now, which is worth 1.3pt on each of the empty lines between blocks.

The fourth recreation was then driven through the menus in eight stages — and
the fourth cause was one of those: the third attempt had set the per-paragraph
spacing *before* a global pass that set every paragraph to none, which wiped it.
Order, not capability. Laid against the original, page one now agrees: every
line breaks where the original breaks it, every x within 0.35pt (the table
indent of -7 twips, which no box states), and y exact for the first third of the
page, 1.3pt through the middle and 3.2pt at the foot. Word's own PDFs of the two
files carry 41 lines each and match line for line.

What is left is deliberate. The original's bullet is a black circle from a font
it embeds and Scriva's is Word's own Symbol dot — worth a third of a point of
height, and Word's bullet is the one a new document should have. Its footer is a
three-column table, which the footer box cannot make, so the page number centres
on the text column rather than in a middle cell, 13pt to the right. And it reads
"Page 1 of 5" against a page 1 of 1, which is what NUMPAGES says about a
recreation of one page.

## Nine pages of a stranger's document (2026-08-23)

`demo.docx` from the calibre project — a deliberate exercise of every feature a
`.docx` has, written by someone else's Word, and the first document in this
repository that nobody here authored. Word lays it out in eight pages. Scriva
laid it out in nine, and no page of the nine was right.

The method was the one adr/0001 sets out and the resume chase refined: render
both to PDF, compare *files* rather than screens, and where a rule was needed
ask Word for it with a probe rather than reading it out of the specification.
Word's own PDF text layer turned out to be untrustworthy for runs in a font it
could not embed — it writes the glyphs of one face and the widths of another —
so the last word on those went to the rendered pixels.

What the document needed, in the order the pages needed it:

- **Embedded fonts.** The package carries Ubuntu, Ubuntu Mono and Tahoma as
  obfuscated `.odttf` parts. Undoing the obfuscation is ECMA-376 §17.8.1 and
  twelve lines; finding out that it mattered took one look at a page laid out in
  Arial. `wp_docx::fonts`.
- **What the machine has outranks what the package carries.** An embedded font
  is Word's fallback for a machine that lacks the face, not an override. This
  machine has Ubuntu Mono installed at 560 units to the em where the package's
  copy measures 500, and preferring the package re-wrapped three paragraphs.
  Telling the two apart needs an index of what is actually installed, by the
  name a document calls it: `ui_kit::catalogue`.
- **The line gap belongs above the baseline.** Word seats a line's baseline at
  the face's ascent *plus* its `hhea` line gap, and every extra point a line
  multiple adds goes below the type, not around it. Measured over Arial,
  Verdana and Georgia at multiples of 1.0, 1.15, 1.5 and 2.0. This one moved
  every baseline in every document.
- **Paragraph and run borders**, with the geometry measured rather than guessed:
  the rule takes `w:space` plus its own thickness out of the page on every side,
  and stands 1.4pt outside the text column — a number that appears nowhere in
  the file.
- **Character styles.** `w:rStyle` was read and modelled and never reached the
  layout, so Strong was not bold and Subtle Emphasis was not grey.
- **A stated face beats an inherited theme reference.** `w:ascii="Ubuntu Mono"`
  on a run lost to `w:asciiTheme="minorHAnsi"` on the document defaults, because
  the two layered separately. They are two ways of saying one thing and now
  layer as one.
- **Table styles.** `w:tblStylePr` — the header row, the stripes, the doubled
  rule above a total — was skipped outright. Five tables rendered as bare grids.
  `wp_model::banding` resolves the scheme; the order the parts apply in is
  §17.7.6 and is not the order they are written in.
- **Floating tables**, with the text set beside them rather than below.
- **Tab leaders**, which is what a table of contents is made of.
- **Footnotes and endnotes** — the mark in the text, the note at the foot of the
  page it landed on, the rule above it, and the room pagination has to keep
  clear for all of it.
- **Drop caps**, which turn out to be the floating table again: a paragraph
  standing beside the flow with the next one wrapping round it.
- **Symbol's laid line pitch**, measured at four sizes and added to
  `measured_base`. A list's bullet is drawn in Symbol whatever face the words
  beside it are in, so that one face decides the height of every bulleted line.
- **A table's own `<w:jc>`**, which moves the whole table within the text column
  and was read but never acted on. The nested table is centred and Scriva put it
  against the left margin, sixty-six points from where Word draws it. Measured:
  a table placed by its justification is measured from the column and not from
  its indent, so the hang `w:tblInd` otherwise gives it does not apply.
- **A percentage table width is a preference, not an instruction.** The
  automatic layout a table gets unless it says `fixed` sizes from the grid.
  Measured across this document's five percentage tables: one asks for 70% of
  the column and Word draws it at its grid's 71.6, another asks for 80% and is
  drawn at its grid's 27.9.

Pages 1, 2, 5, 6 and 8 now match Word's own PDF line for line, within about a
tenth of a point. What is left is named in "Where the demonstration document
still differs" below.

## Where the demonstration document still differs

- **A table's columns are the grid's, not a fit to what is in them.** Word
  autofits a table whose width is `auto` or a percentage: it measures the
  content of each column and settles widths of its own, which is why the
  calendar's day names break as `Su`/`n` in Word and `S`/`un` here. Where Word
  wrote the grid out after laying the table — every table in this document but
  the calendar — the grid *is* Word's answer and the columns now agree with it
  to the twip. Content-measured autofit is a body of work of its own.
- ~~**A bulleted line is laid half a point tall.**~~ *Settled — see "A line is
  measured above and below the baseline, separately" below.* It did want the
  treatment the size-in-a-cell rule got, and got it: seventeen mixtures of six
  faces resolved over COM. The height of a line that mixes faces is not the
  maximum of the faces' own heights but the largest ascent-plus-gap on it plus
  the largest descent, and a list's label counts on one side only. Every case
  now lands within four hundredths of a point of Word.
- **Glyph advances drift about a third of a point across a line.** epaint
  measures on a pixel grid where Word measures in design units. It has never
  changed a line ending in this document; it is the whole of the residual on
  pages 1 and 2.

A bullet may be a picture rather than a character. `<w:lvlPicBulletId>` names
one of the numbering part's `<w:numPicBullet>` entries, and the level keeps an
ordinary `<w:lvlText>` beside it as a fallback — so a reader that takes the text
draws a Symbol dot where the author put an icon, which is what the demonstration
document's "This bullet uses an image as the bullet item" was sitting next to.
The picture is now read, fetched through the *numbering* part's relationships
and drawn at the size the shape states. See LEARNINGS.md for why both of those
qualifications matter.

Measured against Word's own printing of it, page by page: the share of Word's
lines that Scriva puts within a point of where Word does was 41% when the
work on pages 3, 4 and 7 began and is 82% now — pages 1, 5 and 6 exactly,
page 7 at 95%, pages 3 and 4 at 81% and 85%, and page 8's 46% is mostly its
list pitch and a bullet that this reader writes into the PDF's text layer as
the private-use character the file actually holds rather than as Word's
translated one. Page 2's 79% is the mono-face paragraph whose *text layer*
Word itself misreports — the picture matches.

Seven rules of Word's came out of the document's tables and its two arrows,
and each is written up in LEARNINGS.md: the size its own Normal declines to
carry into a table cell, a table style's `pPr` being the base of its scheme
rather than a paragraph's properties, a merged cell running down the rows it
spans and drawing no rule where those rows meet, the empty paragraph a cell
must end a table with taking no height, the rule between two rows being paid
for once by the row below, and a float at a margin narrowing whatever lands
beside it rather than what it is anchored to. That last one made the layout
run twice, the way a `{ PAGE }` field already did.

The document's links are followed rather than drawn. A `<w:hyperlink>` says
either `r:id`, whose relationship target is a URL, or `w:anchor`, naming a
bookmark; the first hands the URL to the desktop and the second is a caret
move and a scroll to the paragraph the mark starts in. Word's gesture, because
the letters of a link are still text to put a caret in: Ctrl+click, with the
hand cursor while the key is down, a tooltip saying where the link goes and
how to go there, and *Open Hyperlink* on the right-click menu for a reader who
never learns the modifier. Only `http`, `https`, `mailto` and `ftp` are handed
over unasked — asking the shell to open a string means asking the registry
what runs it, and a document is not a trustworthy author. Anything else is
reported to the user instead of run.

The document opened in the window and took it straight down. A face the
package carries is registered and asked for in the same breath, and
`set_fonts` does not land until the next frame begins: for the length of
one frame the family is known to everything except epaint, which answers a
family it has never been given by panicking rather than substituting. The
page is now laid out a frame later, and the shaper asks epaint whether it
really has a name before drawing with it, so no ordering mistake can take
the window down again.


The arrow keys leave the page they are on. A step up or down is a point one
line away and the caret nearest it, and the nearest was measured against every
line on whichever page that point fell on — so on a page whose text stops
halfway down, the point below the last line was nearer to that line than to
anything on the page after it, and Down chose the line it had just come from.
The demonstration document opens on such a page and the caret could not be
walked off it. A line is now weighed only if it is past the one the caret is
on, in the direction of travel, and the whole stack of pages is weighed at
once. Where a column is too narrow for a word, the piece of it on the first
line used to claim the whole word's bytes as well, so an offset at the end of
the word was on two lines at once and the caret bounced between them; each
piece now carries only its own.


A legacy `.doc` table showed no borders and its table of contents ran page
numbers straight into the entry text, because `wp-doc` read a paragraph's
own formatting and a table's cell and row marks but neither a table's
geometry nor a style's own formatting — two gaps the crate's own doc
comments admitted to. Both close from the same handful of sprms: a table
row states its column widths and each column's default border in
`sprmTDefTable`, and a uniform row border in `sprmTTableBorders80`, on the
row-mark paragraph the whole row's cells trail; a style states its own tab
stops the same way a direct paragraph does, just under a different opcode
(`sprmPChgTabsPapx` rather than `sprmPChgTabs`) and reached by walking past
a style's name into the stylesheet's own `UpxPapx`, a record this reader
had never opened before. Two length bugs came with the territory and would
have corrupted every sprm after them, not just these: `sprmTDefTable` is
one of exactly two sprms in the whole format whose length is not "one byte
then that many more" — its `cb` is two bytes, not one, a rule the walker
did not know — and the existing extended-form handling for `sprmPChgTabs`
sized its add-list at six bytes per tab stop instead of three. The TOC's
tab stops turned out to live entirely in its paragraph styles rather than
on the paragraphs themselves, so the border fix alone would not have
reached them.


The same legacy `.doc` showed no header or footer at all, because `wp-doc`
never read the header document — a fourth text range the FIB already
counted (alongside the body, the footnotes and the rest) but that nothing
ever turned into paragraphs. A `Plcfhdd` splits that range into stories:
six fixed footnote/endnote separators, then six per section in a fixed
order (even header, odd header, even footer, odd footer, first-page
header, first-page footer), each ending in a guard paragraph mark that is
not part of its own content. Reading it needed nothing new — the same
`Reader::blocks` that already turns a character range into paragraphs and
tables handles a header's own title-block table exactly as it handles one
in the body — only the range to hand it and the trailing guard mark to
trim. The trap was trusting the stories' presence as the sign of what to
show: this file's leftover first-page and even-page stories still carry
bytes from whenever "different first page" was last turned on, long after
it was turned back off, and Word does not clear them. Showing whichever
story merely has bytes would have put stale letterhead on page one instead
of the real header. What decides which stories are live is a flag, not a
byte count — `sprmSFTitlePage` on the section for the first page, and
`DopBase.fFacingPages` for even-and-odd, the latter a document-wide
setting `wp-doc` had never read at all — so both are read from the file
now and the unused stories simply sit in the model unreferenced, the same
shape an OOXML document takes when it defines a first-page header it does
not currently turn on.

The header that arrived was still not the header Word draws, because half of
it is merged cells and `wp-doc` was reading a table's grid off its first row
and stopping there. A `.doc` states the grid once *per row*, in
`sprmTDefTable`'s `rgdxaCenter`, and the rows of one table do not have to
agree: a row whose last cell covers two of the table's columns simply states
one boundary fewer. That is one of the two ways the format spells a
horizontal merge, and the commoner one — Word 97's other way, the
`fFirstMerged`/`fMerged` flags in each cell's `TC80`, keeps the cells in the
row and asks for them to be drawn as one. So the grid is now the union of
every row's boundaries, worked out once the last row is in, and a cell spans
however many of the union's columns its own two boundaries enclose; a row
that starts part way across states `grid_before` instead of empty cells, the
same shape `<w:tblGrid>` and `<w:gridSpan>` give an OOXML table, so
`wp-layout` needed no changes at all. Vertical merges come from the same
`TC80`: `fVertMerge` with `fVertRestart` begins one, `fVertMerge` alone
continues it, and a continuing cell holds no content — reading it as an
ordinary empty cell is what put a white box under the letterhead. The cell
padding came with them, from `sprmTCellPaddingDefault` when the file states
it and from `sprmTDxaGapHalf` when it only states the old half-gap, which is
also the reason a table flush with the margin says its first boundary is at
-108 twips rather than at zero.

Then the watermark, which is not text and not a picture: Word writes one as
a piece of WordArt in the header, a shape carrying a string, a face, a
colour and an angle, with no bitmap anywhere. It lives in the drawing layer
— OfficeArt, a tree of records in the table stream that a `.doc` shares
between every picture, shape and text effect in the document — and the whole
tree is uniform, every record a version-and-instance word, a type and a
length, which is what makes it safe to look for the four record types that
matter and step over the several dozen that do not. Two things about it are
easy to get wrong. The first is that a property table says how many
properties it holds *in its record header and nowhere else*: the overflow
where the long values live follows the fixed-width entries with nothing
between, so a reader that walks to the end of the record reads the first six
bytes of somebody's string as a property. The second is that the anchor's
rectangle is not the last word on where a shape sits — `posh` and `posv`,
which Word keeps in the *tertiary* property table, override it, and reading
only the rectangle puts a watermark meant for the middle of the page at the
top of it, where the paragraph that anchors it happens to be. Word's own
export of the same document has the glyph outlines centred on (305.5, 396.0)
of a 612 by 792 page, which is the middle to within half a point.

Drawing a shape that *is* its words needed the model to be able to say so,
so a drawing now carries either a picture, a chart, its own words, or a
rectangle's fill and line. The words are measured in `wp-layout` rather than
in each renderer: WordArt has no point size — the glyphs are stretched until
they fill the shape — so somebody has to decide what size that is, and if
the screen decides one thing and the PDF another the two disagree by
whatever their measuring differs by. The header and footer also became a
*layer* rather than a band beside the body: Word draws every shape anchored
in one before the page's own words, which is what stops a watermark from
striking out the page it stamps.

Inline pictures came with the same work, through `sprmCPicLocation` into the
`Data` stream, where a `PICF` header is followed by the shape and usually by
the picture's own bytes. The trap there is that the same property points at
something else entirely when the character's `sprmCFData` is set — a form
field's binary data, not a picture — and this document has ninety-one of
those against eleven real pictures, so reading them all as pictures turns a
document of checkboxes into a document of broken image frames. What a `.doc`
hands over is bytes rather than parts, so they travel beside the document
and are put into the package at the first save, before the document part is
written: a relationship that names no part is not a missing picture to Word,
it is a damaged file. **Metafiles are still not drawn.** A `.doc` written by
Word 97 stores a pasted chart or diagram as a deflated EMF, and this
document's eleven pictures are all of them; their size and their place on
the page are right, and the frame that stands in for them is the honest
answer until there is something here that can play a metafile's records.

The header was right and still looked wrong, because the whole document was
in the wrong face. A `.doc` names no font anywhere: a run says
`sprmCRgFtc0 = 3` and means the fourth entry of `SttbfFfn`, and that table
had never been read, so every word in the file fell back to the reader's own
default. Word's export of page one is Arial throughout; the same page here
was Times New Roman throughout, which is a different document rather than a
plainer one. The names come out of an `FFN` at a fixed offset past a header
this does not otherwise need, and they end at their own terminator — a face
the author asked Word to substitute for keeps its alternate name in the same
entry, and reading to the end of the entry glues the two into a font nobody
has.

The size was wrong for the same reason one level up. A style's own
formatting is a variant record whose members depend on which kind of style
it is, and only the paragraph half of it was being read; the table of
contents therefore kept the document's ten points where the `TOC 1` style
says eight, and thirteen pages of entries did not fit on the page Word fits
them on. Each member is a counted `grpprl` **padded to an even length that
its own count does not include**, so a reader that steps by the count alone
lands a byte early and takes the last byte of one property list as the
length of the next. Reading the character half also meant reading
`istdBase`, which is an index into the same table and points forward as
often as back, so the chain can only be joined once every entry is in — and
it meant that `ico 0` had to stop meaning "nothing stated": automatic is a
colour a run has chosen, and answering silence lets a style's own colour
through where Word shows black.

Then the lines the header had and Word's did not. Two came from reading a
`.doc`'s cell borders as though there were one way to have none. There are
two: a `TC80` side that is all zeroes has not spoken, and the table's own
rule — `sprmTTableBorders80`, which is where an ordinary grid comes from —
runs there; a side that is all *ones* is `Brc80MayBeNil` saying the cell has
spoken and wants no rule at all. Answering `None` to both let the table's
rule run straight through the middle of the letterhead's title block, which
is exactly the line the file had struck out. The rest of the header was half
a gap out of place: a `.doc` states `rgdxaCenter` half a gap to the left of
the edge it rules, which is why a table flush with the margin says -108 and
not zero, and Word puts every boundary back before it draws. `w:tblInd` then
measures to the text *inside* the first cell rather than to its edge, so the
padding is added a second time, to the same edge, for a different reason —
and with both additions every rule and every column boundary of both header
tables now lands within a rounding of Word's own.

Last, the line height. Word's table of contents sets an eight-point entry on
an eight-point line, though the tab between the number and the title carries
the document's default eleven-point face — which is what a file written in
one Word and opened in another leaves tabs in. Measured, in a document built
for the purpose: a twenty-two point tab in the middle of an eight-point
paragraph does not raise the line at all, while a twenty-two point *space*
does. So a tab is now passed over when a line is measured, and a line with
nothing but tabs on it falls back to the paragraph mark, the same as a line
with nothing on it. Page one holds the whole table of contents now, as
Word's does.

**What still differs on that page.** The entries are blue and underlined
where Word draws them black: the runs carry `rStyle="Hyperlink"` and Word
does not apply it there, though it applies the same style to a plain run in
the same document. Word's own `.docx` conversion of this file renders the
same way in Word and the same way here, so this is one rule the layout does
not know and not something the legacy reader invents. The page frame sits
five and a half points left of Word's, because a shape whose anchor is
measured from the column is measured here from the page's text margin rather
than from the column of the paragraph it is anchored in — which for this
frame is a paragraph inside a table cell.

The frame really was five and a half points left, and the reason is that
"the column" a shape measures from is not always the page's text column. A
`.doc` states a floating shape's rectangle against one of three origins —
the page's margin, the page's edge, or the text — and this document's page
frame states the third with an offset of minus a hundred and eight twips.
Read from the margin that puts it at thirty and a half points; Word puts it
at thirty-six, because the paragraph the frame is anchored to is inside the
letterhead table's first cell and Word measures from *that* cell's text,
which begins a hundred and eight twips in. So `anchor_position` and
`anchor_base` now take the place the anchoring paragraph begins — the left of
its column and the top of its first line — instead of the line alone, and
every caller already had it to hand in the placement pagination gave them.
The frame is now the same rectangle Word draws, to the point, and it lines
up with the tables it surrounds again.

Then the thing that had been quietly wrong in every line of the document.
Word's table of contents lays an eight-point Arial entry on an 8.94pt line
where this laid it on a 9.20pt one, and thirty lines of that is a quarter of
an inch and eventually a page. It is not the style and it is not the tab: it
is `fNoLeading`, one of the compatibility options a document converted from
an older word processor carries, which tells Word to drop the gap a face
asks for between one line's descender and the next line's ascender. Arial
asks for sixty-seven units of its two thousand and forty-eight, and eight
points of that is the quarter point a line that was missing. The bit was
found by saving one document twice out of Word with the option on and off
and diffing the two `Dop`s — it is the fourth flag of the ones Word 97 added
to `Copts80`, which begins at offset eighty-four — and confirmed against a
document laid out both ways, in a table and out of one, because a table row
follows the same rule. It is read from `settings.xml` for a `.docx` too, and
carried through layout as a document-wide setting beside the default tab
stop. Every table of contents entry on page one now sits within a point and
a half of where Word sets it, against thirteen and a half at the foot of the
page before.

**What still differs on that page.** The entries are still blue and
underlined; that is unchanged and untouched. The header table's rows are one
to two and a half points shorter than Word's — Word gives a row a little
more than the type on it needs, and dropping the leading took away what had
been masking it. Word draws a table's outermost border wholly inside the box
rather than centred on its edge, which is another three quarters of a point
at the top. And the watermark is set in the right face at the wrong shape:
WordArt stretches its words to fill the rectangle it was drawn in, so Word's
letters are as tall as the box and these are as tall as the width alone
makes them. None of the three is a legacy-format question — a `.docx` says
all three the same way and would be laid out the same.

The blue underlined table of contents was Word's own doing and not ours. A
contents built with `\h` is a column of hyperlinks and Word writes the
`Hyperlink` character style onto every run of every entry — then draws them
in the contents style's plain black. Four measurements pinned the rule down,
because three plausible ones are wrong. Giving that style a loud colour and
size changes every other run in the document wearing it and changes nothing
in the contents, so Word is not applying it there. Swapping those same runs
to `Emphasis` italicises them at once, so it is not that character styles go
unread inside a field result. A link to a bookmark in the body of the same
document is drawn in the link colour like any other, so it is not that Word
declines to decorate an internal link. And a run given the `Hyperlink` style
inside an ordinary `HYPERLINK` field result *is* drawn in it, so it is not
hyperlink fields either. What is left is the one thing all four share: a run
inside a **table of contents field's result** does not take that one style,
and the layout now knows which paragraphs those are.

Knowing that took a `.doc` reader change of its own. A field's instruction —
` TOC \o "1-3" \h ` — was being read as the document's words and merely
skipped at drawing time, which left nothing anywhere able to say what kind of
field anything was. It is read as code now, which is what the model has
always called it and what `wp-docx` has always produced. Two consequences
beyond the contents: a contents field opens in one paragraph and closes forty
later, so which paragraphs lie inside one is walked once over the document
and carried on the layout context beside the note marks; and `{ PAGE }` and
`{ NUMPAGES }` in a legacy document are recomputed now instead of showing
whatever Word last cached, so the header of page two says two.

Last, the watermark, which was the right face at the wrong proportion.
WordArt is not type at a size: the words are stretched until they fill the
shape they were drawn in, across *and* down, and the two need not agree.
Word's watermark here is Courier New filling 609 points of width in a box 152
points tall, so its letters stand a little over half again as tall as that
width alone would make them — measured off Word's own glyph outlines, whose
ink box comes to 475.8 points across the diagonal against the 475.7 that
filling both directions predicts. A shape's words now carry a stretch beside
their size, and each renderer applies it to the glyphs: the PDF in the second
column of its text matrix, so the stretch happens in the glyphs' own space
before the turn; GDI by naming the average character width the unstretched
face settled on, which is the only way that interface has of saying "taller
than wide"; and the screen by tessellating the galley once, unturned, and
stretching and turning its vertices, because epaint will turn a galley but
not squash one.

**What still differs on that page.** The header table's rows are one to two
and a half points shorter than Word's, and Word draws a table's outermost
border wholly inside the box rather than centred on its edge. The header says
"1 of 18" where Word says "1 of 16", which is the two blank pages the
undrawable metafile figures make and not a fault in the field.

### The rules a table draws, the rules a row is spaced by, and the lists a `.doc` numbers from

**A rule between two cells is one line.** Every internal rule of a table was
being drawn twice — once by the cell above it and once by the cell below, once
by the cell to its left and once by the cell to its right. On paper the second
stroke lands exactly on the first and nothing shows; on a screen it darkens the
first one's anti-aliased edges, and a 0.75pt hairline between columns laid down
half again the ink it should until it read as heavy as the frame around the
table. Page one of the demonstration document drew 68 strokes it did not need.
The rule now belongs to one side of itself — the row above has already gone by
when the row below is laid out, so the horizontal rule is drawn by the row
below as the heavier of the two edges that meet, and the vertical by the cell
to the left, which is still at hand.

**A row is spaced by the rule above it even where nothing draws that rule.**
Word charges a row for the heaviest border on its top edge across every cell of
it, and a cell whose rule is hidden because a vertical merge runs through it is
still one of them. The letterhead of the demonstration document rules its cells
at a point and a half and its table between them at a half, and its first
column is merged down all four rows: every row under that merge sat a point too
high, because the point-and-a-half was dropped along with the line nobody
draws. Counting it puts Word's own content tops within measurement error —
46.80 against 46.86, 37.44 against 37.45 — and the four rows of that table now
pitch 9.30, 14.91, 9.39 against Word's 9.39, 14.88, 9.36.

**The four keep flags were rotated.** `sprmPFKeep` keeps a paragraph's *lines*
together and `sprmPFKeepFollow` keeps it with the next one, which is the
opposite of what the two names suggest; they sit next to each other, and widow
control is nowhere near them. Read one place out, "keep these lines together"
was set wherever Word had said "keep this with the next". The cost was a blank
page: a heading that opens with a hand-written page break lays out as two lines
— the empty one the break ends and the heading itself — and the keep rules,
believing those two lines must not part, dragged the empty one onto the new
page and left the heading for the page after that. Word says KeepWithNext true
and KeepTogether false for those headings; measured through its own object
model rather than argued from the names.

**And a hand-written page break divides a paragraph.** Two things follow from
the author having put the break there. The lines before it and the lines after
it are on different pages whatever the keep rules ask for, so they are separate
groups now and nothing reaches across. And a break with *nothing* in front of
it takes the whole paragraph with it, the space above included — Word starts
such a paragraph a clear twelve points below the header of the page it breaks
to, which is the heading's own space before, and leaving an empty line behind
spends that space where it does no good and puts every line of the new page
twelve points high. The number is not in the document's runs either; it is made
during layout, and a leading break now goes in front of it so that the
paragraph arrives on its new page numbered.

**Lists.** `wp-doc` reads `PlfLst` and `PlfLfo` now, so a legacy document's
numbering is the same model a `.docx` builds and nothing downstream knows which
kind of file it came from. The definitions and their levels, the instances that
stand between a definition and a paragraph, the level overrides, the number
text with its placeholders rewritten from the zero-based levels a `.doc` stores
to the `%1`-through-`%9` the model reads, and `sprmPIlfo`/`sprmPIlvl` on both a
paragraph and a style. The headings of the demonstration document are numbered
by their styles — Heading 1, 2 and 3 are levels 0, 1 and 2 of one list — and
they now read "1. Scope", "4. Evolution Radio Hardware", "4.1. Evolution Two
Chip Solution" where before they read as their bare text, hanging into the
margin where the number should have been.

**Where page two stands.** Sixteen pages against Word's sixteen. Every line of
page two agrees with Word's within four hundredths of a point across and
six tenths down, headings and numbers included.

**What still differs.** Word still draws a table's outermost border wholly
inside the box rather than centred on its edge, which is half a line width at
each outer edge.

## Pages three and four: the diagrams

**A pasted diagram is a recording, not a picture.** Every figure in the
demonstration document is a deflated EMF in the drawing layer — the GDI calls
the drawing program made, kept verbatim — and until now they were counted and
dropped, so eleven diagrams over sixteen pages were reserved space and white
paper. The new `metafile` crate plays one: it keeps the state a device context
keeps and turns the drawing calls into filled outlines, stroked runs and words
on a baseline, which is exactly the ink `wp_print::ops::Op` already carries.
Nothing downstream had to learn what a metafile is — `draw_metafiles` expands a
picture box the way `draw_charts` expands a chart's, and the screen painter
does the same translation so the page on glass and the page on paper cannot
disagree. Thirty-two record types, which is all eleven diagrams between them
use: pens, brushes, fonts, the world transform, paths and their fills and
strokes, the sixteen-bit point forms, and `ExtTextOutW` with the per-character
advances the drawing recorded, so a label is set exactly as wide as it was
drawn rather than as this machine's copy of Arial would set it.

Reading them at all meant two other things. A metafile blip is compressed
behind an `OfficeArtMetafileHeader` and has to be inflated; and an *inline*
picture's bytes are wrapped in an `OfficeArtFBSE` just as the shared store's
are, which `picture.rs` was walking past — so the pictures a `.doc` keeps with
its text were never found even when their format was one that could be read. A
metafile now travels into the saved `.docx` too, under its own content type,
so the copy carries the diagrams the original had.

**Two spacing faults fell out of measuring against the figures.** A paragraph
mark is a real character with a real size, and `wp-doc` was not reading its
character properties at all: this document separates every heading from its
text with an empty five-point paragraph, and spacing those at the body's ten
points cost half a line at each of them. And a page break typed at the head of
a paragraph left an empty line in front of everything, which the line-filling
code then counted as the paragraph's first — so a numbered heading that starts
a page lost its hanging indent and stood eighteen points to the right of where
Word puts it.

**Where pages three and four stand.** The diagrams draw, and their words land
within nine tenths of a point of Word's down the page and six tenths across —
placement, type size, line weights and fill colours from the recording itself.
Page four's heading and body agree with Word to a fifth of a point.

**What still differs there.** An inline picture's line is a descent taller here
than in Word, so each figure pushes what follows it about two and a quarter
points down the page; the difference accumulates over a page with two figures
on it and is the largest gap left. Word's own measurement of a `.docx` says the
line *is* the picture plus the run's descent, and this document says it is the
picture alone, so the rule that reconciles them has not been found yet and the
measured one stands.

### A click on the body is a click on the body

**The document could not be typed in.** Every click anywhere on a page put the
handles round the page frame instead of a caret in the text, and with a picture
picked there is nothing for a keystroke to do. The frame is a rectangle five
hundred and forty points by seven hundred and twenty anchored in the header,
and the watermark is another shape beside it; both are painted on every page
and the pick walked everything painted, so the frame — painted last — took
every click on the paper.

Word does not arbitrate between the two, it separates them. A watermark and a
page frame belong to the header layer, and the body cannot select one at all:
opening the header is the way to reach them, which is also how one is changed
or removed. So the pick now walks the page's own content and nothing else.

That is the honest answer as well as the matching one, because a header's
shapes cannot be named here. A selection says which paragraph and which drawing
of it, counted through the body's walk, and a header is flowed on its own with
its count starting again — so the frame of this four-hundred-and-forty
paragraph document called itself a drawing of paragraph zero, and a drag on its
corner would have resized whatever picture that paragraph really holds.

**And a shape put behind the words gives up a click that lands on one.** Word's
second rule, on the same page and inside the body this time: a graphic under
the text is reached with the Select Objects tool, not with an ordinary click,
because the letters drawn over it are still text to edit. The shape is still
picked everywhere it is not covered, so nothing becomes unreachable by being
large.

**What is not here.** There is no header layer to open, so a `.docx`
watermark can be seen but not changed. For a `.doc` that costs nothing — the
reader is read-only and its shapes were never written into the copy anyway.


### The watermark, seen and changed

**A `.docx` watermark was not drawn at all.** Word writes one as VML — a
`<v:shape>` of the WordArt type inside a `<w:pict>` in the header — and the
reader kept those bytes perfectly and laid out nothing, so a watermarked
document opened looking like a document with no watermark. Only the legacy
`.doc` reader built a shape-of-words the renderer could draw. Nothing in the
corpus exercised `<w:pict>`, which is why it went unnoticed; there is a
`watermark.docx` in it now, made by the same Word that makes the rest.

`crates/wp-docx/src/pict.rs` reads the shape into the same `ShapeText` the
`.doc` path already produced: the string, the face, the fill, the size, the
turn and where on the page it sits. Everything else about the element stays in
the bytes, so an untouched watermark is written back byte for byte — the
fidelity harness now proves that over a real Word watermark, on the untouched
save and on the save-after-edit both. A `<w:pict>` that is a *picture*
watermark stays opaque and undrawn: the washout it needs is not modelled, and
stamping a photograph over the text at full strength would be further from the
truth than leaving the space empty.

**Insert ▸ Watermark…** states the word, the face, the colour and whether it
lies diagonally, and offers to remove the one that is there. Word keeps this on
a Design tab; there is no such menu here, and the command belongs beside the
two that also put things in the header.

**The size is derived rather than asked for**, because Word's own box does not
ask either. Turned through forty-five degrees a shape's bounding box is
`(w + h) / √2` each way, so the width that just fits the text area is
`side · √2 / (1 + 1/aspect)`, and the aspect is the string's own, measured. On
US Letter with "CONFIDENTIAL" that gives 529.5 points against the 527.75 Word
itself wrote.

**What Word says about the result.** A document with a watermark this
application authored opens in Word without repair, with one shape in the
header, named `PowerPlusWaterMarkObject1`, of the text-effect type, turned 315
degrees, behind the text — which is what its own Remove Watermark looks for.
Rendered to PDF by Word and by `wp-print` side by side, the two agree on where
the ink starts to a tenth of a point.

**Two things only the real Word could have told us.** A `<w:pict>` whose
namespace prefixes are undeclared is not drawn badly, it is refused outright;
and a shape type without its `<o:lock shapetype="t"/>` is counted as a second
shape, so a watermarked page carried two objects and Remove Watermark would
have left one behind. Both were found by opening the file through COM and
asking what was in it, and neither would have shown up in any test written
against our own reader.

**What still differs.** The letters are about four and a half per cent smaller
than Word's for the same box. Word's WordArt fits the glyph *outline* to the
shape; `shape_words` scales by the em box, which is a little smaller. The
placement is exact — both renderings start their ink at the same point — so
this is the fitting rule and not the geometry, and it is the same rule the
`.doc` watermark has always been drawn with.

**A side effect worth stating.** A `.doc` opened as a copy now carries its
watermark into the saved `.docx`, because the shape it was read into finally
has a writer. The page frame it draws with a plain rectangle still does not:
there is nothing to write it as. The dialog that says so has been corrected.


### The header is edited where it is drawn

**A box that takes a header's text and gives text back is not an editor.** The
one this replaces asked for the words, a face, a size and an alignment, and
wrote a header of plain paragraphs from them. The header of the document that
prompted it holds a table — ECN#, DATE, BY, REVISION, ISSUE — and applying that
box to it would have written one line of text where the table used to be. It
was also on the wrong menu: Insert is for putting something in, and a header
that is already there is being *changed*.

**The reason it was a box was structural, not a choice.** The editor could only
edit one flow. A caret said which paragraph, counted through
`Document::paragraphs` — the body's own walk — and a header is flowed on its
own with its count starting again at zero, so there was no way to name a
position inside one. Nothing in the caret, the hit test or the undo stack could
say *which* body it meant.

So `wp_model::Scope` names one: the body, or one header or footer by the
identity its section references. Every editing operation takes one; the undo
stack records it with each change, because taking back a header's edit against
the body would restore the header's paragraph over whatever paragraph of the
text happened to share its number. A `Page` now says which body each of its two
bands was laid out from, so a line in the band names a paragraph of *that*
header — and which header it is depends on the page, since a section can name a
different one for its first page and its even ones.

**What that buys is that a header is edited by the editor.** Double-click the
top margin and the caret moves into the band; the text washes out; a dashed
rule and a tab say which band is open. Type, select, bold, tab to the centre,
insert a table, drag the watermark, Ctrl+Z. A header holding a table is still a
header holding a table afterwards, because nothing was re-authored from a
description of it. Escape, the bar's own Close, or a double-click on the page
puts the caret back in the text exactly where it left.

**A click on the part of the page that is not being edited does nothing**, which
is what the wash over it promises. Word's own rule, and the alternative — a
click dragging the caret out from under the keyboard while the header still
looks open — is worse than an unresponsive one.

**Where the three commands live now.** Scriva has a menu bar and no ribbon, so
they went where Word's own menu bar had them rather than where its ribbon does:
**View ▸ Header and Footer** opens the band, **Insert ▸ Header / Footer** makes
one and drops the caret in it, **Insert ▸ Page Number** puts a PAGE field where
the caret is, and **Format ▸ Watermark…** is Word 2003's Format ▸ Background ▸
Printed Watermark under a shorter name. A watermark keeps its box — it is one
shape with four properties and nobody has ever wanted to place one by hand —
but it is now also a shape that can be selected and dragged once the header it
lives in is open, which is exactly how Word behaves.

**A new band is not a bare paragraph.** Word's Header and Footer styles hold
the line to single, take the space off both ends of it, and put a centre tab at
the middle of the text column and a right tab at its end — which is what makes
Tab walk a name, a title and a page number across the width. A paragraph
without them inherits the *body's* defaults, and a document set with an inch of
space after every paragraph would push its own text down the page to make room
under a one-line header.

**Two things only the running application could have said.** A wash of full
white at two-thirds alpha does not grey a page, it paints over it: the colour
is premultiplied, and the first attempt erased the very text it was meant to
leave showing — the body went blank, table rules and all, and only a screenshot
said so. And a double-click in the *other* margin while one band is open has to
switch to it, not take a word: the first version fell through to the ordinary
double-click and selected a word in the header while the pointer was down in the
footer.

### The rest of the band, and the flows a search walks

**Find looks through the headers and footers now.** It walked
`Document::paragraphs` — the body's own walk — so a spec number typed into a
header could not be found, and Find reported no matches for a word printed on
every page. Every match carries the flow it was found in, all the way to the
highlight the painter draws; Find Next opens the band a match is in the same
way a double-click on it would, and Replace All reaches every story. Word does
the same, and the alternative was a search that quietly lied.

**The two switches that decide how many bands a section has** — "Different
first page" and "Different odd & even pages" — sit on the band bar, which is
where Word keeps them and the only place they are ever wanted. Flicking one
moves the caret into whichever band the page in front of you now asks for, and
turning it back off leaves the band it stopped using in the document: a switch
is not a delete, and flicking it back has to bring the header with it. The
even/odd one is a *document* setting rather than a section's, so it rides in
the same undo entry as the bodies and references it decides the use of.

**How far the bands sit from the paper's edge** is two more fields on the
margins box, under "From edge", and a button on the band bar that opens it.
Word's own Page Setup keeps them on the same sheet as the margins, and they
belong there: they are measured against the same four edges.

**What still speaks only for the text.** The navigation pane, the reviewer, and
`revise`'s accept and reject all walk the body. A comment asked for while a
band was open would have wrapped whatever body paragraph wore the caret's
number — a silent wrong edit, and the only one this change introduced. Those
commands close the band before they act, which is honest and is not yet the
whole answer. *(Settled below: the reviewer speaks in flows now, and Word
turns out to refuse a comment outside the main story itself.)*

**A field is not one piece and cannot be inserted one piece at a time.** Its
start, its instruction, its separator, its cached result and its end carry no
text between them, so every one of them lands at the same offset — and a second
insertion at that offset goes *before* what the first one put there. The first
page number came out written backwards, end first. The whole field arrives as
one splice now.

### The review reaches the header, and Word says where a comment may go

**A tracked change in a header was invisible.** `revise` walked
`Document::paragraphs` — the body's own walk — so the reviewing pane listed
nothing for an edit to a running head, Accept All left it standing, and Next
Change stepped past it. Every change now carries the flow it is in, all the way
to the pane's *Go to*, which opens the band; the pane says "in the header" or
"in the footer" beside a change that is not in the text, because two entries
that read alike and settle different pages are two entries nobody can act on.
Accepting works flow by flow rather than over one run of paragraphs, since a
range recorded against the body's numbering would restore a header's paragraphs
into the text.

**A comment cannot go in a header, and that is Word's own answer.** Asked over
COM to comment on a header's range, Word declines in those words: *"Comments,
endnotes and footnotes can only be added to the main story."* So the command
refuses and says why, which beats both of the alternatives — silently
commenting whatever body paragraph wore the caret's number, and inventing a
capability the format's own producer does not have. A comment a *file* carries
in a header is still found and still removable: the schema allows one, some
other producer may write one, and a reviewer that cannot see it is a reviewer
that lies.

**The table of contents is the one command still body-only**, and honestly so:
it is built from the headings of the text and lands in the text.

### Link to Previous, and the pages a band belongs to

**A section that named no header showed none.** Word's "Link to Previous"
writes exactly that — asked to link, its `<w:sectPr>` comes out holding no
reference at all, and asked to unlink only the primary header it writes one
`<w:headerReference w:type="default">` and leaves the first-page and even-page
bands inherited. So the link is per kind and per band, and silence is the
instruction rather than the absence of one. `wp_model::section::Bands` resolves
each section's three headers and three footers by walking back through the
sections before it, and the layout asks that rather than the section alone. A
two-section document Word wrote showed its running head on page one and nothing
on page two until this was followed.

**Breaking the link copies, it does not empty.** Word, measured: unlink a
second section's header and the words are still on the page while the first
section keeps a copy of its own, so the two can then be changed apart. Linking
again drops this section's reference and leaves its body in the document, so
flicking the switch back costs nobody the words they typed. The switch sits on
the band bar beside the other two, and is not offered at all in the first
section, which has nothing to link to.

**A band belongs to a page, and the caret cannot say which.** The same header
stands on every page of its section, so asking the layout where the caret is
answers with the first of them — and every question a band command asks is
really about the page in front of the user: which section it is in, whether
that section is linked, which of the three kinds of band the page wants. Found
by driving the application: opening the running head of a second section
scrolled the window back to page one to show the very words the pointer was
already on, and the second section's own switch was missing from the bar
because the bar was answering for page one. The page is remembered when the
band is opened.

**Every section's references get their relationship, not just the last one.**
All but one of a document's sections hang off the paragraph that ends them, and
a header made by unlinking one of those is named from there alone — so the
writer wrote the part into the package and left nothing pointing at it.

### A picture in a header, and the washout that makes it a watermark

**A header numbers its relationships from `rId1`, and so does the document.**
Only the numbering part's were ever qualified, so a logo in a letterhead was
fetched as whatever the document's own first relationship pointed at — usually
not an image, so the picture simply did not appear. Every header and footer
part's relationships are now filed under the part that named them, which is
also what makes a picture watermark reachable at all. The writer strips the
qualification again, because the file names a relationship by its bare id and
which part's it is has already been settled by which part the element is in.

**A picture watermark is the picture washed out, not the picture drawn
faintly.** There is no transparency involved: Word turns the brightness up and
the contrast down and bakes the result. Measured against Word itself — a ramp
of every grey from 0 to 255 put through seven settings of brightness and
contrast and exported to PDF each time — the rule is

```text
gain   = 1 + contrast
offset = (1 - gain) / 2 + (bright / 2) * (1 + gain)
```

with `bright` and `contrast` the `<a:lum>` attributes over a hundred thousand.
Word's own washout states `bright="70000" contrast="-70000"`, so black comes
out at 205 and everything above about 170 is white. VML's older `<v:imagedata
gain blacklevel>` says the same thing in different words — handed a shape
stating `gain="19661f" blacklevel="22938f"`, Word reports the picture's
brightness as 0.85 and its contrast as 0.15, which are the very numbers its own
washout sets. Both notations are read; the tone is baked into the decoded
picture, which is what keeps the screen and the printer agreeing. Our washed
samples match Word's own to one part in 255 across the whole ramp.

`corpus/docx/picture-watermark.docx` is the file that holds this, and it is the
only one in the corpus with a picture inside a header at all.

### WordArt is fitted by its ink

**A watermark's letters were two thirds the height Word draws them.** The
fitting rule was the face's ascent plus its descent, which has nothing to do
with the letters actually on the page. Word fits the *outline*: measured over
four strings in a 400 by 200 point shape — "CONFIDENTIAL", "gypsy", "Hg" and
"xxxx", whose proportions could hardly differ more — the drawn ink spans 400 by
200 in every case, to a fifth of a point. An all-capitals string therefore
comes out half again as tall as the em box would make it.

So the shaper learned to answer a new question — the box a string's glyphs
really fill — and `wp_print::ttf` learned to read the first ten bytes of a
`glyf` entry, which is where every glyph states its own bounding box,
composites included. A face whose outlines are CFF rather than `glyf` answers
nothing and the line box stands, which is what the fixed test shaper does too.
The box the renderers centre in the shape is the ink's, and the pen stands a
side bearing to the left of it — so the screen had to stop centring epaint's
galley and start working from the same pen and baseline paper does, or the two
would place the same watermark differently. Every edge of every one of the four
strings now agrees with Word's own rendering to within a fifth of a point,
which is the error in measuring a curve by sampling it.

### A keystroke lays one paragraph, not eight thousand

**A document is laid out on every edit, and an edit changes one paragraph.**
Eight thousand paragraphs cost about a third of a second, and a third of a
second between a key going down and the letter appearing is the difference
between an editor and a form. Two changes, one idea — *a laid line is settled
once and then only carried about*:

**A line is shared rather than copied.** It was copied twice on its way to the
glass, into the item pagination breaks and again into the page that item lands
on, and a measurement of where the time actually went put a third of it there
rather than in the shaping. `Placed::Line` holds the line by reference count
now. Only one thing ever changes a line after it is laid — Word's half-point
accumulator, which lengthens the line that tips its debt — and that one takes a
copy, which is a copy every seventh line of a face that drifts and none at all
of a face that does not.

**A paragraph's lines are kept, and handed back when nothing that could change
them has changed.** The whole of `wp_layout::memo` is in what "nothing" means,
because a cache that is wrong shows the user a document that is not theirs. The
key is everything the inline layout reads *about the paragraph* — the paragraph,
the style layers resolved for it, its list label, its measure, any float beside
it — compared as values rather than hashed, because what a hash collision
produces is a paragraph silently drawn as a different one. The guard is
everything it reads that is *not* about the paragraph — the style table, the
theme, the note marks, four settings — compared once per layout, so that
editing a style empties the cache without any command having to remember to say
so. That was the design's one real decision: a cache invalidated by being told
is a cache that will one day not be told.

Three things are never kept. A paragraph holding a field, because what a field
draws is settled by the page it lands on and that is decided after it is laid.
A header, a footer or a note, because each numbers its paragraphs from zero in
a flow of its own and would answer with the body's. Neither is what a long
document is made of.

A paragraph is looked up by its own index, so inserting one would shift every
paragraph after it out of place; it is looked for two either side, and the
offset that worked is tried first next time. That is what makes pressing Return
cost the same as typing a letter. A change bigger than the window — a paste of
fifty paragraphs — costs one slow layout and is then remembered where it now
stands.

Measured, `cargo xtask perf`: 8000 paragraphs went from 330ms on every keystroke
to 255ms to open and **26ms** to type into. 2000 paragraphs cost 5ms, 500 cost
one. The correctness case is the one that matters and it is asked of the whole
corpus: every document laid three ways — with no memo, with an empty one, and
with the one the previous layout filled — and the pages compared whole, every
placement and every coordinate, then again after a paragraph is typed into,
after one is inserted, and after a style is changed underneath it.

**What it costs, and what still costs the whole document.** The memo holds a
copy of every paragraph it remembers, with the style layers resolved for it —
about the document again, in memory. An entry moves from the last layout's shelf
to this one's as it is matched rather than being copied onto it, so the two
shelves are one document between them and not two.

Assembling the pages — every line placed at an absolute point on the page it
landed on — is what the remaining 26ms is, and it is proportional to the
document rather than to the edit. Pages after an edit that changed no height are
identical to the pages before it and could be kept whole; that is the next step
and it is not this one. A header is laid again for every page that shows it,
which a document with a running head pays two hundred and fifty times over.

### A line is measured above and below the baseline, separately

**A line of two faces can be taller than either face's own line.** The height
was the tallest fragment's whole pitch, which cannot produce such a number —
and Word's answers are full of them. Measured over sixteen mixtures of Arial,
Verdana, Georgia, Calibri, Times New Roman and Courier New at 8, 10, 11 and 16
points, thirty lines each and the pitch read back over COM: the height is the
largest *ascent plus line gap* on the line plus the largest *descent* on it,
and it is symmetric — swapping which face holds the words and which holds the
one odd character gives the same answer to a hundredth of a point. Courier
New's descent is half again Arial's while its ascent is far shorter, so an
Arial line with one Courier letter on it comes out at 13.61 against Arial's own
12.66 and Courier's own 12.47.

The measured single-face pitches keep their calibration: the fragment that owns
the room above the baseline brings its own laid pitch, and the deepest descent
on the line takes the place of that face's own. A line of one face still comes
out at that face's own number.

**A list's label raises a line but does not deepen it.** A bulleted Arial 11
line pitches at 13.39 — Symbol's ascent, because the bullet is drawn in Symbol
whatever the words are in, plus *Arial's* descent. Symbol hangs deeper than
Arial and is not counted for it. The same character typed in as ordinary text
is, and gives 13.47. This is what made every bulleted line in the demonstration
document a tenth of a point too tall.

Seventeen cases, Word against this application: the worst disagreement is four
hundredths of a point, where before it was nine tenths.

**What still differs, and why it was left.** Word draws a table's outermost
border wholly inside its box rather than centred on its edge — measured on
tables Word wrote, a rule's ink lies on the far side of its gridline, never
across it. It was not changed: on the demonstration document, whose tables come
from a `.doc` and carry that format's own half-gap convention, every column
boundary already lands within fifteen hundredths of a point of Word's, and
moving every rule by half its width to satisfy a synthetic file would spoil a
real one. The two oracles disagree at a level below a quarter of a point and
the honest answer is to say so rather than to pick one. Content-measured
autofit — a table's columns fitted to what is in them rather than taken from
its grid — is untouched and remains a body of work of its own.

## A watermark is not stretched, and a page does not space away from its top (2026-08-27)

Three things `table_render_test.doc` — sixteen pages of engineering
specification, a `.doc`, a diagonal `CONFIDENTIAL` on every page — still got
wrong, all found by exporting Word's own PDF of it and measuring the glyph
outlines against ours.

**WordArt has two behaviours and this only had one.** The words of a shape are
either pulled about until their drawn outline fills the box — which is what
Word's WordArt gallery makes, and what was measured into this application — or
set at the size whose *advances* fill the box's width and only scaled down the
page. A watermark is the second kind, and drawing it as the first made its
letters three times as fat as Word draws them. The shape says which in
`fGtextFStretch`, bit 3 of the second byte of `geometryTextBooleanProperties`,
and the two shapes differ in that one bit and nothing else: `0xC0860000` for the
watermark against `0x57000080` for a piece of gallery WordArt saved beside it.
The mask in the high half is not consulted, because the gallery WordArt sets the
bit without marking it used and a reader that believed the mask would draw flat
the one shape that is not.

The geometry of the second kind, measured against Word's own outlines: the pen
starts at the shape's left edge and twelve advances of 50.758 points come to
609.10, the shape's width exactly; one em is the shape's height, so Courier
New's cap at 1170 of 2048 units draws 86.99 points tall in a box of 152.25; and
the baseline stands a descent — 615 units of the same em — above the shape's
foot. Every letter's width and height then matched the face's own bounds to a
hundredth of a point. VML cannot say which kind a shape is — Word writes the
same `<v:textpath>` for both — so a shape read out of a `.docx` is stretched,
which is what Word does with one.

**Word does not space a paragraph away from the top of a page it fell onto.**
Page five of this document begins with a heading set twelve points before, and
Word puts it at the top margin exactly. Pages two and four begin with the same
heading style and Word gives those their twelve points. The difference is in the
paragraphs: the headings on pages two and four *carry their own page break*,
`` as the first character of the paragraph, and the space follows the break
as it would follow anything else. The one on page five simply ran out of room on
page four. So the space is flowed in like any other and taken off again by
whichever page the item turns out to open — which is not known until the flow is
paginated, so it is taken off in two places, by `paginate` on the heights it
breaks on and by the placement that draws them. A paragraph at the very start of
the document, where no page ended at all, keeps its space; Word's own
compatibility list has an option to suppress the typed-break case as well, which
is the plainest evidence that it is not suppressed by default.

**Both sides of a row's cut have to hold a line.** A row's content box is taller
than its lines — the cell's padding stands below the last of them — so the
bottom of the last line was being taken as a place to break, which left a piece
holding nothing but padding. A heading row that very nearly fitted at the foot
of page five had all of its words placed there and one and nine tenths of a
point of padding sent over, where Word moved the whole row to page six.

Page five now holds the same fifty-three lines Word holds, in the same order,
and fifty-one of them are within a point of where Word puts them.

**What still differs on this document.** The watermark's shape is centred on the
middle of the page's text area, which is what Word does to within a constant
8.23 points — measured across five page setups, varying the top margin, the
bottom margin, the page height and the header distance, and the residual never
moved. What that constant is has not been found and is not guessed at here. The
letterhead's `ENGINEERING SPECIFICATIONS` cell sits four and a half points high
against Word on every page, which is a cell's vertical alignment and not this
work. Five pages hold embedded Visio drawings, which Word draws as text and
vectors and this draws as one picture.

## A watermark is drawn at the strength it states, and a line is measured by what it draws (2026-08-27)

Three more differences on `table_render_test.doc`, all of them measured against
Word's own export of the same file.

**A watermark is half transparent, and the shape says so.** OfficeArt's
`fillOpacity` — property `0x0182`, a sixteen-sixteen fraction — is what Word's
own watermark carries its half in, and nothing was reading it. Folded toward the
paper at the point the colour is read, which is what the `.docx` side already
does with VML's `<v:fill opacity>` and is honest for a watermark in particular:
it is behind everything and therefore always over the page rather than over
other ink. Measured against Word's export: the grey that reaches white paper is
223, where the stated `#C0C0C0` on its own puts down 192 — the stamp was twice
as dark as Word draws it on all sixteen pages.

**A field's mark does not say how tall a line is.** Every diagram in this
document is the result of an `EMBED` field, so a field-end mark follows the
picture, and the picture is a little wider than the measure — which puts the
mark on a line of its own. Word lays that line at the *paragraph mark's* five
points; this laid it at the ten points of the run the field is written in,
because the field's end had been given a placeholder of no width and the
placeholder was being measured. A field that begins, separates and ends draws
nothing at those three places, and Word measures a line by what it draws. The
rule that was already there for tabs is the same rule.

**A picture alone on a line has no descent under it.** The room below a line
belongs to the type on it: a picture beside words takes the words' descent —
which is where the rule came from, a 162.15pt picture in a 12pt run on a
164.74pt line — but a picture with nothing beside it takes none. Measured over
all eleven inline figures of this document by asking Word for each paragraph's
own top and the next one's: every one of them came out exactly as tall as its
picture, to the six tenths of a point Word reports a position in.

Together these took eight points off every figure paragraph. Page ten had been
pushing its last two lines onto page eleven; it now holds the same lines Word
holds and every one of them within six tenths of a point, and pages nine, twelve
and fourteen came in from two and a half points to under one.

**What the earlier note listed as still differing.** The five pages of embedded
drawings are no longer drawn as one picture — they are played as text and
vectors and now agree with Word to within six tenths of a point, so that entry
is retired. What is left of it is smaller and quite specific: a dashed pen
inside those metafiles is drawn about three times too thick.

**The watermark's eight points, narrowed.** The shape is centred on the middle
of the page's text area and Word puts it 8.22 points lower, which is where the
last session left it. Two of the three things it could have been are now ruled
out. The band is not it: changing the top margin, the bottom margin, the header
distance and the footer distance each moved Word's watermark by exactly half of
what it moved the band, so Word measures the same band this does, header and
footer overflow included. The geometry inside the box is not it either: with the
shape unturned and pinned to a known rectangle at four different heights, Word's
pen starts at the box's left edge and its baseline stands `615/2048` of the
box's height above the box's foot — the rule already written here, to a
hundredth of a point at every height. What is left is that Word places *this
stored shape* 8.22 points below the middle of that band, constantly, whatever
the page is; a shape centred afresh through Word's own object model lands
exactly on the middle. So it is something Word does when it reads the shape and
not something in the centring, and it is still not guessed at.

**The letterhead cell, and why it is not fixed.** The `ENGINEERING
SPECIFICATIONS` cell is merged down two rows and vertically centred, and Word
centres its line in the whole of what the merge covers: 4.65 points, which is
exactly half of what those two rows have over the line. This aligns it in the
first of the rows instead, which is shorter than the line, so the line stays at
the top. The obvious fix is to align a merged cell in the room the whole span
gives it — and it breaks the cell beside it, which is merged down all four rows,
is also centred, and which Word leaves at the top where centring it in the four
would put it nine and three quarter points down. A four-row merge built for the
purpose, saved as `.docx` and as `.doc` and exported both ways, Word centres.
The two cells are in the same row of the same table with the same alignment and
Word treats them differently, and the condition that tells them apart has not
been found. A rule that fixes one and breaks the other is not an improvement, so
nothing was shipped for this. *(Found, later the same week: the cell Word
leaves at the top is the one the document anchors its page frame and its
watermark in, and Word does not vertically align a cell that holds a floating
shape. See the entry below.)*

## A table is ruled where its row says, and a word is measured without the kerning Word never asked for (2026-08-28)

Two more differences on `table_render_test.doc`, page five, both measured
against Word's own export of the same file.

**A `.doc` states a table's edge, not where its text starts.** Every table in
section six sat five and two fifths points to the right of where Word rules it.
The first of a row's `rgdxaCenter` boundaries is the table's leading edge, and
this was adding half a gap to it before handing it on as `w:tblInd` — which is
measured to the text inside the first cell, so the padding went on twice. Word
rules those tables at the 720 twips their rows state and sets their text a cell
margin further in at 828, and the file carries that 828 itself in the
`sprmTWidthIndent` beside it. The half-gap had never been checked against
anything: the two tables it was written for — the letterhead and the revision
table — are *centred*, and a centred table ignores its indent altogether. So
`sprmTJc` is read now as well, which is what keeps those two where they are;
without it they moved half a gap into the margin the moment the double padding
went.

**Word does not kern, and neither should this.** Word closes up a kerning pair
only where the run asks it to and nothing in an ordinary document asks; epaint
shapes through HarfBuzz, which kerns whatever the face offers. The difference is
a fraction of a point in a line of prose, and twice in this document it was
enough to pull onto a line a word Word puts on the next — section 6.1's first
line ended with `media options, message` where Word ends it with `media
options,`, and the superframe paragraph on page eleven took an `is` the same
way. Both lines were inside two tenths of a point of the measure, and both come
right when the kerning goes: 468.04 against a 468.00 measure, and 445.79 against
445.50. So a run of letters that stand on their own is measured character by
character, which is the same shaping without the kerning; a script whose letters
change shape beside each other is left shaped, because taking those characters
apart would measure forms the reader never sees.

Nothing else in the document moved. Every table page now agrees with Word
horizontally to within half a point — which is the letterhead cell below, still
the largest difference left on any page.

## A list's number is followed by a tab, and this document says a hanging indent is not one (2026-08-28)

The lettered headings of page thirteen — `a) Rx Scan`, `b) Rx Network Packet
state:` — stood eighteen points right of where Word puts them, and so did the
`c)` and `d)` that continue on page fifteen, and the `13.1.`–`13.7.` of page
sixteen, and every bulleted line in the document. All of it is one rule.

**The tab after a list's number was being sent straight to the paragraph's own
left indent**, ahead of any stop that stood before it. That is right, and
measured: a list indented to 1600 twips whose label ends at 1400 puts its text
on the 1600 and not on the default stop at 1440 in between. What it is not is
unconditional. Word has carried a compatibility flag since Word 6 — "don't add
automatic tab stop for hanging indent", `Copts60.fNoTabForInd`, the first bit of
the first byte of the `Copts` a `.doc` keeps at offset 84 of its `Dop`, and
`<w:doNotUseIndentAsNumberingTabStop/>` in a `.docx` — and this document sets
it. With it set the indent is not a stop at all and the tab is an ordinary one:
it goes to the next stop the paragraph states, and failing that to the next of
the document's default interval.

Measured against Word, five places in the document, all of which now agree with
it to a twentieth of a point: the `a)` label ends at 1438 twips and its text
lands on the default stop at 1440, not on the 1620 the paragraph is indented to;
`13.1.` ends at 1344 and lands on the 1530 its own paragraph states, past both
the default 1440 and the 1350 of its indent; `7.1.` and the bulleted lines land
on the stops their paragraphs state at 1080; and the `b)` of page sixteen, whose
list states no stop at all, carries past its 1800 indent to the default 2160.
The flag was found by asking Word for both documents' compatibility options and
diffing the two lists: this one has the first of them and the corpus's own
`.doc` files do not, and the byte at 84 agrees with Word both ways.

The unflagged rule was checked rather than assumed — a hand-edited copy of
`corpus/docx/lists-numbering.docx` at three indents, exported by Word each time
— which is what says the indent really does outrank a nearer stop when nothing
has turned it off. Page thirteen and page fifteen now hold what Word holds, and
page sixteen came from fourteen mismatched lines to eight.

## A cell that holds a floating shape is not aligned, and a merged cell is aligned in all of it (2026-08-28)

`ENGINEERING SPECIFICATIONS` sat four and two thirds of a point high in the
letterhead of every one of the sixteen pages — the last difference the earlier
notes left standing, and the one whose obvious fix had been tried twice and
reverted because it broke the cell beside it. Two rules, both measured, and the
second is the reason the first kept failing.

**A merged cell is aligned in the whole of what it covers.** `ENGINEERING
SPECIFICATIONS` covers the first two rows of the letterhead and is centred;
this centred it in the first row alone, which is shorter than the line, so it
clamped to the top. The rows below a merge have not been laid out when the row
that starts it is, so the shift now waits: the lines go down where they fall,
the room is accumulated row by row (the rule above each of them counts — a merge
draws no line between its own rows and the room the line would have taken is the
cell's), and the last row of the span moves them. Measured against Word: the
merge is 22.71 points of room around a 13.40 point line, and Word puts it 4.68
points down.

**A cell that holds a floating shape is not vertically aligned at all.** This is
what made the first rule look wrong: `CHAMBERLAIN GROUP` is centred too, covers
all four rows, and Word leaves it at the top, where centring it in the span puts
it nine points lower. The cell is the one the document anchors its page frame
and its watermark in. Take those two shapes out of that one cell — in a `.docx`
Word itself converted the file to, so nothing else changed — and Word centres it,
moving `CHAMBERLAIN GROUP` from 50.16 to 59.04. Every synthetic four-row merge
built for the earlier attempt was centred by Word because none of them held a
shape.

The setting named for this, `<w:doNotVertAlignCellWithSp/>`, turns out not to
govern it: taken out of the same converted copy, and with the document put in
Word 2013's compatibility mode besides, Word still leaves the cell alone. So the
rule is applied unconditionally, which is what was measured, rather than gated on
a flag that does nothing.

Every header line in the document now lands within nine hundredths of a point of
Word's, and the worst vertical difference on any page is down from the 4.65 that
stood on all sixteen to under one and a third — which is the metafile text of
the diagrams, not the text of the document.

While the converted copy was open: the `.docx` reader now takes `<w:noTabHangInd/>`
for the same rule as `<w:doNotUseIndentAsNumberingTabStop/>`, since a `.doc` of
this age states the first as a bit and a conversion of one arrives carrying both.

## The retrospective, and the tool it names (2026-08-28)

Sixteen pages now match Word closely enough that the remaining differences are
inside the diagrams rather than in the document, so the question turned to how
it was done. Four days, four sittings, one reported difference each, roughly
three quarters of an hour a turn — and every one of those turns ran the same
comparison by hand, with the probe scripts rewritten from scratch and thrown
away each time.

The finding is recorded as **adr/0003**: the comparison itself must be a tool.
`wp-print` already takes the same pages the screen paints, so laying a document
and diffing its glyph origins against Word's own export needs no window, no
release build, no deployment and no screenshot — and none of those were ever
needed to find the faults that were found. What was needed was a ranked list
and a number, and neither existed, which is why one tab rule shipped wrong
twice and why the letterhead's 4.65 points sat deferred through two sessions
for fear of the cell beside it.

- [ ] **`cargo xtask compare <file>`** — lay through `wp-print`, export the same
      file from Word over COM, compare glyph origins by baseline and
      insensitive to whitespace, report the worst residuals per page plus one
      scalar per document, and run it over the whole corpus rather than one
      file at a time. Estimated at a few hours; adr/0003 has the reasoning, the
      grouping caveats already learned, and what a residual total cannot see.

## The comparison becomes a tool (2026-08-28, after the retrospective)

`cargo xtask compare <file>` — the instrument adr/0003 decided on, built. It
lays a document with the application's own shaper and view, asks Word for its
own rendering of the same file, and prints what disagrees: the shifts ranked
worst first, and one number for the document. No window, no release build, no
deployment, no screenshot, and nobody looking at two pages side by side.

The crate is `crates/wp-compare`, a binary rather than part of `xtask`, because
measuring a page with the shaper the screen uses means depending on the
application, on egui and on wgpu — and `xtask` is the one thing that has to
keep working when the rest of the workspace does not build. It shells out, the
way `check` and `dist` already do.

**Word's half goes through paper, and that was not the first plan.** Reading
positions over COM is what every probe here has done since adr/0001, and it is
unusable for a whole document: `Range.Information(5|6)` costs Word a layout
pass per call, measured at about 110ms — 200 words took 22.7 seconds, so the
sixteen-page document is hours. One `ExportAsFixedFormat` is seconds, and the
rendered page is better evidence besides: it gives the *baseline* of every
word, and a baseline is the one horizontal two renderers can be compared on
without either having to guess where the other thinks a line begins. Word's
`Words` collection turned out to be the wrong unit too — it splits punctuation
off and keeps the trailing space — so the words now come from the PDF, split
where a reader would split them.

The answer is cached against the document's length and modification time, so
the render is paid once and every later comparison is a file read. A first run
on the demonstration document is about 45 seconds; the second is a second.

**What it says about the document as it stands.** Sixteen pages both ways,
3,747 words matched, the middle shift dx +0.06 and dy +0.30 — so the two sides
agree about the frame, and what is left is real. 174 words sit more than a
point from where Word put them, the worst 4.77, and they are almost all
horizontal drift *within justified lines* on pages 7, 8 and 11: our
distribution of the slack a justified line spreads between its words is not
Word's. That is a finding this file has never recorded, and it was the first
thing the tool said.

**It was made to agree with a case already settled by hand before any of that
was believed.** Today's harness, run in a throwaway worktree against the layout
of `a1b0ed9` — before the table indent and the kerning were fixed — reports 635
words out of place against HEAD's 174, a worst of 53.63 points against 4.77,
and **58 words on page 5 shifted about 5.4 points** where HEAD has none. The 58
are the table — `0x03`, `Enabled`, `Timer`, `CCP1_CCP2_USE_TIMER1` — at exactly the
half-gap that took an afternoon to find by eye. The kerning shows up as eighteen more words
neither side can place — a line that breaks differently stops being the same
line, which is the truthful way for that to appear. A harness that cannot find
a known answer is not evidence, so this was the acceptance test rather than a
nicety.

**The first version of the matching was wrong, and one document was not enough
to find that out.** Words were aligned to words with a longest common
subsequence, which lies on any page that repeats itself: it will pair one
occurrence with a far-away other at no cost to its own score the moment one
side holds something the other does not. `watermark.docx` is one phrase three
times a line and forty lines down, and Word renders a watermark this does not
gather; the harness reported 298 words hundreds of points out of place on a
page whose real fault is under a point. The demonstration document has enough
unique lines that it never showed there — a green result on the one file it had
been built against. It now matches lines before words, anchors on the lines
whose text occurs exactly once on each side and therefore cannot slide, and
refuses a pairing that sits more than three lines from the page's own median
offset. HEAD's numbers on the demonstration document did not move by a single
word, which is how the fix is known not to have cost resolution. The tell to
remember: a large *median* shift means the matching is wrong rather than the
layout, and it is the first line of every report for that reason.

**The corpus has a number for the first time.** Twenty-five documents, out of
place and unplaceable in separate columns:

```
file                                         out  unplaced     worst  pages
content-controls.docx                          0         0    0.23pt      1
file-sample_1MB.docx                           0         0    0.25pt      2
lists-numbering.docx                           0         2    0.13pt      1
minimal.docx                                   0         0    0.55pt      1
tracked-changes.docx                           0        21    0.00pt      1
hyperlinks-bookmarks.docx                      3         0    2.49pt      1
character-formatting.doc                       4         0    3.47pt      1
comments.docx                                  4         0    1.27pt      1
simple-table.doc                               6         0    5.38pt      1
header-footer-footnote.doc                     8         8    4.42pt      1
styles-headings-toc.docx                       8         0    1.89pt      1
headings-and-list.doc                          9         4    4.64pt      1
unicode-text.doc                               9         0    4.02pt      1
nested-tables.docx                            17         0   11.33pt      1
plain-paragraphs.doc                          29         0    6.82pt      1
watermark.docx                               115       366   67.39pt      1
picture-watermark.docx                       169       140  115.20pt      1
table-spanning-pages.docx                    244        16   25.26pt      3
floating-image-wrap.docx                     244       152   50.23pt      1
headers-footers.docx                         328         0    2.34pt      1
---------------------------------------------------------------------------
                                            1201       761
```

Eight files are already at nothing. The work is concentrated in five, and none
of them had a number before today. `rtl-and-cjk.docx` at a worst of 29.48 with
only two words out is worth a look on its own terms — one bad word rather than
a bad page.

**And the corpus said what causes the justified drift.** On `watermark.docx`
one phrase repeated along a line drifts +1.24, +7.93 and +14.62 points across
its three repetitions — about 0.45 of a point for every space. Our space
advance is wider than Word's, which is very probably the same fault as the
justified-line drift on the demonstration document. Two unrelated documents,
one cause, and the tool found it on both without being asked.


The 1,217 words Word laid and we did not are the floor adr/0003 warned about
and not a regression: they are the text inside the pasted Visio metafiles,
which Scriva draws from a recording rather than from a line, so this gathers
none of them. A tab's leader dots are excluded from both sides — they are a
rule that happens to be made of full stops, and both renderers draw as many as
fit rather than a number either of them chose.

### The instrument is sharpened, and half of what it reported was itself

The harness of the morning was a first cut, and going back over it in one
sitting found more wrong with the *measurement* than with the layout. Every one
of the following was a fault in the tool that was reporting faults.

**A word is not a thing a page has.** Neither renderer writes words down; both
put marks on paper, and a reader is what finds words in them. Word's export
breaks `I/O` into three positioning calls against the one this project's own
playback draws. Two geometric rules for joining marks back into words were
tried, and both were wrong the same way: they *invent* a token, and a token one
side has invented can be paired with nothing. A diagram sets its labels in
whatever order it pleases, so one of them ran `SPI` together with a `Radio`
fifty-three points to its *left* and produced a `SPIRadio` that was on neither
page. The rule that survived is to report every mark where it fell and
reconcile the two tokenisations in the matcher — `glued` in
`crates/wp-compare/src/diff.rs`. Page 3 of the demonstration document, which
carried a dozen of these, now reports nothing at all.

**A word is measured at its left edge, not at its first letter.** For type set
left to right these are the same point. For a right-to-left run they are
opposite ends of the same word, and the tool had been measuring one end of ours
against the other end of Word's: `rtl-and-cjk.docx` reported one Arabic word
**29.48pt** out of place, which is roughly that word's own width. Measuring
both sides at the left edge collapses it to 0.68pt. The document's real worst
is 3.34pt, on the English word after it.

**The fallback walked both sides in lockstep, which looks right and is not.**
Word sets a footnote's reference on its own raised baseline about four points
above the line; this project sets it on the body's. So Word's page has a line
ours has not, every line after it stood one place out of step, each pairing
failed, and both sides advanced together. `footnotes-endnotes.docx` came back
with **nothing matched at all** — forty words called unplaceable on a page
whose every word both sides had laid within a fifth of a point. Each of our
lines now looks a little way ahead among Word's unpaired ones. Forty became
six, and the worst on that page is 0.58pt.

**The silent cap is gone.** A stretch too large to pair used to be paired off
in the order it arrived in, and said nothing about having done so — which reads
exactly like a page that agrees. It now refuses, counts the refusal, and the
report prints it. The limit is stated as the size of the table rather than as a
word count, and is three orders of magnitude above anything in the corpus.

**The threshold that decides what is one line was sitting on a measurement.**
Across every cached reading of Word's own rendering — 2,413 distinct baselines
— the tightest gap between two genuinely separate lines is **3.00pt exactly**,
in `file-sample_500kB.docx`, and the constant was 3.0. That is the one place a
threshold must never be: the two sides fall on opposite sides of it by
rounding and group a page into lines two different ways. It is 2.0 now — a
point clear of the tightest real line, three times the widest within-line
parting (0.60pt, `nested-tables.docx`), and byte-for-byte identical on the
whole corpus, which is the evidence that nothing is balanced on it.

**The floor is gone.** The 1,217 words the morning's report could not place on
the demonstration document were the type inside the pasted Visio diagrams. They
are gathered now, and placed by handing the recording to the paper renderer's
own player rather than by restating how a diagram is scaled into its box — so a
diagram that moves on paper moves here too. Matched words went from 3,738 to
**4,897**, and unplaceable from 1,244 to **161**.

Worth setting down what happened in between, because it is the whole lesson of
the day in one document. As each of the faults above was fixed, the diagrams
first reported a label 56.78pt out of place, then 35.59pt, and finally nothing
at all: the worst on that document is 4.77pt and is the justified-line drift
that was already known. Both of those alarming numbers were the instrument,
found and dissolved within the hour, and either would have been perfectly
convincing as a layout bug to go and chase. **A finding from an instrument
nobody has measured is a hypothesis about the instrument.**

**And one thing was deliberately *un*-gathered, on a measurement.** A shape's
own words — a watermark, a piece of WordArt — are not collected, because Word
draws them into a PDF as **outlines and not as text**. `watermark.docx`,
`picture-watermark.docx` and the demonstration document all export pages whose
only words are the body's. Gathering ours would have put sixteen words on one
side that nothing on the other could ever answer: the leader-dot mistake in the
other direction.

### Four more, after a second look

**The gate had a hole the shape of the one it was built to close.** `--check`
walked what it measured and looked each document up in the record; nothing
walked the record looking for names that no longer turn up. Delete or rename a
corpus file and it silently stopped being checked, while its row sat in
`LAYOUT.md` looking like coverage — the same failure as the silent cap, in the
half of the tool written to prevent it. It now fails in four directions: worse,
unrecorded, unmeasurable, and gone.

**The record holds page counts.** Pagination moving is the largest layout event
there is and it reached the gate only indirectly, as words that stopped
matching. Any change at all, in either direction, now has to be recorded
deliberately.

**And a second, coarser count.** Three numbers that only say *how many* cannot
see work moving about: one word going from three points out to half a point
while another goes the other way leaves every one of them unchanged. `>5pt`
sits beside `out` and separates a word that is slightly out from one that is
somewhere else. It paid for itself on the first run — `headers-footers.docx` is
328 words out of place and **none** of them past five points, which is a
systematic sub-point drift and not 328 broken words, while
`floating-image-wrap.docx` is 244 out and 244 past five. One number had been
saying the same thing about both.

`--check` also now refuses a `--threshold` other than the one the record was
written with, which would otherwise pass or fail for that reason alone.

### The corpus had no CJK in it at all

Checked properly: not one glyph in the CJK, Hangul or fullwidth ranges across
every document — in a corpus containing a file named `rtl-and-cjk.docx`. That
file held one Arabic word and three lines' worth of nothing else.

The generator's intent was six lines in five scripts. `wp-docx/tests/corpus.rs`
had recorded the cause months ago — PowerShell's `+` on two `[char]`s is
addition, not concatenation — and that had since been fixed with `-join`. The
artefact was never regenerated, and regenerating it produced the same one line,
because there was a *second* bug underneath: the loop did `$p =
$d.Paragraphs.Add()` and then `$p.Range.Text = $line`, and a fresh paragraph's
range includes its own paragraph mark, so each assignment wrote the mark away
and merged the paragraph into the next. Six lines in, one line out. One
`$d.Content.Text = ($lines -join "`r")` fixes it.

The document now carries Chinese, Japanese, Hebrew and Arabic, the reader test
asserts the scripts it is named for rather than the accident it contained, and
fidelity still passes 32/32 twice. **Text with no spaces between its words is
the one case where "what is a word" has no easy answer, and it had never once
been measured.** It matched cleanly on the first run — 0 unplaced, which is the
real test of `glued` — and immediately reported two findings: a vertical drift
accumulating about a point per line down a page of mixed scripts (-1.02, -2.00,
-3.83, -5.89, -7.12), and CJK runs about 2.3pt wider than Word's, which pushes
everything after them along. The document measured one word out of place this
morning; it measures thirteen now, and all thirteen are real.

The same paragraph-merging bug is why `lists-numbering.docx` has one bullet and
`styles-headings-toc.docx` has no headings. Those are left alone deliberately —
regenerating them churns the corpus further than this was scoped for — but they
are the same one-line fix when somebody wants the depth.

### A document set apart, rather than given a figure that meant nothing

`tracked-changes.docx` stood in the record at 21 unplaceable words. Every part
of that was true and none of it meant anything: Word renders a document under
revision as though every change had been accepted, Scriva lays out what the
file stores, and the twenty-one words are the difference between those two
positions rather than between two attempts at the same page. A number like that
is worse than no number, because it reads as a score — and the only way to
drive it down would be to make Scriva draw something it should not.

It is now named in `NOT_COMPARED` in the code, with the reason, and written
into `LAYOUT.md` as its own section rather than as a row among the figures. It
is still laid out, still measured, still printed in the table with `not
compared` beside it — and still watched: taking the file out of the corpus
fails `--check` just as a deleted recorded document does. What it is not is
counted. The corpus totals went from 25 documents to 24, and the unplaceable
total from 727 to 706.

The distinction worth keeping: a document that is *hard* belongs in the table
with a large number, because that is where the work is. A document where the
two sides are answering different questions belongs out of it, argued for in
code where somebody reviews the argument. The failure this guards against is
not a wrong number, it is a quiet zero — the way a thing stops being measured
without anybody deciding that it should.

### The gate runs itself, and Word is no longer in the loop

The layout check was a gate only in the sense that somebody might run it. It
could not join `cargo xtask check`, because that has to work on a machine with
no Word — and driving Word over 25 documents is half an hour, not eight
seconds.

The way out was to notice what is actually true of the oracle: **Word's reading
of a document cannot change until the document does.** So the readings are
committed, under `corpus/rendered/` — one TSV per corpus document, 157K for the
lot, our own Word reading our own files, the same provenance as the corpus
itself. The comparison then needs no Word at all. It is pure arithmetic over
two lists of positions, it runs in **3.8 seconds with no PDFs on disk**, and it
now sits inside `cargo xtask check` alongside fmt, clippy and the tests.

Two things had to change to make a committed reading honest:

**The key became the document's contents, not its timestamp.** Length and mtime
were fine for a cache under `target/`; they are worthless for a file in git,
which records content and not when anything was written, so every fresh clone
would have missed. It is an FNV digest of the document and of both probe
scripts now, written into the file's own header — a reading states what it is a
reading *of*, so one that has gone stale says so instead of being quietly
believed. Renaming `topdf.ps1` away, the comparison reports that the reading
"was taken from an older minimal.docx, or with older probe scripts, and
renewing it needs Word" rather than failing as though Office were missing.

**Only `corpus/` is kept this way.** A reading holds every word of the document
it read, so a reading of somebody's real document *is* that document's text.
Those still go to `target/`. It is the `manual_examples/` rule, one step
further along, and it is worth stating because the tempting version of this
change commits everything.

Word is now needed for exactly one thing: renewing the reading of a document
that actually changed, with `--refresh`. Everything else — every check, every
ranked list, every run of the gate — goes on without it.

### The 161 unaccounted words, accounted for

Down to 31, and every one of the 130 was the instrument again. Four causes, in
the order they came out:

**A symbol font's glyph number is not a character — 54 words.** A bulleted list
stores its bullet as Symbol 0xB7, which reaches the document as U+F0B7 in the
private use area: a codepoint meaning nothing but "the 0xB7th glyph of whatever
face this run names". Word's PDF export writes down the character it *drew*,
U+2022. Both put the same ink in the same place to a tenth of a point, and
comparing the names rather than the marks made 27 words on each side that only
one side had laid. One mapping, and only one, because U+F0B7 is the only
private-use character anywhere in the corpus or in the demonstration document —
checked rather than assumed.

**Both sides must cut the page into lines the same way — 65 words.** The
threshold that decides what is one line was 2.0pt. Word's diagram on page 8
puts two rows of labels 2.1pt apart; ours puts the same two rows 1.9pt apart.
Word saw two lines, we saw one, nothing could be paired with anything, and
forty-seven words each side had laid within half a point of the other were each
reported as a word only one side had. This is the same fault as the 3.00pt
collision found this morning, and moving the number again would only have moved
where it strikes. **The cuts are now made once, over both readings' baselines
together, and both sides are cut in the same places** — the partition stops
being a property of one rendering and becomes a property of the page.

**And so must the order words are read in.** A shared cut sometimes puts two
rows of a tight diagram in one group, and sorting such a group by `x` alone
shuffles the rows into each other: Word draws `network_ook_sm.c` as one word
from 370.3 and we draw it in seven pieces from 370.7, with a `Manager` from the
row above at 377.5 landing in the middle of our filename. Rows first, then `x`
— and the rows cut jointly too, for exactly the same reason. Cutting them per
side, which is the version this had first, put four of our words on a row of
their own and made things worse.

**The strongest evidence in a line has to be read first — 19 words.** Several
words running together into exactly the other side's one word is not something
a page does by accident; two identical short words in different places is
something every page does. Left to itself the subsequence pairs the second and
destroys the first: on page 9 Word draws `RX_` where we draw `RX` and `_`, and
the subsequence paired our `RX` with a *different* `RX` thirty-two points away,
which cost it nothing and left nothing that could be welded. Welds are found
first now, and the subsequence runs between them.

**What is left is 31 words, all in one diagram on page 4**, from two causes
worth naming rather than chasing. A label set vertically — `Interrupt`, drawn
one character at a time up the page — arrives as nine words on nine baselines,
and nothing here has a notion of a line running downwards. And three rows of
that diagram stand 0.7 to 0.9pt apart while the two renderers disagree about
their baselines by up to 0.9pt, so no threshold whatever separates them: ours
reads `radio_test.c Radio API Library API/` and Word's reads `radio_test.c
Library API/ Radio API`. That is the floor for that diagram, and it is a floor
of about half a per cent of the document.

**Two more things came out of it.** `--lines` is now a flag: it prints how each
reading was cut into lines, which is the first thing to look at when a report
says a page matched nothing, and which I reached for twice through a temporary
hack before building it. And the corpus gained two findings the bullet fix
uncovered — `headings-and-list.doc` has four bullets 2.7 to 4.6pt out
vertically, which had been hiding as unplaceable words rather than as the
misplacement they are. `--check` refused to pass until that was recorded, which
is the workflow working: the count of words out of place went *up* because
words that could not be placed at all can now be measured.

### The number is a gate now, not a note

`LAYOUT.md` records what every corpus document measures, and
`cargo xtask compare --check` fails on any that got worse — counts with no
slack at all, and the worst single shift with half a point of it. That is the
difference between a tool somebody remembers to run and a gate that catches
them. It is deliberately *not* part of `cargo xtask check`, which has to keep
working on a machine with no Word.

Two flags were added for the same reason the harness was: `--page N` for
working one page at a time, and `--words` for printing both readings
uncompared. The second exists because the first thing anyone does when a report
says nothing matched is write a throwaway script to look at both sides, and a
throwaway written three times is a tool that was never built.

The corpus as it now stands:

```
file                                         out  unplaced     worst  pages
---------------------------------------------------------------------------
character-formatting.doc                       4         0    3.47pt      1
header-footer-footnote.doc                     8         8    4.42pt      1
headings-and-list.doc                          9         4    4.64pt      1
plain-paragraphs.doc                          29         0    6.82pt      1
simple-table.doc                               6         0    5.38pt      1
unicode-text.doc                               9         0    4.02pt      1
comments.docx                                  4         0    1.27pt      1
content-controls.docx                          0         0    0.23pt      1
file-sample_100kB.docx                         0         0    0.29pt      2
file-sample_1MB.docx                           0         0    0.25pt      2
file-sample_500kB.docx                         0        12    0.26pt      2
floating-image-wrap.docx                     244       152   50.23pt      1
footnotes-endnotes.docx                        0         6    0.58pt      1
headers-footers.docx                         328         0    2.34pt      1
hyperlinks-bookmarks.docx                      3         0    2.49pt      1
lists-numbering.docx                           0         2    0.13pt      1
minimal.docx                                   0         0    0.55pt      1
nested-tables.docx                            17         0   11.33pt      1
picture-watermark.docx                       169       140  115.20pt      1
rtl-and-cjk.docx                               1         0    3.34pt      1
sections-mixed-orientation.docx                2         0    1.11pt      3
styles-headings-toc.docx                       8         0    1.89pt      1
table-spanning-pages.docx                    244        16   25.26pt      3
tracked-changes.docx                           0        21    0.00pt      1
watermark.docx                               115       366   67.39pt      1
---------------------------------------------------------------------------
                                            1200       727
```

**Two real findings the sharpening surfaced.** On `watermark.docx` our leading
is about **15.98pt against Word's 16.94** — six per cent tight, which
accumulates down the page until the lines stop corresponding at all, and is
why that document reports both a large shift and a large unplaceable count. And
`tracked-changes.docx` is not really comparable at all: Word renders a revised
document as though every change were accepted, and Scriva draws what the file
stores, so its twenty-one unplaceable words measure that difference and not
this one. Both are recorded rather than fixed — the remit was the instrument.

### The page around the type, and the machine with no Word

Two debts of the same kind, both of them a claim the tool made that nothing had
measured.

**Only the type was compared.** A rule, a shading, a border, a picture — none of
them moved a number. It was the largest thing the instrument was blind to and
the worst kind of blindness: a page whose whole table has slid half a centimetre
reads as a page with a handful of words out of place. `wp-compare` now gathers
every rectangle of ink that is not type, on both sides, and compares them the
same way — a word by what it says, a rule by where it is — with two columns of
their own in `LAYOUT.md`.

The hard part was that **neither renderer draws a border the way the other
does.** Word's export lays a table's top edge as a little filled square at each
corner with the spans between them; Scriva lays one rule per cell. A page of ink
the two agree about to a hundredth of a point arrives as thirty rectangles
against nine. So both readings are reduced to their ink — duplicates dropped,
touching collinear pieces run together — by one function that cannot tell which
side it is working on. The third time that lesson has arrived.

There is a second half, which the first attempt got wrong. **A rule is broken
where another rule crosses it.** Word's corner square can be run into the
horizontal border or into the vertical one but not into both, and whichever pass
takes it leaves the other with a rule in five pieces, each one crossing short of
its neighbour: nine rules against one, with nothing wrong on the page. A break as
wide as the rule's own thickness is bridged now; a gap between two boxes is a gap
somebody meant, and is left alone.

**What it found on its first run.** `floating-image-wrap.docx` puts one of its
two images 120 points below where Word puts it and the other exactly right — a
fault no word count could name, because the words around a floated image were
already being reported as unplaceable. `header-footer-footnote.doc` draws no
footnote separator, because it lays no footnote band at all, and the missing rule
and the eight unplaced words are one fault rather than two. `file-sample_500kB
.docx` has a framed box on page two that Scriva does not draw at all. And on
`nested-tables.docx` and `table-spanning-pages.docx` the borders are out by the
same 5.7 and 24.9 points as the words — the furniture corroborating the type,
which is the most useful thing a second measurement can do.

Every word column in `LAYOUT.md` came back byte for byte identical after the
probe was rewritten, which is the check that the rewrite measured the same
document it used to.

**The standing floor is `watermark.docx`, twelve marks.** Word draws a WordArt
watermark into a PDF as *outlines*, so its rendering has a filled shape per
letter where ours has type. A picture's box is set aside when Word draws into it
— that is as much as the two can honestly say to each other about a diagram —
but a watermark's box is *transparent*, the body's own rules run under it, and
setting it aside would take them with it. So those are left in the count and
named in the record as what they are.

**The machine with no Word had never existed.** Everything rests on the corpus
being checkable without Office, which is why `--check` may sit inside `cargo
xtask check`; it is written down in three files and had never once been
executed. Every run has been here, where Word is installed, a stale reading is
renewed in seconds, and nobody notices. `crates/wp-compare/tests/without_word.rs`
now runs the tool with **nothing on its PATH** — Word is reached only by starting
`powershell` and the rendering read only by starting `python`, and neither can be
found without one. The corpus checks clean. The two ways of genuinely needing
Word have also been made to say which is which: a document nobody has a reading
of, and a reading that has gone stale, want different things done about them and
no longer read alike.

The corpus as it now stands, with the two new columns:

```
file                                    out  >5pt unplaced  marks  lost    worst pages
--------------------------------------------------------------------------------------
character-formatting.doc                  4     0        0      1     0   3.47pt     1
header-footer-footnote.doc                8     0        8      0     1   4.42pt     1
headings-and-list.doc                    11     0        0      0     0   4.64pt     1
plain-paragraphs.doc                     29     3        0      0     0   6.82pt     1
simple-table.doc                          6     6        0      0     0   5.38pt     1
unicode-text.doc                          9     0        0      0     0   4.02pt     1
comments.docx                             4     0        0      0     0   1.27pt     1
content-controls.docx                     0     0        0      0     0   0.23pt     1
file-sample_100kB.docx                    0     0        0      0     0   0.29pt     2
file-sample_1MB.docx                      0     0        0      0     0   0.25pt     2
file-sample_500kB.docx                    0     0       12      0     7   0.26pt     2
floating-image-wrap.docx                244   244      152      0     2  50.23pt     1
footnotes-endnotes.docx                   0     0        6      0     0   0.58pt     1
headers-footers.docx                    328     0        0      0     0   2.34pt     1
hyperlinks-bookmarks.docx                 3     0        0      0     0   2.49pt     1
lists-numbering.docx                      0     0        0      0     0   0.13pt     1
minimal.docx                              0     0        0      0     0   0.55pt     1
nested-tables.docx                       17    17        0     12     2  11.33pt     1
picture-watermark.docx                  169   166      140      0     0 115.20pt     1
rtl-and-cjk.docx                         13     5        0      0     0   7.12pt     1
sections-mixed-orientation.docx           2     0        0      0     0   1.11pt     3
styles-headings-toc.docx                  8     0        0      0     0   1.89pt     1
table-spanning-pages.docx               244   244       16     76     4  25.26pt     3
tracked-changes.docx                      0     0       21      0     0   0.00pt     1  not compared
watermark.docx                          115   101      366      0    12  67.39pt     1
--------------------------------------------------------------------------------------
                                       1214   786      700     89    28
```

### The check was fetching its own evidence

`d53bdd9` shipped broken and the gate said it was fine. A docstring fix to
`pdfink.py` landed after `cargo xtask compare --record` and before the commit,
so all twenty-five committed readings named a probe script that no longer
existed. Here, `read()` found them stale, asked Word for twenty-five fresh
renderings and measured against those — four silent minutes and a green gate. On
a machine without Office every document came back "taken from an older probe
script, and renewing it needs Word". Proved rather than reasoned about: a
throwaway worktree at `d53bdd9`, `PATH` emptied, twenty-four failures.

A cache that repairs itself is a cache. A *check* that repairs itself is a
formality — it holds the corpus to evidence it was willing to manufacture.
`--check` is now `Renew::Never` and cannot start Word at all; a stale reading is
the finding, and the message says to run `--refresh` and commit what it writes.
The unit test for it runs on a machine that *has* Word, because that is the only
machine where the fault cannot be seen.

Two other things came out of the same sitting, both from the workflow that was
supposed to be diagnosing a misplaced picture:

**The picture's cause was found and the fix was refuted.** The inline picture in
`floating-image-wrap.docx` sits 120 pt low, and 120 pt is exactly the anchored
picture's height — which made "the float is contributing its height to the line"
irresistible and wrong. `units` never sees an anchored drawing at all
(`inline.rs:1154` guards on `!drawing.anchored`); the 120 pt is a *stacked flow
item*, pushed by `push_paragraph` because `displaces()` classifies this float as
one whose height must come out of the flow. 282.013 = 72 + 120 + 90.013, which
is addition and not a taller line. Patching only that predicate puts the line at
exactly Word's 72.000 — and makes the page worse: `out 244 to 245, worst 50.23pt
to 320.45pt`, because `Wraps::of` gives text only the wider side of a float while
Word sets this document's text on both. The reservation is a second mechanism
that should not exist — `LEARNINGS.md` already records Word as having one, the
float being a rectangle every line goes round. Text on both sides of a float is
the missing work, and it is a feature rather than a repair.

**And a real one-line fault, latent.** `Wraps::of` destructured the standoffs as
`(above, left, below, right)` where `wp:anchor` states them clockwise and the
model keeps them that way — `distT, distR, distB, distL`, which `push_paragraph`
three hundred lines away reads correctly. The two sides were swapped. Nothing in
the corpus could see it, because every float there stands off its two sides by
the same amount.

### Text goes round a float, on both sides of it

The first thing the harness found once it could see a page's furniture, and it
took the engine's line breaker to fix rather than a predicate.

`floating-image-wrap.docx` holds one paragraph with two pictures: an inline one
and an anchored one, `wrapText="bothSides"`, a hundred points into the column.
The inline picture sat 120 points below Word's — and 120 points is exactly the
*anchored* picture's height, which makes "the float is adding its height to the
line" irresistible. It is also wrong: a float never enters a line at all. The
120 was a flow item, reserved ahead of the paragraph by `displaces()`, and the
reservation was the whole fault.

**A float is a rectangle and there is only one mechanism.** Word does not have
one rule for text beside a picture and another for text below it; every line that
meets the rectangle goes round it. Reserving the height is a second mechanism and
it is right only where going round is impossible — a picture as wide as the
column, which is what `file-sample_500kB.docx` has and why the reservation was
written. `displaces()` now asks whether any measure is left beside the float, and
the boundary is deliberately the least the evidence supports: at −18 points of
leftover Word sets the text below, at +290 it sets it beside, and inventing a
threshold in between would be a number no oracle was ever asked for.

**And text goes on both sides of one line.** Stopping the reservation alone was
built and measured before it was believed, and it made the page worse — a 50
point vertical error traded for a 320 point horizontal one, because the engine
could narrow a line to one side of a float and Word uses both channels of the
same line. So `Obstacle` gained where its band *starts*, which is what lets a
paragraph that began above a float be narrowed from its fourth line down, and a
*hole*: a stretch out of the middle of the measure that the pen steps over
rather than through. A line needed no new shape for it — a fragment's x was
already a free offset from the line's own.

One document moved, and `file-sample_500kB.docx` did not:

```
                                          out  >5pt unplaced  marks  lost    worst
floating-image-wrap.docx   before          244   244      152      0     2  50.23pt
floating-image-wrap.docx   after           319     0        0      0     0   4.29pt
```

The count of words out of place went *up* while everything else collapsed, which
is the record doing its job: a hundred and fifty-two words that could not be
placed at all can now be measured, and they are about two points out. The middle
shift left is dx +2.03, dy −2.06 — the space advance and the leading this project
already has recorded from two other documents, showing through now that the wrap
no longer drowns them.

Three faults were found on the way and are worth keeping:

- `Wraps::add` rebuilt an obstacle field by field from an empty one, so the two
  new fields were silently dropped and the hole never reached a line. A merge
  that names its fields is a merge that forgets the next one added.
- `flow_paragraph` consulted the body's float table for headers, footers and
  notes, whose paragraphs number from zero in flows of their own — the same
  hazard the memo three lines below it already guards against.
- `Wraps::of` destructured the standoffs as top-left-bottom-right where
  `wp:anchor` states them clockwise, so the two sides were swapped. Latent:
  every float in the corpus stands off both sides by the same amount.

## Deferred

- [x] **PDF** — was dropped per Q3; built after ship as `wp-print`. See above.
- [-] **.doc / .xls writing** — never; save-as-modern is the escape hatch.
- [-] **Macros / VBA** — preserved verbatim, never executed.
- [-] **PowerPoint** — out of scope.

## The corpus, measured to nothing over five points

The compare harness was pointed at its own record and the worst differences
worked through in order. Every document in the corpus now measures **no word
more than five points out of place**, and the worst single difference anywhere
is 3.06pt, down from 115.20.

```
                                    out  >5pt unplaced  marks  lost    worst
totals   before                    1289   542      548     89    26  115.20pt
totals   after                      325     0       21      0    22    3.06pt
```

Eight faults, in the order the ranking gave them:

- **Kerning.** `<w:kern>` names the size at or above which Word closes up its
  pairs, and Word's own document defaults name two half-points — so every
  ordinary run kerns, and the shaper was measuring every one of them letter by
  letter. `headers-footers.docx` went from 328 words out to none.
- **Aptos.** Office keeps its default face in a cloud-font cache rather than in
  the font directory, so the machine had it and no lookup could find it.
  `picture-watermark.docx` went from 115.20pt worst and 140 unplaceable words to
  0.33pt and none; `watermark.docx` from 67.39pt and 366 to 0.42pt and none.
- **Repeated header rows** were paginated for and never drawn.
- **A table style's `<w:tblInd w:w="0">`** was read as a stated indent, hanging
  every ordinary table's rule a cell's padding into the margin.
- **A picture's line** took the type's own descent below the baseline rather
  than the paragraph's spacing, losing two and a third points on the line that
  holds an inline figure and every line after it.
- **`sprmTWidthIndent`** — the `.doc` equivalent of `w:tblInd`, and the only
  place the format states the indent to the *text* rather than to the rule.
- **`STSHI.rgftcStandardChpStsh`** — where a `.doc` keeps the default font, for
  want of which every modern document saved as `.doc` was laid in Times New
  Roman. With it read, `sprmPFContextualSpacing` and `sprmCIss` were next.
- **`.doc` footnotes** were read and never referenced: `PlcffndRef` joins the
  marks in the running text to the notes, `PlcffndTxt` says where each note
  begins, and the separator rule above them is the first of the six stories the
  header document opens with.

What is left is under three points and mostly one thing: a face not in the
measured line-pitch table is laid at its ideal rounded to a twenty-fourth of a
point, and Aptos's own laid pitch — measured now, but with a six-tenths
correction where the accumulator pays halves — does not fit that machinery.
`floating-image-wrap.docx` holds 296 of the remaining 325, all of it that drift.

## Documents from outside the corpus, measured the same way

Eight `.doc` and `.docx` files in `C:\Adnan\test` — none of them written by
the same producer as the corpus, several written by no version of Word at all
— put through `cargo xtask compare`. They were worth measuring precisely
because the corpus is self-made: everything in it was written by this machine's
Word, so every habit that Word has and nobody else shares reads as the format
rather than as one producer's dialect. Four documents held real faults.

| file | before | after |
|---|---|---|
| `two_sections.docx` | 0 | 0 |
| `file-sample_100kB/500kB/1MB.docx` | 21 | 21 — all chart labels, the harness's stated blind spot |
| `resume.docx` | 1147 | 155 |
| `demo.docx` | 1455 | 444 |
| `table_render_test.doc` | 221 | 221 |
| `sample-docx-file-for-testing.docx` | 3755 | 3755 |

- **`w:tblInd` was being measured to the text.** A rule fitted to one binary
  document had the cell's padding taken off every stated indent in every
  format; a second producer that pads 115 twips and indents -7 came out nearly
  six points left on every line of the document. The measurement to the text is
  the binary format's, and turning it into the other one now happens in the
  reader that knows the format.
- **`<w:rPr/>` inside a list level swallowed every list definition after it.**
  An empty element has no end tag; the reader went looking for one and found
  the next level's. One document's second list drew no bullets at all and kept
  the hanging indent that had made room for them.
- **A paragraph that forced its own page break lost the space above it.** The
  rule about not spacing away from the top of a page is about a page that ran
  out of room, not one the writer asked for — a heading style pairing
  `<w:pageBreakBefore/>` with 24 points before stood every page of `demo.docx`
  that far up.
- **The instrument was reading documents in the wrong type.** The harness
  registered the machine's faces and not the package's, so a document embedding
  its own type was measured in a substitute. Fixed in `wp-compare` rather than
  in the layout, because the layout was never wrong.

Then the largest of them was measured properly rather than set aside.

- **Word closes the spaces of a justified line up to hold one more word.**
  Found by removing the justification from a copy of the worst document: ragged,
  it measured 0 out against Word on every one of its eleven pages, which said
  the widths and the break points were exactly right and the fault was in the
  justification alone. Measured with a probe — one paragraph of fixed text, the
  right indent stepped a tenth of a point at a time so the column crosses the
  line's natural width by a known amount — Word holds the last word until the
  spaces stand at three quarters of their natural width. With that,
  `sample-docx-file-for-testing.docx` went from 3755 to 338 and from eleven
  pages to Word's ten.
- **It is `compatibilityMode` fifteen that turns it on.** Closing spaces up in
  every document made three others measurably worse — `file-sample_100kB.docx`
  from 21 to 167, the `.doc` from 31 to 244 — and the documents that wanted it
  and the documents that did not divided exactly on whether `settings.xml`
  declares mode fifteen. The probe had inherited the package of a mode-fifteen
  document, which is why it measured the new behaviour so cleanly.
- **A face embedded under a null key is not one Word draws with.** Registering
  a document's own faces in the harness made `resume.docx` five times worse:
  it carries plain TrueType under `{00000000-…-000000000000}`, and a symbol
  face 1.48em tall was then raising every bulleted line by four and three
  quarter points. Word pitches those lines at the text face's own ascent, so it
  is substituting; refused at the key, the document went back to 155 with
  nothing unmatched.

| file | first measured | now |
|---|---|---|
| `two_sections.docx` | 0 | 0 |
| `file-sample_100kB.docx` | 21 | 21 |
| `file-sample_500kB.docx` | 21 | 21 |
| `file-sample_1MB.docx` | 21 | 21 |
| `resume.docx` | 1147 | 155 |
| `demo.docx` | 1455 | 444 |
| `table_render_test.doc` | 221 | 221 |
| `sample-docx-file-for-testing.docx` | 3755 | 338 |

What is left is diagnosed and deliberately not fixed:

- `sample-docx-file-for-testing.docx`'s remaining 338 is the boundary of the
  rule above: three of its ten pages are exact and the rest hold a handful of
  lines where Word chose the other way. Word is not simply squeezing whenever
  it can — the same document has it accept a line needing three quarters and
  refuse one needing seven eighths, at the same number of spaces — so a second
  condition is at work that the document itself cannot settle. It would take
  the probe again, over the shape of the line rather than over the column.
- `demo.docx`'s remaining 444 is 400 on the two pages that use its embedded
  Ubuntu Mono. Both renderings agree on where the run ends — it is the same
  width in both — and disagree only about where each glyph inside it sits,
  which is Word's PDF export substituting glyphs inside an embedded subset.
  The plain Ubuntu around it matches to a fortieth of a point.
- `table_render_test.doc`'s 221 is all under five points, and its 179 unmatched
  marks are the eleven pictures Word draws into rather than drawing a box for —
  the blind spot `NOT_COMPARED` already names.

