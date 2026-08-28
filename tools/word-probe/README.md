# word-probe — measuring Word's layout behaviour

The oracle loop behind ADR 0001. Word's laid line pitches are hinted,
per-ppem quantities that no formula over the font's tables reproduces; when a
face or size needs exact parity, it is measured, and the measured base goes
into `measured_base()` in `crates/app-scriva/src/shaper.rs`.

Requires a machine with Word installed (any license state — COM reading
works on an unlicensed install; exporting and printing do not).

1. **`makeprobes.py`** — writes minimal probe `.docx` files: N identical
   single-spaced lines of one face at one size, and bordered-table variants.
   Edit the lists at the bottom for the faces and sizes in question.
2. **`dumpall.ps1 -Dir <probes>`** — opens each probe in a hidden Word over
   COM and writes a CSV beside it: every paragraph's position in points
   (`Range.Information(6)`, 0.05pt resolution), whether it is in a table,
   its font, size, and page.
3. **`fit.py <csv>...`** — fits the base pitch and half-point corrections to
   the positions. A max residual at or under 0.51 twips means the model
   reproduces every measured value to within Word's own reporting; larger
   means the model is missing something — look at the raw diffs before
   trusting any number.
4. **Compare Scriva** — `cargo test -p scriva --test anchors -- --ignored
   --nocapture` with `SCRIVA_MAP_DOC=<probe.docx>` prints Scriva's answer
   for the same file, line by line.

`wordmap.ps1 -DocPath <doc> -Page <n>` is the same COM read for one page of
a *real* document, with paragraph formats alongside — the tool for "where
exactly does Word put this?" during a fidelity chase.

`topdf.ps1 -Path <doc> -Out <pdf>` and `pdfwords.py <pdf>` are the oracle
half of `cargo xtask compare`: Word's own rendering of a whole document, and
every word of it with the baseline it was set on, as TSV for a program rather
than for a reader. See adr/0003 for why that comparison is a tool rather than
an afternoon.

**Why that goes through paper when everything else here goes through COM.**
`Range.Information(5|6)` costs Word a layout pass per call — measured on this
machine at about 110ms, for every word — so the sixteen-page document behind
adr/0003 is hours of COM and seconds of export. It is also more honest: a
rendered page gives the *baseline*, and a baseline is the one horizontal two
renderers can be compared on without either having to guess at the other's
idea of where a line begins. Exporting needs a licensed Word, which the COM
reads above do not.

A word there is what a reader would call one, and it is reported at its **left
edge** rather than at the character the pen went down on first: a right-to-left
run is drawn from its right end, and measuring one side of a word against the
other side of the same word put one Arabic word 29 points out of place in
`rtl-and-cjk.docx` — a fault that was entirely the instrument’s. Nothing there
joins two marks that merely abut, either: Word’s export breaks "I/O" into three
and it is tempting to put them back, but a diagram draws its labels in whatever
order it likes and the same rule ran "SPI" together with a "Radio" fifty-three
points to its left. An invented word matches nothing on either side, where a
split one can still be paired — so where the two sides cut a word differently,
the matcher pairs them; see `glued` in `crates/wp-compare/src/diff.rs`.

`pdfwords.py` needs PyMuPDF, which is AGPL: a developer's measuring
instrument on a developer's machine, never linked into either application and
never redistributed with them — nothing under `tools/` ships. If that is
unwelcome, `pypdf` is BSD and can be made to answer the same question with
more work.

Probe documents are throwaways: generate, measure, delete. They are not
corpus files and must not become test dependencies.
