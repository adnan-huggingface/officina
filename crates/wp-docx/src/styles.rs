//! `styles.xml` — document defaults and every named style.

use quick_xml::events::Event;
use quick_xml::Reader;

use wp_model::style::{DocDefaults, Style, StyleKind};

use crate::ctx::Ctx;
use crate::props;
use crate::xml::{attr, attr_i32, end_local_name, local_name, on_off, val};

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
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
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
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"style" => break,
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
