"""Hand-written OpenDocument, in a dialect no word processor writes.

The twin of ``strangers.py`` beside this, and there for the same reason: every
other file in the corpus was written by one producer, and one producer's
dialect is not the format. A `.docx` from Google Docs broke the reader in five
ways while twenty-seven from Word passed, and there is no reason to expect ODF
to be kinder — LibreOffice writes one dialect of it, Word's export writes
another, and a report generator writes a third.

So this writes a third of its own. Nothing here came out of an application:
the styles are named the way the specification names things rather than the way
any tool does, the two ways of stating a list indent are both used, the tables
span cells in both directions, and one entry of the package is left out of the
manifest on purpose — because a part with no declared media type is exactly the
case a reader is most likely to drop, and the sample from Word's own export has
one.

Deterministic: the same run gives the same bytes, so a rebuild is a no-op in
the history rather than a churn of timestamps.

    python odf.py

Writes ``odt/second-producer.odt`` beside this. Needs Pillow, for the pictures.
"""

import io
import zipfile
from pathlib import Path

from PIL import Image

STAMP = (1980, 1, 1, 0, 0, 0)
MIMETYPE = "application/vnd.oasis.opendocument.text"

NS = """xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" \
xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" \
xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" \
xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" \
xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" \
xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" \
xmlns:xlink="http://www.w3.org/1999/xlink" \
xmlns:dc="http://purl.org/dc/elements/1.1/" \
xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" \
xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" \
xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\""""

HEAD = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

LOREM = (
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod "
    "tempor incididunt ut labore et dolore magna aliqua enim ad minim veniam "
    "quis nostrud exercitation ullamco laboris nisi aliquip ex ea commodo "
    "consequat duis aute irure in reprehenderit voluptate velit esse cillum "
    "eu fugiat nulla pariatur excepteur sint occaecat cupidatat non proident "
    "sunt culpa qui officia deserunt mollit anim id est laborum"
).split()


def prose(sentences, seed):
    """A paragraph of a stated length, the same one every run."""
    out = []
    at = seed
    for _ in range(sentences):
        length = 7 + (at * 5) % 12
        words = [LOREM[(at + i * 3) % len(LOREM)] for i in range(length)]
        at += length
        words[0] = words[0].capitalize()
        out.append(" ".join(words) + ".")
    return " ".join(out)


