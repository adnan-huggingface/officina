"""A structural rubbing of an ``.odt``: its shape, and none of its words.

Real documents are where the awkwardness is — thirty-three list definitions, a
header that is a table, a cell covered from two directions, a style that scales
its parent instead of restating a size. None of that can be invented, and none
of it is anybody's to publish. So this takes a document nobody may commit and
writes one that measures the same and says nothing.

**What survives.** Every element and attribute of ``content.xml`` and
``styles.xml``: the page layout, the master pages, the styles and their names,
the list definitions, the tables and their spans, the frames and their sizes,
the bookmarks. Every token of text keeps its length, its capitalisation and its
punctuation, so that a line breaks where it broke and a page ends where it
ended — which is the entire reason to start from a real document rather than a
made-up one.

**What does not.** Every word, replaced with a generated one of the same shape.
Every picture, replaced with a generated one of the same pixel size. The whole
of ``meta.xml``, which is where the authors' names and the editing history are.
Every external link, pointed at a domain reserved for the purpose. The result is
a document with the same skeleton and no content at all — and a file this
repository may hold, which the original is not.

Deterministic: the same input gives the same output, byte for byte, so a rebuild
is a no-op in the history rather than a churn of timestamps.

    python scrub-odt.py <source.odt> odt/word-odf-export.odt

Needs Pillow, for the pictures.
"""

import io
import sys
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image

# Fixed, so the zip is the same every time. `strangers.py` beside this uses the
# same stamp for the same reason.
STAMP = (1980, 1, 1, 0, 0, 0)

MIMETYPE = "application/vnd.oasis.opendocument.text"

# Every prefix the ODF specification itself uses. Registered so that the
# rewritten parts read like the ones they came from, rather than like `ns0:`.
NAMESPACES = {
    "office": "urn:oasis:names:tc:opendocument:xmlns:office:1.0",
    "style": "urn:oasis:names:tc:opendocument:xmlns:style:1.0",
    "text": "urn:oasis:names:tc:opendocument:xmlns:text:1.0",
    "table": "urn:oasis:names:tc:opendocument:xmlns:table:1.0",
    "draw": "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0",
    "fo": "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0",
    "xlink": "http://www.w3.org/1999/xlink",
    "dc": "http://purl.org/dc/elements/1.1/",
    "meta": "urn:oasis:names:tc:opendocument:xmlns:meta:1.0",
    "number": "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0",
    "svg": "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0",
    "chart": "urn:oasis:names:tc:opendocument:xmlns:chart:1.0",
    "dr3d": "urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0",
    "math": "http://www.w3.org/1998/Math/MathML",
    "form": "urn:oasis:names:tc:opendocument:xmlns:form:1.0",
    "script": "urn:oasis:names:tc:opendocument:xmlns:script:1.0",
    "config": "urn:oasis:names:tc:opendocument:xmlns:config:1.0",
    "ooo": "http://openoffice.org/2004/office",
    "manifest": "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0",
    "smil": "urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0",
    "anim": "urn:oasis:names:tc:opendocument:xmlns:animation:1.0",
    "presentation": "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0",
    "db": "urn:oasis:names:tc:opendocument:xmlns:database:1.0",
    "grddl": "http://www.w3.org/2003/g/data-view#",
    "xhtml": "http://www.w3.org/1999/xhtml",
    "xforms": "http://www.w3.org/2002/xforms",
    "xsd": "http://www.w3.org/2001/XMLSchema",
    "xsi": "http://www.w3.org/2001/XMLSchema-instance",
    "loext": "urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0",
}

XLINK_HREF = "{http://www.w3.org/1999/xlink}href"
TEXT_A = "{urn:oasis:names:tc:opendocument:xmlns:text:1.0}a"
DRAW_IMAGE = "{urn:oasis:names:tc:opendocument:xmlns:drawing:1.0}image"

# The letters a replacement word is cut from. Nothing here spells anything.
LETTERS = "loremipsumdolorsitametconsecteturadipiscingelitseddoeiusmodtempor"


