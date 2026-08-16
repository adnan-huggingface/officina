import zipfile, os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "probes")
os.makedirs(OUT, exist_ok=True)

CT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"""
RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"""

def rpr(font, half):
    return f'<w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}"/><w:sz w:val="{half}"/><w:szCs w:val="{half}"/></w:rPr>'

def para(font, half, text="Xg"):
    return (f'<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{rpr(font, half)}</w:pPr>'
            f'<w:r>{rpr(font, half)}<w:t>{text}</w:t></w:r></w:p>')

def doc(body):
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}'
            f'<w:sectPr><w:pgSz w:w="12240" w:h="15840"/>'
            f'<w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720" w:header="360" w:footer="360"/></w:sectPr>'
            f'</w:body></w:document>')

def save(name, body):
    path = os.path.join(OUT, name)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CT)
        z.writestr("_rels/.rels", RELS)
        z.writestr("word/document.xml", doc(body))
    print("wrote", name)

# Plain runs of identical lines: pitch over 29 gaps nails the line height.
for font, half in [("Verdana", 20), ("Verdana", 28), ("Times New Roman", 20),
                   ("Arial", 20), ("Calibri", 22)]:
    body = "".join(para(font, half) for _ in range(30))
    save(f"lines-{font.split()[0].lower()}-{half}.docx", body)

# Tables: 12 one-line rows, cell margins zero, three border weights.
def table(border):
    borders = ""
    if border:
        b = f'<w:top w:val="single" w:sz="{border}"/><w:insideH w:val="single" w:sz="{border}"/><w:bottom w:val="single" w:sz="{border}"/>'
        borders = f"<w:tblBorders>{b}</w:tblBorders>"
    margins = '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar>'
    rows = "".join(
        f'<w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>{para("Verdana", 20)}</w:tc></w:tr>'
        for _ in range(12))
    return (f'<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/>{borders}{margins}</w:tblPr>'
            f'<w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{rows}</w:tbl>')

for border, name in [(0, "none"), (4, "half"), (16, "two")]:
    save(f"table-border-{name}.docx", table(border) + para("Verdana", 20, ""))
