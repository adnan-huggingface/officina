"""Every word of a rendered page, and the baseline it was set on.

The second half of the oracle behind ``cargo xtask compare``: ``topdf.ps1``
beside this asks Word for its own rendering of a document, and this reads where
the words landed in it. Emits TSV on stdout, one word per line::

    page <TAB> x <TAB> baseline <TAB> text

**Baselines, not bounding boxes.** A glyph's box begins at its own ink, which
starts a different distance below the line in every face and at every size; the
baseline is the one horizontal the two renderers can be asked about without
either of them having to guess at the other's idea of a line. PyMuPDF reports
it directly as each character's ``origin``, in points from the top-left of the
page — the same frame and the same unit ``wp_layout`` lays a page in.

A word here is what a reader would call one: the characters between two
stretches of whitespace. That is deliberately *not* what Word's own ``Words``
collection means by the term — it splits punctuation off and keeps the trailing
space — and it is why the comparison goes through paper rather than through
that collection.

Requires PyMuPDF. Note that PyMuPDF is AGPL: it is used here as a developer's
measuring instrument on a developer's machine, never linked into either
application and never redistributed with them, and nothing under ``tools/``
ships. If that is unwelcome, ``pypdf`` is BSD and can be made to answer the
same question with more work.
"""

import sys

try:
    import fitz  # PyMuPDF
except ImportError:
    sys.exit("pdfwords.py needs PyMuPDF: python -m pip install pymupdf")


def words(page):
    """(x, baseline, text) for every word on one page, in the order drawn."""
    text, start, baseline = "", 0.0, 0.0
    for block in page.get_text("rawdict").get("blocks", ()):
        for line in block.get("lines", ()):
            for span in line.get("spans", ()):
                for char in span.get("chars", ()):
                    glyph, origin = char.get("c", ""), char.get("origin")
                    if origin is None:
                        continue
                    if glyph.isspace():
                        if text:
                            yield start, baseline, text
                            text = ""
                        continue
                    if not text:
                        start, baseline = origin[0], origin[1]
                    text += glyph
            # A line ends a word even where the PDF drew no space for it.
            if text:
                yield start, baseline, text
                text = ""
    if text:
        yield start, baseline, text


def main(path):
    with fitz.open(path) as document:
        for number, page in enumerate(document, start=1):
            for x, baseline, text in words(page):
                print(f"{number}\t{x:.3f}\t{baseline:.3f}\t{text}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: pdfwords.py <pdf>")
    main(sys.argv[1])
