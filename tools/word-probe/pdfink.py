"""Every mark of a rendered page, and where it was put.

The second half of the oracle behind ``cargo xtask compare``: ``topdf.ps1``
beside this asks Word for its own rendering of a document, and this reads what
landed on it. Emits TSV on stdout, one mark per line, in one of three kinds::

    word    <TAB> page <TAB> x <TAB> baseline <TAB> text
    mark    <TAB> page <TAB> x0 <TAB> y0 <TAB> x1 <TAB> y1
    picture <TAB> page <TAB> x0 <TAB> y0 <TAB> x1 <TAB> y1

A *mark* is a rectangle of ink that is not type: a border, an underline, a
shading, the box a picture went in. A page's furniture used to be invisible to
the comparison — a rule could move an inch and no number moved with it — and
that is the whole reason it is here.

**Nothing here is joined, merged or classified.** Word draws one table border
as a row of small squares at the corners with the spans between them, and this
project draws the same border as one rule per cell; both have to be reduced to
the ink they put down before either can answer for the other. That reduction
lives in ``crates/wp-compare/src/marks.rs``, where the *same* code does it to
both sides — the lesson the line cutting taught twice over. A rule applied to
one reading alone is a rule the other reading never agreed to.

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
    sys.exit("pdfink.py needs PyMuPDF: python -m pip install pymupdf")


#: The thinnest a stroke is taken to be. PDF's zero width means "one device
#: pixel", which is not a measurement in points at all; a twentieth of a point
#: is thinner than anything Word draws deliberately, and keeps a hairline from
#: reducing to a rectangle with no inside.
HAIRLINE = 0.05


def words(page):
    """(x, baseline, text) for every word on one page, in the order drawn.

    A word ends at whitespace, and at the end of one of the PDF's own lines --
    which is a real boundary even where no space was drawn for it, because a
    diagram sets each of its labels with its own positioning operator.

    Nothing here joins two marks that merely abut, and the temptation is real:
    Word's export breaks "I/O" into three of them, against the one this
    project's own playback draws, and joining by geometry does put those back
    together. It also produces words that were never on the page. A diagram
    sets its labels in whatever order it likes, so the same rule ran "SPI"
    together with a "Radio" fifty-three points to its *left*, and the resulting
    "SPIRadio" matched nothing on either side -- an invented token is worse
    than a split one, because a split one can still be paired. So the marks are
    reported where they fell, and the matcher pairs a word one side cut in
    three; see ``glued`` in ``crates/wp-compare/src/diff.rs``.
    """
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
                    # The left edge, not the first pen-down: a right-to-left
                    # run is drawn from its right end, and a word measured at
                    # the end it happens to be drawn from is measured against
                    # the other end of the same word on the other side. It came
                    # out as one Arabic word its own width out of place.
                    start = min(start, origin[0])
                    text += glyph
            if text:
                yield start, baseline, text
                text = ""
    if text:
        yield start, baseline, text


def edges(rect, width):
    """A stroked rectangle as the four rules it actually draws.

    A frame is not a box of ink: the middle of it is paper. Scriva draws a
    bordered cell as four rules, and a stroked rectangle reported whole would
    be compared against a fill it has nothing in common with.
    """
    half = max(width, HAIRLINE) / 2.0
    x0, y0, x1, y1 = rect.x0, rect.y0, rect.x1, rect.y1
    yield x0, y0 - half, x1, y0 + half
    yield x0, y1 - half, x1, y1 + half
    yield x0 - half, y0, x0 + half, y1
    yield x1 - half, y0, x1 + half, y1


def segment(start, end, width):
    """A stroked line as the ink it lays down, which has a thickness.

    Half the thickness to either side of the line and nothing added to its
    length: a stroke's default cap stops at its end point, and a rule that
    reported itself half a point longer than it is would never sit flush
    against the one the other renderer drew. `ours.rs` widens Scriva's rules by
    the same rule, which is what lets the two be compared at all.
    """
    half = max(width, HAIRLINE) / 2.0
    x0, x1 = min(start.x, end.x), max(start.x, end.x)
    y0, y1 = min(start.y, end.y), max(start.y, end.y)
    if x1 - x0 >= y1 - y0:
        return x0, y0 - half, x1, y1 + half
    return x0 - half, y0, x1 + half, y1


def marks(page):
    """(x0, y0, x1, y1) for every rectangle of ink on a page that is not type.

    Each drawing operation is reported as the ink it lays down and as nothing
    more. A curve is the one thing with no honest rectangle, so the group it
    belongs to is reported as a single box around the whole of it: a diagram's
    hundreds of strokes are one drawing rather than hundreds of findings, and a
    diagram is compared as a box in any case.
    """
    for path in page.get_drawings():
        width = path.get("width") or 0.0
        curves = None
        for item in path.get("items", ()):
            if item[0] == "re":
                if path.get("fill") is not None:
                    rect = item[1]
                    yield rect.x0, rect.y0, rect.x1, rect.y1
                if path.get("color") is not None:
                    yield from edges(item[1], width)
            elif item[0] == "l":
                yield segment(item[1], item[2], width)
            elif item[0] == "qu":
                rect = item[1].rect
                yield rect.x0, rect.y0, rect.x1, rect.y1
            else:
                points = [p for p in item[1:] if isinstance(p, fitz.Point)]
                box = (
                    min(p.x for p in points),
                    min(p.y for p in points),
                    max(p.x for p in points),
                    max(p.y for p in points),
                )
                curves = box if curves is None else (
                    min(curves[0], box[0]),
                    min(curves[1], box[1]),
                    max(curves[2], box[2]),
                    max(curves[3], box[3]),
                )
        if curves is not None:
            yield curves

def pictures(page):
    """The box of every raster picture on the page.

    A picture is not a drawing operation and has to be asked for separately, and
    it is reported apart from the rest because it is the one mark whose box both
    renderings state outright. Everything else a picture is made of reaches a
    rendering as the strokes that draw it, and a box is not among them — which
    is what lets a raster picture be held to a tenth of a point while a diagram
    is only ever held to having been drawn into at all.
    """
    for image in page.get_image_info():
        yield tuple(image["bbox"])


def main(path):
    with fitz.open(path) as document:
        for number, page in enumerate(document, start=1):
            for x, baseline, text in words(page):
                print(f"word\t{number}\t{x:.3f}\t{baseline:.3f}\t{text}")
            for kind, boxes in (("mark", marks(page)), ("picture", pictures(page))):
                for x0, y0, x1, y1 in boxes:
                    print(
                        f"{kind}\t{number}\t{x0:.3f}\t{y0:.3f}"
                        f"\t{x1:.3f}\t{y1:.3f}"
                    )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: pdfink.py <pdf>")
    main(sys.argv[1])
