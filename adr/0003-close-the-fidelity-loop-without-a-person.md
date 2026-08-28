# ADR 0003 — Close the fidelity loop without a person in it

**Status:** accepted (2026-08-28). The harness this record decides on **is not
yet built**; what is recorded here is the decision to build it and the evidence
that forced it — four days of doing its work by hand.
**Commits paid for by its absence:** `a1b0ed9`, `83ec414`, `303fd69`, `f0b70e0`
— four sittings, one reported difference each.

## Context

ADR 0001 ends with a sentence this repository then half-followed for a
fortnight: *build the oracle early; every hour spent on probe machinery repaid
itself the same day.* That was learned for **probes** — synthetic documents
isolating one variable, measured over COM — and `tools/word-probe/` is the
machinery it bought. Nothing equivalent was ever built for the other half of
the work: **a whole real document, every page, compared against Word's own
rendering of it.**

So the last four days ran that comparison by hand, four times over. The loop,
each time, was: export the document from Word over COM to PDF; pull character
origins out of both PDFs with PyMuPDF, whose `rawdict` origins are true
baselines; group them by baseline, because PyMuPDF's own notion of a line is
not a reliable key — a wide tab gap splits one of Word's lines into two of
ours, and Word's PDF carries a space glyph where we emit none; strip
whitespace; diff; read the survivors. The scripts that did it were written from
scratch in a scratch directory and thrown away three or four times over.

**The instrument was in the repository the whole time.** `wp-print`'s own first
line says what it is: *"This crate takes the same `wp_layout::block::Page`s the
screen paints."* Scriva's exact layout is therefore available as a PDF with no
window, no release build, no deployment and no screenshot. The missing half of
the oracle was never hard; it was simply never named as a tool, so it was
rebuilt by hand whenever it was needed and never once improved.

What that cost, itemised from the four sittings:

| step in the loop | cost | needed per attempt? |
| --- | --- | --- |
| full `cargo xtask check` (1,767 tests) | minutes | no — once, before the commit |
| release build | minutes | no |
| stop, redeploy, relaunch the window | minutes | no |
| screenshot; a person compares by eye | **the user** | **no** |
| the probe script, rewritten | an hour, repeatedly | no |

An attempt cost the best part of an hour and returned a verdict on **one**
difference, because the difference had been found by a person looking at a
screen and describing it in prose. Three consequences followed, each with a
name:

1. **Wrong rules shipped for want of a second document.** The rule for the tab
   after a list number was wrong twice. *The indent always wins* broke `13.1.`;
   *the nearer of the indent and the next stop* broke page 16's `b)`. Either is
   refutable in seconds against a second document — and instead each was
   refuted by the user's eyes, a gate and a deployment later.

2. **Right rules deferred for want of a number.** The letterhead's 4.65 points
   stood on all sixteen pages through two earlier sessions, deferred each time
   because the obvious fix visibly broke the cell beside it. Whether a change
   trades one error for another is arithmetic over a residual total; without
   one it is a judgement made from a screenshot, and the safe judgement is
   always to defer. It was settled within the hour of there being a number.

3. **What remains is unranked.** Nobody can say what the twentieth-worst
   difference in that document is, because finding it costs an afternoon of
   somebody's attention.

Underneath all three lies the reason eyes are the wrong instrument for this
work at all. The differences that mattered here were **5.4 points** (a table
ruled half a column gap right of its edge), **4.65 points** (the letterhead),
and **0.14 points** — a kerning pair Word never asked for, a fifth of a
millimetre across a line of Arial, which decided whether the word "message"
wrapped. ADR 0001 already measured what a screenshot resolves: about half a
point. Two of those three differences sit at or under the noise floor of
looking.

## Decision

**A person may point at a difference; a person may not be the instrument that
measures it.** Three parts.

**1. The comparison harness is the inner loop.** `cargo xtask compare <file>`:
lay the document through `wp-print` to PDF; export the same file from Word over
COM; compare glyph origins; report the worst residuals per page, ranked, and
one scalar for the document. No window, no deployment, no screenshot, no
person. The grouping rules above belong to the harness rather than to whoever
runs it — compare by baseline and insensitive to whitespace, because that is
what the two producers actually agree on.

