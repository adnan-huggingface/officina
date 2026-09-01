# The OpenDocument writer

What is left of `.odt` support. **This file is immutable while the work runs.**
Nothing that does the work may edit it — not to reword an item, not to remove
one, and above all not to mark one done. A definition of done that the worker
can edit is not a definition of done.

**Nothing here is ticked, because nothing here is ticked by hand.** Each item
carries a `verify:` command, and the item is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all and reports the ones
that do not. There is no ledger to keep and none to forge: the repository
either answers the question or it does not.

What that buys, stated plainly: an agent cannot finish this by claiming to have
finished it. It can only make commands pass. The failure this replaces is a real
one from the session that wrote the reader — the plan was staged, the agent
treated a stage boundary as permission to stop, and wrote the shortfall up as a
considered design position rather than as work skipped.

Two consequences worth being honest about:

- A `verify:` command is only as good as what it checks, and the first draft of
  this file got that wrong in a way worth remembering: it used
  `cargo test -p wp-odf splice`, and **a cargo filter that matches no test at
  all exits zero**. Nine of these fourteen items reported themselves finished on
  the day the plan was written. They now go through
  `.claude/hooks/proved.py`, which fails unless at least one test actually ran.
- The rest can only check that a file says something, which is weaker. They are
  marked, and they are the ones to distrust.
- If an item turns out to be wrong or impossible, that is a conversation with a
  person and an edit to this file by that person. It is not something to work
  around. Say so in `PROGRESS.md` and stop.

`PROGRESS.md` is where the work is narrated, and `LEARNINGS.md` is where what
the format taught goes. Neither is read by the gate. They are for people.

---

## The writer

The reader is done and measured. The container reads every entry of a package,
filters none and writes them all back — including the one Word's own ODF export
leaves with no media type — and `cargo xtask fidelity` holds it to that. What is
missing is the half that rewrites `content.xml` a paragraph at a time.

**Reprinting the part whole is not an option.** It would pass a test that the
edit came back and fail the one that matters: that everything the reader does
not model came back too. `crates/wp-docx/src/write/mod.rs` and its `splice.rs`
state the design; read them first.

### W1 — a splicer that keeps the bytes an event came from

So that an element can be copied exactly or replaced whole.

    verify: python .claude/hooks/proved.py -p wp-odf splice

### W2 — `content_out` copies everything it did not change

Walks `content.xml` and copies every byte of it except the `<text:p>`,
`<text:h>` and `<table:table>` elements that *read back differently* from the
model. Changed is defined by re-reading, never by remembering: that is the only
definition that cannot drift from the reader.

    verify: python .claude/hooks/proved.py -p wp-odf content_out

### W3 — a changed paragraph is emitted as ODF

`<text:p>`/`<text:h>` with its style, `<text:span>` for a run that carries one,
`<text:s text:c="n"/>` for the second and later spaces of a run, `<text:tab/>`,
`<text:line-break/>`, and text escaped the way the format escapes it.

    verify: python .claude/hooks/proved.py -p wp-odf emit

### W4 — direct formatting mints an automatic style

ODF has nowhere else to put it: a run made bold by hand is not bold in the file,
it names a `<style:style style:family="text">` that is. The minted styles go into
`<office:automatic-styles>`, which stands before the body and so must be written
after it is known what the body needs.

    verify: python .claude/hooks/proved.py -p wp-odf automatic_style

### W5 — `save` and `flush`, with the signatures `wp_docx` uses

    verify: python .claude/hooks/proved.py -p wp-odf write::

### W6 — `container_for` authors a package for a document that never had one

So Save As `.odt` works from a `.docx` or a markdown file.

    verify: python .claude/hooks/proved.py -p wp-odf blank

## The proof

### P1 — an untouched save is byte-identical

Not "a person noticed once". A test.

    verify: python .claude/hooks/proved.py -p wp-odf untouched

### P2 — fidelity check 2 covers `.odt`

Open, change the text of one paragraph, save, reopen, and account for every byte
that moved. Both corpus documents pass it, and the entry with no media type is
still there afterwards.

    verify: python .claude/hooks/covers_odt.py

## The application

### A1 — an `.odt` saves in place

`Format::Odt::is_writable()` is true, `save` writes an `.odt`, and the path is
no longer rewritten to `.docx`.

    verify: python .claude/hooks/proved.py -p scriva odt

### A2 — `FORMATS.md` says read + write, and the excuse is gone

Weak check: it reads the file rather than the behaviour. A1 is the real one.

    verify: python -c "import sys,pathlib; t=pathlib.Path('FORMATS.md').read_text(encoding='utf-8'); sys.exit(0 if 'read + write' in t.split('.odt')[1][:200] and 'why it is not written' not in t else 1)"

## The page

Two differences from the reference are known, traced and unfixed. Neither is a
mystery; both are work.

### L1 — `word-odf-export.odt` lays in five pages

As the reference does. The per-page shortfall is traced to the last paragraph of
its header, which holds the watermark this reader empties.

    verify: python -c "import sys,pathlib,re; t=pathlib.Path('LAYOUT.md').read_text(encoding='utf-8'); m=re.search(r'`word-odf-export.odt`\s*\|\s*(\d+)',t); sys.exit(0 if m and m.group(1)=='5' else 1)"

### L2 — the watermark is drawn, or its absence is argued

A `<draw:custom-shape>`. If it is decided that it should not be drawn,
`PROGRESS.md` argues why and `LAYOUT.md` records the residue deliberately — and
then this item is closed by a person editing this file, not by the worker.

    verify: python .claude/hooks/proved.py -p wp-odf custom_shape

## The record

### R1 — `PROGRESS.md` narrates the writer

Weak check: it looks for a section, not for whether it is any good.

    verify: python -c "import sys,pathlib; sys.exit(0 if 'The OpenDocument writer' in pathlib.Path('PROGRESS.md').read_text(encoding='utf-8') else 1)"

### R2 — `LEARNINGS.md` records what writing the format taught

In the voice of the entries already there: what was believed, what was measured,
what it cost. Weak check, same reason.

    verify: python -c "import sys,pathlib; t=pathlib.Path('LEARNINGS.md').read_text(encoding='utf-8').lower(); sys.exit(0 if 'automatic style' in t else 1)"
