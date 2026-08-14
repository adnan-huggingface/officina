//! The small parts: `settings.xml`, `theme1.xml`, and `people.xml`.

use quick_xml::events::Event;
use quick_xml::Reader;

use wp_model::color::Theme;
use wp_model::doc::Settings;
use wp_model::revision::People;
use wp_model::units::Twips;

use crate::xml::{attr, attr_twips, attr_u32, local_name, on_off, val};

/// `settings.xml`.
///
/// The part is mostly compatibility flags from Word 6 and revision-save
/// identifiers, none of which change what is drawn. What is read is what does.
pub(crate) fn settings(xml: &[u8]) -> Settings {
    let mut settings = Settings::default();
    let mut reader = Reader::from_reader(xml);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"evenAndOddHeaders" => settings.even_and_odd_headers = on_off(&e),
                b"mirrorMargins" => settings.mirror_margins = on_off(&e),
                b"defaultTabStop" => {
                    settings.default_tab_stop = attr_twips(&e, b"val").unwrap_or(Twips(720))
                }
                b"trackChanges" => settings.track_changes = on_off(&e),
                b"documentProtection" => settings.protected = true,
                b"autoHyphenation" => settings.hyphenate = on_off(&e),
                b"consecutiveHyphenLimit" => {
                    settings.hyphen_limit = attr_u32(&e, b"val").unwrap_or(0)
                }
                b"zoom" => settings.zoom = attr_u32(&e, b"percent"),
                b"rsids" => settings.has_rsids = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    settings
}

/// `theme1.xml` — the colour scheme, in the order `<a:clrScheme>` writes it.
///
/// Each slot holds either an `<a:srgbClr val="…">` or an `<a:sysClr lastClr="…">`,
/// and the two are equally common: Office's own themes spell dk1 and lt1 as
/// system colours with the resolved value cached beside them. Reading only
/// `srgbClr` loses black and white, which are the two the whole document is
/// drawn in.
pub(crate) fn theme(xml: &[u8]) -> Theme {
    let mut scheme: Vec<[u8; 3]> = Vec::with_capacity(12);
    let mut reader = Reader::from_reader(xml);
    let mut in_scheme = false;
    let mut pending: Option<[u8; 3]> = None;
    let mut depth = 0usize;

    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let (e, empty) = match event {
                    Event::Empty(e) => (e, true),
                    Event::Start(e) => (e, false),
                    _ => unreachable!(),
                };
                match local_name(&e) {
                    b"clrScheme" => {
                        in_scheme = true;
                        depth = 0;
                    }
                    b"srgbClr" | b"sysClr" if in_scheme => {
                        let hex = if local_name(&e) == b"sysClr" {
                            attr(&e, b"lastClr")
                        } else {
                            val(&e)
                        };
                        if let Some(rgb) = hex.as_deref().and_then(parse_rgb) {
                            pending = Some(rgb);
                        }
                    }
                    _ if in_scheme => {
                        // A direct child of `<a:clrScheme>` is a slot: dk1, lt1,
                        // dk2, lt2, accent1..6, hlink, folHlink. Their names are
                        // the slot names and their order is the scheme's order.
                        if depth == 0 && !empty {
                            pending = None;
                        }
                        if !empty {
                            depth += 1;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = crate::xml::end_local_name(&e);
                if name == b"clrScheme" {
                    break;
                }
                if in_scheme && depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        scheme.push(pending.take().unwrap_or([0, 0, 0]));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Theme::from_scheme(&scheme)
}

fn parse_rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim();
    if hex.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

/// `people.xml` — the authors of a document's tracked changes, as Word records
/// them for its presence indicator. Read so an author list exists without
/// walking the whole document.
pub(crate) fn people(xml: &[u8]) -> People {
    let mut people = People::default();
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"person" => {
                if let Some(author) = attr(&e, b"author") {
                    people.record(&author.into());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    people
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::color::ThemeSlot;

    #[test]
    fn the_settings_that_change_what_is_drawn_are_read() {
        let settings = settings(
            br#"<w:settings><w:zoom w:percent="120"/><w:defaultTabStop w:val="720"/><w:evenAndOddHeaders/><w:trackChanges/><w:autoHyphenation w:val="0"/><w:rsids><w:rsidRoot w:val="002A5EF5"/></w:rsids></w:settings>"#,
        );
        assert_eq!(settings.zoom, Some(120));
        assert_eq!(settings.default_tab_stop, Twips(720));
        assert!(settings.even_and_odd_headers);
        assert!(settings.track_changes);
        assert!(!settings.hyphenate);
        assert!(settings.has_rsids);
        assert!(!settings.protected);
    }

    #[test]
    fn a_system_colour_slot_is_read_from_its_cached_value() {
        // Office's own themes spell dk1 and lt1 as `<a:sysClr>`, so a reader
        // that only understands `<a:srgbClr>` loses black and white — the two
        // colours the entire document is drawn in.
        let theme = theme(
            br#"<a:theme><a:themeElements><a:clrScheme name="Office">
              <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
              <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
              <a:dk2><a:srgbClr val="44546A"/></a:dk2>
              <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
              <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
            </a:clrScheme></a:themeElements></a:theme>"#,
        );
        assert_eq!(theme.color(ThemeSlot::Dark1), Some([0, 0, 0]));
        assert_eq!(theme.color(ThemeSlot::Light1), Some([0xFF, 0xFF, 0xFF]));
        assert_eq!(theme.color(ThemeSlot::Dark2), Some([0x44, 0x54, 0x6A]));
        assert_eq!(theme.color(ThemeSlot::Accent1), Some([0x44, 0x72, 0xC4]));
    }

    #[test]
    fn a_theme_that_cannot_be_read_falls_back_rather_than_going_black() {
        let theme = theme(b"<a:theme/>");
        assert_eq!(theme.color(ThemeSlot::Light1), Some([0xFF, 0xFF, 0xFF]));
    }

    #[test]
    fn the_author_list_comes_out_of_people_xml() {
        let people = people(
            br#"<w15:people><w15:person w15:author="Adnan Khan"><w15:presenceInfo/></w15:person><w15:person w15:author="Reviewer"/></w15:people>"#,
        );
        assert_eq!(people.authors.len(), 2);
        assert_eq!(people.authors[0].as_ref(), "Adnan Khan");
    }
}
