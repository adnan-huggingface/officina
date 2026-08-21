# ADR 0002 — Test by using it as a human

**Status:** accepted (2026-08-20)
**Commits:** `40c8d38` (everything one afternoon of it found), with the corpus
recreations left outside the repository as working artifacts.

## Context

The suite stood at some 1,350 tests and the fidelity harness at 30/30 on both
checks when three real documents — a lorem-ipsum sample, a two-page resume
built out of layout tables, a 23,000-cell engineering workbook — were
recreated from scratch **through the applications' own menus, keyboard and
mouse**, exactly as a person at the keyboard would. Nothing else changed, and
one afternoon of it found:

1. Scriva had no way to insert a table at all, and a new document offered no
   styles to apply — two features every recreation needed in its first minute.
2. Ctrl+B on an empty paragraph did nothing, so "make the next thing I type
   bold" — the way people actually format — silently produced plain text.
3. Typing an en dash could crash the whole application: a per-frame probe
   sliced the paragraph text at `len - 1`, which is the middle byte of a
   trailing multi-byte character.
4. A cell merge made in Calx was lost by the very next save: the model had
   it, the menu showed it checked, and the writer never emitted it.
5. Small frictions a spreadsheet user hits in seconds: a sheet tab that does
   not rename on double-click, a size list without 10.5pt, a Name Box that
   displays the address but will not accept one, a File ▸ New that discarded
   unsaved work without the question Ctrl+N asks.

None of this is a gap in the suite's diligence. The tests exercise models,
readers, writers and layout — and every one of those was correct. What no
test exercised is the **path from a keystroke to the model**: the command
that is missing rather than wrong, the caret state nobody's fixture is in,
the feature whose model half and writer half were each tested and never
introduced to each other. The merge bug is the type specimen: `merges` was
read faithfully, preserved faithfully, compared faithfully — and a merge
*born in the UI* fell into the one seam no unit crosses.

## Decision

**Recreating real documents through the running application is a standing
test practice, on equal footing with the suite.** The exercise, repeatable:

1. **Pick real documents** — files from the wild, not fixtures. Their value
   is exactly that nobody wrote them to be recreatable.
2. **Drive the real binaries** with OS-level input — `SendInput` keystrokes
   and clicks, screenshots read back after every few steps. No test hooks,
   no reaching into the model: if a human cannot reach a feature through the
   menus, the exercise cannot either, and that inability *is the finding*.
3. **Go start to finish** — New through Save As. Half a recreation proves the
   entrypoint; the save is where the silent losses live.
4. **Diff the saved file against the original with an independent reader,
   then render both through real Word or Excel and look at the pages side by
   side.** A text diff is structurally blind to everything that is not a
   character: the first visual pass over "validated" recreations found body
   text uniformly bold-italic where the original had three bold runs, a
   missing chart, bullet lists flattened to plain paragraphs, and a missing
   title that lived in a header part no `document.xml` diff would ever
   count. The recreation does not have to be byte-faithful; it has to
   contain everything — and "everything" is judged by eyes on rendered
   pages, not by word counts.
5. **Fix what it finds before finishing** — the recreation is not done while
   a wall it hit still stands. Each wall becomes either a fix in the same
   sitting or a named, recorded limitation.

Rules for the driver, each one paid for:

- **Find the target by process name, never by window-title substring.** A
  terminal whose title echoes the command line also "matches Scriva", and
  after the en-dash crash the remaining keystrokes typed half a resume into
  the wrong application's window.
- **Verify, before every single input, that the foreground window belongs to
  the target process — and abort otherwise.** The dead-app case above is one
  reason; the other is that keystrokes into the wrong window are not a failed
  test but an incident.
- **Trust nothing about coordinates.** The same binary opened at two window
  geometries on the same monitor in one afternoon; a freshly typed two-line
  cell makes its row taller and moves every target below it. Keyboard
  navigation survives what pixel positions do not; when pixels are the only
  way, recalibrate from a screenshot after anything that changes layout.
- **Native dialogs are windows of the same process, not the same window** —
  the foreground check must accept them, and one opened entirely off-screen;
  find them by window enumeration, never by faith.

## Consequences

**Won:** five real defects in one afternoon — a crash, two silent data
losses, two absent features — every one invisible to a green 1,350-test
suite, plus the fixes, landed and gated the same day. And a second-order
prize: the recreations themselves are exactly the "files from a second
producer" the corpus README asks for, made by the most honest second
producer there is — this suite's own UI.

**Paid:**

- A run takes minutes and a desktop; this does not go in CI. It is an
  afternoon practice for a person (or an agent) after meaningful UI work,
  not a gate on every commit.
- Pixel-driven steps flake, and each flake costs a diagnosis before it can
  be trusted as a finding rather than driver noise — the one lost sentence
  that looked like an editor bug was a timing artifact.
- The exercise is only as honest as its validation. The first run declared
  three recreations "text-identical" and shipped; a later visual pass showed
  one looked nothing like its original and another held 62 of the original's
  46,437 cells. A scope cut the report does not state loudly is a claim the
  report makes falsely.
- Validation-by-Office needs a machine with a licensed Word and Excel, the
  same constraint the corpus generator already carries.

**The transferable lesson:** the suite proves the code does what the code
intends; only using the product proves a person can reach that code. Silent
data loss lives in the seams — between a menu and a command, a command and a
model, a model and a save — and an end-to-end recreation is the only test
that crosses every seam in one motion. When a feature ships, the question is
not "do its tests pass" but "has anyone, human or driver, actually done the
thing with their hands."