class Words:
    """Replacement tokens, deterministic and the same shape as the originals.

    A token keeps its length, whether it was digits or letters, and where its
    capitals were, because all three decide how wide it sets and therefore
    where the line breaks. What it does not keep is what it said.
    """

    def __init__(self):
        self.at = 0

    def like(self, token):
        if not token:
            return token
        lead = "".join(self._take_edge(token, False))
        trail = "".join(reversed(list(self._take_edge(token, True))))
        core = token[len(lead) : len(token) - len(trail)]
        if not core:
            return token
        return lead + self._core(core) + trail

    @staticmethod
    def _take_edge(token, from_end):
        source = reversed(token) if from_end else token
        for ch in source:
            if ch.isalnum():
                return
            yield ch

    def _core(self, core):
        out = []
        for ch in core:
            if ch.isdigit():
                out.append(str((self.at * 7 + len(out) * 3) % 10))
            elif ch.isalpha():
                letter = LETTERS[(self.at * 5 + len(out)) % len(LETTERS)]
                out.append(letter.upper() if ch.isupper() else letter)
            else:
                out.append(ch)
        self.at += 1
        return "".join(out)


def scrubbed(text, words):
    """One text node, token for token, with the whitespace left where it was."""
    if not text or not text.strip():
        return text
    out = []
    token = []
    for ch in text:
        if ch.isspace():
            if token:
                out.append(words.like("".join(token)))
                token = []
            out.append(ch)
        else:
            token.append(ch)
    if token:
        out.append(words.like("".join(token)))
    return "".join(out)


def rewrite(xml, words):
    """A part, with its structure kept and its words replaced."""
    for prefix, uri in NAMESPACES.items():
        ET.register_namespace(prefix, uri)
    root = ET.fromstring(xml)
    for element in root.iter():
        element.text = scrubbed(element.text, words)
        element.tail = scrubbed(element.tail, words)
        # A link points somewhere real, and where it points is content. The
        # picture references are paths inside the package and must not move.
        if element.tag == TEXT_A and XLINK_HREF in element.attrib:
            element.attrib[XLINK_HREF] = "https://example.invalid/"
    body = ET.tostring(root, encoding="unicode")
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n' + body).encode(
        "utf-8"
    )


def picture(data):
    """A generated picture the size of the one it stands in for.

    The size is what matters: a frame states its own width and height, but a
    renderer still asks the picture for its aspect and its resolution, and a
    stand-in of the wrong size is a stand-in that changes the layout.
    """
    original = Image.open(io.BytesIO(data))
    width, height = original.size
    made = Image.new("RGB", (width, height), (245, 245, 245))
    pixels = made.load()
    # A ramp with a diagonal through it: something to see, nothing to read.
    for y in range(height):
        for x in range(width):
            if abs(x * height - y * width) < max(width, height):
                pixels[x, y] = (30, 111, 92)
            elif (x // 32 + y // 32) % 2 == 0:
                pixels[x, y] = (225, 228, 226)
    out = io.BytesIO()
    made.save(out, format="PNG", optimize=True)
    return out.getvalue()


META = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<office:document-meta \
xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" \
xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" \
xmlns:dc="http://purl.org/dc/elements/1.1/" office:version="1.4">\
<office:meta><meta:generator>officina/corpus/scrub-odt.py</meta:generator>\
<dc:title>Structural rubbing of a Word ODF export</dc:title>\
<meta:creation-date>1980-01-01T00:00:00</meta:creation-date>\
</office:meta></office:document-meta>
"""


def scrub(source, target):
    words = Words()
    with zipfile.ZipFile(source) as zin:
        entries = [(item.filename, zin.read(item.filename)) for item in zin.infolist()
                   if not item.is_dir()]

    made = []
    for name, data in entries:
        if name == "mimetype":
            continue
        elif name in ("content.xml", "styles.xml"):
            data = rewrite(data, words)
        elif name == "meta.xml":
            data = META.encode("utf-8")
        elif name == "settings.xml":
            # A cursor position, a zoom, and the name of whoever last had the
            # file open. None of it changes the page.
            continue
        elif name.lower().endswith((".png", ".jpg", ".jpeg", ".gif", ".bmp")):
            data = picture(data)
        made.append((name, data))

    target.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(target, "w") as out:
        # First and stored, as ODF 1.4 part 2 §3.3 requires: the file must be
        # identifiable from the bytes at a fixed offset.
        first = zipfile.ZipInfo("mimetype", STAMP)
        first.compress_type = zipfile.ZIP_STORED
        out.writestr(first, MIMETYPE)
        for name, data in made:
            item = zipfile.ZipInfo(name, STAMP)
            item.compress_type = zipfile.ZIP_DEFLATED
            out.writestr(item, data)
    return target


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    source = Path(sys.argv[1])
    target = Path(sys.argv[2])
    if not source.exists():
        print(f"{source} is not there")
        return 1
    scrub(source, target)
    print(f"{target} written, {target.stat().st_size} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
