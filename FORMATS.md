# What these applications understand

Written so that you can decide, before trusting a file to them, whether they will
give it back intact. Nothing here is aspirational: every "yes" is checked by a
test, and every "no" is a thing that is genuinely not implemented rather than a
thing that is nearly done.

Three columns, because a file format has three different questions:

- **Read** — the feature is understood and shown on screen.
- **Write** — a change you make to it is written back.
- **Preserved** — if you do not touch it, it survives a save untouched. This is
  the column that matters most, and it is `yes` for very nearly everything,
  including features in the "not read" rows: an unread feature is *copied*, not
  dropped. See [DESIGN.md](DESIGN.md) §3.

---

## Files

| Format | Calx | Scriva | Notes |
|---|---|---|---|
| `.xlsx` / `.xlsm` | read + write | — | The main format. Saves by splicing, never by reprinting. |
| `.xltx` | read + write | — | Opens as a template. |
| `.xls` (Excel 97–2003) | read only | — | Save as `.xlsx`. Writing one would mean rebuilding every byte offset in a memory image; a single wrong offset makes a file Excel opens as something else. |
| `.csv` / `.tsv` | read + write | — | Delimiter, quoting and encoding are detected and written back the same way. |
| `.docx` / `.docm` | — | read + write | The main format. |
| `.dotx` | — | read + write | Word's own 41 shipped templates are part of the test corpus. |
| `.doc` (Word 97–2003) | — | read only | Opens as a copy, saves as `.docx`. Same reason as `.xls`. |
| `.odt` / `.ott` (OpenDocument) | — | read only | Opens as a copy, saves as `.docx`. The container preserves every part of one and is checked doing so; what is missing is the writer, not the understanding. See below. |
| `.md` | — | read + write | Headings become real heading *styles*, not bold text. |
| `.txt` | — | read + write | Encoding and line endings are kept as they arrived. |
| `.pdf` | no | no | Dropped from scope. |

---

## Calx — spreadsheets

| Feature | Read | Write | Preserved |
|---|---|---|---|
| Cells: numbers, text, booleans, errors, dates | yes | yes | yes |
| Formulas — 277 functions | yes | yes | yes |
| Dynamic arrays and spilling | yes | yes | yes |
| Defined names, including sheet-scoped | yes | yes | yes |
| 3-D references (`Sheet1:Sheet3!A1`) | yes | yes | yes |
| Shared strings | yes | yes | yes |
| Number formats, including custom | yes | yes | yes |
| Fonts, fills, borders, alignment | yes | yes | yes |
| Themes and theme colours | yes | — | yes |
| Conditional formatting | yes | yes | yes |
| Data validation | yes | yes | yes |
| Merged cells, frozen panes, split panes | yes | yes | yes |
| Row and column sizes, hiding, grouping | yes | yes | yes |
| Sort, filter, autofilter | yes | yes | yes |
| Sheet add, delete, rename, reorder, tab colour | yes | yes | yes |
| Comments and notes | yes | yes | yes |
| Charts | yes, rendered | — | yes |
| Pictures | yes | insert | yes |
| Pivot tables | yes, as read | — | yes |
| Macros (VBA) | no | no | **yes** — preserved verbatim, never executed |
| External workbook links | no | no | yes |
| Printing | no | — | yes |

## Scriva — documents

| Feature | Read | Write | Preserved |
|---|---|---|---|
| Paragraphs, runs, and the four-layer style resolution | yes | yes | yes |
| Direct formatting, including all 13 toggle properties | yes | yes | yes |
| Styles: paragraph, character, table, numbering | yes | yes | yes |
| Numbering and multilevel lists | yes | yes | yes |
| Tables, including nested and spanning pages | yes | yes | yes |
| Sections, headers, footers, first/even/odd | yes | yes | yes |
| Footnotes and endnotes | yes | yes | yes |
| Fields (`PAGE`, `NUMPAGES`, `DATE`, `TOC`, …) | yes | yes | yes |
| Table of contents | yes, generated | yes | yes |
| Bookmarks and hyperlinks | yes | yes | yes |
| Tracked changes | yes | yes, editable | yes |
| Comments | yes | yes, editable | yes |
| Pictures, inline and anchored | yes, drawn | move, resize, delete | yes |
| Shapes, text boxes, SmartArt, WordArt | no | no | **yes** — kept whole |
| Equations (OMML) | no | no | **yes** — kept whole |
| Content controls | as content | yes | yes |
| Text wrap around a floating picture | no | — | yes |
| Column balancing | no | — | yes |
| Printing (Ctrl+P, system dialog) | — | yes | — |
| PDF export, fonts embedded | — | yes | — |