def picture(width, height, dark):
    made = Image.new("RGB", (width, height), (250, 250, 250))
    pixels = made.load()
    for y in range(height):
        for x in range(width):
            if (x + y) % 37 < 3:
                pixels[x, y] = dark
            elif (x // 16 + y // 16) % 2 == 0:
                pixels[x, y] = (228, 232, 230)
    out = io.BytesIO()
    made.save(out, format="PNG", optimize=True)
    return out.getvalue()


def styles_xml():
    """The named styles, the page, and the master page drawn on every sheet."""
    return HEAD + f"""<office:document-styles {NS} office:version="1.4">
<office:font-face-decls>
<style:font-face style:name="Body" svg:font-family="Calibri" style:font-family-generic="swiss"/>
<style:font-face style:name="Display" svg:font-family="'Cambria', serif" style:font-family-generic="roman"/>
<style:font-face style:name="Fixed" svg:font-family="'Courier New'" style:font-family-generic="modern" style:font-pitch="fixed"/>
</office:font-face-decls>
<office:styles>
<style:default-style style:family="paragraph">
<style:paragraph-properties fo:margin-top="0in" fo:margin-bottom="0.08in" fo:line-height="115%" fo:orphans="2" fo:widows="2"/>
<style:text-properties style:font-name="Body" fo:font-size="11pt" fo:language="en" fo:country="GB"/>
</style:default-style>
<style:style style:name="Standard" style:family="paragraph" style:display-name="Default"/>
<style:style style:name="Body_20_Text" style:family="paragraph" style:display-name="Body Text" style:parent-style-name="Standard">
<style:paragraph-properties fo:text-align="justify"/>
</style:style>
<style:style style:name="Heading" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Body_20_Text">
<style:paragraph-properties fo:margin-top="0.17in" fo:margin-bottom="0.06in" fo:keep-with-next="always"/>
<style:text-properties style:font-name="Display" fo:font-weight="bold" fo:color="#1e6f5c"/>
</style:style>
<style:style style:name="Heading_20_1" style:family="paragraph" style:display-name="Heading 1" style:parent-style-name="Heading">
<style:text-properties fo:font-size="18pt"/>
</style:style>
<style:style style:name="Heading_20_2" style:family="paragraph" style:display-name="Heading 2" style:parent-style-name="Heading">
<style:text-properties fo:font-size="14pt"/>
</style:style>
<style:style style:name="Quotation" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties fo:margin-left="0.5in" fo:margin-right="0.5in" fo:text-indent="0in"/>
<style:text-properties fo:font-style="italic"/>
</style:style>
<style:style style:name="Contents" style:family="paragraph" style:parent-style-name="Standard">
<style:paragraph-properties>
<style:tab-stops>
<style:tab-stop style:position="6.0in" style:type="right" style:leader-style="dotted" style:leader-text="."/>
</style:tab-stops>
</style:paragraph-properties>
</style:style>
<style:style style:name="Emphasis" style:family="text">
<style:text-properties fo:font-style="italic" fo:font-weight="bold"/>
</style:style>
<style:style style:name="Code" style:family="text">
<style:text-properties style:font-name="Fixed" fo:font-size="10pt" fo:background-color="#eef2f0"/>
</style:style>
<style:style style:name="Struck" style:family="text">
<style:text-properties style:text-line-through-style="solid" style:text-underline-style="dotted"/>
</style:style>
<style:style style:name="Framed" style:family="graphic">
<style:graphic-properties style:wrap="parallel" style:vertical-pos="from-top" style:horizontal-pos="from-left"/>
</style:style>
<text:outline-style style:name="Outline">
<text:outline-level-style text:level="1" style:num-format=""/>
<text:outline-level-style text:level="2" style:num-format=""/>
<text:outline-level-style text:level="3" style:num-format=""/>
</text:outline-style>
<text:list-style style:name="Numbered">
<text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix=".">
<style:list-level-properties text:list-level-position-and-space-mode="label-alignment">
<style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="0.5in" fo:text-indent="-0.25in" fo:margin-left="0.5in"/>
</style:list-level-properties>
</text:list-level-style-number>
<text:list-level-style-number text:level="2" style:num-format="a" style:num-suffix=")" text:display-levels="1">
<style:list-level-properties text:list-level-position-and-space-mode="label-alignment">
<style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="1.0in" fo:text-indent="-0.25in" fo:margin-left="1.0in"/>
</style:list-level-properties>
</text:list-level-style-number>
<text:list-level-style-number text:level="3" style:num-format="i" style:num-suffix="." text:display-levels="1">
<style:list-level-properties text:list-level-position-and-space-mode="label-alignment">
<style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="1.5in" fo:text-indent="-0.25in" fo:margin-left="1.5in"/>
</style:list-level-properties>
</text:list-level-style-number>
</text:list-style>
<text:list-style style:name="Bulleted">
<text:list-level-style-bullet text:level="1" text:bullet-char="&#8226;">
<style:list-level-properties text:space-before="0.25in" text:min-label-width="0.25in"/>
<style:text-properties style:font-name="Body"/>
</text:list-level-style-bullet>
<text:list-level-style-bullet text:level="2" text:bullet-char="&#9702;">
<style:list-level-properties text:space-before="0.75in" text:min-label-width="0.25in"/>
<style:text-properties style:font-name="Body"/>
</text:list-level-style-bullet>
</text:list-style>
<text:notes-configuration text:note-class="footnote" style:num-format="1" text:start-value="0" text:footnotes-position="page"/>
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="Paper">
<style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" style:print-orientation="portrait" fo:margin-top="0.75in" fo:margin-bottom="0.75in" fo:margin-left="1in" fo:margin-right="1in" style:writing-mode="lr-tb"/>
<style:header-style>
<style:header-footer-properties fo:min-height="0.3in" fo:margin-bottom="0.15in"/>
</style:header-style>
<style:footer-style>
<style:header-footer-properties fo:min-height="0.3in" fo:margin-top="0.15in"/>
</style:footer-style>
</style:page-layout>
<style:style style:name="Banner" style:family="paragraph">
<style:paragraph-properties fo:text-align="center" fo:border-bottom="0.5pt solid #1e6f5c" fo:padding-bottom="0.03in"/>
<style:text-properties fo:font-size="9pt" fo:font-variant="small-caps"/>
</style:style>
<style:style style:name="Colophon" style:family="paragraph">
<style:paragraph-properties fo:text-align="center"/>
<style:text-properties fo:font-size="9pt"/>
</style:style>
</office:automatic-styles>
<office:master-styles>
<style:master-page style:name="Standard" style:page-layout-name="Paper">
<style:header>
<text:p text:style-name="Banner">Second producer, hand written</text:p>
</style:header>
<style:footer>
<text:p text:style-name="Colophon">Page <text:page-number text:select-page="current">1</text:page-number> of <text:page-count>4</text:page-count></text:p>
</style:footer>
</style:master-page>
</office:master-styles>
</office:document-styles>
"""


def content_xml():
    body = []
    add = body.append

    add('<text:h text:style-name="Heading_20_1" text:outline-level="1">Second producer</text:h>')
    add(f'<text:p text:style-name="Body_20_Text">{prose(3, 1)}</text:p>')
    add(
        '<text:p text:style-name="Body_20_Text">A run may be '
        '<text:span text:style-name="Emphasis">emphasised</text:span>, set in '
        '<text:span text:style-name="Code">a fixed face</text:span>, or '
        '<text:span text:style-name="Struck">struck through and underlined at once</text:span>. '
        'Spaces are<text:s text:c="5"/>written out when there are several, '
        'and a tab<text:tab/>lands on a stop.<text:line-break/>A line may be broken '
        'without ending the paragraph.</text:p>'
    )
    add(
        '<text:p text:style-name="Contents">Where the first table is'
        '<text:tab/>2</text:p>'
    )

    add('<text:h text:style-name="Heading_20_2" text:outline-level="2">Lists, both ways of stating one</text:h>')
    add(
        '<text:list text:style-name="Numbered">'
        '<text:list-item><text:p text:style-name="Standard">' + prose(1, 4) + '</text:p></text:list-item>'
        '<text:list-item><text:p text:style-name="Standard">' + prose(1, 9) + '</text:p>'
        '<text:list><text:list-item><text:p text:style-name="Standard">' + prose(1, 13) + '</text:p>'
        '<text:list><text:list-item><text:p text:style-name="Standard">' + prose(1, 17) + '</text:p>'
        '</text:list-item></text:list>'
        '</text:list-item></text:list>'
        '</text:list-item>'
        '<text:list-item><text:p text:style-name="Standard">' + prose(1, 21) + '</text:p></text:list-item>'
        '</text:list>'
    )
    add(
        '<text:list text:style-name="Bulleted">'
        '<text:list-item><text:p text:style-name="Standard">' + prose(1, 25) + '</text:p></text:list-item>'
        '<text:list-item><text:p text:style-name="Standard">' + prose(1, 29) + '</text:p>'
        '<text:list><text:list-item><text:p text:style-name="Standard">' + prose(1, 33) + '</text:p>'
        '</text:list-item></text:list></text:list-item>'
        '</text:list>'
    )

    add('<text:h text:style-name="Heading_20_2" text:outline-level="2">A quotation and a note</text:h>')
    add(f'<text:p text:style-name="Quotation">{prose(2, 37)}</text:p>')
    add(
        '<text:p text:style-name="Body_20_Text">' + prose(2, 41) +
        '<text:note text:id="ftn1" text:note-class="footnote">'
        '<text:note-citation>1</text:note-citation>'
        '<text:note-body><text:p text:style-name="Standard">' + prose(1, 45) + '</text:p></text:note-body>'
        '</text:note> ' + prose(1, 49) + '</text:p>'
    )

    add('<text:h text:style-name="Heading_20_1" text:outline-level="1">'
        '<text:bookmark-start text:name="tables"/>Tables<text:bookmark-end text:name="tables"/></text:h>')
    add(
        '<text:p text:style-name="Body_20_Text">The first table is bookmarked, and '
        '<text:a xlink:href="#tables" xlink:type="simple">this link points at it</text:a>. '
        'A second link goes <text:a xlink:href="https://example.invalid/spec" xlink:type="simple">'
        'outside the document</text:a>.</text:p>'
    )
    add(table_one())
    add(f'<text:p text:style-name="Body_20_Text">{prose(2, 53)}</text:p>')
    add(table_two())

    add('<text:h text:style-name="Heading_20_1" text:outline-level="1">Pictures</text:h>')
    add(
        '<text:p text:style-name="Body_20_Text">A picture in the line of the text '
        '<draw:frame draw:style-name="Framed" draw:name="inline" text:anchor-type="as-char" '
        'svg:width="0.55in" svg:height="0.55in">'
        '<draw:image xlink:href="Pictures/mark.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>'
        '<svg:title>A small square</svg:title>'
        '</draw:frame> sits on the baseline beside it.</text:p>'
    )
    add(
        '<text:p text:style-name="Body_20_Text">'
        '<draw:frame draw:style-name="Framed" draw:name="floated" text:anchor-type="paragraph" '
        'svg:x="0.2in" svg:y="0.1in" svg:width="2.2in" svg:height="1.4in">'
        '<draw:image xlink:href="Pictures/plate.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>'
        '</draw:frame>' + prose(4, 57) + '</text:p>'
    )
    add(f'<text:p text:style-name="Body_20_Text">{prose(4, 61)}</text:p>')

    add('<text:h text:style-name="Heading_20_1" text:outline-level="1">Enough to run over</text:h>')
    for seed in range(65, 105, 4):
        add(f'<text:p text:style-name="Body_20_Text">{prose(4, seed)}</text:p>')

    return HEAD + f"""<office:document-content {NS} office:version="1.4">
<office:font-face-decls>
<style:font-face style:name="Body" svg:font-family="Calibri" style:font-family-generic="swiss"/>
<style:font-face style:name="Display" svg:font-family="'Cambria', serif" style:font-family-generic="roman"/>
<style:font-face style:name="Fixed" svg:font-family="'Courier New'" style:font-family-generic="modern" style:font-pitch="fixed"/>
</office:font-face-decls>
<office:automatic-styles>
<style:style style:name="Grid" style:family="table">
<style:table-properties style:width="6.5in" table:align="left" fo:margin-left="0in"/>
</style:style>
<style:style style:name="Grid.A" style:family="table-column">
<style:table-column-properties style:column-width="1.5in"/>
</style:style>
<style:style style:name="Grid.B" style:family="table-column">
<style:table-column-properties style:column-width="2.5in"/>
</style:style>
<style:style style:name="Grid.C" style:family="table-column">
<style:table-column-properties style:column-width="2.5in"/>
</style:style>
<style:style style:name="Grid.head" style:family="table-cell">
<style:table-cell-properties fo:background-color="#1e6f5c" fo:border="0.5pt solid #1e6f5c" fo:padding="0.04in"/>
</style:style>
<style:style style:name="Grid.body" style:family="table-cell">
<style:table-cell-properties fo:border="0.5pt solid #9aa8a3" fo:padding="0.04in" style:vertical-align="middle"/>
</style:style>
<style:style style:name="Grid.row" style:family="table-row">
<style:table-row-properties style:min-row-height="0.22in"/>
</style:style>
<style:style style:name="OnHead" style:family="paragraph">
<style:text-properties fo:font-weight="bold" fo:color="#ffffff"/>
</style:style>
<style:style style:name="Wide" style:family="table">
<style:table-properties style:rel-width="80%" table:align="center"/>
</style:style>
<style:style style:name="Wide.col" style:family="table-column">
<style:table-column-properties style:rel-column-width="1*"/>
</style:style>
<style:style style:name="Wide.cell" style:family="table-cell">
<style:table-cell-properties fo:border="0.75pt solid #333333" fo:padding="0.05in"/>
</style:style>
</office:automatic-styles>
<office:body><office:text>
{chr(10).join(body)}
</office:text></office:body>
</office:document-content>
"""


def table_one():
    """Three columns, a header row, and a cell spanning two of them."""
    rows = [
        '<table:table-header-rows><table:table-row table:style-name="Grid.row">'
        + "".join(
            f'<table:table-cell table:style-name="Grid.head" office:value-type="string">'
            f'<text:p text:style-name="OnHead">{head}</text:p></table:table-cell>'
            for head in ("Name", "What it states", "Where")
        )
        + "</table:table-row></table:table-header-rows>"
    ]
    rows.append(
        '<table:table-row table:style-name="Grid.row">'
        '<table:table-cell table:style-name="Grid.body" table:number-columns-spanned="2">'
        f'<text:p text:style-name="Standard">{prose(1, 71)}</text:p></table:table-cell>'
        '<table:covered-table-cell/>'
        '<table:table-cell table:style-name="Grid.body">'
        '<text:p text:style-name="Standard">§3.3</text:p></table:table-cell>'
        "</table:table-row>"
    )
    for at, seed in enumerate((75, 79, 83)):
        rows.append(
            '<table:table-row table:style-name="Grid.row">'
            f'<table:table-cell table:style-name="Grid.body"><text:p text:style-name="Standard">Row {at + 2}</text:p></table:table-cell>'
            f'<table:table-cell table:style-name="Grid.body"><text:p text:style-name="Standard">{prose(1, seed)}</text:p></table:table-cell>'
            f'<table:table-cell table:style-name="Grid.body"><text:p text:style-name="Standard">§{at + 4}.1</text:p></table:table-cell>'
            "</table:table-row>"
        )
    return (
        '<table:table table:name="Grid" table:style-name="Grid">'
        '<table:table-column table:style-name="Grid.A"/>'
        '<table:table-column table:style-name="Grid.B"/>'
        '<table:table-column table:style-name="Grid.C"/>'
        + "".join(rows)
        + "</table:table>"
    )


def table_two():
    """Four columns written as one, and a cell spanning two rows."""
    rows = [
        '<table:table-row>'
        '<table:table-cell table:style-name="Wide.cell" table:number-rows-spanned="2">'
        '<text:p text:style-name="Standard">Two rows tall</text:p></table:table-cell>'
        '<table:table-cell table:style-name="Wide.cell"><text:p text:style-name="Standard">b1</text:p></table:table-cell>'
        '<table:table-cell table:style-name="Wide.cell"><text:p text:style-name="Standard">c1</text:p></table:table-cell>'
        '<table:table-cell table:style-name="Wide.cell"><text:p text:style-name="Standard">d1</text:p></table:table-cell>'
        "</table:table-row>",
        '<table:table-row>'
        '<table:covered-table-cell/>'
        '<table:table-cell table:style-name="Wide.cell" table:number-columns-repeated="3">'
        '<text:p text:style-name="Standard">repeated</text:p></table:table-cell>'
        "</table:table-row>",
    ]
    return (
        '<table:table table:name="Wide" table:style-name="Wide">'
        '<table:table-column table:style-name="Wide.col" table:number-columns-repeated="4"/>'
        + "".join(rows)
        + "</table:table>"
    )


META = HEAD + f"""<office:document-meta {NS} office:version="1.4">
<office:meta><meta:generator>officina/corpus/odf.py</meta:generator>
<dc:title>Hand-written OpenDocument</dc:title>
<meta:creation-date>1980-01-01T00:00:00</meta:creation-date>
</office:meta></office:document-meta>
"""

# `notes/rider.xml` is listed with **no media type at all**, which is the case a
# reader is most likely to drop and the one Word's own ODF export produces.
#
# It is listed, and that is not a softening of the test. An entry the manifest
# never mentions was tried first, and LibreOffice refuses to open the package at
# all — not the entry, the package. An ODF manifest is not the advisory listing
# `[Content_Types].xml` is; a file that is there and unlisted makes the document
# unopenable. So the awkward case a corpus can actually hold is this one: an
# entry that is declared, has no type, and must come back unchanged.
MANIFEST = HEAD + """<manifest:manifest \
xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.4">
<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text" manifest:version="1.4"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="Pictures/" manifest:media-type=""/>
<manifest:file-entry manifest:full-path="Pictures/mark.png" manifest:media-type="image/png"/>
<manifest:file-entry manifest:full-path="Pictures/plate.png" manifest:media-type="image/png"/>
<manifest:file-entry manifest:full-path="notes/rider.xml" manifest:media-type=""/>
</manifest:manifest>
"""

RIDER = HEAD + """<rider xmlns="https://example.invalid/rider">\
<note>An entry the manifest gives no media type. It must come back unchanged.</note>\
</rider>
"""


def main():
    here = Path(__file__).resolve().parent
    target = here / "odt" / "second-producer.odt"
    target.parent.mkdir(parents=True, exist_ok=True)

    parts = [
        ("META-INF/manifest.xml", MANIFEST.encode("utf-8")),
        ("meta.xml", META.encode("utf-8")),
        ("styles.xml", styles_xml().encode("utf-8")),
        ("content.xml", content_xml().encode("utf-8")),
        ("Pictures/mark.png", picture(96, 96, (30, 111, 92))),
        ("Pictures/plate.png", picture(440, 280, (60, 70, 90))),
        ("notes/rider.xml", RIDER.encode("utf-8")),
    ]

    with zipfile.ZipFile(target, "w") as out:
        first = zipfile.ZipInfo("mimetype", STAMP)
        first.compress_type = zipfile.ZIP_STORED
        out.writestr(first, MIMETYPE)
        for name, data in parts:
            item = zipfile.ZipInfo(name, STAMP)
            item.compress_type = zipfile.ZIP_DEFLATED
            out.writestr(item, data)
    print(f"{target} written, {target.stat().st_size} bytes")


if __name__ == "__main__":
    main()
