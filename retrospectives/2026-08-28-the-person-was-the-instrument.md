# 2026-08-28 — The person was the instrument

**Subject:** the four days that finished a sixteen-page Word 97-2003
specification in Scriva — what the loop cost, and what it should have been.
**Companion record:** `adr/0003`, which turns the first finding below into a
decision binding on this repository. This file is the wider discussion,
including the parts that are about working with an agent rather than about
Officina.

---

## What had just happened

A real document off a real desk — sixteen pages, a letterhead built out of
merged cells, an anchored page frame, a watermark, embedded Visio diagrams, a
table of contents and a numbered specification — was brought from "obviously
wrong" to "the remaining differences are inside the diagrams, not in the
document". Four sittings across four days. Every one of them ended in a
measured rule, a green gate, a deployment and a commit; none of them was wasted
work, and the result is the point of departure for the questions rather than a
complaint about them.

Each sitting took roughly three quarters of an hour, and each closed **one**
difference. That ratio is what the retrospective is about.

## The four questions

1. Was there too much back and forth, and could there have been less?
2. Could a single prompt have produced this, rather than a conversation?
3. Would a different model have done better — in particular one with strong
   vision, since the work looked visual?
4. If the work really is iterative and visual, would a lower reasoning effort
   have reached the same quality in less time?

---

## 1. The loop had a person inside it, and that was the whole cost

Every sitting ran the same comparison: export the document from the incumbent
application, extract glyph positions from both its output and ours, align them,
and read what disagreed. That comparison was never committed as a tool. It was
rebuilt from scratch, in a scratch directory, three or four times, and thrown
away each time. What survived between sittings was not the instrument — it was
a person who had looked at two screenshots and could describe, in prose, one
thing that was wrong.

Itemised, an attempt paid for:

| step | cost | needed to learn what was learned? |
| --- | --- | --- |
| the full test suite (1,767 tests) | minutes | no — once, before the commit |
| a release build | minutes | no |
| stop, redeploy, relaunch the GUI | minutes | no |
| screenshot, and a human comparing by eye | **the user's attention** | **no** |
| the comparison script, rewritten | an hour, repeatedly | no |

Everything in the "no" column existed because the comparison was not a program.
Where it was — the layout crate already rendered to PDF through *the same page
objects the screen paints* — the whole apparatus of window, build, deployment
and screenshot was decoration. The comparison could have cost thirty seconds
and produced a **ranked list** instead of one complaint.

Three specific failures came out of not having it, and all three generalise
past this project:

- **Wrong rules ship for want of a second case.** One rule here was wrong
  twice; each wrong version was refutable in seconds against a second document,
  and instead each was refuted by the user's eyes a build and a deployment
  later.
- **Right rules get deferred for want of a number.** One four-and-a-half-point
  error stood through two earlier sessions because the obvious fix visibly
  broke something adjacent. Whether a change trades one error for another is
  arithmetic over a residual total; without a total it is a judgement made from
  a screenshot, and the safe judgement is always to defer. It was settled
  within an hour of there being a number.
- **The remainder stays unranked.** Nobody could say what the twentieth-worst
  difference in the document was, because finding out cost an afternoon of
  somebody's attention. Work therefore arrived in the order a human happened to
  notice it, which is not the order of importance.

**The transferable rule.** Any project that must match an incumbent
implementation needs a *comparator* — and if the comparator is not a committed
program, it silently becomes a person. Two tells that this has happened:

> If you have written the same throwaway script three times, you have found a
> tool you refused to build.

> If a human is comparing two outputs by eye, that is not review. It is an
> unbuilt tool, and it is being paid for in hours.

And the shape the comparator should take, in any domain: **a ranked list of
differences, plus one scalar per case, computed across a corpus rather than the
single case in front of you.** The ranking directs the work. The scalar makes
regression arithmetic instead of judgement. The corpus is what stops a fix that
helps one case and harms two.

## 2. Could one prompt have done it? No — but the unit could be far larger

Not for want of prompting skill. The reason is structural, and it is worth
recognising in other projects because it decides how the work can be organised.

**The specification for this work was a running binary**, not a document. The
rules landed in those four days are not in any published standard: they are
measurements. That a table's stated indent is measured to its edge while the
modern format measures to the text, so the difference is the cell's padding.
That the application does not close up a kerning pair unless the run asks —
worth a fifth of a millimetre, which decided whether a word wrapped. That a
cell anchoring a floating shape is never vertically aligned, and the
compatibility flag named for that behaviour is inert. Each cost a round trip to
an oracle that exists only as software on one machine. No amount of thinking up
front removes a round trip to an oracle.

What *is* available is a much bigger unit of work per prompt — not one defect,
but a phase:

> Here is the comparator. Drive the residual to zero across these thirty cases.
> Every rule you land must be justified by a measurement against the oracle,
> recorded, and locked as a test with the number in it. Do not ask me between
> fixes; report at the end with what you could not explain.

