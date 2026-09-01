# The OpenDocument writer

What is left of `.odt` support, as boxes. **This file is the definition of done**
— `python .claude/hooks/gate.py` fails while any box here is unticked, whatever
else is green, and `.claude/hooks/stop_gate.py` will not let a session end while
it does.

Tick a box only when the thing it names is true *and* something proves it. A box
ticked because the code was written is a box ticked too early; a box ticked
because a test passes is a box.

If one of these turns out to be wrong, or impossible, say so in `PROGRESS.md`
and leave it unticked. An honest unticked box is worth more than a tick nobody
believes.

## The writer

The reader is done and measured. The container reads every entry of a package,
filters none and writes them all back — including the one Word's own ODF export
leaves with no media type — and `cargo xtask fidelity` holds it to that. What is
missing is the half that rewrites `content.xml` a paragraph at a time.

**Reprinting the part whole is not an option.** It would pass a test that the
edit came back and fail the one that matters: that everything the reader does
not model came back too. `crates/wp-docx/src/write/mod.rs` and its `splice.rs`
state the design; read them first.

- [ ] A splicer over `content.xml` that hands back each XML event together with
      the source bytes it came from, so an element can be copied exactly or
      replaced whole.
- [ ] `content_out` walks the part and copies every byte of it except the
      `<text:p>`, `<text:h>` and `<table:table>` elements that *read back
      differently* from the model. Changed is defined by re-reading, never by
      remembering: that is the only definition that cannot drift from the
      reader.
- [ ] A changed paragraph is emitted as ODF — `<text:p>`/`<text:h>` with its
      style, `<text:span>` for a run that carries one, `<text:s text:c="n"/>`
      for the second and later spaces of a run, `<text:tab/>`,
      `<text:line-break/>`, and text escaped the way the format escapes it.
- [ ] Direct formatting mints an **automatic style**. ODF has nowhere else to
      put it: a run made bold by hand is not bold in the file, it names a
      `<style:style style:family="text">` that is. The minted styles go into
      `<office:automatic-styles>`, which stands before the body and so must be
      written after it is known what the body needs.
- [ ] `wp_odf::save` and `wp_odf::flush`, with the signatures `wp_docx` uses.
- [ ] `wp_odf::write::blank::container_for(&Document)` authors a package for a
      document that never had one, so Save As `.odt` works from a `.docx` or a
      markdown file.

## The proof

- [ ] `xtask fidelity` check 2 covers `.odt`: open, change the text of one
      paragraph, save, reopen, and account for every byte that moved. Both
      corpus documents pass it, and the entry with no media type is still
      there afterwards.
- [ ] An `.odt` saved with no edit at all is byte-identical to the one that was
      opened, and a test says so rather than a person having noticed once.

## The application

- [ ] `Format::Odt::is_writable()` is true, `save` writes an `.odt` in place,
      and the "opens as a copy" message is gone along with the `.docx` the path
      was being rewritten to.
- [ ] `FORMATS.md` says read + write for `.odt`, and the section explaining why
      it is not written goes with it. Nothing in that file is aspirational.

## The page

Two differences from the reference are known, traced and unfixed. Neither is a
mystery; both are work.

- [ ] `word-odf-export.odt` lays in five pages, as the reference does. The
      per-page shortfall is traced to the last paragraph of its header, which
      holds the watermark this reader empties.
- [ ] The `<draw:custom-shape>` watermark is drawn, or — if it is decided that
      it should not be — `PROGRESS.md` argues why and `LAYOUT.md` records the
      residue deliberately.

## The record

- [ ] `PROGRESS.md` has the work log for the writer.
- [ ] `LEARNINGS.md` records what writing the format taught, in the voice of the
      entries already there: what was believed, what was measured, what it cost.
