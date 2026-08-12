# Calx parity audit — the full test suite

The goal: Calx behaves like Excel wherever Excel's behavior is observable. Every line
below is a concrete, testable claim about what Excel does. Each gets one of four marks:

- `[ok]`   — verified matching (by reading the code AND by a test where one is possible)
- `[FIX]`  — mismatch found, must fix
- `[gap]`  — feature absent; decide build vs. defer
- `[n/a]`  — deliberately out of scope (printing, collaboration, VBA)

Work proceeds area by area, in the order below. An area is done when every item is
marked, every `[FIX]` is fixed with a regression test, and the fidelity harness is green.

---

## A. Selection

- A1. Click a cell: it becomes the active cell and the sole selection; anchor = active.
- A2. Click-drag: range grows from anchor; active cell stays at the anchor corner; the
      anchor cell is white (unfilled) inside the highlighted range.
- A3. Shift+click: extends from existing anchor to clicked cell (anchor does not move).
- A4. Shift+arrows: extends the range; active cell stays put, the moving corner moves.
- A5. Ctrl+click: adds a new disjoint range to the selection; active cell moves to the
      newly clicked cell. Ctrl+drag adds a range. (Excel allows overlapping ranges.)
- A6. Row header click: selects entire row; active cell = A<row>. Drag over headers
      selects multiple rows. Shift+click extends, Ctrl+click adds disjoint rows.
- A7. Column header click: same for columns; active = <col>1.
- A8. Select-all corner: selects the whole sheet; active cell stays where it was (Excel
      keeps the previous active cell).
- A9. Ctrl+A: first press selects the current region (island of data around the active
      cell); second press selects the whole sheet. On an empty region it selects all.
- A10. Ctrl+Space selects the active cell's column(s); Shift+Space selects row(s);
      Ctrl+Shift+Space selects all.
- A11. Selection rendering: translucent fill, thick border around the range, the active
      cell unfilled, row+column headers of selected cells shaded, header fully
      highlighted when the whole row/col is selected.
- A12. Name box shows active cell address; during a drag it shows "3R x 2C"; typing an
      address/range into it navigates/selects; typing a defined name jumps to it.
- A13. Merged cells: clicking one selects the whole merge; arrowing through treats it
      as one cell; a range selection snaps to include whole merges.
- A14. Selecting with a whole row/col selected, then typing, edits the active cell only.

## B. Navigation

- B1. Arrows move the active cell one step; selection collapses to it.
- B2. Ctrl+arrow: jump to the edge of the data region (last filled before a blank, or
      next filled cell if starting on a blank/edge), else to the sheet edge.
- B3. Ctrl+Shift+arrow extends selection with the same jump rule.
- B4. Tab moves right, Shift+Tab left; Enter moves down, Shift+Enter up.
- B5. Within a multi-cell selection, Tab/Enter cycle through the selection only
      (row-major for Tab, column-major for Enter), wrapping range to range.
- B6. Home goes to column A of the current row. Ctrl+Home goes to A1 (or the top-left
      unfrozen cell when panes are frozen). Ctrl+End goes to the bottom-right used cell.
- B7. PageUp/PageDown move one screenful vertically; Alt+PageUp/Down horizontally.
- B8. Scrolling never moves the active cell; typing/arrows snap the view back to it.
- B9. Ctrl+G / F5 (Go To) accepts an address and navigates.
- B10. Wheel scrolls rows; Shift+wheel scrolls columns; Ctrl+wheel zooms centred on
      the pointer.
- B11. The view scrolls to keep the active cell visible when it moves off-screen
      (minimal scroll, not centring).

## C. Editing

