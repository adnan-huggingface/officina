//! The document theme's colour scheme.
//!
//! Almost every colour in a modern workbook is `theme="4" tint="-0.25"` rather
//! than an RGB triple, so without this part a normally-formatted document is
//! rendered in black on white. The rest of `theme1.xml` — fonts, effects, the
//! whole DrawingML format scheme — is not modeled and is preserved verbatim.
//!
//! Only the *first* `<a:clrScheme>` counts. The part usually ends with an
//! `<a:extraClrSchemeLst>` holding several more, and a reader that takes the
//! last one paints the workbook in a palette its author never chose.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::color::Theme;

use crate::error::{xml_err, Result};
use crate::xml::{attr_text, end_local_name, local_name};

/// The twelve slots, in the order `<a:clrScheme>` writes them.
const SLOTS: [&[u8]; 12] = [
    b"dk1",
    b"lt1",
    b"dk2",
    b"lt2",
    b"accent1",
    b"accent2",
    b"accent3",
    b"accent4",
    b"accent5",
    b"accent6",
    b"hlink",
    b"folHlink",
];

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<Theme> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;

    let mut scheme: Vec<[u8; 3]> = Vec::with_capacity(12);
    let mut inside = false;
    let mut slot: Option<usize> = None;
    let mut buf = Vec::new();

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = local_name(e);
                if name == b"clrScheme" {
                    inside = true;
                } else if inside {
                    if let Some(index) = SLOTS.iter().position(|s| *s == name) {
                        slot = Some(index);
                    } else if slot.is_some_and(|i| i == scheme.len()) {
                        if let Some(rgb) = color_of(e) {
                            scheme.push(rgb);
                            slot = None;
                        }
                    }
                }
            }
            Event::End(ref e) if end_local_name(e) == b"clrScheme" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(Theme::from_scheme(&scheme))
}

/// `<a:srgbClr val="44546A"/>` or `<a:sysClr val="windowText" lastClr="000000"/>`.
///
/// A system colour has no fixed value, which is why the file carries `lastClr`:
/// the colour the producing machine last resolved it to. Using it is what makes
/// "text 1" come out black rather than unresolved.
fn color_of(e: &BytesStart<'_>) -> Option<[u8; 3]> {
    let hex = match local_name(e) {
        b"srgbClr" => attr_text(e, b"val")?,
        b"sysClr" => attr_text(e, b"lastClr").or_else(|| {
            Some(match attr_text(e, b"val")?.as_str() {
                "window" => "FFFFFF".to_string(),
                _ => "000000".to_string(),
            })
        })?,
        _ => return None,
    };
    match ss_model::color::Color::from_hex(&hex) {
        Some(ss_model::color::Color::Rgb([_, r, g, b])) => Some([r, g, b]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r#"<?xml version="1.0"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
 <a:themeElements>
  <a:clrScheme name="Office">
   <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
   <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
   <a:dk2><a:srgbClr val="44546A"/></a:dk2>
   <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
   <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
   <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
   <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
   <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
   <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
   <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
   <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
   <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
  </a:clrScheme>
 </a:themeElements>
 <a:extraClrSchemeLst>
  <a:clrScheme name="Wrong"><a:dk1><a:srgbClr val="FF0000"/></a:dk1></a:clrScheme>
 </a:extraClrSchemeLst>
</a:theme>"#;

    #[test]
    fn the_scheme_is_read_and_reordered_into_excels_indices() {
        let theme = parse("theme1.xml", THEME.as_bytes()).expect("parses");
        assert_eq!(theme.color(0), Some([0xFF, 0xFF, 0xFF]), "theme 0 is lt1");
        assert_eq!(theme.color(1), Some([0x00, 0x00, 0x00]), "theme 1 is dk1");
        assert_eq!(theme.color(4), Some([0x44, 0x72, 0xC4]), "accent1");
        assert_eq!(theme.color(11), Some([0x95, 0x4F, 0x72]));
        assert_eq!(theme.len(), 12);
    }

    #[test]
    fn only_the_first_scheme_counts() {
        // The part usually carries several more in `<a:extraClrSchemeLst>`.
        let theme = parse("theme1.xml", THEME.as_bytes()).expect("parses");
        assert_ne!(theme.color(1), Some([0xFF, 0, 0]));
    }

    #[test]
    fn a_theme_we_cannot_read_falls_back_to_the_office_default() {
        let theme = parse("theme1.xml", b"<a:theme/>").expect("parses");
        assert_eq!(theme.color(4), Some([0x44, 0x72, 0xC4]));
    }
}