That is hours of unattended work ending in something to read, and it is a
direct consequence of the comparator existing. The general test for whether
work can be batched this way:

> Can the result be checked without you? If yes, batch it. If no, then building
> the check *is* the work — do that first, and the batching follows.

The conversational, one-defect-per-turn shape was not a failure of instruction.
It is what the work necessarily collapses to when the person is the only
available check.

## 3. Vision was the weakest signal available, and a better vision model changes nothing

This one inverted on inspection. The work looked visual — pages, layout,
screenshots — so a model with strong vision looks like the obvious lever. The
magnitudes say otherwise. The three differences closed in the final two days
were **5.4 points**, **4.65 points**, and **0.14 points**. A screenshot resolves
roughly half a point. Two of the three sat at or under the noise floor of
looking, and the smallest was found by summing font advance widths and checking
five near-miss lines against the incumbent's own decisions about them. No
perception model, open or closed, sees a fifth of a millimetre spread across a
line of text and infers a kerning rule from it.

Vision had exactly one job in this project, and a person did it: **noticing that
a page looked wrong and saying where.** A ranked list of residuals retires that
job — it points more precisely, at more things, without being asked.

**The transferable rule:** before choosing a model for a matching problem, ask
what the smallest defect you must catch actually measures, and what instrument
resolves it. If the answer is arithmetic, no amount of perception helps, and
paying for perception buys nothing.

What the work *did* demand: long-context reasoning across a large workspace,
decoding binary layouts from specification prose, and **deriving** algorithms
rather than recalling them — this project forbids reading another
implementation, so a recalled answer is not merely unhelpful but inadmissible.
That is where capability is worth paying for. Running the comparator and
triaging its output is not, and a cheaper model is right for that half. But
note the ordering: **the heterogeneous split only becomes available once the
comparator exists.** Without it there is no cheap half to delegate.

## 4. Effort was not the constraint; evidence was

The honest post-mortem on the two wrong versions of that one rule: neither came
of thinking too little. Both were reasonable hypotheses that were cheap to
refute and got refuted expensively — a build, a deployment and a person's
attention, where a second test case and ninety seconds would have done it. More
reasoning depth would not have prevented either; a second case would have
prevented both.

So the rule is the opposite of the intuition:

> Work at the lower effort setting by default. Escalate when the measurement
> refuses to move after an attempt that was *properly measured* — that stall is
> the signal that the rule is **hidden** rather than merely unknown.

Two of the findings in those four days were genuinely hidden: one lived behind
a 1993-era compatibility bit that had to be found by diffing the incumbent's
own options list between two files, and one required deleting elements from a
converted copy of the document to see what the incumbent then did differently.
Those earned the depth. Roughly one difference in five did.

**The trap** is that effort feels like the quality dial whenever attempts are
expensive, because if every attempt costs an hour then every attempt has to be
right. Make attempts cheap and that pressure disappears — which means the
effort question and the comparator question are the same question wearing
different clothes.

---

## What I would do differently on day one of a project like this

1. **Build the comparator before building the thing it compares.** Its output
   is a ranked list of differences and one number per case. It runs headlessly.
   Estimate hours, not days; it pays for itself in the first week.
2. **Assemble a corpus, not a case.** A single flagship example teaches you to
   overfit to it, and hides every regression it does not contain.
3. **Let the ranked list choose the work**, in batches, unattended, reporting at
   the end. The human chooses what matters, not what is next.
4. **Ship every rule with its measurement inside its test.** "The incumbent puts
   this at 53.04" is a regression test. "Looks right" is not, and cannot be
   re-checked by anyone later.
5. **Keep the expensive rituals at the edges** — the full gate and the
   by-hand walkthrough belong before a commit, never inside a loop that ought
   to cost seconds.
6. **Keep the person for pointing and prioritising, never for measuring.** That
   is the whole of it, and everything above is a way of arranging it.

## What was already right, and should not be reformed on the strength of this

A retrospective's enthusiasm is dangerous to the things that were working. What
held up, and stays:

- **Treating the incumbent implementation as the oracle** (`adr/0001`). Every
  fix from those four days is a rule with a measurement behind it rather than a
  constant tuned until a page looked right, and that is why they compose
  instead of fighting each other.
- **The preservation invariant** — never rewriting what was not edited — and
  the harness that checks it on every case, twice, before every commit.
- **Using the application by hand after meaningful work** (`adr/0002`). It found
  a crash and two silent data losses that a green suite never touched. It is
  slow and it stays; it just does not belong inside the inner loop.
- **One commit per measured rule, with the story in the message**, a log of what
  was done, and a separate record of what each format taught. The commit
  history is legible a month later precisely because of this.

None of the above is what cost the time. The missing comparator is.