- C1. Typing on a selected cell replaces content and opens the editor (Enter mode:
      arrows COMMIT and move, they don't move the caret).
- C2. F2 / double-click opens the editor in Edit mode: arrows move the caret within
      the text. F2 toggles Enter/Edit mode while the editor is open.
- C3. Enter commits and moves down (or per B4/B5 rules); Tab commits and moves right;
      Escape discards; clicking another cell commits (except in formula point mode).
- C4. Delete clears contents of the whole selection (values only, not formats).
      Backspace clears the active cell and opens the editor empty.
- C5. The editor shows the formula, the cell shows the value; editing shows raw text
      (no number formatting applied while editing).
- C6. Formula entry: typing `=` starts a formula; clicking a cell/range inserts its
      reference (point mode); arrows in point mode move the reference, not the caret;
      operators re-arm point mode; Enter commits.
- C7. F4 in the editor cycles the reference under the caret: A1→$A$1→A$1→$A1→A1.
- C8. Range references while pointing render with a colored border on the grid, one
      color per reference, matching the colored text in the editor.
- C9. Autocomplete: typing a function name offers completions; typing a value that
      prefix-matches another value in the column offers inline completion (accept with
      Enter/Tab, keep typing to override).
- C10. Ctrl+; inserts today's date, Ctrl+Shift+; the current time.
- C11. Alt+Enter inserts a line break in the cell.
- C12. Editing a merged cell edits its top-left; commit keeps the merge.
- C13. Typing into a multi-cell selection then Ctrl+Enter fills the whole selection.
- C14. Formula bar (if present) mirrors the in-cell editor both ways.

## D. Fill handle & autofill

- D1. The selection's bottom-right corner shows a small square; hovering it shows a
      thin cross cursor.
- D2. Dragging it fills: copies for text/formulas (relative refs adjust), increments
      series for numbers when the source is 2+ cells with a step, extends
      dates/weekdays/months, cycles custom lists (Mon, Tue…; Jan, Feb…).
- D3. Single number drags copy; Ctrl+drag toggles copy↔series.
- D4. Dragging back over the source shrinks/clears.
- D5. Double-clicking the handle fills down to match the neighbouring column's extent.
- D6. Ctrl+D fills down from the top row of the selection; Ctrl+R fills right.

## E. Clipboard

- E1. Copy draws marching ants around the source; Escape or an edit dismisses them.
- E2. Cut: ants; paste MOVES the content (source cleared, references to the moved
      cells follow them); cut-paste happens once, copy-paste can repeat.
- E3. Paste into a selection: single-cell source tiles the whole selection; range
      source pastes at active cell (Excel refuses mismatched multi-range paste).
- E4. Pasting a formula adjusts relative references by the offset.
- E5. External clipboard: copies as TSV (and HTML if feasible); pasting TSV from
      outside splits into cells; a single line with tabs still splits.
- E6. Paste Special: values, formats, formulas, transpose at minimum.
- E7. Enter pastes-once-and-dismisses ants (Excel: Enter completes the paste).
- E8. Inserting/deleting rows dismisses ants; typing dismisses ants.

## F. Rows, columns, cells

- F1. Drag a header boundary resizes; the tooltip shows size (pt/px for rows, chars/px
      for cols); double-click the boundary autofits to content.
- F2. Resizing when multiple rows/cols are selected resizes all of them alike.
- F3. Header context menu: Insert, Delete, Clear Contents, Format, Hide, Unhide,
      row Height / column Width dialog.
- F4. Inserting shifts references in formulas; deleting turns refs to deleted cells
      into #REF!.
- F5. Inserting N rows when N rows are selected inserts N. Ctrl+Plus / Ctrl+Minus
      insert/delete (whole rows/cols when whole rows/cols selected; else a dialog to
      shift cells).
- F6. Hidden rows/cols: header shows a gap marker; unhide via selecting across and
      context menu; Ctrl+arrow skips hidden cells; dragging the doubled boundary
      unhides.
- F7. Merge/unmerge across selection; merged cell keeps top-left value; warning when
      merging would drop values (Excel warns).
- F8. Cell drag-move: dragging the selection border moves cells (with formulas'
      absolute identity preserved — references to them follow); Ctrl+drag copies.

## G. Formatting

- G1. Font family, size, bold/italic/underline, strikethrough, font color, fill color
      apply to the whole selection; toolbar reflects the active cell's state.
- G2. Ctrl+B/I/U toggle; mixed-state selection toggles all on first press.
- G3. Number formats: General, Number (decimals, thousands), Currency, Accounting,
      Percent (Ctrl+Shift+%), Date, Time, Text, custom format strings; Ctrl+1 opens
      Format Cells.
- G4. Alignment: left/center/right, top/middle/bottom, wrap text, indent, merge &
      center button, orientation if supported.
- G5. Borders: the toolbar border menu applies outline/all/none etc. to the selection.
- G6. Format painter: copies format of active cell, next click/drag applies it;
      double-click locks it until Escape.
- G7. Increase/decrease decimals buttons.
- G8. Text that overflows shows over empty neighbours, is clipped by filled ones;
      numbers too wide show ####; wrap text grows row height.
- G9. Entering "1/2" or "5%" or "$3" or "1e3" coerces type and format like Excel;
      entering text starting with `'` stores literal text.

## H. Sorting & filtering

- H1. Quick sort A→Z / Z→A on the selection or current region; header guessed
      (text-over-anything = header, number/date top = data); status feedback.
- H2. Sort dialog: multiple keys, per-key order, has-headers checkbox.
- H3. Sort is stable; case-insensitive by default; numbers before text; blanks last
      in both directions.
- H4. AutoFilter: dropdown arrows on the header row, checklist of values, sort links,
      text/number filters; filtered rows hidden (blue row numbers in Excel).
- H5. Filter + sort interact: sorting a filtered range sorts visible rows only? (No —
      Excel sorts the whole range but keeps hidden rows hidden. Verify our story.)

## I. Formulas in the UI

- I1. Errors render as #DIV/0!, #N/A, etc.; error values are left-aligned? (No —
      centred in Excel). Confirm rendering matches.
