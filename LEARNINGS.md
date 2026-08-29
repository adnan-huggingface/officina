# Learnings from Calx

Calx is finished as far as the audit reaches: 72 commits, 911 tests, 271 formula
functions, 35 audit findings. This file is what came out of it that is worth
carrying into Scriva.

`PROGRESS.md` keeps the watch items that are specific to spreadsheets — `spans`
is a sixteen-row hint, `FONT` index 4 does not exist, theme index 0 is `lt1`.
Those stay there. What follows is the part that is not about Excel, and that
will therefore be true again in Word.

---

## 1. Preservation is a mechanism, not an intention

**The vault was the right first chunk.** C1 was built before anything could open
a file, and every chunk since has been able to say "we do not support X" and mean
*X survives untouched* rather than *X is gone*. Retrofitting that after a reader
exists is not possible: the reader would already have thrown the bytes away.

**Classify parts, then classify inside them.** Modeled / Retained / Derived is
the outer half. The inner half — unknown elements and attributes captured as
opaque nodes on their parent and re-emitted in document order — is what makes
"modeled" survivable. A modeled part is never fully understood.

**Never author what you cannot read back.** Removing a part means removing its
`.rels` companion and its content-type override in the same breath; an override
naming a part that is not there is not a missing feature, it is a package the
host application calls damaged. Conversely, an *orphaned* part is untidy and
opens fine — so `remove_part` deliberately does not follow the removed part's own
relationships. Untidy beats invalid, every time.

**Relationship ids count past the highest in use, not past the count.** Reusing
a freed id makes a stale reference elsewhere resolve to the wrong thing rather
than to nothing, which is much worse.

**Preserve what you cannot verify.** Sheet protection's password hash is stored
and never interpreted, because a hash we cannot check is not a lock we may open.
The same rule will apply to Word's document protection and to every checksum,
signature and rsid in a docx.

## 2. Reading a format

**Walk the relationship graph; never assume a path.** `/xl/workbook.xml` is a
convention, not a rule. Doing it properly is what made Strict-profile and
third-party files open with no extra work.

**Every quirk is invisible until it is wrong, and it never looks like a bug in
the reader.** The pattern repeated so many times it is worth stating as a law:

| What was wrong | What the user saw |
|---|---|
| `cellStyleXfs` and `cellXfs` read into one list | the whole sheet formatted plausibly wrongly |
| theme colours read in file order | headings painted white on white — "the text vanished" |
| `showDropDown="1"` taken at face value | validation dropdowns missing |
| a `dxf`'s colour read from `fgColor` | every conditional format uncoloured |
| `tint` scaled in RGB instead of HSL | a colour of the wrong hue, invisible alone |
| BIFF `FONT` index read as a plain index | every cell drawn in the wrong font |
| shared-string `CONTINUE` payloads joined | mojibake from the first split onward |

None of these raised an error. Not one. A format reader's failure mode is a
document that opens and is quietly wrong, so the only defence is comparing
against an independent implementation (§5).

**An inverted or absent attribute is normal.** `showDropDown` hides. A row with
no `ht` is not a default-height row — it is one the host measures at paint time.
Absence usually means "compute it", not "use the default".

**Entities are not text.** quick-xml 0.41 hands `&amp;` back as a separate
`Event::GeneralRef`, so any accumulator that matches only `Event::Text` silently
drops every `&`, `<` and `>` in the document. One helper (`xml::push_text`) is
the whole answer, and every text accumulator in Scriva must go through it.

**Parse attributes in one pass.** quick-xml's per-lookup duplicate check is
quadratic and re-walks the tag. Three lookups on `<c>` versus one pass with
`with_checks(false)` was 3.6 s to 2.1 s on a 1.3M-cell file — a bigger win than
everything in the store, the value building and the interning put together.

## 3. Writing a format

**Edit the file; do not reprint it.** This is the single most valuable technique
in the codebase. `write::splice` is an XML reader that hands back each event
*and the span of source bytes it came from*; copying the span rather than
re-serializing keeps the producer's whitespace, its `<c/>` versus `<c></c>`, and
its entity escaping. The writer then replaces only what actually differs from
what the file says.

The consequence is that a part nobody touched goes back byte for byte, and an
edited element keeps its own start tag — so attributes we never modeled (`cm`,
`vm`, `ca="1"`) ride through an edit instead of being dropped.

**Write back only what changed, and check first that anything did.**
`reconcile_sheets` does nothing at all when nothing structural differs. That is
what keeps the no-edit fidelity check honest rather than merely passing.

**Round-trip the file's own element when the model is a partial view.** An
`<autoFilter>` can hold five things we do not model, so the writer parses the
file's element back, and returns the *original bytes* whenever the model agrees
with it. Only something the user actually changed gets rebuilt. Same shape for
`<tabColor>`, so a themed colour is not flattened to the rgb it resolves to.

**An element the model needs and the file lacks belongs in sequence.** Appending
a `<col>` after the file's own spans is legal and every reader copes — and no
spreadsheet has ever written them out of order, which is reason enough not to be
the first. Find the file's spans in one pass ahead of the rewrite and thread the
new ones between them.

**Index-addressed tables are append-only.** Styles, fonts, fills, borders: the
only mutation is "look it up, change a field, ask for the style that has that
look". Nothing is edited in place and nothing is inserted, because a font
inserted at index 2 re-letters every cell past it.

**Write beside the target and rename over it.** A refusal, or a crash mid-write,
must not be able to leave a half-written document. And a save refused by the OS
because the file is open elsewhere is not a bug in the save — say so, at the
moment it happens.

## 4. Modelling

**The model is a view over the package, not a replacement for it.** Reading
leaves parts `Retained`; the model carries indices into the file's tables rather
than copies of them. Everything else follows from this.

**Store what the file stores, in the shape the file stores it.** `Axis` is a
default plus a sorted exception list with running totals — one binary search for
"where does row 900,000 start?" — which is also exactly how the file spells it.
When the model's shape matches the file's, the reader and the writer are both
trivial and nothing is O(document).

**Two coordinate systems is one bug waiting.** Grid positions are `f64` and
screen positions are `f32`, because above sixteen million an `f32` counts in twos
and a cell would be painted where it could not be clicked. Word has *four* unit
systems in one file — twips, half-points, EMUs, eighths of a point — and mixing
them will be the same class of bug. Newtypes, and convert exactly once.

**Zero-based inside, one-based at the display boundary. Convert once.**

**Undo is a value, not a second implementation.** Applying a change returns the
change that undoes it, so redo is the undo of the undo and the two directions
cannot drift apart.

**Undo cost is bounded by what changed, not by what it changed inside.** A sort
is a permutation, and its inverse is the inverse permutation — a list of row
numbers, not a copy of 1.33M cells. That one insight took a sort's undo from
100 MB to half a megabyte. Build the inverse *first*, because building it is the
check that the operation is invertible at all.

**One place where changes land.** `perform` is where protection is enforced and
where every guard belongs. `blank_slate` is the one place a document is emptied,
so New and Close cannot drift. When two paths can answer the same command, they
eventually answer differently.

**Structural edits are document-wide.** A formula on Sheet2 is just as wrong
after Sheet1 gains a row. Scriva's equivalents — bookmarks, cross-references,
comment anchors, revision ranges — will all need the same sweep.

## 5. What tests do not catch

This is the section that cost the most to learn.

**Every test can pass while the file is still wrong.** The `<cols>` formatting
gap survived a green workspace *and* a green fidelity run, because the model
round-tripped through itself perfectly and nothing ever asked the file. A
self-consistent system proves nothing about the outside world.

Three answers, in increasing strength:

1. **Read the output with an independent implementation.** openpyxl found the
   `<cols>` bug in a minute. Scriva has `python-docx`, and Word itself.
2. **Use the document's own cached answers as a conformance suite.** Every
   formula cell in an xlsx carries the value Excel last computed for it, so the
   corpus was a conformance suite nobody had to write — 30 of 30 matched, and
   the skips are stated by rule rather than by hard-coded cell. Word's equivalent
   is weaker but real: `docProps/app.xml` carries page and word counts, and a
   `<w:lastRenderedPageBreak>` is Word's own opinion of where a page ended.
   That is the closest thing to a pagination oracle that exists.
3. **Regenerate everything, not just what changed.** `flush_regenerating` writes
   every cell out of the model and compares — a save never does this, but it
   found three divergences the ordinary pass could not see.

**Ask the painter, not the model.** Finding 28 was a bar chart drawing every bar
upside down, which egui fills with nothing. No bar chart had ever shown a bar. It
survived a whole chunk, an audit, and an insert-a-chart task, because every test
asked the model and none ran a frame and read the shapes back. Scriva's layout
engine is *entirely* this risk: a test asserting line breaks from the model is
asserting nothing about the page.

**Hand-built fixtures test what you understood.** The BIFF fixtures were laid out
byte by byte from the record layouts, and they were green — while a real file
proved the shared-formula column offset was eight bits, not fourteen. Fixtures
catch regressions; only a document written by the real application catches
misunderstandings.

**Audit by writing down what the real application does, before looking at what
yours does.** `AUDIT.md` is eighteen areas of concrete claims about Excel written
in advance. Twenty-seven mismatches came out of it, and **most of them were about
the mouse and the keyboard, not the file format** — right-click collapsed the
selection, Ctrl+A always took the whole sheet, End did nothing, there was no
drag-move, no marching ants. None of that is visible from a test of the model,
and all of it is what using the application *is*.

