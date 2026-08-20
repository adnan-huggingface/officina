#!/usr/bin/env python3
"""Three strangers for the corpus, written by nobody's word processor.

The corpus README explains that every file generate.ps1 makes is Word's own
dialect, and that documents from a second producer are the most valuable thing
the corpus can hold. The first three strangers were sample files downloaded
from the internet; their redistribution terms were never recorded, so they
were replaced by these — same filenames, same approximate sizes, same
features (lorem body text, an inline picture, a column-wide square-wrapped
float, one clustered bar chart), but built by this script out of hand-written
OOXML, which makes them a producer dialect all their own. The pictures are
procedurally generated here too: film grain over a gradient, owned by no one.

    python strangers.py

writes file-sample_100kB.docx, file-sample_500kB.docx and file-sample_1MB.docx
into docx/ beside this script. Deterministic: the same bytes every run.

Requires Pillow.
"""

import io
import random
import zipfile
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
STAMP = (2026, 8, 20, 0, 0, 0)  # fixed zip timestamps, so reruns are identical
CREATED = "2026-08-20T00:00:00Z"

NS_W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS_WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
NS_A = "http://schemas.openxmlformats.org/drawingml/2006/main"
NS_PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
NS_C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
NS_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"

WORDS = (
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod "
    "tempor incididunt ut labore et dolore magna aliqua enim ad minim veniam "
    "quis nostrud exercitation ullamco laboris nisi aliquip ex ea commodo "
    "consequat duis aute irure in reprehenderit voluptate velit esse cillum "
    "fugiat nulla pariatur excepteur sint occaecat cupidatat non proident "
    "sunt culpa qui officia deserunt mollit anim id est laborum"
).split()


def lorem(rng, sentences):
    out = []
    for _ in range(sentences):
        n = rng.randint(8, 16)
        words = [rng.choice(WORDS) for _ in range(n)]
        out.append(words[0].capitalize() + " " + " ".join(words[1:]) + ".")
    return " ".join(out)