**2. The corpus is the unit, not the document.** One scalar per file across the
whole corpus, so that a rule which helps one document and harms two says so on
the run that introduced it rather than two sessions later through somebody's
eyes.

**3. The gate and the human ritual move to the edges.** `cargo xtask check` and
`cargo xtask fidelity` before a commit, as now. ADR 0002's recreation by hand
after meaningful UI work, as now — it found a crash and two silent data losses
and it stays. Neither belongs inside a loop that ought to cost thirty seconds.

What the person is still for: deciding which differences are worth closing,
judging the ones no residual can score, and the overlaid printout that ADR 0001
makes the last judge. Pointing is not measuring, and only the pointing was ever
theirs.

## Consequences

**Won** — projected, and labelled as projected until the harness exists: an
attempt costs seconds rather than an hour; a sitting closes the top of a ranked
list rather than one reported complaint; a regression is a number going up. And
the second-order prize, which is the larger one: the work becomes
**supervisable rather than interactive**. *Drive the residual down across the
corpus, justify every rule with a measurement, report what you cannot explain*
is hours of unattended work ending in a report, where today it is a person and
an agent taking turns at forty-five minutes a turn.

**Paid:**

- A licensed Word and COM, the same constraint `corpus/generate.ps1` and
  `tools/word-probe/` already carry. The harness must skip itself cleanly where
  Word is absent, as those tests already do.
- A glyph-origin diff carries its own noise, and a harness is not evidence
  until it has been made to agree with a case already settled by hand. ADR 0002
  paid this same bill for its pixel-driven steps: an unexamined flake gets
  reported as a finding.
- **A residual total rewards only what it can see.** Colour, rules, images and
  the metafile text inside this document's diagrams do not move a number
  computed from glyph positions, and those pages will still need eyes. A scope
  the report does not state loudly is a claim the report makes falsely.
- It does not go in CI, for the same reason nothing else that needs Word does.

## On instruments, models and effort

The retrospective that produced this record asked three further questions, and
their answers belong with it.

**Vision is the weakest oracle available for this work, and a better vision
model does not change that.** The three fixes of the final two days turned on
5.4, 0.14 and 4.65 points. No model reading a screenshot sees any of them; the
kerning difference was settled by summing Arial's `hmtx` advances and checking
five near-miss lines against Word's own decisions about them. Vision had
exactly one job in this project — a person noticing that a page looked wrong
and saying where — and a ranked residual list retires that job.

**What the work does demand** is long-context reasoning across the workspace,
decoding binary layouts from specification prose, and *deriving* every
algorithm rather than recalling one, because the provenance rule forbids
reading another implementation. That is where capability is worth paying for.
Running the harness and triaging its list is not, and splitting the work at
that boundary is sound — but the boundary only exists once the harness does.

**Effort was not the constraint; evidence was.** Neither wrong tab rule came of
thinking too little. Both were cheap-to-refute hypotheses refuted expensively,
because refuting them cost a build, a deployment and a person's attention
instead of a second document and ninety seconds. The rule that follows: work at
the lower setting by default, and escalate when the residual will not move
after an attempt that was properly measured — that stall is the signal that a
rule is *hidden* rather than merely unknown, which is what `Copts60.fNoTabForInd`
and the float-suppresses-alignment rule both turned out to be. Roughly one
difference in five. The trap is that effort feels like the quality dial
whenever attempts are expensive, because every attempt must then be right; make
attempts cheap and the pressure goes away.

**The transferable lesson:** ADR 0001 said to build the oracle early, and this
is the bill for having built half of it. A probe answers a question you already
knew to ask; a whole-document diff **tells you which questions to ask, and ranks
them**. That is the difference between an instrument and a search. Until the
loop closes with nobody inside it, the person is the instrument — and a person
resolves half a point at best, and reports one defect per look.