## 6. Performance

**Measure before deciding what is slow.** Saving a real workbook took 5 s, and
2 s of it was five questions — tab colour, panes, autofilter, protection,
conditional formats — that each walked 1.33M cells to reach an element that
cannot be among them. One copy of the part with the body emptied answered all
five in kilobytes.

**The cost is nearly always allocation in a loop, or a lookup repeated.**
- Case folding in a comparator: two allocations × 2M comparisons. Folded once
  per row into the sort key, 182 ms became 20.
- Ten columns of a row live in one chunk, so one map lookup answers for all ten;
  going through `get` and `set` paid for it ten times.
- Deciding once per *style* whether a style can want a taller row, instead of
  once per cell: 130 ms became 40.

**Decide per-kind, not per-instance.** That is the same insight three times.

**Keep the escape hatch honest.** `CALX_BIG` points a test at a real file and it
prints where the time goes. Guesses about performance were wrong every single
time; that harness was right every time.

## 7. The UI

**A theme that suits one surface ruins another.** The shell draws buttons flat
and borderless, which is right for a toolbar and makes a dialog built from
`ui.button` look unfinished. So dialogs go through `ui_kit::dialog` and controls
that must read as inputs scope their own visuals. Any widget that must look like
a *control* has to say so.

**Icons are drawn, not typed.** `⏴`, `⏎`, `↶` are in Unicode blocks that Arial,
Segoe UI and egui's own fonts all decline to cover, so they render as hollow
boxes — silently, with everything compiling and green. Draw them from lines.

**A popup measures itself in a pass where `available_width` is the screen.**
Anything inside a menu that asks costs you a 700-point-wide File menu. Guard with
`ui.is_sizing_pass()`.

**Consuming a key leaves the text event beside it.** `consume_key` removes
`Event::Key`; anything accepting typing reads `Event::Text`. A mnemonic has to be
*taken*, not read, or Alt+V P splits the panes and types "p" into the cell.

**Every command through one enum and one dispatcher.** The menu and the toolbar
cannot answer the same command differently if there is only one answer.

**The canvas is paper, whatever the chrome does.** A cell with no fill is white.
Tinting the document surface to match a dark application theme shows the user a
document they did not make, and makes light-themed content vanish. Scriva's page
is white on a grey desk, and no theme touches it.

**A window can open bigger than the screen**, and the part that goes missing is
the bottom — which is where the tabs and the status line live. `with_maximized`
is dropped when an explicit inner size sits beside it, and stated sizes are
logical pixels. Send `Maximized` for the first few frames; the very first is too
early.

**State the affordance.** A boundary that can be dragged must change the cursor,
and a drag has to name its own cursor for its whole duration — hovering cannot
answer for it, because the pointer leaves the boundary the moment the drag
begins.

## 8. Process

**Chunks with an exit criterion, and a handoff note that is updated before the
session ends.** `PROGRESS.md` is the reason work resumed cleanly across dozens
of sessions. The watch items in it have paid for themselves repeatedly.

**Log the limits, out loud, next to the feature.** "3-D charts are drawn flat",
"Excel itself has not opened the files with authored charts", "icon sets are not
drawn". A stated limit is a decision; an unstated one is a bug someone else finds.

**Run `cargo fmt --all` after any scripted edit.** Edits made by script do not
run the formatter, and it had drifted across twenty files before anyone checked.

**Driving Office through COM in-process hangs.** Not a licensing problem, though
it looks like one: `Documents.Add()` never returns and Word spins at 100%. The
identical calls in a child process work, so the corpus generator runs every
document under `Start-Job` with a timeout. This matters more for Scriva than it
did for Calx.

**Windows PowerShell 5.1 reads BOM-less `.ps1` as ANSI**, so any script with
non-ASCII must be saved UTF-8 *with* BOM. And `[char]0x41 + [char]0x42` is `131`,
not `"AB"`.

---

## 9. What this predicts about Scriva

- **The layout engine is where the untested-painter risk concentrates.** Calx
  could check itself against Excel's cached values; Word's cached opinion is only
  `<w:lastRenderedPageBreak>` and the app.xml counts. Build the line-break and
  pagination tests to read *the laid-out result*, from the first day, or the same
  class of bug as the upside-down bar chart will live for months.
- **Style resolution is Word's version of the styles.xml index trap.** Four
  layers — docDefaults, the style chain through `basedOn`, numbering, direct
  formatting — each with its own toggle semantics (`w:b` with no `w:val` means
  on; a toggle inherited twice cancels). Resolve lazily, cache by identity, and
  test the resolution order directly rather than through rendering.
- **Word's units will be the `f32`/`f64` bug again.** Twips, half-points, EMUs,
  eighths of a point, and percentages in fiftieths. Newtypes.
- **The revision and comment layers must be preserved before they are editable**,
  which means the model has to carry `w:ins`/`w:del`/`w:moveFrom` ranges and
  their ids from C16, not from C24. A reader that flattens them destroys them.
- **`document.xml` is one part and the whole document**, so splice-writing it is
  higher-stakes than splicing a worksheet, and the win is bigger — settings,
  rsids, `w:proofErr`, content controls and every vendor extension ride through.
- **The corpus is fifteen documents generated through real Word.** It is clean by
  construction, and clean is where preservation bugs do *not* live. Real-world
  documents with the accumulated oddities of many Word versions are the gap, and
  the audit-by-claim method from §5 is what covers the rest.

---

## 10. What phase 3 added to the list

Written after building Scriva through C22, so the next phase inherits it.

- **Microsoft ships a corpus with Office.** `C:\Program Files\Microsoft
  Office
oot\Templates` holds 41 finished `.dotx` files — 604 paragraphs, 278
  content controls, tables, floating art, glossary parts — written by Word's own
  designers rather than by a script of ours. Every reader and writer bug found
  in C17 and C22 came from them and none from our own corpus. This is the
  SOLVSAMP.XLS move, and it is available for every format Office writes.
- **A test that measures with arithmetic cannot see a fault that needs real
  metrics.** Making the shaper a trait was right — it is what makes line
  breaking testable at all — and it draws a line around what those tests can
  say. One heading of a real template overflows its page in the running
  application and lays out correctly under the fixed-width shaper. Layout needs
  *both* kinds of test, and the second kind is a screenshot.
- **A reader must not add content, ever.** Repairing a malformed cell on read
  looked defensive and was destructive: the invented paragraph pushed every
  later paragraph into the wrong cell on save. Repair on *author*, never on
  read — the file is what has to round-trip.
- **Byte offsets from an XML reader are not byte offsets in the file.** A UTF-8
  BOM is skipped and not counted, so every span is three bytes short. Copying
  spans hides it completely, because they still tile the input; the damage
  appears the first time one is replaced.
- **Preserve inside a modelled part, not only around it.** A `<w:drawing>` is a
  whole DrawingML document. Parsing out the four fields layout needs and
  re-emitting from them destroys the rest — so the element's bytes are carried
  in the model and written back verbatim. The Preservation Vault, one level in.
- **A shadowed loop variable is a real bug, not a style problem.** `index` for
  the paragraph and `index` for the line meant every laid-out line reported
  paragraph zero, so every click landed in the first paragraph of the document.
  The compiler is happy; only a test that clicks catches it.
- **The lessons in this file do not apply themselves.** §7 says toolbar icons
  are drawn rather than typed, and the first Scriva toolbar shipped `⯇` and `↶`
  and came out as a row of hollow boxes. Reading it is not the same as checking
  against it.

## 11. What phase 4 added to the list

- **Preservation and editing are not opposites, but keeping them apart costs
  the user the edit.** A paragraph holding a picture was excluded from the
  writer so the picture would survive. That was right and it was also why
  dragging a picture did nothing on save: the model changed, the file did not,
  and the document looked correct until it was reopened. The fix is to splice
  *within* the preserved bytes — rewrite the four numbers the editor can change
  and copy the rest. Preserve what is not understood, not what is not modelled.
- **The number a format writes is not the number the specification leads with.**
  Word spells direct bold in a `.doc` as `129` — "invert what the style says" —
  and never as `1`. A reader built from the specification alone handles `0` and
  `1`, opens a real document, and shows it with no formatting whatever and no
  error to explain why. Only a file written by the real producer says which of
  the legal spellings is the one that happens.
- **A read-only format needs a way out or it is a museum piece.** Reading a
  `.doc` is worth nothing if the words cannot be got into a file that can be
  written. That made authoring a package from nothing a prerequisite rather
  than a nicety — and it turned out to be what a *new document* had needed all
  along, which had been failing with an apology since C22.
- **One code path for writing, even when there is no file.** The authored
  package holds an empty body, and the paragraphs are put in by the same splice
  writer that edits a document Word wrote. A separate "write it all out" path
  for new documents would be a second writer, tested half as much, diverging
  quietly.
- **A test that measures with arithmetic still cannot see what needs metrics.**
  §10 recorded this; phase 4 did not fix it. It is the one open item from
  Scriva that a test suite of 1341 cannot close.

## 12. What phase 5 added to the list

- **The known slow thing was slower than the note about it said.** C6 recorded
  "`dependents_of` is a linear scan, so building the order is quadratic". True,
  and it undersold it: the *sort* inside the topological loop was n² log n, and
  the cycle check was a linear search of a growing vector. A performance note
  written while building something is a hypothesis. Measure before believing it,
  and measure again after fixing what it named.
