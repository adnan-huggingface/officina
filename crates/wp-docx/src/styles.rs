//! `styles.xml` — document defaults and every named style.

use quick_xml::events::Event;
use quick_xml::Reader;

use wp_model::style::{DocDefaults, Style, StyleKind};

use crate::ctx::Ctx;
use crate::props;
use crate::xml::{attr, attr_i32, attr_u32, end_local_name, local_name, on_off, val};

/// Reads the whole part into `ctx.styles`.
///
/// Into rather than returning one, because the document may have been read
/// first and already interned the ids that `<w:pStyle>` referred to. Inserting
/// over those placeholders is what ties the two halves together.
pub(crate) fn read(xml: &[u8], ctx: &mut Ctx<'_>) {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"docDefaults" => {
                    let defaults = read_defaults(&mut reader, ctx);
                    ctx.styles.set_doc_defaults(defaults);
                }
                b"style" => {
                    let kind = attr(&e, b"type")
                        .as_deref()
                        .and_then(StyleKind::from_val)
                        .unwrap_or(StyleKind::Paragraph);
                    let Some(id) = attr(&e, b"styleId") else {
                        continue;
                    };
                    let mut style = Style::new(id.as_str(), kind);
                    style.default = attr(&e, b"default")
                        .as_deref()
                        .map(|v| wp_model::prop::on_off(Some(v)))
                        .unwrap_or(false);
                    style.custom = attr(&e, b"customStyle")
                        .as_deref()
                        .map(|v| wp_model::prop::on_off(Some(v)))
                        .unwrap_or(false);
                    read_style(&mut reader, ctx, &mut style, kind);
                    ctx.styles.insert(style);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
}

fn read_defaults(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> DocDefaults {
    let mut defaults = DocDefaults::default();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"rPr" => defaults.run = props::run_props(reader, ctx).props,
                b"pPr" => defaults.para = props::para_props(reader, ctx).props,
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"docDefaults" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    defaults
}

fn read_style(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, style: &mut Style, kind: StyleKind) {
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"name" => style.name = val(&e).map(Into::into),
                    b"basedOn" => {
                        // A style is based on one of its own kind, and interning it
                        // with the wrong kind would make a placeholder that the real
                        // definition later has to correct. Cheap to get right here.
                        style.based_on = val(&e).map(|id| ctx.styles.intern(&id, kind));
                    }
                    b"next" => style.next = val(&e).map(|id| ctx.styles.intern(&id, kind)),
                    // `<w:link>` pairs a paragraph style with a character style, so
                    // the linked one is always of the *other* kind.
                    b"link" => {
                        let linked = match kind {
                            StyleKind::Paragraph => StyleKind::Character,
                            StyleKind::Character => StyleKind::Paragraph,
                            other => other,
                        };
                        style.link = val(&e).map(|id| ctx.styles.intern(&id, linked));
                    }
                    b"uiPriority" => style.priority = attr_i32(&e, b"val"),
                    b"qFormat" => style.quick = on_off(&e),
                    b"semiHidden" => style.semi_hidden = on_off(&e),
                    b"unhideWhenUsed" => style.unhide_when_used = on_off(&e),
                    b"pPr" => {
                        let read = props::para_props(reader, ctx);
                        style.para = read.props;
                    }
                    b"rPr" => style.run = props::run_props(reader, ctx).props,
                    // The one slice of a table style's `tblPr` the layout
                    // resolves. The resume that found this keeps its cell
                    // margins here and nowhere else.
                    b"tblPr" if !empty => {
                        let scheme = table_scheme(style);
                        read_style_table(reader, b"tblPr", &mut scheme.whole);
                        if let Some(size) = scheme.whole.row_band {
                            scheme.row_band = size.max(1);
                        }
                        if let Some(size) = scheme.whole.column_band {
                            scheme.column_band = size.max(1);
                        }
                        style.cell_margins = scheme.whole.cell_margins;
                    }
                    // A conditional band holds its own pPr, rPr, tblPr and
                    // tcPr; falling through would blend the header row's white
                    // bold into every cell of the table.
                    b"tblStylePr" if !empty => {
                        // `w:type`, not `w:val`: the one element in the style
                        // part whose name is not in the usual attribute.
                        let band = attr(&e, b"type")
                            .as_deref()
                            .and_then(wp_model::Band::from_val);
                        let mut part = wp_model::TablePart::default();
                        read_band(reader, ctx, &mut part);
                        if let Some(band) = band {
                            if !part.is_empty() {
                                table_scheme(style).parts.insert(band, part);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"style" => break,
            Event::Eof => break,
            _ => {}
        }
    }
}

/// The style's conditional scheme, made on first use.
fn table_scheme(style: &mut Style) -> &mut wp_model::TableScheme {
    style
        .table
        .get_or_insert_with(|| Box::new(wp_model::TableScheme::default()))
}

/// A `<w:tblPr>` as a *style* states it: the grid it draws, the padding inside
/// every cell, and how many rows a stripe covers.
fn read_style_table(reader: &mut Reader<&[u8]>, until: &[u8], part: &mut wp_model::TablePart) {
    let mut band = (None, None);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"tblCellMar" => {
                    part.cell_margins = crate::body::read_cell_margins(reader, b"tblCellMar");
                }
                b"tblBorders" => {
                    part.borders = crate::body::read_table_borders(reader, b"tblBorders").0;
                }
                b"tblInd" => part.indent = Some(crate::body::width(&e)),
                b"shd" => part.cell_shading = Some(props::shading(&e)),
                b"tblStyleRowBandSize" => band.0 = attr_u32(&e, b"val"),
                b"tblStyleColBandSize" => band.1 = attr_u32(&e, b"val"),
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == until => break,
            Event::Eof => break,
            _ => {}
        }
    }
    part.row_band = band.0;
    part.column_band = band.1;
}