---

## OpenDocument, and why it opens as a copy

ODF is read against the OASIS standard rather than against a running
application: v1.4 became an OASIS Standard on 6 October 2025, and every decision
in `wp-odf` can cite a clause instead of a measurement. What is *rendered* is
still held to a running application — LibreOffice, the implementation ODF is
defined against in practice — by `cargo xtask compare`.

**What is read.** The text, with its paragraphs, headings and spans. Both
stylesheets, the named and the automatic, kept as styles rather than flattened
into the paragraphs that point at them — direct formatting in ODF *is* a style,
and turning it back into direct formatting would lose the difference between a
run that states twelve points and one that inherits them. The page: size,
orientation, margins, columns, and the header and footer of every master page.
Lists, both the definitions and the nesting that says which level a paragraph is
at. Tables, with their columns, repeated columns, spanned and covered cells, and
borders. Frames and the pictures in them, whether the package holds them as
files or the frame carries them as base64. Footnotes and endnotes. Bookmarks and
links. Tab stops. The faces the document names, and the ones it carries.

**What is not read.** Change tracking, forms, embedded objects, charts, and the
drawing layer beyond a frame holding a picture — a watermark written as a custom
shape is one of those, and it is the visible gap on a page that has one.

**What is preserved.** Everything. `wp_odf::Container` reads every entry of the
package into memory, filters none of them, and writes them all back — including
an entry the manifest gives no media type, which Word's own ODF export leaves
behind and which a reader that trusted the manifest would drop. `cargo xtask
fidelity` holds it to that like any other container.

**Why it is not written.** Not because the container cannot: it can, and the
round trip is checked. Because nothing yet rewrites `content.xml` a paragraph at
a time. Reprinting the part whole would pass a test that the edit came back and
fail the one that matters — that everything the reader does not model came back
too. A save that quietly loses what it did not understand is the one thing this
project exists to prevent, so until the writer splices, an edited `.odt` is
saved as a `.docx`, and the application says so when it opens one.

---

## What a `.doc` gives up

The legacy reader is the one place where "preserved" does not apply, because
nothing is written back. It reads:

text (through the piece table), the body separated from headers, footers,
footnotes, endnotes and text boxes, paragraphs, tables, direct character and
paragraph formatting, style names, and the first section's page setup.

It does not read: pictures, drawings, fields, revision marks, table geometry
(column widths, borders, cell shading), the style definitions themselves, or the
page setup of the second and later sections. A `.doc` opens as a *copy* and says
so, which is why the title bar shows a `.docx` name from the moment it opens.

---

## The known gaps, stated plainly

1. **Text does not wrap around a floating picture.** The picture is drawn in the
   right place; the text runs behind it rather than around it.
2. **One heading in one Office template draws past the right margin.** It does
   not reproduce under the arithmetic shaper the layout tests use, which is a
   limit of the tests as much as of the layout.
3. **A table row splits across a page break only where every cell agrees.** The
   row is cut between the lines of its cells, at a height that is a line
   boundary in all of them at once — which is the ordinary case, since most rows
   have one column of text and the rest short. Where two columns of text line up
   on nothing, the row moves whole instead: Word would cut each cell at its own
   line boundary and leave the halves of one row at different heights.
4. **Pivot tables are shown as they were last calculated by Excel**, not
   recalculated.
5. **A large document is laid out in full on every edit.** Eight thousand
   paragraphs cost about a third of a second; a twenty-page document is
   imperceptible. Incremental layout is not implemented.
6. **No printing in Calx.** Scriva prints through the system dialog and
   exports PDF; the spreadsheet's page-setup model (print areas, scaling,
   repeat rows) is its own project and has not been started.

---

## How the "preserved" column is checked

`cargo xtask fidelity` runs two checks over every file in `corpus/`:

1. Open and save with no edits. Every part must be byte-identical.
2. Open, edit one thing, save. Every part except the one holding the edit must be
   byte-identical, and the edit must read back.

Both are green on all 27 corpus files. The corpus is real files produced by Word
and Excel — not files this project generated, which would only prove it agrees
with itself. Word's own 41 shipped `.dotx` templates are a second, independent
corpus, and every reader and writer bug found in the document phases came from
them rather than from ours.