- **Tests get slow the same way products do.** One test rebuilt a dependency
  graph inside a doubly-nested loop and cost 117 seconds of every single run —
  more than the entire rest of the suite. Nobody noticed because a slow test
  suite is a background ache rather than a bug. `cargo test` printing "has been
  running for over 60 seconds" is the only reason it was found.
- **An index whose buckets are too big is the scan again, wearing a costume.**
  The first version bucketed at 256 cells square, which put every formula in a
  dense column into one bucket and made the lookup a scan of 256 candidates.
  Dropping to 64 and storing the *area* beside the node — so a candidate costs a
  containment test rather than a map lookup — was 16× on top of the first fix.
- **Do a thing twice only when the second time can differ.** Scriva laid every
  document out twice, because `{ PAGE }` cannot be resolved until the pages
  exist. Correct on the first layout, and pure waste on every keystroke after —
  the page number was already right. Two passes is a fixed point iteration, and
  a fixed point iteration should stop when it stops changing.
- **Write the documentation from the source, not from memory.** Every key in the
  user guide was read out of the menu definitions, and the function count was
  counted rather than recalled. The number in the first draft was wrong by
  seventeen.
- **A report is worth more generated than written.** `FIDELITY.md` is produced by
  the harness that does the checking, so it cannot drift from what is true. A
  hand-written claim that "all 27 files round-trip" would have been correct on
  the day it was written and unfalsifiable afterwards.

## 13. What one real document added to the list

Everything above was learned from a corpus. This was learned from a file off
somebody's desk, and it is a different kind of lesson.

**A corpus of one producer tests one producer's dialect.** Twenty-seven files,
every one written by Word or Excel, all passing — and a `.docx` that had been
through Google Docs was unreadable. Not subtly wrong: every table column
collapsed to a single character per line.

**Be liberal about the *type* of a number and strict about its absence.**
`ST_DecimalNumber` is an integer and Word has never written anything else, so
`"10397.0"` was refused, and refusing it produced a zero — which is a real width
meaning "collapse this". Returning `None` for absent is worth keeping; refusing
a value that is plainly a number is not. Round it.

**A revision record can contain a whole copy of what it replaced.**
`<w:tblGridChange>` holds a complete `<w:tblGrid>`. Any reader that descends
into unknown children on the way to a known one will read the past as if it were
the present. Skip what you do not recognise; do not walk through it.

**A variant that carries no data is a variant a renderer can only skip.**
`Content::Label` said "a bullet goes here" and nothing more, so the painter
matched `Content::Text`, fell through, and drew nothing — for the entire life of
the project. The layout was correct throughout: it measured the label, reserved
its width, indented the text. Nothing in the tests or on the screen pointed at
the hole, because an indented list looks like a list. **Make the data structure
carry what the consumer needs, or the consumer will silently do without it.**

**The same content laid out twice is two questions, not one.** A footer holding
`{ PAGE }` is laid out again for every page. Keying the answer by paragraph and
ordinal alone gave every page the last page's number. The identity of a field
result includes *which instance of the band* it was drawn in.

**A height that is a sum of parts is rarely the height of the whole.** A
footer's height was the sum of its placements, and a table row of three cells
counted three times. Ask the thing that stacked them how tall the stack is.

**Splitting a shared edge into pieces is a rendering problem.** Once a table row
is laid out in bands so a page can break inside it, its side borders arrive one
band at a time. Abutting anti-aliased segments leave a hairline of paper between
them, and a ruled column comes out dotted. Overlap by half the stroke.

**A namespace-blind parser will bless a file Word refuses to open.** The
section writer put `r:id` on a `headerReference` and the root, authored from
blank, declared only `xmlns:w` — an unbound prefix, and Word rejected the
entire package with a permissions-shaped error that says nothing about XML.
Every test passed, because quick_xml reads prefixes as spelling rather than
as bindings; only opening the file in the real application caught it. When a
writer emits a prefixed attribute, it must own the declaration — putting
`xmlns:r` on the element itself is legal, harmless when the root declares it
too, and correct when nothing else does.

**A document chart needs no workbook behind it.** Every chart Word writes into
a `.docx` carries a `<c:externalData>` pointing at an embedded xlsx, so it was
natural to assume the reference was load-bearing. Measured, it is not: a chart
part holding only its caches — no embedded workbook, no colour part, no style
part — opens in Word without complaint, is counted as a chart by its object
model, and renders every bar from the cached values. What the embedded workbook
buys is Edit Data, nothing else. That is what makes a chart clipboard between
two applications cheap: the `<c:chartSpace>` alone is the whole payload, since
`xl/charts/chart1.xml` and `word/charts/chart1.xml` are the same element. One
cosmetic note from the same probe: a series that states no fill is coloured by
the reader's own defaults, and Word's default varies the colour per point where
Calx paints one blue — stating the fill is what pins the look across readers.

**Excel classifies a scatter by its series' lines, not by its stated style.**
A `scatterChart` authored with `<c:scatterStyle val="marker"/>` and nothing on
the series read back through Excel's object model as `xlXYScatterLines`, and
its export drew the joining lines: silence on a scatter series means an
*automatic* line, however the style protests. Real Excel spells markers-only
per series, `<c:spPr><a:ln><a:noFill/></a:ln></c:spPr>`, and once the writer
did the same the type read back as asked. The same probe confirmed two
guessed conventions against Excel's own render of our parts: a bar chart laid
on its side puts its first category at the *bottom* and its first series
nearest the axis — a column chart turned anticlockwise, both ends — and a
radar runs its first category from the top, clockwise.

**egui's widget colours are shared in ways a theme has to respect.** A
`WidgetVisuals::bg_fill` paints a checkbox's box *and* a slider's rail; a
`Visuals::selection.bg_fill` paints a selected row, a selected tab *and* a
slider's travelled part; `bg_stroke` edges a button, a combo and a checkbox
alike. A look set for one widget is set for all of them, so a theme made
flat for a toolbar strips the boxes off every form, and a rail greyed for a
slider greys every checkbox. The way through is scope: the toolbar sets its
own flatness, a form its own edges, a slider its own rail, each on the `Ui`
that holds it (2026-08-22).

**An egui popup is sized before it is measured.** A `ComboBox` list longer
than `Spacing::combo_height` scrolls, and asking for more height does not
give it: the popup's `Area` is laid out in a sizing pass bounded by
`Spacing::default_area_size` (400 points tall) before its content has been
seen, and the scroll area inside shrinks to that. A list that must show
whole has to be short — about ten rows — which is a reason to split a long
choice into two short ones rather than a limit to work around (2026-08-22).