/// One `<w:tblStylePr>`: what this band says about the text, the grid and the
/// cells it covers.
fn read_band(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, part: &mut wp_model::TablePart) {
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"pPr" => part.para = props::para_props(reader, ctx).props,
                b"rPr" => part.run = props::run_props(reader, ctx).props,
                b"tblPr" => read_style_table(reader, b"tblPr", part),
                b"tcPr" => read_band_cell(reader, part),
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"tblStylePr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
}

/// The `<w:tcPr>` of a band — the edges and fill it gives every cell it covers.
fn read_band_cell(reader: &mut Reader<&[u8]>, part: &mut wp_model::TablePart) {
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"tcBorders" => {
                    part.cell_borders = crate::body::read_table_borders(reader, b"tcBorders").0;
                }
                b"shd" => part.cell_shading = Some(props::shading(&e)),
                b"vAlign" => {
                    part.cell_v_align = val(&e)
                        .as_deref()
                        .and_then(wp_model::table::CellVAlign::from_val);
                }
                b"tcMar" => part.cell_margins = crate::body::read_cell_margins(reader, b"tcMar"),
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"tcPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_ctx;
    use wp_model::prop::Toggle;
    use wp_model::units::HalfPoint;

    fn styles_of(xml: &str) -> wp_model::StyleTable {
        let (mut styles, mut headers) = test_ctx();
        {
            let mut ctx = Ctx::new(&mut styles, &mut headers);
            read(xml.as_bytes(), &mut ctx);
        }
        styles
    }

    #[test]
    fn document_defaults_are_a_layer_of_their_own() {
        let styles = styles_of(
            r#"<w:styles><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults></w:styles>"#,
        );
        let defaults = styles.doc_defaults();
        assert_eq!(defaults.run.size, Some(HalfPoint(22)));
        assert_eq!(defaults.run.fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(defaults.para.spacing.after, Some(wp_model::Twips(160)));
    }

    #[test]
    fn a_table_styles_cell_margins_are_read_and_a_bands_formatting_is_not() {
        // The margins are written the way Google Docs writes them — decimal
        // strings in a style's tblPr, nowhere near the table itself. The
        // firstRow band carries bold that must NOT become the base style's.
        let styles = styles_of(
            r#"<w:styles>
              <w:style w:type="table" w:styleId="Boxed">
                <w:name w:val="Boxed"/>
                <w:tblPr>
                  <w:tblBorders><w:top w:val="single" w:sz="4"/></w:tblBorders>
                  <w:tblCellMar><w:top w:w="55.0" w:type="dxa"/><w:left w:w="55.0" w:type="dxa"/><w:bottom w:w="55.0" w:type="dxa"/><w:right w:w="55.0" w:type="dxa"/></w:tblCellMar>
                </w:tblPr>
                <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr></w:tblStylePr>
              </w:style>
            </w:styles>"#,
        );
        let boxed = styles.lookup("Boxed").expect("the style is defined");
        let style = styles.get(boxed).unwrap();
        use wp_model::table::Width;
        use wp_model::Twips;
        assert_eq!(style.cell_margins.top, Some(Width::Fixed(Twips(55))));
        assert_eq!(style.cell_margins.start, Some(Width::Fixed(Twips(55))));
        assert_eq!(style.cell_margins.bottom, Some(Width::Fixed(Twips(55))));
        assert_eq!(style.cell_margins.end, Some(Width::Fixed(Twips(55))));
        assert_eq!(
            style.run.toggles.get(Toggle::Bold),
            None,
            "the first-row band's bold stayed in its band"
        );
    }

    #[test]
    fn a_style_reads_its_chain_its_name_and_its_formatting() {
        let styles = styles_of(
            r#"<w:styles>
              <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
              <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:link w:val="Heading1Char"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
            </w:styles>"#,
        );
        let normal = styles.lookup("Normal").expect("Normal is defined");
        let heading = styles.lookup("Heading1").expect("Heading1 is defined");
        assert!(styles.get(normal).unwrap().default);
        assert_eq!(styles.default_style(StyleKind::Paragraph), Some(normal));

        let heading = styles.get(heading).unwrap();
        assert_eq!(heading.name.as_deref(), Some("heading 1"));
        assert_eq!(heading.based_on, Some(normal));
        assert_eq!(heading.next, Some(normal));
        assert_eq!(heading.priority, Some(9));
        assert!(heading.quick);
        assert_eq!(heading.para.outline_level, Some(0));
        assert_eq!(heading.run.toggles.get(Toggle::Bold), Some(true));
        assert_eq!(heading.run.size, Some(HalfPoint(32)));
    }

    #[test]
    fn a_linked_style_is_interned_as_the_other_kind() {
        let styles = styles_of(
            r#"<w:styles><w:style w:type="paragraph" w:styleId="Quote"><w:link w:val="QuoteChar"/></w:style></w:styles>"#,
        );
        let linked = styles.lookup("QuoteChar").expect("the link makes an id");
        assert_eq!(styles.get(linked).unwrap().kind, StyleKind::Character);
    }

    #[test]
    fn a_style_read_after_it_was_referred_to_keeps_its_id() {
        // The document may be read before styles.xml — the relationship order
        // decides — so a `<w:pStyle w:val="Heading1"/>` interns a placeholder
        // that the real definition must land on rather than beside.
        let (mut styles, mut headers) = test_ctx();
        let placeholder = styles.intern("Heading1", StyleKind::Paragraph);
        {
            let mut ctx = Ctx::new(&mut styles, &mut headers);
            read(
                br#"<w:styles><w:style w:type="paragraph" w:styleId="Heading1"><w:rPr><w:b/></w:rPr></w:style></w:styles>"#,
                &mut ctx,
            );
        }
        assert_eq!(styles.lookup("Heading1"), Some(placeholder));
        assert!(styles.get(placeholder).unwrap().run.bold());
    }
}
