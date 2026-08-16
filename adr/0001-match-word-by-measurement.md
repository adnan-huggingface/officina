# ADR 0001 — Match Word by measurement, not by specification

**Status:** accepted (2026-08-15)
**Commits:** `4c00195` (the padding that lived in the style), `9ef798b` (the
half-point dance), with `59b176c` (printing/PDF) as the forcing function.

## Context

The goal for Scriva is a page that is **indistinguishable from Word's on
paper** — the user prints the same document from both applications and lays
one sheet over the other. Getting there exposed a fact that changes how
layout fidelity work must be done in this repository:

**Word's layout behaviour is not derivable from any document it ships or any
file it reads.** Three escalating examples from one resume:

1. *The specification is incomplete in practice.* Table cell padding lived in
   a table **style** (`styles.xml`), written as decimal strings
   (`w:w="55.0"`) that the schema calls integers — the Google Docs dialect.
   ECMA-376 describes the elements; nothing describes which producers put
   them where, and a corpus of Word-authored files never exercises the
   difference.

2. *The rendering model has unstated mechanics.* A table's horizontal rules
   **occupy their thickness**: content starts below the rule above it and
   every row is taller by it. A 2pt border moves text down exactly 2pt. No
   document states this; it is simply what Word draws.

3. *The metrics are not in the font.* Word lays a single-spaced line of
   Verdana 10 at **12.085pt**. The font file offers 12.1533 (hhea = win),
   10.698 (typo), and per-ppem VDMX values — and after fitting every
   combination of table, rounding unit, and ppem multiplier against pitches
   measured at nine sizes across seven faces, **no formula over the font's
   tables reproduces the laid pitches**. They are hinted, per-ppem quantities
   internal to Word's text stack. Worse, the laid pitch is not even what
   accumulates: Word tracks the exact design height in an accumulator and
   lays one line half a point taller whenever the debt reaches half a point —
   twelve lines of "identical" text contain three different heights, and
   *which* lines are tall depends on the page, because the accumulator resets
   at every page top.

Deriving this from first principles failed. Reading it from the OOXML file
was never possible — none of it is in the file.

## Decision

**Word itself is the oracle, and the unit of progress is a measured law.**

The measurement loop, now part of the repository (`tools/word-probe/`):

1. **Generate probe documents** — minimal synthetic `.docx` files isolating
   one variable: thirty to fifty-five identical lines of one face at one
   size; a table with one border weight and zero margins; a cell with thirty
   lines; three pages of one paragraph repeated.
2. **Measure in Word over COM** — a hidden `Word.Application` on a copy,
   `Range.Information(6)` reading every paragraph's position to the twip.
   Screenshots resolve ~0.5pt; COM resolves 0.05pt, and averaging a pitch
   over 28–54 line gaps resolves 0.002pt.
3. **Fit the law, not the numbers** — a model with structure (base pitch +
   correction quantum + threshold + reset scope) fitted until it reproduces
   *every* measured position within reporting noise. A fit that needs a
   contortion (a jump every second line) means the model is wrong, not the
   data.
4. **Implement the law with an honest fallback** — measured constants where
   we have them, and a mechanism that **bounds** the error where we do not.
   The half-point accumulator is the exemplar: for an unprobed face the base
   pitch is the design height rounded to 1/24pt, and the accumulator
   guarantees the cumulative divergence from Word never exceeds half a point,
   ever, on any page.
5. **Verify against the oracle, then against paper** — the `anchors`
   integration test prints where any line landed under the real fonts;
   Word's COM map of the same document sits beside it. The final judge is
   the user's overlaid printout.

Consequences for the code:

- `Shaper::pitch` returns a laid and an ideal height per face and size; the
  measured bases are a table in the application shaper with the provenance in
  its doc comment. **Empirical constants are architecture here**, on equal
  footing with parsed properties — they are what the file does not contain.
- The drift accumulator lives in the layout's `Flow`; page-top resets ride a
  second flow pass on the same two-pass frame the `PAGE` field already
  needed.
- The default `pitch` answers `base == ideal`, so the arithmetic test shaper
  and every engine test are untouched by the dance.

## Consequences

**Won:** page 4 printed from Scriva and from Word is indistinguishable —
line tops within 0.1–0.3pt of Word's across a page, with the half-point
payments landing on the *same lines*. The method is repeatable: any future
fidelity gap starts with a probe, not a theory.

**Paid:**

- A per-face, per-size table of measured constants that cannot be derived at
  build time and must be extended by running the probe loop on a machine
  with Word. The fallback bounds the cost of an unprobed face at ±0.5pt.
- Layout costs up to two flow passes when any half-point was ever paid.
- The law is Word-version-dependent in principle. The constants were
  measured against the user's Word 2016-era build; a future Word could
  change them. The probes make re-measuring an afternoon, not an
  archaeology.

**Stated limits carried forward:** a row resumed after a page break sits
about a point higher than Word resumes it; line-spacing multiples other than
single inherit the law by scaling, unprobed.

**The transferable lesson:** for any target whose behaviour is the product —
a renderer, a formula engine, a printer driver — the specification tells you
the vocabulary, the corpus tells you the dialect, and only *measurement of
the implementation itself* tells you the behaviour. Build the oracle early;
every hour spent on probe machinery repaid itself the same day.