**A series has three places for a colour, and only one of them is the
series'.** `<c:ser>` carries its own `<c:spPr>`, and so does every `<c:dPt>`
(one point's override — Excel writes one per slice of a pie) and every
`<c:dLbls>` (the labels' ink) inside it. A reader that takes the first
`srgbClr` it meets anywhere below `<c:ser>` colours a whole series with its
first slice, or with its labels' text. The series' colour is the `spPr` that
is a *direct child* of `<c:ser>`, and a writer changing it replaces exactly
that one — `dPt` overrides stand, as they do in Excel (2026-08-22).

**A chart part's silences are Excel's defaults, and they are not the
schema's.** Fourteen charts authored by Calx, each stating only its type, its
series and its axes, opened in Excel as fourteen different charts: the line
chart came up *smoothed* with a diamond and a square on every point, the
one-series scatter gave every point its own colour and shape, every axis grew
ticks crossing it at every major and minor unit, and none of them had a
gridline. None of that was in the part. A missing `<c:smooth>` is smoothed;
a missing `<c:varyColors>` varies; a missing `<c:marker>` on a series is an
automatic marker; a missing `majorTickMark` is `cross` and a missing
`minorTickMark` is too; a missing `holeSize` on a doughnut is *no hole at all*
(measured from Excel's export of the part: rings from the centre outward,
where the schema says ten percent); a missing `crossBetween` is `midCat` for
an area and `between` for everything else; and a missing `overlap` on a
*stacked* bar chart is nought — the series stand side by side, each bar's
foot on the one before's total, which the first pass of this comparison
looked straight at and let go. Excel's Insert writes `overlap="100"` on
every stack. Excel's own Insert writes every one
of these out, which is why nobody sees the defaults until a second producer
leaves them blank. Two rules follow. The reader fills a silence the way Excel
does, so what Calx draws from a sparse part is what Excel draws from it. The
writer never leaves one: a chart Calx inserts says `varyColors="0"`,
`<c:smooth val="0"/>`, `<c:symbol val="none"/>`, ticks `none`, gridlines up
the value axis and a seventy-five percent hole, and states them on the model
too, so the chart drawn the moment it is inserted is the chart Excel draws
from the saved file. The same afternoon pinned two conventions of Excel's
layout: a stack's legend reads top down (the series on top first) and a bar
laid on its side reads bottom up, and one that is both is back in order; a
doughnut draws its first series as the innermost ring and shares the band
between hole and rim equally among the rest.

**Word's default cell padding lives in the Table Normal style, not in the
table.** Word pads a cell 0.08in on each side — but only because its built-in
Table Normal style says so, and Word writes that style into every document.
A document with no Table Normal — one Scriva authored, or some second
producers — has no such padding, and Word draws its tables' text hard against
the cell edge. A layout that hard-codes the 0.08in as *the* default insets
text a producer never asked for, narrows every cell's text column by a tenth
of an inch, and rewraps paragraphs that were a word from the edge — which is
how one resume's page came out a line taller in Scriva than in Word off a
single bullet. Cell-margin resolution now starts from nothing and takes its
padding from the document's default table style when one exists, so the
padding is Word's wherever Word's style is and absent wherever it is not.

**A centre or right tab positions the text after it, not the pen before it.**
A left tab only advances: it moves the pen to the stop and the next character
starts there, so a reader can lay it out knowing nothing of what follows. A
centre tab and a right tab cannot — they place the run *between this tab and the
next stop* so that its middle, or its right edge, sits on the stop, which means
measuring that run before deciding where the tab ends. This is how a footer sets
a name at the margin and a page number in the middle of the same line, and a
layout that treats every tab as a left tab draws the number half its width to
the right of centre. The run's width for this is its advance, spaces between the
pieces included; only the final piece's trailing space hangs, exactly as it does
when the line measures its own width.

**A paragraph mark is formatting with no text to hang it on.** A blank line has
no run: its height, and the face a caret typing there inherits, come from the
paragraph mark alone. So a pass that sets a whole document in one face has to
reach the mark, and Word's rule for when it does is that the mark counts as
selected once the selection reaches the end of its paragraph. A reader that
formats only runs leaves every blank line standing at whatever size the document
was started in — invisible on screen until the blank lines between blocks add up
and push the last of the text onto a page of its own.

**A header inherits from the body unless something stops it.** Word's built-in
Header and Footer styles hold the line to single and take the space off both
ends, and its galleries write header paragraphs in those styles. A bare
paragraph in a header takes the document's own `docDefaults` instead, so a
document spaced 8pt after every paragraph grows its header by 8pt and pushes its
own text down the page to make room. Whatever writes a header has to state the
spacing, because the thing it would otherwise inherit is about the body.


An embedded font is a fallback, not an override.

A `.docx` may carry the type it is set in, obfuscated, inside the package. It is
tempting to treat that as the authoritative copy — the author put it there, so
draw with it. Word does the opposite: it uses the face the machine has installed
and reaches into the package only when there is none. The difference is not
academic. The demonstration document embeds Ubuntu Mono at 500 units to the em;
the copy installed on this machine measures 560, and preferring the package
re-wrapped three paragraphs and moved a page break. Getting this right needs
something a font stack usually does not have: an index of what is installed, by
the name a document would call it, rather than a table of the dozen faces a
reader expects to meet.

The line gap sits above the baseline, and extra leading sits below it.

A face states three vertical numbers and a renderer has to decide what to do
with the third. Word adds `hhea.lineGap` to the ascent — the baseline sits that
much further down — and when a paragraph asks for one-and-a-half lines, every
extra point goes *below* the type rather than being shared around it. Both
halves were measured rather than derived: three faces, four line multiples, and
the first baseline of a spaced paragraph never moved. A renderer that centres
the extra instead is wrong by half of it on every line of every document, which
is small enough to look like a rounding error and large enough to move a page
break.

A seam in the measuring is not a place a line may end.

A paragraph is cut into pieces for reasons that have nothing to do with line
breaking: a run changes face, a character belongs to another script, a field
begins. If the line filler treats those cuts as break opportunities it will
break at them, and the results are the kind of wrong that looks like a font bug
— a bold opening quotation mark left alone at the end of a line with the words
it opens on the next. The break table has to be asked about the *text*, at every
seam, and not merely within each piece.

A table style is a scheme, not a set of properties.

`<w:tblStyle>` names one entry in `styles.xml`, but what that entry contributes
to a cell depends on where the cell sits — the header row, the last row, the
first column, which stripe. Word keeps each as a `<w:tblStylePr>` and the table
says through `<w:tblLook>` which of them it wants. A reader that takes only the
style's base properties draws every such table as a bare grid, which is what
five tables of the demonstration document looked like. The order the parts apply
in is ECMA-376 §17.7.6 and is not the order they are written in: whole table,
then the column bands, then the row bands, then the first and last column, then
the first and last row, and the corner cells last.

Word's own PDF is not a reliable oracle for text it could not embed.

The rule in adr/0001 is to measure Word rather than read the specification, and
the usual instrument is a PDF exported from Word and read back. That instrument
has a blind spot. Where a run is set in a font Word cannot embed — a variable
font, in this case — Word writes the glyphs of the real face and the widths of a
substitute, so the text layer reports positions the page does not show. Two
lines that matched Word exactly were reported as twenty points adrift. When the
text layer and the picture disagree, the picture is the document.

Word's own Normal does not hand its size to a table cell.

The demonstration document sets its body at the twelve points its `Normal`
style states and every one of its tables at the eleven the document defaults
state — the same paragraphs, the same style, two sizes. The rule is narrow and
was found by varying one file: a size stated by the style named `Normal`, and
only when it is exactly the twelve points Word's own built-in Normal states,
does not reach a run inside a table that names a style. Everything else Normal
says reaches it — its face, its first-line indent, all of it. Rename the style
and the size applies; state twelve points in a style of the document's own,
even one based on Normal, and it applies. A document that states no default
size has nothing to fall back to, so the twelve points stand. Twenty-three
variants of one document, each opened in Word and asked over COM for the size
it resolved a cell to; the table is in `wp_model::style`.

A table style's own `pPr` is the base of its scheme, not the properties of a
paragraph that names it.

No paragraph can name a table style, so the `<w:pPr>` written directly inside
one is not a paragraph style's properties — it is what the style says about
every cell, the base that its `<w:tblStylePr>` bands are laid over. Every
built-in table style writes `<w:spacing w:line="240"/>` there, which is how a
table in a document spaced at one and a sixth comes out spaced singly. Reading
only the bands left the header row right and every row after it two points
too far apart.

A vertically merged cell runs down the rows it spans.

The obvious reading — the row is as tall as its tallest cell — makes the first
row of a merge tall enough to hold all of the merged cell's text, which puts
the rest of the table a row lower than Word does. Word gives each row the
height its own cells need and lets the merged text run on through them; only
the last row of the span grows, and only if what is left of the text needs it.
The same document's nested table draws "Three" beside "Four" for exactly this
reason.

A bullet may be a picture, and the character beside it is a decoy.

`<w:lvlPicBulletId>` points at a `<w:numPicBullet>` in the numbering part's own
list, written as VML: a `<v:shape>` whose CSS `style` states the size and whose
`<v:imagedata>` names the image. The level *also* carries an ordinary
`<w:lvlText>` — a Symbol dot — for a reader that cannot fetch the picture, so a
reader that takes the text and ignores the id draws something plausible and
wrong. The demonstration document says so in words: "This bullet uses an image
as the bullet item", beside a dot.

Two things about it are easy to get wrong. The size is the shape's and not the
image's: the icons Word ships are hundreds of pixels across and go on a line
nine points tall. And the relationship is the *numbering part's*, which numbers
from `rId1` exactly as `document.xml` does — resolving it against the document's
relationships fetches whatever that file's first image happens to be.

And where its rows meet, nothing is drawn.

Between the rows of a vertical merge there is no edge: the reader sees one
tall cell, so Word suppresses both the rule below the cell whose merge carries
on and the rule above the cell that carries it. Only that column goes unruled
— the rest of the row is ruled as usual, and the height the row pays for its
rules is the widest cell's and not this one's. The nested table in the
demonstration document says so in words, beside a cell whose "One" and "Three"
had a line straight through them.

And the paragraph a cell must end a table with is not a line.

A cell may not end with a table, so Word writes an empty paragraph after one
and then gives it no height at all — not the line, not its spacing before.
Put a single letter in it and the row grows by a whole line. It is emptiness
and position together that make it disappear.

The rule between two rows is paid for once, by the row below it.

A row is taller by the rule that bounds it, but the rule between two rows is
one line and both rows have an opinion about it: the row above states a
`bottom` and the row below a `top`. The height it takes is the heavier of the
two and it is charged to the row below. A header row that rules three points
under it and nothing above the row that follows is three points nobody pays
for otherwise — every row after it sat three points too high — and a calendar
that rules a hairline under one row and over the next must not pay twice.

A float that does not travel with the text narrows whatever lands beside it.

An anchored drawing belongs to a paragraph, and it is tempting to let the
paragraph it belongs to be the one that makes room. Word does not: the float
is a rectangle on the page and every line that lands within it goes round.
The demonstration document proves the difference — its two arrows stand at the
two margins of one page, and the right-hand one is anchored to a paragraph five
lines below the text it narrows. Where such a float sits is known only once the
document has been paginated, so the layout is run, the floats are read off the
pages, and the paragraphs beside them are laid again. One correction pass, not
a loop: a wrap that moved the float that caused it would never settle.

And an old format names nothing it can name by number instead.

A `.doc` has no font names in it anywhere a run can see. A run says
`sprmCRgFtc0 = 3` and means the fourth entry of a string table on the other
side of the file. Skip that table and nothing fails: every run simply says
nothing about its face, the fallback answers, and a document that is Arial
from top to bottom renders as Times New Roman from top to bottom. It is the
worst kind of gap — no error, no missing text, no crash, and a rendering that
is wrong in a way only a side-by-side shows.

A counted list padded to an even length is not a list you can step through by
its counts.

A style's own formatting sits in a variant record whose members depend on
which kind of style it is, and each member is a length followed by that many
bytes — *and then a pad byte, when the length is odd, that the length does not
include*. Stepping by the count alone lands one byte early on the next member
and reads the tail of one property list as the length of the next. The
paragraph half of every style parsed correctly for as long as it was the only
half anybody asked for.

Saying nothing and saying "none" are different answers, and a format that has
two spellings for one of them has two for a reason.

A cell's `TC80` states four borders whether the cell has an opinion or not. All
zeroes means it has none, and the table's own rule runs there — which is where
an ordinary grid comes from, since almost no cell states its own. All bits set
is `Brc80MayBeNil` saying the cell *has* an opinion and it is "no rule". Map
both to "unstated" and the table's rule runs straight through a border the file
struck out; map both to "none" and the grid disappears. The two spellings exist
because the format needs both answers.

A tab is the gap between type, and Word does not let it decide how tall a line
is.

An entry in a table of contents is eight-point type with an eleven-point tab in
the middle of it — the tab keeps the default face when a document written in
one Word is opened in another — and Word sets it on an eight-point line.
Measured in a document built to ask: a twenty-two point tab in an eight-point
paragraph raises the line not at all, while a twenty-two point *space* raises it
fully. A paragraph mark behaves the same way — it decides the height of a line
that has nothing else on it and of no other. Thirteen pages of a document
paginated wrongly because a tab was measured like a letter.

The column a shape is measured from is the column it was anchored in, and a
table cell is a column.

A `.doc` states a floating shape's rectangle against the page's margin, the
page's edge, or the text — and "the text" is not the page's text column when
the paragraph holding the anchor is inside a table. Word measures from that
cell's own text edge. The page frame of this document says minus a hundred and
eight twips and means thirty-six points, because its cell begins a hundred and
eight twips inside the margin; read from the margin it lands five and a half
points left of everything it is meant to enclose, which is exactly wrong enough
to look like the page is off centre rather than like the frame is.

A document can be told not to breathe between its lines, and then every line in
it is shorter.

A face states its leading as three numbers — an ascent, a descent, and a gap to
hold between one line's descender and the next line's ascender — and Word
ordinarily honours all three. `fNoLeading`, one of the compatibility options a
document converted from an older word processor carries, drops the third.
Arial asks for sixty-seven units of its two thousand and forty-eight, so an
eight-point line goes from 9.20pt to 8.94pt: a quarter of a point, invisible
once and a page of difference over a document. It is a document-wide setting
and it applies inside table cells as well as outside them, which is worth
measuring rather than assuming, because a row that shrinks with its lines and
one that does not are a page apart too.

Word writes a style onto its table of contents and then declines to draw it.

Every entry of a contents built with the hyperlink switch wears the
`Hyperlink` character style, and Word draws the entries black and unlined. It
takes four measurements to say why, because three plausible readings are
wrong: the style is not being applied there (make it loud and the contents does
not change), character styles *are* read inside a field result (swap those runs
to Emphasis and they italicise), internal links *are* decorated elsewhere (one
in the body draws in the link colour), and the hyperlink field is not the
reason (a run given the style inside one draws in it). What is left is the
table of contents field itself. A rule found by one measurement would have been
any of the four.

A field's instruction is not the document's words, and a reader that keeps it
as text can never say what kind of field it is looking at.

Skipping a field's code at drawing time looks like enough — nothing wrong
appears on the page. But the kind of a field is only in its code, and the kind
is what decides whether a page number is recomputed, whether a contents entry
takes a character style, and which of forty paragraphs are inside one. A
contents field opens in one paragraph and closes forty later, so the answer has
to be walked once over the whole document; nothing looking at a paragraph on
its own can tell.

WordArt is not type at a size.

The words of a piece of WordArt are stretched until they fill the shape they
were drawn in, across and down, and the two need not agree: Word's watermark
here is Courier New filling 609 points of width in a box 152 points tall, so
its letters stand half again as tall as that width alone would make them. A
size is the only thing a shaper can be asked for, so the difference has to be
carried separately and applied to the glyphs by each renderer — which a PDF
does in its text matrix, GDI by naming the width the unstretched face settled
on, and a screen only by tessellating the galley and moving its vertices,
because epaint will turn a galley but not squash one.

A rule between two cells is one line, and only one of them may draw it.

Both cells name the rule that runs between them, and drawing it from both
sides is invisible on paper — the second stroke lands exactly on the first.
On a screen it is not: anti-aliasing gives the stroke soft edges, and laying
one over another darkens them until a hairline reads as heavy as the frame
around the table. The rule has to be given to one side of it, and the side has
to be the one still at hand: a row's neighbour above has already been laid out
and cannot be revisited, so the row below draws their shared rule, while the
cell to the left draws the vertical because its neighbour is in the same row.

A rule nobody draws still spaces the rows it separates.

Word charges a row for the heaviest border on its top edge across every cell
of the row, including a cell whose rule is not drawn because a vertical merge
runs through it. A letterhead whose cells rule at a point and a half and whose
table rules at a half, with the first column merged down the whole table, sets
every row under the merge a point too high if the undrawn rule is dropped
instead of counted. Height and drawing are separate questions about the same
border.

`sprmPFKeep` is keep-lines-together and `sprmPFKeepFollow` is keep-with-next.

The names read the other way round, the two opcodes are adjacent, and widow
control is nowhere near either of them. Nothing complains if they are swapped:
a document merely paginates a little differently, which looks like a
pagination bug rather than a reading one. The oracle is Word's own object
model — `KeepTogether` and `KeepWithNext` on the paragraph — not the names.

A page break the author typed is a decision the keep rules do not get to
overrule.

Lines before such a break and lines after it are on different pages whatever
"keep these lines together" asks for, so treating the paragraph as one
unbreakable group only wastes a page: the rules drag the lines above the break
forward, and the break then pushes everything after it forward again. And a
break with nothing in front of it takes the whole paragraph, its space above
included — there is nothing left behind for that space to sit under, and an
empty line left on the old page to hold it puts every line of the new page a
space-before too high.

A list number is made, not read, so a leading page break has to go in front of
the making.

The number beside a numbered paragraph is nowhere in the document's runs; it
is generated during layout at the head of the paragraph's first line. If the
paragraph opens with a page break and the number is emitted before it, the
number is left on the page the paragraph came from and the paragraph arrives
bare. Word makes the number at the head of the first line of *text*, which the
break has already moved.

`lcbPlfLst` does not cover the levels it points at.

A `.doc` writes its list definitions as a count, an array of fixed-size
records, and then — past the end of the length the file header states — the
levels themselves, nine to a list or one if the list says it is simple. The
length stops at the end of the record array. Reading only the slice the header
describes finds every list and not one of their levels, and the failure is
silent: the lists are all there, correctly identified, and number nothing.

A metafile is a recording of the calls that drew a picture, and Word makes
those calls in sixteenths.

An EMF pasted into a Word document is not pixels: it is the GDI a drawing
program issued, and playing it back means keeping the state a device context
keeps. Two things about Word's own recordings are not obvious. It multiplies
the world transform by one sixteenth, emits a shape in whole units of that
finer grid, and multiplies by sixteen again — so a *geometric pen* five units
wide draws five sixteenths of a unit, because a geometric pen is measured in
the logical units in force when it strokes. Reading the width without the
transform draws every line in the document sixteen times too thick. And the
picture's size is not the bounding box of its ink: the header states the frame
in hundredths of a millimetre and the recording machine's screen in pixels and
millimetres both, and that ratio is what turns a coordinate into a length. A
player that stretches the ink to fill the box it was given loses the drawing's
own margins and draws every diagram at a slightly different scale.

`ExtTextOutW` carries the advance of every character it wrote. Honouring them
sets a label exactly as wide as the drawing program set it, which is what makes
a played diagram line up with Word's to a third of a point rather than to
whatever this machine's copy of Arial happens to measure.

An inline picture's bytes are wrapped the same way the shared store's are.

`PICFAndOfficeArtData` ends in an array of `OfficeArtBStoreContainerFileBlock`,
and a block is either a BLIP or an `OfficeArtFBSE` with a BLIP inside it — the
same choice the document's shared picture store offers. A reader that handles
the wrapper in one place and not the other finds the pictures a document shares
and misses every picture that came with its own text, and the miss is silent:
the drawing is there, the size is right, and there is nothing in it.

An empty paragraph is as tall as its mark, and the mark has its own size.

The paragraph mark is a real character with real character properties, and a
document that separates a heading from its text with an empty paragraph very
often sets that paragraph's mark small on purpose. Spacing it at the body's
size instead is half a line too tall at every heading, which by the foot of a
page is a line and by the end of a document is a page. The properties are on
the mark's own character position, not on the paragraph's first character —
the same trap the paragraph properties set, one character further on.

A page break typed at the head of a paragraph does not spend its first line.

The break leaves an empty line in front of everything, and code that asks "is
this line index zero?" to decide where the first-line indent goes gives the
indent to that empty line and the paragraph's real first line the ordinary one.
On a numbered heading with a hanging indent the effect is exact and visible:
the number stands where the text should, a hanging indent to the right of where
Word puts it.

A watermark and a page frame are shapes in the header, and Word will not let
the body pick one.

Both are drawn across the whole page, so an editor that hit-tests every shape
it painted hands every click on the words to whichever of them was painted
last, and the document cannot be typed in at all. Word's rule is not a
tie-break, it is a layer: a watermark "is usually part of the header, even
though it appears in the middle of the page", and reaching it means opening the
header first. The same rule holds a second time inside the body — a shape put
*behind* the text does not take a click that lands on a letter, and Word gives
it the Select Objects tool as the deliberate way in rather than letting an
ordinary click find it.

The index a placement carries is only meaningful in the band it was flowed in.

A header is laid out on its own and its paragraph counter starts again at
nought, so a shape anchored in one reports itself as a drawing of paragraph
zero. Read against the body's walk that names a real paragraph, which is a
worse failure than not finding one: the click selects something, the handles
are drawn round the shape, and a drag resizes a picture elsewhere in the
document.

A watermark is VML, in a notation Word deprecated and still writes.

Everything else a modern document draws is DrawingML under `<w:drawing>`.
Design ▸ Watermark writes a `<v:shape>` of type `#_x0000_t136` — WordArt —
inside a `<w:pict>` in the header, with the words in a `string` attribute on
`<v:textpath>`, the size and the turn in a CSS `style`, and the colour in a
`fillcolor` that is usually a *name* rather than hex: Word writes `silver`,
so a reader that only knows `#rrggbb` loses the colour of the commonest
watermark there is. `<v:fill opacity=".5"/>` beside it is why Word's grey
draws about twice as light as its name suggests.

The `<o:lock shapetype="t"/>` is what makes a shape type a template.

A `<w:pict>` carries the `<v:shapetype>` the shape is an instance of, and
without that lock at the end of it Word counts the template itself as a second
shape in the header — a page with two objects on it, one of them not even
indexable through the object model, and its own Remove Watermark then leaves
that one behind. Its `<v:formulas>` are not decoration either: the `path`
refers to `@7` and `@8`, and a shape type whose path names formulas it does
not carry is not one Word will draw text along.

An undeclared namespace prefix is not a document Word draws badly.

It is a document Word refuses to open at all — "Word experienced an error
trying to open the file", with no hint as to which byte. A `<w:pict>` this
application authors declares `v`, `o` and `w10` on itself, because the file it
is being written into may declare only `w`.

Word stamps a watermark on the headers a page will actually show.

A document commonly names three — default, first and even — while `<w:titlePg>`
and the document-wide even/odd setting are both off, so two of them are never
drawn. Word puts the shape in the default header and leaves the other parts
empty; stamping all three writes shapes into stories no reader sees, and Word
then reports three watermarks on a document that shows one. The rule that
decides which headers are live is the same one that picks a header for a page.

A document is not one body, and a paragraph index is not a position.

Its own text is one flow of blocks; every header and footer is another, edited
the same way and counting its paragraphs from zero. An editor that names a
position by number alone can only ever reach one of them — and it will not fail
loudly, it will quietly agree that paragraph three of the header is paragraph
three of the text. That is one bug on the way in (a click on the body selected
a shape drawn in the header) and a worse one on the way back out (undo
restoring a header's paragraph over a paragraph of the text). The flow belongs
beside the index everywhere the index goes: in the caret, in the hit test, in
the layout's placements, and in every entry of the undo stack.

An editor that can only edit one flow grows a dialog for every other one.

Scriva's header box asked for text, a face, a size and an alignment because
that was all the editor could put into a header — not because that is what a
header is. Real ones hold tables, logos and fields, and a box like that
flattens them on Apply. The dialog was not a design decision that turned out
badly; it was the shape of a missing capability, and it stayed convincing until
someone opened a document whose header had a table in it. When a feature can
only be offered through a form, ask whether the form is standing in for
something the editor cannot yet do.

A field's pieces all sit at the same offset.

`{ PAGE }` is five pieces — start, instruction, separator, cached result, end —
and only the cached result carries any text. Inserting them one at a time at a
caret offset writes them in reverse, because each insertion lands at the same
offset and goes in front of the last. Anything made of several pieces that
carry no text between them has to arrive as one splice.

A premultiplied white at two-thirds alpha is not a veil, it is paint.

Greying the part of a page that is not being edited looked like a two-line
change: a translucent white rectangle over it. Full-white channels with an
alpha of 0xAA are super-luminous in premultiplied space — they add white while
attenuating only a third — and the body went blank, table rules and all. The
suite could not have caught it and neither could reading the code; a screenshot
of the running application did, in one glance. Premultiply the channels with
the alpha, or use the helper that does.

Giving an editor a second flow puts every body-only walk on the clock.

The moment a caret could stand in a header, every function that had quietly
meant "the body" became a bug waiting for the right keystroke. Most were found
by the compiler, because the scope had to be passed in. The ones that were not
are the ones that take a `Selection` and go looking for the paragraph
themselves: the finder, the reviewer, `revise`'s add-a-comment. Those compiled
perfectly and would have wrapped the wrong paragraph in comment anchors. After
widening a model, grep for the old accessor rather than trusting the build.

Ask the application whether it can do the thing before building it.

The reviewer was widened so that a comment could be made on a header, on the
strength of a note that said Word could. Word cannot: asked over COM to comment
on a header's range it answers *"Comments, endnotes and footnotes can only be
added to the main story."* The work was not wasted — tracked changes in a
header are real and now settle — but the comment half would have been a
capability the format's own producer refuses, offered as if it were fidelity.
One COM call, thirty seconds, before the design and not after it.

Silence is an instruction.

A `<w:sectPr>` with no `<w:headerReference>` in it is not a section with no
header; it is Word's "Link to Previous", and the section shows the one before
it. The same shape appears twice more in this codebase — a `.doc` cell side of
all zeroes has not spoken while one of all ones has spoken and wants no rule,
and a missing first-page header on a title page means *no* header rather than
the default one. When a format leaves something out, find out whether the
omission is the message.

Word fits WordArt to the ink, not to the em box.

Four strings of wildly different proportions — "CONFIDENTIAL", "gypsy", "Hg",
"xxxx" — put in the same 400 by 200 point shape all come out with their drawn
outlines spanning exactly 400 by 200. The face's ascent and descent have
nothing to do with it, which is why an all-capitals watermark came out two
thirds the height Word draws it. The number nobody had was the box the glyphs
themselves fill, and it is ten bytes into each `glyf` entry.

A washed-out picture is a recoloured picture, not a faint one.

A picture watermark has no transparency in it at all. Word turns the brightness
up and the contrast down and bakes the answer into the pixels — into its own
PDF export, which is what made the rule measurable: a ramp of every grey there
is, run through seven settings, gives `out = in·(1 + contrast) + offset`
exactly. Baking it where the picture is decoded is also what keeps the screen
and the printer agreeing, because neither renderer then has to know.

A line has two halves and they may come from different faces.

A line's height was the tallest fragment's whole line height, which can never
exceed the tallest face's own. Word's answers routinely do: an Arial line with
one Courier New letter on it is taller than an Arial line *and* taller than a
Courier line, because Arial reaches higher and Courier hangs lower. The height
is the largest ascent-plus-gap on the line plus the largest descent on it,
measured separately. Any rule of the form "the biggest one wins" should be
suspected the moment the quantity is really a sum of two independent ones.

A list's label is measured on one side only.

A bullet raises a line and does not deepen it — Symbol hangs lower than Arial
and Word does not charge the line for it, though it charges for the very same
character typed in as text. A label is not a run, and the layout has to know
the difference to get the pitch right.

A caret in a header is in several places at once.

The same header stands on every page of its section, so "where is the caret" has
as many answers as there are pages, and the first of them is almost never the
right one. Opening the running head on page two scrolled the window back to page
one to show the same words, and every band command — which section, which kind
of band, whether it is linked — answered for page one. A position in a flow that
is drawn more than once has to carry the page it means.

Two oracles can disagree below the level you care about.

Word's own PDF of a table it wrote puts a rule's ink wholly on one side of its
gridline. The demonstration document, a `.doc`, has our rules landing within
fifteen hundredths of a point of Word's while straddling theirs. Both
measurements are real, they cannot both be the whole rule, and the gap between
them is a quarter of a point. Changing the renderer to satisfy one would spoil
the other. When the oracles disagree by less than the thing is worth, record
both and change nothing.

Measure where the time goes before deciding what to make faster.

The note said a long document was slow because it was laid out in full on every
edit, and the obvious fix was to stop laying out the paragraphs that had not
changed. That was two thirds of the answer. Of the third of a second eight
thousand paragraphs cost, shaping and breaking them was 240 milliseconds and
*copying the finished lines about* was 130 — into the item pagination breaks,
and again onto the page. Caching the shaping alone would have saved half of what
it looked like it would save, because a cache hit still copied everything it
handed back. Sharing the lines had to come first; then the cache was worth
having. Both numbers came from four `Instant::now()` calls in a throwaway test,
which is fifteen minutes against a week of building the wrong thing.

A cache that is invalidated by being told will one day not be told.

Every paragraph's lines depend on things that are not in the paragraph — the
style table, the theme, a handful of settings — and the cheap way to handle that
is for whatever edits a style to say so. It is also the way that fails silently
and late: the one command that forgets shows the user a document that is not
theirs, with nothing in the layout at fault. So the cache keeps a copy of
everything outside the paragraphs that it reads, compares it at the start of
every layout, and empties itself when it differs. It costs a struct comparison
per keystroke and it cannot be forgotten, because there is nobody to forget.

Do not fingerprint what you can compare.

The obvious key for a laid paragraph is a hash of it. The obvious key is wrong:
a hash collision does not produce a slow layout or an error, it produces a
paragraph drawn as some other paragraph, at a moment nobody can reproduce. The
model is `PartialEq` all the way down, comparing two paragraphs costs about a
hundred and twenty-five nanoseconds, and eight thousand of those is a
millisecond out of a budget of twenty-six. The exact answer was affordable, and
it was only the habit of reaching for a hash map that made it look otherwise.

A format that has one name for a thing may still have two behaviours for it.

WordArt was measured once, from shapes made by Word's own gallery, and the
answer was clear: the drawn outline of the words fills the shape's box across
and down, whatever the string. That answer is right and it is half the story. A
watermark is WordArt too, and Word draws it at the size whose advances fill the
width and only scales it down the page — three times narrower letters than the
other rule gives. The two shapes are the same shape type, carry the same VML
when Word writes them to a `.docx`, and differ in one bit of one binary
property. The lesson is not about that bit. It is that "measured against Word"
is a claim about the document that was measured, and a second document of the
same kind is a second measurement, not a confirmation.

A page break the writer typed is part of the paragraph; one the page ran out of
room for is not.

Word drops a paragraph's space-before at the top of a page — except when the
paragraph carries the break itself, as `` in its own text, in which case the
space follows the break as it would follow any other character. Two headings of
one style, twelve points before, one at the top margin and one twelve points
down, and nothing in either paragraph's formatting to tell them apart: the
difference was a character. Probing this in a document made for the purpose gave
the wrong answer twice, because inserting a page break through Word's own
automation puts it in a paragraph of its own and so produces the case that
suppresses. The document that showed the behaviour was the one the bug was
reported against.

A boundary is not a place to break unless both sides of it hold something.

A table row can be cut between its lines, and the bottom of the last line is a
line boundary like any other — so it was offered as a cut, and a row at the foot
of a page had all of its words placed there and its cell's bottom padding sent
to the next page. Word moves the whole row. The rule that was missing is not
about padding: it is that a cut has to leave something worth seeing on both
sides of it, and a filter written as "not inside a line" admits every boundary
that is merely outside all of them.

A line is as tall as what is drawn on it, and some of what is in it draws
nothing.

A field that begins, separates and ends puts three marks in the text and no
glyph on the page, and a placeholder of no width was standing in for the last of
them and being measured like type. Every diagram in the document is an `EMBED`
field, so its end sat on the line under the picture with only the paragraph mark
beside it, and that line came out at the run's ten points where Word gives it
the mark's five — eight points under every figure, and two lines pushed off the
foot of a page. The crate already had this rule for tabs and had written down
why: Word measures a line by the type on it, and a tab is the gap between type.
A field's mark is not even that.

The room under a line belongs to the type on it, so a picture with nothing
beside it takes none.

The rule "a picture sits on the baseline and the run holding it still hangs
below" was measured, correctly, on a line that also had words. On a line that is
only a picture there is nothing to hang: eleven inline figures, asked of Word one
at a time, each made a paragraph exactly as tall as its own picture. A
measurement is of the case it was made in, and the case it was made in is worth
writing down beside the number.

Two cells in one row, given the same rule by the file, and the application does
not treat them the same.

A letterhead's two merged cells both say "centre me": Word centres the one that
covers two rows and leaves the one that covers four at the top, where centring
would move it nine points. The same four-row merge built for the purpose, in
both file formats, Word centres. Whatever tells the two apart is not the row
count, the format, the paragraph count or the alignment, and the generalisation
that fixes the first breaks the second by more than it was wrong by. The honest
answer to a rule that fits one measurement and contradicts another is to keep
looking, not to ship the half of it that flatters the document in front of you.
A number that was never checked is not a rule; it is the residue of the two
things that happened to be in front of it.

A `.doc` states a table's leading edge in the first of its row's boundaries, and
the reader was adding half the gap between columns to it. That was written for
the two tables at the top of the demonstration document, and both of them are
centred — a centred table ignores its indent entirely, so the half-gap could
have been any number at all and those two would have looked right. Five pages
in, the first table that is actually placed by its indent sat five and two
fifths points off. The correction and the thing it was hiding came out together:
the table justification had never been read either.

Word does not kern, and a modern shaper does.

Kerning in Word is a checkbox, off unless the run turns it on, and nothing in an
ordinary document turns it on. HarfBuzz closes up every pair the face offers.
Over a line of prose that is a fraction of a point — and a fraction of a point
is the whole of the question at the end of a line, where a word either fits or
does not. Two paragraphs in one document broke a word early because of it. The
fix is not to stop shaping: it is to measure the letters of a run that stands on
its own one at a time, which is the same shaping without the pair adjustment,
and to leave a joining script shaped, where a letter measured alone is not the
letter that gets drawn.
A rule that is right can still be conditional, and Word keeps the condition in a
flag from 1993.

The tab after a list's number goes to the paragraph's hanging indent — measured,
and it beats a default stop standing between the two. But Word has carried
"don't add automatic tab stop for hanging indent" since Word 6, and a document
that sets it wants the other reading entirely: the indent stops being a tab stop
and the tab becomes an ordinary one. Both readings look like the whole truth
from inside a single document. The way the flag was found was not by reading
about it: it was by asking Word for the compatibility options of the document
that disagreed and of one that did not, and diffing the two lists — which is
worth doing the moment two documents of the same shape lay out differently.
A cell that holds a floating shape is not vertically aligned, and the document
that says so is not the reason.

Two cells side by side in one letterhead, both centred, both merged down the
rows beneath them: Word centres one and leaves the other at the top. Nothing in
the two cells' own properties tells them apart — the alignment, the merge, the
row heights and the paragraph formats all match. What tells them apart is that
one of them is where the document anchors its page frame and its watermark.
Take the shapes out and Word centres it like the other. The compatibility
setting named for exactly this behaviour is present in the file and turns out to
be inert: removing it, even with the document put in a modern compatibility mode,
changes nothing. So the rule is Word's, not the file's, and the honest thing is
to apply it unconditionally and to write down that the flag was tried.

The lesson underneath is about how the two rules hid each other. Aligning a
merged cell in its whole span is right, and every attempt to ship it was reverted
because it moved the shape's cell nine points. Neither rule can be seen while the
other is missing, and a synthetic reproduction built to settle it centred both
cells, because nobody thinks to anchor a page frame in a test table.

Asking Word where something is costs Word a layout pass, so a whole document
cannot be asked one question at a time.

`Range.Information(5|6)` is the oracle every probe here has used since the line
pitches were fitted, and it is exact to a twentieth of a point. It is also
about 110 milliseconds a call on this machine — 200 words took 22.7 seconds —
because Word lays the document out to answer. That is fine for one page read by
eye and hopeless for five thousand words: the same document exports to PDF in
seconds, and the export is *better* evidence, because a rendered page reports
each word's baseline and a baseline is the one horizontal two renderers can be
compared on without either having to guess where the other thinks a line
begins. The general shape: an oracle that is exact per question can still be
the wrong oracle, and the thing to measure before building on it is what one
question costs.

And the unit an application names is not always the unit it means. Word's
`Words` collection is not a list of words — it splits punctuation into its own
entries and hands back the trailing space as part of the token — so "options,"
is two of them and neither is what a reader would point at. A tab's leader dots
are the same trap from the other end: both renderers draw as many stops as the
stretch takes, neither count was chosen by anyone, and a comparison that counts
them as words buries every real difference under a page of full stops.

Two renderings of the same page cannot be matched word by word, and the way
that fails is silent.

The obvious way to compare two layouts is to line the words up with a longest
common subsequence and read off the differences. It does not work, because a
page repeats itself: the same heading word, the same "the", the same sentence
down a table column. The moment one side holds something the other does not —
a watermark whose words one renderer gathers and the other does not — the
subsequence is free to pair one occurrence with a far-away other at no cost to
its own score. `watermark.docx` is one phrase repeated three times a line and
forty lines down the page; the first version of the harness reported three
hundred words hundreds of points out of place on a page whose real fault was
sub-point. Nothing about the output said it was fiction. The tell, once looked
for, was the *median* difference being large: two renderings of one page agree
about the page, so a middle shift of fifty-five points is the matching lying,
not the layout.

Three things fixed it, and the order matters. **Match lines before words** — a
line is a far more distinctive token than a word, and words are then only ever
matched within a line. **Anchor on the lines that cannot slide** — a line whose
text occurs exactly once on each side has only one place it can go, and the
longest rising run of those is a set of fixed points; everything else is
matched only between two of them. That is patience diff, used for the property
it was invented for. **And refuse a pairing that cannot be true** — where a
page has no unique line at all, nothing in the text can say which repetition is
which, so a pair whose baseline sits more than about three lines from what
every other pairing on the page says the offset is gets dropped rather than
believed. The median carries a page that has really moved; the outlier is the
repetition talking.

The general lesson is about the shape of the check rather than about documents:
when two orderings of the same thing are aligned by content alone, repetition
in the content is an unbounded source of confident nonsense, and the guard has
to come from a quantity the content does not control.

## A measuring instrument is a thing to be measured

Every fault below was in the tool that reports faults, and each of them read
exactly like a fault in the layout. Half a day of sharpening `cargo xtask
compare` found more wrong with the measurement than with the thing measured.

**A word is not a thing a page has.** Neither renderer writes words down; both
put marks on paper, and a reader is what finds words in them. Word's export
breaks `I/O` into three positioning calls where our playback draws one. Two
rules for joining marks back together by geometry were tried, and both failed
the same way: they *invent* a token, and a token one side has invented can be
paired with nothing. A diagram draws its labels in whatever order it likes, so
one of those rules ran `SPI` together with a `Radio` fifty-three points to its
left. An invented word is worse than a split one, because a split one can still
be paired. Report every mark where it fell, and reconcile the two tokenisations
in the matcher instead.

**Measure a word at its left edge, never at its first letter.** The two are the
same point for type set left to right and opposite ends of the same word for a
run set right to left. Measuring one end of ours against the other end of
Word's put an Arabic word 29.48pt out of place; the real difference was 0.68pt.
A convention that is invisible in the common case is exactly the one to state
explicitly on both sides.

**Two sequences that fall out of step cannot be walked in step.** The line
matcher's fallback compared our line *i* with Word's line *i*, and advanced
both whenever they failed. One line that exists on one side only — Word raises
a footnote's reference onto its own baseline and we do not — put every line
after it one place out, so every comparison failed and the page came back with
nothing matched at all. Forty words called unplaceable on a page where nothing
had moved by more than a fifth of a point. Look ahead a bounded distance, and
advance only the side that was matched.

**A threshold sitting on a measured value is a coin toss.** The gap that
decides whether two words share a line was 3.0pt; the tightest gap between two
genuinely separate lines anywhere in the corpus is 3.00pt exactly. Two
renderers reporting that gap a hundredth apart then group the same page two
different ways. Measure the distribution before choosing the number, and put it
where there is room on both sides.

**A cache must be keyed on everything that would change its answer, including
the code.** The oracle here is two scripts and a document; keying the cache on
the document alone means an improvement to the reading is not the thing you are
looking at. Keyed separately, an improvement to how words are found re-reads
every rendering already on disk without asking Word for any of them again —
which is the difference between a minute and an afternoon, and therefore the
difference between improving it and leaving it alone.

**A corpus file is not evidence of what its name says.** The document named
`rtl-and-cjk.docx` contained no CJK for months, and a reader test had been
written to assert what was actually in it — which made the gap permanent and
documented rather than permanent and hidden. It surfaced only when a *different*
tool asked a question of the whole corpus at once ("is there any CJK anywhere in
this?") and got `none` back. Ask the corpus what it holds, not what it is
called; a sweep over all of it answers questions no per-file test is shaped to
ask.

**Assigning to a fresh paragraph's `Range.Text` deletes the paragraph.** In Word
COM, `Paragraphs.Add()` gives a paragraph whose range *includes its own
paragraph mark*, so `$p.Range.Text = $line` writes that mark away and merges the
paragraph into the next one. A loop that adds six paragraphs and sets each one's
text leaves a document holding the sixth. Set the whole content once with
carriage returns between the lines instead. Three corpus documents are thinner
than their generators read.

**An oracle's answer is worth committing when it cannot change without the
question changing.** Word's rendering of a document is fixed until the document
is edited, so keeping it in the repository turns a check that needed Office and
half an hour into four seconds of arithmetic that runs anywhere. Two conditions
make it honest rather than a trap: key the answer on the *contents* of
everything it depends on, never on a timestamp — git records content, so an
mtime-keyed cache misses on every clone — and have the stored answer state what
it is an answer *to*, so that one which has gone stale says so instead of being
quietly believed.

**A cached measurement of somebody's document is that document.** A reading
holds every word in it. The rule that keeps real documents out of the
repository has to follow them into the derived files, or it is not a rule.

**Any decision that shapes both sides of a comparison must be taken from both
sides at once.** Two renderings of one page are cut into lines by the gaps
between baselines, and the two report those baselines a few tenths of a point
apart — so *any* fixed threshold eventually has a real gap sitting across it,
and then the two sides cut the same page differently and agree about nothing.
Moving the number only moves where it strikes; it was moved twice and struck
twice. Cutting once, over both readings' baselines together, ends the class of
bug: the partition stops being a property of one rendering and becomes a
property of the page. The same then applies to the order words are read in,
which is a second decision of the same kind and was got wrong once more before
being cut jointly too.

**Read the strongest evidence first, or a cheap match will destroy it.** Several
words running together into exactly the other side's one word is not a
coincidence a page produces; two identical short words in different places is
what every page produces. A longest-common-subsequence sees only equality and
will happily spend a short word on a distant twin, leaving the real
correspondence with nothing to build from — it paired an `RX` with another `RX`
thirty-two points away and cost nineteen words on that page. Find the composite
matches first and let the subsequence work between them.

**A symbol font's glyph number is not a character.** A bullet stored as Symbol
0xB7 arrives as U+F0B7 in the private use area, which means only "the 0xB7th
glyph of whatever face this run names"; Word's PDF export writes the character
it drew, U+2022. Two names for one mark in one place. Map the ones that actually
occur, having looked — never guess at what a glyph number might mean.

**Two renderers do not cut the same ink into the same pieces, either.** The
words taught this and the page's furniture taught it again, harder. Word's
export lays a table's top border as a small filled square at each corner with
the spans between them; this project lays one rule per cell. The same border,
in the same place, to a hundredth of a point — thirty rectangles against nine,
and a document with nothing wrong reported as a document with nothing right.
Reduce both readings to the ink they put down, with one function that cannot
tell which side it is working on. The rule that follows from it is the same one
the line cutting gave twice: any decision that shapes what is compared has to be
taken from both readings at once, or one side is being held to a convention it
never agreed to.

**And a rule is broken where another rule crosses it.** A corner square can be
run into the horizontal border or into the vertical one but not into both, so
whichever pass takes it leaves the other with a rule in five pieces — each one
crossing short of its neighbour, and unmatchable against the single rule the
other renderer drew. A break as wide as the rule's own thickness is a break; a
gap between two boxes is a gap somebody meant.

**A float is a rectangle, and there is only one mechanism.** Word does not
have one rule for text beside a picture and another for text below it: the
picture is a rectangle on the page and every line that meets it goes round.
Reserving a float's height in the flow is a second mechanism, and a second
mechanism is right only where going round is impossible — a picture as wide as
the column. Where it was applied to a float with room beside it, the whole
paragraph started a picture's height too low, its own inline neighbours
included, and the picture beside the text was correct all along.

**Text goes on *both* sides of a float, on the same line.** Choosing the wider
side is not a smaller version of the truth, it is a different page: every word
Word had set in the ninety-one point channel to the picture's left was thrown
three hundred and twenty points across it. A line does not need a new shape to
express this — a fragment's x is already a free offset — only a pen that steps
over the hole instead of through it.

**A cache that repairs itself turns a check into a formality.** The corpus is
held to Word's committed readings, and a reading names the probe script it came
from. Change the script — a docstring was enough — and every reading is stale.
On the machine where Word is installed the check then *renewed all twenty-five
and passed*, and the commit went out with readings stamped for a script that no
longer existed. It failed on the first machine without Office, which is the only
kind of machine that could ever have noticed. A check may not fetch the evidence
it needs in order to pass: where the evidence is stale, that is the finding.

**A claim about what happens when something is missing has to be executed.**
That the comparison needs no Word — its readings being committed — is the reason
it may sit inside the gate, and it was written down in three files and had never
once been run. Every run was on the machine where Word is installed, where a
stale reading is renewed in seconds and nobody notices. Take the thing away and
run it: an empty PATH is enough, and it cost one test file. The absent-dependency
path is the one that rots without a sound, because the machine that would notice
is the machine nobody develops on.


**Word's own default face was not on the machine, and nobody had noticed.**
Aptos ships with Office rather than with Windows, and Office does not install
it: it is fetched into `%LOCALAPPDATA%\Microsoft\FontCache\4\CloudFonts`, one
directory per family, holding files whose names are opaque numbers. Every
lookup by file name therefore said the machine did not have the face Word had
just laid the document in, and every document written since 2024 — which is to
say every document a new Word writes — was measured in a stand-in. One of them
was a hundred and fifteen points out. The directory name is the family and the
style is in the file, so reading it is thirty lines; finding it took a search of
the whole disk, because the question "is this face installed?" had only ever
been asked of the places faces are installed.

**`<w:kern>` is a threshold and Word's own document defaults set it.** The
shaper measured a run letter by letter to keep HarfBuzz from closing up pairs
Word leaves open — sound, except that `docDefaults` says `<w:kern w:val="2"/>`
in every file Word writes, which means every ordinary run kerns. Three tenths of
a point per word, invisible in a line and three hundred and twenty-eight words
out of place over a page.

**A style's `<w:tblInd w:w="0">` is not a stated indent.** Word's built-in
Normal Table names one, so every ordinary table inherits it; read as an indent
stated by the table it hangs the table's edge a cell's padding into the margin,
and the rule down the left of a plain table sits where nothing draws it. What
the file says and what the *table* says are different questions, and only the
second moves the table.

**A `.doc` says its default font in the stylesheet's header, not in a style.**
There is no `docDefaults` in the binary format; `STSHI.rgftcStandardChpStsh`
holds the three faces — Latin, East Asian, complex — a run falls back to, and
without it every document Word wrote after 2007 was laid in Times New Roman
rather than in the Calibri it says right there. The same document then wanted
`sprmPFContextualSpacing` and `sprmCIss`: a list whose items do not close up and
a footnote number set full size in the middle of a word are both the same kind
of miss — a property the modern format has and the reader only implemented on
that side.

**A repeated table header row was paginated for and never drawn.** The page
made room for it — the arithmetic was right, and every page after the first held
one row fewer — and then nothing put the ink there, so a fifty-row table showed
a blank band at the top of every continuation page and its rows sat a row's
height too high. Reserving room is half of it; the half that shows is the other.
