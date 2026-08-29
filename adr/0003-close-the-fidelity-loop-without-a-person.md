# ADR 0003 — Close the fidelity loop without a person in it

**Status:** accepted (2026-08-28), and **built the same day** — see the
postscript at the end for what it cost and what it found.
**Held out of:** the retrospective of 2026-08-28, kept in full at
`retrospectives/2026-08-28-the-person-was-the-instrument.md`.
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

---

## Postscript: built, and made to agree with a settled case (2026-08-28)

`cargo xtask compare` exists, as `crates/wp-compare`. Three things about it
differ from what this record decided, and all three were forced by measurement.

**Word's half goes through paper, not through COM.** The decision above assumed
`Range.Information(5|6)`, which is how every probe in `tools/word-probe/` has
worked since ADR 0001. It cannot do a whole document: each call costs Word a
layout pass, measured here at about **110ms per word** — 200 words in 22.7
seconds — so the sixteen-page document is hours. One `ExportAsFixedFormat` is
seconds. The rendered page is also the better evidence: it gives each word's
**baseline**, and a baseline is the one horizontal two renderers can be
compared on without either having to guess where the other thinks a line
begins. The proof that this is the right frame is that the middle shift over
the whole document is dx +0.06 and dy +0.30 — the two sides agree about the
page, so what the tool reports is real.

**Words come from the rendering, not from Word's `Words` collection**, which
turns out not to be a list of words at all: it splits punctuation off and keeps
the trailing space.

**And the matching is by line, not by word — which the first version got wrong
in a way that no amount of care would have caught by reading it.** Aligning
words with a subsequence lies on any page that repeats itself, because the
alignment may pair one occurrence with a far-away other at no cost to its own
score the moment one side holds something the other does not. On
`watermark.docx` — one phrase three times a line, forty lines down, with a
watermark Word renders and this does not gather — it reported 298 words
hundreds of points out of place on a page whose real fault is sub-point. The
demonstration document has enough unique lines that it never showed there at
all, so **one validated document was not enough to trust the instrument**. It
now matches lines first, anchors on the lines whose text occurs exactly once on
each side and so cannot slide, and refuses any pairing sitting more than three
lines from the page's own median offset. The tell, worth keeping: a large
*median* shift means the matching is wrong, not the layout, and it is now the
first line of every report.

**The acceptance test this record demanded.** The finished harness was run
against the layout of `a1b0ed9` — before the table indent and the kerning were
fixed by hand — in a throwaway worktree, and against HEAD:

|                                      |   before | at HEAD |
| ------------------------------------ | -------: | ------: |
| words more than a point out of place  |      635 |     174 |
| worst single word                     | 53.63pt  |  4.77pt |
| page 5, shifted about 5.4pt           |       58 |       0 |
| words neither side could place        |    1,262 |   1,244 |

The 58 are the demonstration document's table — `0x03`, `Enabled`, `Timer`,
`CCP1_CCP2_USE_TIMER1` — at the half-gap this project spent an afternoon
finding by eye. The eighteen extra unplaceable words are the line that
rewrapped for want of the kerning Word never asked for: a line that breaks
differently stops being the same line, which is the truthful way for that to
appear. **The instrument finds both known answers without being told where to
look, and says nothing about either once they are fixed.** That, and not the
code, is what makes it evidence.

**What it found that nobody had pointed at.** The 174 words still out of place
at HEAD are almost all horizontal drift *inside justified lines* on pages 7, 8
and 11. The corpus then said what causes it: on `watermark.docx` one phrase
repeated across a line drifts +1.24, +7.93 and +14.62 points over three
repetitions — about **0.45pt per space**. Our space advance is wider than
Word's. Sixteen sittings of looking at these pages never surfaced it; two runs
of the tool did, on two unrelated documents, and agreed.

**The corpus, measured for the first time.** Twenty-five documents, one number
each: eight of them are already at zero, and the work is concentrated in five —
`headers-footers.docx` (328 out), `floating-image-wrap.docx` (244, worst
50.23pt), `table-spanning-pages.docx` (244, worst 25.26pt),
`picture-watermark.docx` (169) and `watermark.docx` (115). None of those five
had a number before today.