- I2. Circular reference: Excel warns and shows 0. Verify our behaviour is sane and
      visible, not silent.
- I3. Recalculation is immediate on every edit; volatile functions (TODAY, RAND)
      recalc on edit anywhere.
- I4. Long formula results: General format shows up to 11 digits then scientific;
      column width affects digit display.
- I5. Defined names usable in formulas; Name box creates them (defer if absent).

## J. Undo / redo

- J1. Ctrl+Z undoes; Ctrl+Y and Ctrl+Shift+Z redo. A new edit truncates redo.
- J2. Every user-visible mutation is one undo step: edit, paste, sort, resize, format,
      insert/delete, merge, fill, move, sheet rename... nothing silently unundoable.
- J3. Undo restores selection to where the change happened.
- J4. Resize-by-drag is one step, not one per pixel.

## K. Sheets & workbook

- K1. Tab strip: click switches, double-click renames inline, drag reorders,
      right-click menu (insert, delete, rename, move/copy, tab color, hide).
- K2. Delete sheet warns if it has content; cannot delete the last sheet.
- K3. New sheet button; names Sheet1, Sheet2… skipping taken names; rename rejects
      duplicates and illegal chars (: \ / ? * [ ]), 31-char limit.
- K4. Cross-sheet references (=Sheet2!A1) work and update on rename.
- K5. Ctrl+PageUp/PageDown switch sheets.

## L. View

- L1. Freeze panes: freeze at selection, freeze top row, freeze first column; frozen
      line drawn; scrolling respects it; Ctrl+Home goes to top-left unfrozen.
