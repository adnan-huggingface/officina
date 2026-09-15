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
| `Ctrl+P` | Print |
| `Alt+F4` | Exit |

### Editing

Typing, selection, `Ctrl+Arrow` by word, `Home` and `End`, `Ctrl+Home` and
`Ctrl+End` — all as expected. A run of typing collapses into one undo at word
boundaries, as Word does.

| | |
|---|---|
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo, redo |
| `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | Cut, copy, paste — with formatting, when the board still holds what Scriva copied |
| `Ctrl+A` | Select all |
| `Ctrl+F` / `Ctrl+H` | The find bar; with the replace row. `Aa` matches case, `ab` whole words only. `Enter` and `F3` find the next match, `Shift+Enter` and `Shift+F3` the previous, `Tab` goes between the fields |
| `Ctrl+Enter` | Page break |

### Formatting

Every box that takes a measure reads it in any unit — `1.25`, `1.25 in`,
`3 cm`, `36 pt` — and shows it in inches. Layout ▸ Page Setup… holds the
paper, its orientation, the margins and the bands' distance from the edge in
one box, with the page drawn from the numbers as they are typed; applying it
is one undo. Format ▸ Text Colour and Highlight, and the toolbar's two colour
buttons, open a row of swatches; More Colours… is the grid of standard colours
in five tints, a hex field, three sliders and the last six chosen.

| | |
|---|---|
| `Ctrl+B` / `Ctrl+I` / `Ctrl+U` | Bold, italic, underline |
| `Ctrl+Shift+=` / `Ctrl+=` | Superscript, subscript |
| `Ctrl+Shift+>` / `Ctrl+Shift+<` | Grow, shrink |
| `Ctrl+Space` | Clear direct formatting |
| `Ctrl+L` / `Ctrl+E` / `Ctrl+R` / `Ctrl+J` | Left, centre, right, justify |
| `Ctrl+1` / `Ctrl+5` / `Ctrl+2` | Single, 1.5, double line spacing |
| `Ctrl+M` / `Ctrl+Shift+M` | Increase, decrease indent |
| `Ctrl+Shift+L` | Bullets |
| `Ctrl+D` | The Font box: family, style, size, colour, highlight and effects, with a preview; applied as one undo |

Styles are in the Styles menu, and applying one is what a heading *is* — Scriva
does not fake a heading with bold text, so the navigation pane and the table of
contents both find it.

### Tables

Insert ▸ Table puts one in above the caret's paragraph — the grid picker on
the toolbar for up to eight by eight, the box for numbers. While the caret is
in a table a strip under the toolbar says its size and offers what the Table
menu offers: `Row above`, `Row below`, `Column left`, `Column right`, `Delete ▾`
(row, column, table), `Merge`, `Borders ▾`, `Shading ▾`, `Width…`, `Margins…`.
Every one is one undo. A new row is shaped like the caret's — height, rules,
shading, cell widths — with empty cells; a new column is as wide as the column
to its right and narrows nothing, which is what Word does. Deleting the last
row or column deletes the table, leaving an empty paragraph where it stood.

| | |
|---|---|
| `Tab` / `Shift+Tab` | Next cell, previous cell, selecting what is in it; `Tab` in the last cell adds a row |
| `Ctrl+Tab` | A tab character inside a cell |
| `Alt+A` | The Table menu; its rows are disabled outside a table, and say so |

### Pictures

Click a picture to select it — a picture just put in is selected already.
Drag its body to move it, a corner to resize it keeping its shape, an edge to
stretch one axis, and press `Delete` to remove it. `Esc` lets it go. The
whole drag is one undo. While one is selected a strip under the toolbar says
its size and offers `Size…`, `Align ▾` (left, centre, right — an inline
picture is aligned with its line, an anchored one on its own), `Original
size` and `Delete`.

Only the size and the position can be changed. Everything else about a picture —
crops, effects, rotations — is kept exactly as it was, because those are not
things this can write back, and showing an edit that a save would throw away
would be worse than not offering it.

### Reviewing

| | |
|---|---|
| `Ctrl+Shift+E` | Track changes on or off |
| `Alt+F7` | Next change |
| `Alt+Shift+F7` | Previous change |
| `Ctrl+Alt+M` | New comment — on the selection, or on the word at the caret; written in the Review pane, `Ctrl+Enter` posts it |
| `Alt+Shift+C` | Review pane |

Tracked changes and comments are editable, not merely preserved: accept, reject,
reply and resolve all work and are written back.

### The view

| | |
|---|---|
| `F6` / `Shift+F6` | Move the keyboard round the window: the document, the Navigate pane, the Review pane, the find bar, the toolbar — skipping what is not open. In a pane the arrows walk the rows and `Enter` goes there |
| `Esc` | From a pane, the find bar or the toolbar: back to the document, closing nothing. In the document: close the header or footer first, then the find bar |
| `Ctrl+Shift+8` | Formatting marks |
| `F9` | Update the table of contents |
| `Shift+F10` | The page's right-click menu, at the caret. Its rows are the menus' own: Cut, Copy, Paste, Paste Unformatted, the three emphases, Paragraph, Styles, New Comment, Select All — with Open Hyperlink and Copy Link Address first on a link, a Table submenu first in a table, Accept and Reject first on a tracked change, and a menu of its own on a picked picture |
| `Ctrl+Shift+V` | Paste the board's text without its formatting |
| `Ctrl+G` | Go To: a small box on the status bar's page count. A page number, or `+3` and `-2` from here, and `Enter` goes; `Heading ▾` lists the headings. Clicking the page count opens the same box, and clicking the word count opens Word Count |

Help ▸ Keyboard Shortcuts… lists every key above, generated from the same
table the keys are read from; Help ▸ User Guide opens this file from beside
the program; Help ▸ About says the version and the licences.

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
