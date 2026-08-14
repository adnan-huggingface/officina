# Using Calx and Scriva

Both applications are deliberately familiar. If you know Excel and Word, you know
these: the menus are in the same order, the keys are the same keys, and where
something is different it is because the difference is the point.

Every menu answers to the keyboard. Press **Alt** and the mnemonics appear —
`Alt+F` for File, `Alt+E` for Edit, and so on — then the underlined letter of the
item you want.

---

## Calx — spreadsheets

### Opening and saving

Calx opens `.xlsx`, `.xlsm`, `.xltx`, `.xls`, `.csv` and `.tsv`. It saves
everything except `.xls`, which is read-only: use **Save As** and choose
`.xlsx`.

A file open in Excel cannot be replaced while Excel holds it. Calx says so and
does not pretend to have saved.

| | |
|---|---|
| `Ctrl+N` | New workbook |
| `Ctrl+O` | Open |
| `Ctrl+S` | Save |
| `Ctrl+Shift+S` | Save As |
| `Ctrl+W` | Close the workbook, keeping the window |

### Moving and selecting

Arrow keys move; `Ctrl+Arrow` jumps to the edge of the block of data, as in
Excel. `Home` goes to column A, `Ctrl+Home` to A1, `End` then an arrow does what
Excel's End mode does. `Ctrl+A` selects the region around the cursor first and
the whole sheet second. `Ctrl+G` goes to an address or a name.

Drag the small square at the corner of the selection to fill; double-click it to
fill down as far as the neighbouring column has data. Hold `Ctrl` while dragging
to copy instead of extending a series.

Drag the selection's border to move the cells; hold `Ctrl` to copy them.

### Typing and formulas

Type to replace, `F2` to edit in place, `Alt+Enter` for a line break inside a
cell, `Ctrl+Enter` to fill the whole selection with what you typed.

While a formula is being typed, clicking or arrowing to a cell inserts its
reference — Excel's point mode. `F4` cycles `A1`, `$A$1`, `A$1`, `$A1`.

277 functions are implemented, including dynamic arrays: `FILTER`, `SORT`,
`UNIQUE`, `SEQUENCE`, `XLOOKUP` and the rest spill into the cells below, and a
spill that would land on occupied cells reports `#SPILL!` rather than
overwriting.

`Alt+=` sums the cells above.

### Formatting

| | |
|---|---|
| `Ctrl+1` | Format Cells |
| `Ctrl+B` / `Ctrl+I` / `Ctrl+U` | Bold, italic, underline |
| `Ctrl+Shift+_` | Remove borders |

Number formats, fonts, fills, borders, alignment, conditional formatting and data
validation are all read from the file, editable, and written back.

### Structure

| | |
|---|---|
| `Ctrl++` / `Ctrl+-` | Insert or delete rows |
| `Ctrl+9` / `Ctrl+Shift+9` | Hide or unhide rows |
| `Ctrl+0` / `Ctrl+Shift+0` | Hide or unhide columns |
| `Ctrl+Shift+L` | Filter |
| `Ctrl+F3` | Name Manager |

Drag a row or column header's edge to resize it; double-click that edge to fit
the contents. Right-click a sheet tab for the full menu — insert, delete, rename,
move, copy, tab colour, hide, protect. Drag a tab to reorder it.

Sort, Text to Columns, Remove Duplicates, Group and Ungroup, Split panes and
Protect Sheet are all under the menus where Excel puts them.

### Clipboard

`Ctrl+X`, `Ctrl+C`, `Ctrl+V`, and `Ctrl+Alt+V` for Paste Special. A cut or copied
range shows the marching-ants border; `Esc` cancels it. Text copied from another
program is parsed the way it would be if you typed it.

---

## Scriva — documents

### Opening and saving

Scriva opens `.docx`, `.docm`, `.dotx`, `.doc`, `.md` and `.txt`.

A `.doc` opens **as a copy**: the old format is read but never written, so the
title bar shows a `.docx` name from the moment it opens and `Ctrl+S` will ask
where to put it. Saving as Markdown or plain text warns first, because both throw
away everything that is not words.

| | |
|---|---|
| `Ctrl+N` | New document |
| `Ctrl+O` | Open |
| `Ctrl+S` | Save |
| `Ctrl+Shift+S` | Save As |
| `Ctrl+W` | Close the document, keeping the window |

### Editing

Typing, selection, `Ctrl+Arrow` by word, `Home` and `End`, `Ctrl+Home` and
`Ctrl+End` — all as expected. Undo is `Ctrl+Z` and redo is `Ctrl+Y`; a run of
typing collapses into one undo at word boundaries, as Word does.

### Formatting

| | |
|---|---|
| `Ctrl+B` / `Ctrl+I` / `Ctrl+U` | Bold, italic, underline |
| `Ctrl+Shift+=` / `Ctrl+=` | Superscript, subscript |
| `Ctrl+Shift+>` / `Ctrl+Shift+<` | Grow, shrink |
| `Ctrl+Space` | Clear direct formatting |
| `Ctrl+L` / `Ctrl+E` / `Ctrl+R` / `Ctrl+J` | Left, centre, right, justify |
| `Ctrl+1` / `Ctrl+5` / `Ctrl+2` | Single, 1.5, double line spacing |
| `Ctrl+M` / `Ctrl+Shift+M` | Increase, decrease indent |

Styles are in the Styles menu, and applying one is what a heading *is* — Scriva
does not fake a heading with bold text, so the navigation pane and the table of
contents both find it.

### Pictures

Click a picture to select it. Drag its body to move it, a corner to resize it
keeping its shape, an edge to stretch one axis, and press `Delete` to remove it.
`Esc` lets it go. The whole drag is one undo.

Only the size and the position can be changed. Everything else about a picture —
crops, effects, rotations — is kept exactly as it was, because those are not
things this can write back, and showing an edit that a save would throw away
would be worse than not offering it.

### Reviewing

| | |
|---|---|
| `Ctrl+Shift+E` | Track changes on or off |
| `Alt+F7` | Next change |
| `Ctrl+Alt+M` | New comment |

Tracked changes and comments are editable, not merely preserved: accept, reject,
reply and resolve all work and are written back.

### The view

| | |
|---|---|
| `Ctrl+F` | Navigation pane |
| `Ctrl+Shift+8` | Formatting marks |
| `F9` | Update the table of contents |

---

## Where your settings live

`~/.config/calx/` and `~/.config/scriva/` — on Windows too, which is not the
Windows convention but is what was asked for. Each holds the window geometry and
the recent-files list, and nothing else. Deleting either directory loses nothing
but that.

## What these cannot do

Neither application prints. Neither reads PDF. See
[FORMATS.md](FORMATS.md) for the full list, including the things that are read
but not editable and the things that are neither, but survive a save regardless.