- L2. Split panes (task #45).
- L3. Zoom: Ctrl+wheel, status-bar control, 10%–400%; zoom preserves the anchor point.
- L4. Gridlines/headings toggles if present.

## M. Cursors & pointer feedback

- M1. Cell body: fat white cross (we use system default/cell cursor — verify it's not
      a text I-beam). Headers: right/down arrow in Excel (system default acceptable).
- M2. Header boundary: horizontal/vertical resize arrows, held for the whole drag. [done]
- M3. Selection border: move cursor; fill handle: thin cross; during drag each keeps
      its own cursor. [partially done]
- M4. Editor: text I-beam over the text area.

## N. Find / replace / Go To

- N1. Ctrl+F opens Find; F3/Enter finds next, Shift+Enter previous; wraps around;
      "not found" message; search by rows/columns, in formulas vs values, match case,
      match entire cell.
- N2. Ctrl+H replace / replace all with count reported.
- N3. Find within selection when a range is selected; else whole sheet.
- N4. Ctrl+G Go To an address or name.

## O. Objects & annotations

- O1. Charts: insert from selection, move, resize, delete, redraw on data change.
- O2. Pictures: insert, move, resize (aspect with Shift), delete.
- O3. Comments/notes (task #49): red corner marker, hover to view, edit, delete.
- O4. Data validation UI (task #43): dropdown arrow for list rules, invalid entry
      rejected with message, dialog to add/edit/clear rules.
- O5. Conditional formatting UI (task #43): dialog for rule add/edit/delete over
      selection; rules render live.
- O6. Hyperlinks: Ctrl+click follows, plain click selects (defer if absent).

## P. File behaviour

- P1. Dirty indicator in title; prompt to save on close/new/open when dirty.
- P2. Open/Save/Save As with xlsx; csv/tsv import-export; .xls read.
- P3. Round-trip fidelity: what we didn't touch, we don't destroy (harness covers).
- P4. Errors on open (corrupt file) reported in a dialog, not a crash or silence.
- P5. Recent files list. Ctrl+N/O/S/Shift+S shortcuts.

## Q. Status bar

- Q1. Selection aggregates: Average, Count, Sum for numeric selections (Count counts
      non-empty; Average/Sum only when ≥1 number; blank shown when single empty cell).
- Q2. Mode indicator: Ready / Enter / Edit / Point.
- Q3. Zoom slider/percent at right.

## R. Context menus

- R1. Cell right-click: Cut, Copy, Paste, Paste Special, Insert…, Delete…, Clear
      Contents, Format Cells, plus contextual items (sort, filter). Right-clicking
      outside the selection moves the selection first (Excel does).
- R2. Header right-click per F3. Tab right-click per K1.
- R3. Object right-click: cut/copy/delete/order if applicable.

---

## Verification machinery

- Grid behaviors get headless egui tests in paint.rs's harness (`frame()` helpers).
- Model behaviors (sort, refs, undo) get unit tests in their crates.
- Every `[FIX]` lands with a test that fails before and passes after.
- Full run per area: `cargo test --workspace`, `cargo clippy`, `cargo xtask fidelity`.

## Findings log

### Fix pass (2026-08-11) — all twenty findings below are FIXED

Commits `4899370` (A), `2f90410` (B), `464aa41` (C), `e084f69` (D),
`c1cfd8b` (E), `0e1b2b8` (F), `5b6d6b6` (G), `2330e16` (H). Each landed with
regression tests; workspace tests, clippy, and the fidelity harness green
throughout. Also fixed on the way, unlisted: the active cell rode along with
a growing selection (typing landed in the wrong corner — selection now
carries a separate lead); cut-paste re-anchored relative references (now
true move semantics via `edit::move_range`); a drawing-splicer bug that
would have clobbered the mandatory `a:ext` inside any chart's graphicFrame;
`NOW`-adjacent date stamps used UTC (Ctrl+; is local time).

### Inventory pass (2026-08-11) — what the code said before the fixes

Confirmed bugs / mismatches, in fix order (tasks #51–#58):

1. **Right-click collapses the selection** — `begin_drag` runs on `any_pressed`,
   unfiltered by button (paint.rs:1375). Excel keeps a selection on right-click
   inside it. → #51
2. **Active cell is filled inside a range** — Excel leaves it white. → #51
3. **No auto-scroll during a selection sweep** past the viewport edge. → #51
4. **No fill-handle cursor** (thin cross). → #51
5. **Ctrl+A always selects the whole sheet** — Excel selects the current region
   first. **Ctrl+Shift+Space adds a column area** instead of select-all. → #52
6. **End / Ctrl+End / Ctrl+Shift+End missing** entirely. → #52
7. **No Go To** (Ctrl+G / F5) — name box only. → #52
8. **Missing keys:** Ctrl+9/0 (hide), Ctrl+Shift+9 (unhide rows), Ctrl+Shift+L
   (filter), Alt+= (autosum), Ctrl+Shift+~!@#$%^ (number formats), Ctrl+; and
   Ctrl+Shift+; (date/time). → #52
9. **No formula point mode** — clicking a cell mid-formula commits instead of
   inserting a reference; arrows never point. → #53
10. **No F4 reference cycling, no Alt+Enter newline** (singleline editor), no
    Ctrl+Enter fill-selection commit. → #53
11. **No marching ants** for copy/cut; Esc cancels nothing; Enter doesn't paste;
    a pending cut is invisible and uncancellable. → #54
12. **Context-menu Paste pastes remembered text**, not the OS clipboard
    (main.rs:3324). → #54
13. **Fill handle:** no double-click fill-down, no Ctrl copy/series toggle, no
    date series. → #55
14. **No drag-move of a cell range** by its border (Excel's most-used mouse move);
    only header bands move. → #56
15. **Charts are inert** — click selects in name only; no chrome, move, resize,
    or delete; Delete clears the cells underneath a "selected" chart. → #57
16. **Dead toolbar buttons** — text-colour and fill-colour icons discard their
    click Response (main.rs:2094/2106). → #58
17. **Escape closes no dialog.** → #58
18. **No mode indicator** (Ready/Enter/Edit/Point) in the status bar. → #58
19. **Sheet tabs cannot be drag-reordered** — dialog only. → #58
20. **Size dialog OK silently no-ops** on unparseable input. → #58

Deferred to the feature tasks already queued: data-validation & CF dialogs (#43),
group/outline (#44), split panes + protect (#45), text-to-columns + remove
duplicates (#46), insert chart (#47), insert picture (#48), comments (#49).

Noted, deliberately not matching Excel (better as-is): sheet delete is undoable
here (Excel warns because it can't undo); "(N hidden)" tab counter; tooltip
wording. Noted, accepted for now: column autofit is estimated not measured;
hide/resize caps at 4096 indices; one generic context menu for all regions;
chart sheets unreachable from the tab strip (needs a chart-sheet view to mean
anything); no function autocomplete; grouped sheet editing absent.

## Reported after the fix pass

21. **Select all, then sort, and the process dies.** The corner box selects
    A1:XFD1048576, and a sort of it asked the store to lift out a band of every
    column of every row at once — seventeen billion cells, which is not a slow
    operation but an allocation failure. Two more of the same shape were sitting
    next to it: Ctrl+A then Ctrl+C built a dense clip of the same size (and
    overflowed the `u32` that sized it), and Ctrl+A then Delete asked every
    address on the sheet whether it held anything, one at a time.

    Fixed by trimming the range to the data before walking it, which is also
    what Excel reports back: the Sort dialog now names the real range instead of
    A1:XFD1048576. A range the user actually drew is left alone — copying ten
    rows of which three hold anything still carries the seven blanks, because
    pasting them is meant to clear what they land on — so only axes covering the
    sheet end to end are trimmed (`Sheet::drawn_range`). Sort is trimmed on both
    axes, where blanks make no difference to the outcome, and refuses outright
    above 16M cells in the band, which is what a stray value out at column XFD
    would otherwise cost.

22. **The in-cell editor did not look like a cell.** Double-clicking swapped the
    cell's text for a rounded widget in a fixed 13-pixel font with its own
    background and a focus ring, top-aligned regardless of the cell, and clipped
    to the column — so editing anything in a narrow column wrapped every second
    character into a growing stack. The caret always went to the end of the
    text, which turns fixing a typo into retyping the entry.

    Rebuilt to be the cell, still being written: the cell's font, size, weight,
    colour and fill, its horizontal alignment (kept while the text still fits,
    so a number does not jump left on F2), and its vertical alignment, so
    nothing moves on screen when editing starts. The box grows rightwards over
    its neighbours as the text outgrows the column and only wraps at the edge of
    the grid, the way Excel's does, painting over what it covers rather than
    mixing with it; a click on the overhanging part stays in the editor. A
    double click leaves the caret in the word it landed on. A merged cell is
    edited over the whole merge.