**The floor, stated plainly.** 1,217 words on the demonstration document that
Word laid and this does not gather: the text inside the pasted Visio metafiles,
which Scriva draws from a recording rather than from a line. Out-of-place and
unplaceable are separate columns everywhere they are reported, because one is
work and the other is mostly what the instrument cannot see, and a single
number holding both tells you neither.

**Postscript, the same evening: what sharpening the instrument found.** Going
back over the harness in one sitting turned up more faults in the *measurement*
than in the layout, and every one of them had been reporting itself as a
layout fault. A word measured at its first letter rather than its left edge put
an Arabic word 29.48pt out of place, where the real difference is 0.68pt. A
fallback that walked both sides in lockstep reported forty unplaceable words on
a page where nothing had moved as much as a fifth of a point, because Word
raises a footnote's reference onto its own baseline and this project does not.
The threshold deciding what counts as one line stood at 3.0pt, and the tightest
gap between two genuinely separate lines in the whole corpus is 3.00pt exactly.
The cap on what could be matched paired a page's words off in order and said
nothing about it. And the 1,217-word floor this record called a floor was not
one: the type inside a pasted diagram is reachable through the paper
renderer's own player, and gathering it took the demonstration document from
3,738 matched words to **4,897**, with the unplaceable count falling from 1,244
to 161.

The sharpest thing the day taught is what happened in between. As the faults
were fixed one at a time, the newly-gathered diagrams reported a label 56.78pt
out of place, then 35.59pt, then nothing: the document's worst is 4.77pt and is
the justified drift already known. Each of those numbers was the matcher, not
the page, and each would have been entirely convincing as a layout bug worth an
afternoon. **A finding from an instrument nobody has measured is a hypothesis
about the instrument** — which is the same argument this record makes about a
person, turned on the thing built to replace them.

This is the record's own claim turned on the record's own tool. *A person may
point at a difference; a person may not be the instrument that measures it* —
and an instrument nobody has measured is not yet evidence either. It is worth
being plain that all of this was found by looking at the tool's own output on
documents whose answers were already known, and none of it by reading the code.

**What made it a gate.** `LAYOUT.md` now records what every corpus document
measures and `cargo xtask compare --check` fails on any that got worse. Until
that existed the harness produced a ranked list — which says where to work, and
says nothing at all about work that has come undone.

It was kept out of `cargo xtask check` at first, because the gate has to keep
working on a machine with no Word. That reason stopped being true the moment
Word's readings of the corpus were committed under `corpus/rendered/`: its
answer for a document cannot change until the document does, so the check needs
Word only to renew a reading of something that actually changed. `--check` now
runs inside `cargo xtask check` with every other gate, which it had to, because
nothing about a layout regression is noticeable — the tests pass, the document
opens, and a line sits a point and a half further down the page.

---

## Postscript: the page, and not only the type (2026-08-28)

Two things were still owed, and they were the same kind of debt: a claim the
tool made that nothing had measured.

**Only the type was compared.** A rule, a shading, a border, a picture — none
of them moved a number. It was the largest thing the instrument was blind to,
and the worst kind of blindness, because a page whose whole table has slid half
a centimetre reads as a page with a handful of words out of place. The fix is
`crates/wp-compare/src/marks.rs`: every rectangle of ink that is not type,
compared the same way and counted in its own column of `LAYOUT.md`.

The one thing that made it hard is worth recording, because it is the third
time the same lesson has arrived. **Neither renderer draws a border the way the
other does.** Word's export lays a table's top edge as a little filled square at
each corner with the spans between them; Scriva lays one rule per cell. A page
of ink the two agree about to a hundredth of a point arrives as thirty
rectangles against nine, and comparing rectangles as they arrive reports a
document with nothing wrong as a document with nothing right. So both readings
are reduced to their ink — duplicates dropped, touching collinear pieces run
together — by one function that cannot tell which side it is working on. Any
decision that shapes what is compared has to be taken from both readings at
once, or one side is held to a convention it never agreed to. The line cutting
taught that twice already.