def picture(rng, width, height, grain):
    """A gradient with translucent discs under film grain.

    The grain is the point: noise does not compress, so its amplitude and the
    canvas area are what set the PNG's size.
    """
    img = Image.new("RGB", (width, height))
    top, bottom = (30, 84, 110), (216, 178, 128)
    for y in range(height):
        t = y / max(1, height - 1)
        row = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        img.paste(row, (0, y, width, y + 1))
    discs = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(discs)
    for _ in range(9):
        r = rng.randint(width // 12, width // 4)
        x, y = rng.randint(-r, width), rng.randint(-r, height)
        tint = rng.choice([(255, 244, 214), (52, 120, 96), (200, 90, 60)])
        draw.ellipse([x, y, x + 2 * r, y + 2 * r], fill=tint + (70,))
    img = Image.alpha_composite(img.convert("RGBA"), discs).convert("RGB")
    noise = Image.effect_noise((width, height), 48).convert("RGB")
    img = Image.blend(img, noise, grain)
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def picture_of_size(rng, target, aspect=0.66):
    """Iterate the canvas area until the PNG lands within 5% of target."""
    width = 480
    for _ in range(12):
        data = picture(random.Random(rng), width, round(width * aspect), 0.32)
        if abs(len(data) - target) <= target * 0.05:
            return data
        width = max(64, round(width * (target / len(data)) ** 0.5))
    return data


def esc(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def paragraph(text, bold=False, size=None):
    props = ""
    if bold or size:
        rpr = ("<w:b/>" if bold else "") + (
            f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>' if size else ""
        )
        props = f"<w:rPr>{rpr}</w:rPr>"
    return f"<w:p><w:r>{props}<w:t xml:space=\"preserve\">{esc(text)}</w:t></w:r></w:p>"


def inline_picture(rel, docpr, name, cx, cy):
    return (
        f'<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
        f'<wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{docpr}" name="{name}"/>'
        f"<a:graphic><a:graphicData uri=\"{NS_PIC}\">{pic_xml(rel, docpr, name, cx, cy)}"
        f"</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
    )


def floating_picture(rel, docpr, name, cx, cy):
    """Anchored to its paragraph, square wrap, as wide as the text column."""
    return (
        f'<w:p><w:r><w:drawing>'
        f'<wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0"'
        f' relativeHeight="251658240" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">'
        f'<wp:simplePos x="0" y="0"/>'
        f'<wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH>'
        f'<wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>'
        f'<wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
        f'<wp:wrapSquare wrapText="bothSides"/>'
        f'<wp:docPr id="{docpr}" name="{name}"/>'
        f"<a:graphic><a:graphicData uri=\"{NS_PIC}\">{pic_xml(rel, docpr, name, cx, cy)}"
        f"</a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"
        f'<w:r><w:t xml:space="preserve">The text resumes below the picture, '
        f"which is the wrap Word gives a column-wide float.</w:t></w:r></w:p>"
    )


def pic_xml(rel, docpr, name, cx, cy):
    return (
        f'<pic:pic><pic:nvPicPr><pic:cNvPr id="{docpr}" name="{name}"/><pic:cNvPicPr/>'
        f'</pic:nvPicPr><pic:blipFill><a:blip r:embed="{rel}"/>'
        f"<a:stretch><a:fillRect/></a:stretch></pic:blipFill>"
        f'<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
        f'<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
    )


def chart_drawing(rel, docpr):
    return (
        f'<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
        f'<wp:extent cx="5486400" cy="3200400"/><wp:docPr id="{docpr}" name="Chart {docpr}"/>'
        f'<a:graphic><a:graphicData uri="{NS_C}">'
        f'<c:chart xmlns:c="{NS_C}" r:id="{rel}"/>'
        f"</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
    )


def chart_part():
    quarters = ["Q1", "Q2", "Q3", "Q4"]
    series = [
        ("North", "4472C4", [12.0, 17.5, 9.0, 21.0]),
        ("South", "ED7D31", [8.5, 11.0, 14.5, 10.0]),
    ]
    sers = []
    for idx, (label, color, values) in enumerate(series):
        cats = "".join(
            f'<c:pt idx="{i}"><c:v>{q}</c:v></c:pt>' for i, q in enumerate(quarters)
        )
        vals = "".join(
            f'<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>' for i, v in enumerate(values)
        )
        col = chr(ord("B") + idx)
        sers.append(
            f'<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/>'
            f"<c:tx><c:strRef><c:f>Sheet1!${col}$1</c:f><c:strCache>"
            f'<c:ptCount val="1"/><c:pt idx="0"><c:v>{label}</c:v></c:pt>'
            f"</c:strCache></c:strRef></c:tx>"
            f'<c:spPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></c:spPr>'
            f"<c:cat><c:strRef><c:f>Sheet1!$A$2:$A$5</c:f><c:strCache>"
            f'<c:ptCount val="4"/>{cats}</c:strCache></c:strRef></c:cat>'
            f"<c:val><c:numRef><c:f>Sheet1!${col}$2:${col}$5</c:f><c:numCache>"
            f'<c:formatCode>General</c:formatCode><c:ptCount val="4"/>{vals}'
            f"</c:numCache></c:numRef></c:val></c:ser>"
        )
    return (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        f'<c:chartSpace xmlns:c="{NS_C}" xmlns:a="{NS_A}" xmlns:r="{NS_R}">'
        f"<c:chart><c:plotArea><c:layout/>"
        f'<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>'
        f'<c:varyColors val="0"/>{"".join(sers)}'
        f'<c:axId val="111111111"/><c:axId val="222222222"/></c:barChart>'
        f'<c:catAx><c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/>'
        f'</c:scaling><c:delete val="0"/><c:axPos val="b"/>'
        f'<c:crossAx val="222222222"/></c:catAx>'
        f'<c:valAx><c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/>'
        f'</c:scaling><c:delete val="0"/><c:axPos val="l"/>'
        f'<c:crossAx val="111111111"/></c:valAx>'
        f"</c:plotArea><c:legend><c:legendPos val=\"b\"/></c:legend>"
        f'<c:plotVisOnly val="1"/></c:chart></c:chartSpace>'
    )


STYLES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
    f'<w:styles xmlns:w="{NS_W}"><w:docDefaults><w:rPrDefault><w:rPr>'
    '<w:rFonts w:ascii="Liberation Serif" w:hAnsi="Liberation Serif"/>'
    '<w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:rPrDefault>'
    "<w:pPrDefault/></w:docDefaults></w:styles>"
)


def build(name, target, rng, with_float_and_chart):
    """One stranger: measure the wrapper once, then size the picture to fit."""
    probe = assemble(name, b"", rng, with_float_and_chart)
    image = picture_of_size(rng + 1, max(20_000, target - len(probe)))
    data = assemble(name, image, rng, with_float_and_chart)
    (HERE / "docx" / name).write_bytes(data)
    print(f"{name}: {len(data):,} bytes (target {target:,})")


def assemble(title, image, rng_seed, with_float_and_chart):
    rng = random.Random(rng_seed)
    body = [paragraph("A stranger in the corpus", bold=True, size="32")]
    body.append(
        paragraph(
            "This document was not written by Word. It exists to hold the "
            "reader to a second producer's dialect: see corpus/strangers.py, "
            "which wrote it and can write it again."
        )
    )
    for _ in range(3):
        body.append(paragraph(lorem(rng, 4)))
    if with_float_and_chart:
        # Column-wide at the default 1in margins: 6.5in of 12700 EMU points.
        body.append(floating_picture("rId20", 1, "grain-wide.png", 5943600, 2200000))
        for _ in range(2):
            body.append(paragraph(lorem(rng, 4)))
        body.append(paragraph("Quarterly figures", bold=True, size="28"))
        body.append(chart_drawing("rId21", 2))
    else:
        body.append(inline_picture("rId20", 1, "grain.png", 3657600, 2413600))
    for _ in range(4):
        body.append(paragraph(lorem(rng, 5)))
    document = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        f'<w:document xmlns:w="{NS_W}" xmlns:wp="{NS_WP}" xmlns:a="{NS_A}"'
        f' xmlns:pic="{NS_PIC}" xmlns:r="{NS_R}">'
        f'<w:body>{"".join(body)}'
        '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"'
        ' w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>'
        "</w:body></w:document>"
    )

    overrides = {
        "/word/document.xml": "application/vnd.openxmlformats-officedocument"
        ".wordprocessingml.document.main+xml",
        "/word/styles.xml": "application/vnd.openxmlformats-officedocument"
        ".wordprocessingml.styles+xml",
        "/docProps/core.xml": "application/vnd.openxmlformats-package"
        ".core-properties+xml",
        "/docProps/app.xml": "application/vnd.openxmlformats-officedocument"
        ".extended-properties+xml",
    }
    doc_rels = [
        (
            "rId1",
            "http://schemas.openxmlformats.org/officeDocument/2006/"
            "relationships/styles",
            "styles.xml",
        ),
        (
            "rId20",
            "http://schemas.openxmlformats.org/officeDocument/2006/"
            "relationships/image",
            "media/image1.png",
        ),
    ]
    parts = {}
    if with_float_and_chart:
        overrides["/word/charts/chart1.xml"] = (
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
        )
        doc_rels.append(
            (
                "rId21",
                "http://schemas.openxmlformats.org/officeDocument/2006/"
                "relationships/chart",
                "charts/chart1.xml",
            )
        )
        parts["word/charts/chart1.xml"] = chart_part().encode()

    content_types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats'
        '-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Default Extension="png" ContentType="image/png"/>'
        + "".join(
            f'<Override PartName="{part}" ContentType="{ct}"/>'
            for part, ct in overrides.items()
        )
        + "</Types>"
    )
    root_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/'
        'officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/'
        'package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>'
        '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/'
        'officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>'
        "</Relationships>"
    )
    document_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(
            f'<Relationship Id="{rid}" Type="{typ}" Target="{target}"/>'
            for rid, typ, target in doc_rels
        )
        + "</Relationships>"
    )
    core = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        "<cp:coreProperties"
        ' xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"'
        ' xmlns:dc="http://purl.org/dc/elements/1.1/"'
        ' xmlns:dcterms="http://purl.org/dc/terms/"'
        ' xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
        f"<dc:title>{esc(title)}</dc:title>"
        "<dc:creator>corpus/strangers.py</dc:creator>"
        f'<dcterms:created xsi:type="dcterms:W3CDTF">{CREATED}</dcterms:created>'
        f'<dcterms:modified xsi:type="dcterms:W3CDTF">{CREATED}</dcterms:modified>'
        "</cp:coreProperties>"
    )
    app = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/"
        '2006/extended-properties">'
        "<Application>Officina corpus/strangers.py</Application></Properties>"
    )

    ordered = {
        "[Content_Types].xml": content_types.encode(),
        "_rels/.rels": root_rels.encode(),
        "word/document.xml": document.encode(),
        "word/_rels/document.xml.rels": document_rels.encode(),
        "word/styles.xml": STYLES.encode(),
        **parts,
        "word/media/image1.png": image,
        "docProps/core.xml": core.encode(),
        "docProps/app.xml": app.encode(),
    }
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in ordered.items():
            zf.writestr(zipfile.ZipInfo(name, STAMP), data)
    return buf.getvalue()


if __name__ == "__main__":
    build("file-sample_100kB.docx", 100_000, 100, with_float_and_chart=False)
    build("file-sample_500kB.docx", 500_000, 500, with_float_and_chart=True)
    build("file-sample_1MB.docx", 1_000_000, 1000, with_float_and_chart=False)
