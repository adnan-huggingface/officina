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

Probe documents are throwaways: generate, measure, delete. They are not
corpus files and must not become test dependencies.