There is a second half to it, which the first attempt got wrong: **a rule is
broken where another rule crosses it.** Word's corner square can be run into the
horizontal border or into the vertical one but not into both, so whichever pass
took it left the other with a rule in five pieces, each one crossing short of
its neighbour — nine rules against one, with nothing wrong on the page at all. A
break as wide as the rule's own thickness is bridged; a gap between two boxes is
a gap somebody meant.

What it found on its first run was worth the trouble. `floating-image-wrap.docx`
puts one of its two images 120 points below where Word puts it, and the other
exactly right — a fault no word count could name, because the words around a
floated image were already reported as unplaceable. `header-footer-footnote.doc`
draws no footnote separator, because it lays no footnote band at all. And on
`nested-tables.docx` and `table-spanning-pages.docx` the borders are out by the
same 5.7 and 24.9 points as the words, which is the more useful kind of answer:
the furniture corroborates the type, and one fault is one fault rather than two.

**What the instrument was for, finally used.** `floating-image-wrap.docx` was
the first thing the marks column found: an inline picture 120 points below where
Word puts it, on a document whose words were already so wrong that nothing could
be read from them. The cause was not where anyone would have guessed — 120 pt is
exactly the *anchored* picture's height, which makes "the float is contributing
to the line" irresistible and wrong. A float never enters a line at all. The 120
was a reserved flow item, and the reservation was the fault.

The fix that follows from that is a feature rather than a repair, and an
adversarial pass refuted the small version of it by building and measuring it:
stopping the reservation alone traded a 50 pt vertical error for a 320 pt
horizontal one, because the engine could only narrow a line to one side of a
float while Word sets text on both. So both halves landed together — a band that
knows where it starts, and a hole a line's pen steps over. One document moved:

|                                      | before | after |
| ------------------------------------ | -----: | ----: |
| words more than a point out of place  |    244 |   319 |
| of those, more than five points out   |    244 | **0** |
| words neither side could place        |    152 | **0** |
| worst single word                     | 50.23pt| 4.29pt|
| marks only one side drew              |      2 | **0** |

`out` rising while everything else collapses is the record working as designed:
a hundred and fifty-two words that could not be placed at all can now be
measured, and they are two points out. What is left is the space advance and the
leading this project already knows about — dx +2.03, dy −2.06 in the middle,
which is the same drift the corpus reported from two other documents.

**And then the check fetched its own evidence, which is the same fault one
level down.** The test above was written, run, and passed — and the very next
commit went out broken anyway. A docstring fix to `pdfink.py` landed after
`--record` and before the commit, so all twenty-five committed readings named a
probe script that no longer existed. `cargo xtask check` passed here, because
`read()` found them stale, asked Word for twenty-five fresh renderings, and
measured against those. On a machine without Office every document came back
"taken from an older probe script". A cache that repairs itself is a cache; a
*check* that repairs itself is a formality. `--check` is now `Renew::Never` and
cannot start Word at all: where a reading is stale, that is the finding, and the
message says to run `--refresh` and commit what it writes. There is a unit test
for it that runs on a machine that *has* Word, because that is the only machine
where the fault is invisible.

**And the machine with no Word had never existed.** Everything above rests on
the corpus being checkable without Office, which is why `--check` may sit inside
`cargo xtask check`. It is written down in three files and it had never once
been executed: every run has been on this machine, where a stale reading is
renewed in seconds and nobody notices. A claim about what happens when something
is missing is exactly the kind that stops being true quietly. So
`crates/wp-compare/tests/without_word.rs` runs the tool with **nothing on its
PATH** — Word is reached only by starting `powershell` and the rendering read
only by starting `python`, and neither can be found without one. The corpus
checks clean, and the two ways of genuinely needing Word now say which is which:
a document nobody has a reading of, and a reading that has gone stale, want
different things done about them and no longer read alike.
