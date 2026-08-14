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
  Officeoot\Templates` holds 41 finished `.dotx` files — 604 paragraphs, 278
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
