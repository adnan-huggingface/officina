# The save nobody watched a person make

What is left of `.odt` support. **This file is immutable while the work runs.**
Nothing that does the work may edit it — not to reword an item, not to remove
one, and above all not to mark one done. A definition of done that the worker
can edit is not a definition of done.

**Nothing here is ticked, because nothing here is ticked by hand.** Each item
carries a `verify:` command, and the item is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all and reports the ones
that do not. There is no ledger to keep and none to forge: the repository
either answers the question or it does not.

`PROGRESS.md` is where the work is narrated, and `LEARNINGS.md` is where what
the format taught goes. Neither is read by the gate. They are for people.

## Why there is a second round

The writer is written, measured and merged, and the round that built it passed
every gate honestly. It was still short, and the shortfall was **in this file
rather than in the work**.

The plan that was approved before any of it started carried six verification
steps. The sixth read: *per `AGENTS.md`, after the UI work, recreate a document
through the running app, New → Save As `.odt` → reopen, by menu and keystroke.*
When that plan was converted into gated items, that step did not survive — not
by decision, but because **every item here has to be a command that exits zero,
and a person at a keyboard is not one**. The whole application half of the
format became a single item proven by two unit tests, and nobody noticed,
because the document that would have said otherwise was the document being
replaced.

That is a bias in the machinery, not an accident: a gate made of shell commands
will always be missing whichever requirements are not shell commands, and it
will be missing them silently. ADR 0002 records what it costs — one afternoon
of driving the real binaries found a crash and two silent data losses that
1,350 passing tests never came near, and every one of them lived in the seam
between a keystroke and the model, which is the one seam no unit test crosses.

So this round finishes the save: the cases the first round's two tests left
out, and the exercise the first round dropped — this time with an exit code of
its own, so that it cannot be dropped again in silence.

---

## The saves that were never tested

The application half of the last round was item A1, and A1 was two tests: a
save in place over an existing `.odt`, and a Save As from a document that never
had a package. Everything below is a save a user will make in the first week
and no test has ever made.

### S1 — Save As writes a new `.odt` and leaves the old one where it was

Open a corpus `.odt`, edit it, save it to a *different* path. The new file
holds the edit, the original is byte-for-byte what it was, and the application
is now editing the new one.

    verify: python .claude/hooks/proved.py -p scriva save_as_odt

### S2 — both cross-format directions

`.odt` saved as `.docx` and `.docx` saved as `.odt`. The chokepoint lets go of
the package the document arrived in and authors the other — `self.container` and
`self.package` are never both live — and what lands on disk opens in the format
its name claims. This is read as correct today and has never been run.

    verify: python .claude/hooks/proved.py -p scriva cross_format

### S3 — saving twice in one session

Edit, save, edit again, save again. The second save writes through a container
the first one already flushed, which is the state no test has ever put it in,
and the parts nobody edited are still byte-identical after the second one.

    verify: python .claude/hooks/proved.py -p scriva saves_twice

### S4 — a save that cannot be written says so and loses nothing

The target is read-only, or the directory is not there. `save_odt` has an error
arm that composes a message about the file being open elsewhere; no test has
ever reached it. Afterwards the document is still dirty, still has its path, and
the file on disk is untouched — a failed save must not be a lost document.

    verify: python .claude/hooks/proved.py -p scriva save_that_fails

## The path from a keystroke to the model

### U1 — a driver that reaches Save As the way a person does

`tools/drive/scriva_odt.py`, run as `--out <dir>`: launches the built Scriva,
makes a document through the menus and the keyboard, saves it as `.odt` through
the real Save As dialog, reopens it, and reads screenshots back as it goes. It
types the sentence `SCRIVA ODT DRIVE` so that what lands on disk can be tied to
the run that made it.

**ADR 0002's driver rules are binding and each was paid for**: find the window
by *process name* and never by title substring; check before every single input
that the foreground window belongs to that process, and abort rather than type
into another application's window; trust no coordinate that a screenshot has not
just confirmed. A driver that types a document into a terminal is not a failed
test, it is an incident.

The exercise is the point, not the script: if a feature cannot be reached
through the menus, **that inability is the finding** and it goes in
`PROGRESS.md` rather than being routed around with a test hook.

    verify: python .claude/hooks/drove_it.py

### U2 — what the drive found is fixed, or written down as a wall

Weak check: it looks for a section, not for whether the walls in it were real
or honestly reported. The recreation is not done while a wall it hit still
stands — each one is either a fix in this sitting or a named, recorded
limitation. ADR 0002 §5.

    verify: python -c "import sys,pathlib; sys.exit(0 if 'What driving it found' in pathlib.Path('PROGRESS.md').read_text(encoding='utf-8') else 1)"

## The record

### R1 — `LEARNINGS.md` records what the keystroke path taught

In the voice of the entries already there: what was believed, what was
measured, what it cost. Weak check, same reason as above.

    verify: python -c "import sys,pathlib; t=pathlib.Path('LEARNINGS.md').read_text(encoding='utf-8').lower(); sys.exit(0 if 'the keystroke path' in t else 1)"
