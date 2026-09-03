//! `<style:page-layout>` and `<style:master-page>` — the paper, and what is
//! printed on every sheet of it.
//!
//! ODF splits in two what WordprocessingML keeps in one. A **page layout** is
//! the paper: its size, its margins, its columns. A **master page** is a layout
//! together with the header and footer drawn on it, and it has a name that a
//! paragraph style can point at — which is how ODF spells a section break. The
//! model's `SectionProps` is both halves at once, so a master page is what
//! becomes one.
//!
//! **The margins do not mean the same thing in the two formats, and reading one
//! as the other puts every line of the body in the wrong place.** Word measures
//! `w:top` from the top of the sheet to the top of the *body*, and `w:header`
//! from the top of the sheet to the top of the header, so the header lives
//! inside the margin. ODF measures `fo:margin-top` to the top of the *header*,
//! and the body begins below whatever the header takes up. So the header
//! distance is ODF's margin as it stands, and the body's is that plus the
//! header's own height and the gap under it — an inch where a careless reading
//! would say half of one.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::section::{Columns, Orientation, PageMargins, PageSize, SectionProps};
use wp_model::units::Twips;

use crate::xml::{attr_in, attr_length, attr_u32, end_local_name, local_name, skip_element};

/// A page layout, kept as the paper it is until a master page claims it.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub page: PageSize,
    /// The margins as ODF states them, before the header and footer are taken
    /// into account.
    pub margins: PageMargins,
    pub columns: Columns,
    /// What `<style:header-style>` and `<style:footer-style>` reserve: the
    /// least height, and the gap between the band and the body.
    pub header: Band,
    pub footer: Band,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Band {
    pub height: Twips,
    pub gap: Twips,
}

/// Reads one `<style:page-layout>`, whose start tag the caller has just seen.
pub fn layout(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    layouts: &mut HashMap<String, Layout>,
) {
    let name = attr_in(e, b"style", b"name").unwrap_or_default();
    let mut layout = Layout {
        page: PageSize {
            width: Twips::LETTER_WIDTH,
            height: Twips::LETTER_HEIGHT,
            orientation: Orientation::Portrait,
            code: None,
        },
        ..Layout::default()
    };
    while let Ok(event) = reader.read_event() {
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let child = local_name(&e).to_vec();
                match child.as_slice() {
                    b"page-layout-properties" => {
                        properties(&e, &mut layout);
                        if !empty {
                            columns(reader, &mut layout);
                        }
                    }
                    b"header-style" if !empty => layout.header = band(reader, b"header-style"),
                    b"footer-style" if !empty => layout.footer = band(reader, b"footer-style"),
                    _ if !empty => skip_element(reader, &child),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"page-layout" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    if !name.is_empty() {
        layouts.insert(name, layout);
    }
}

fn properties(e: &BytesStart<'_>, layout: &mut Layout) {
    if let Some(width) = attr_length(e, b"page-width") {
        layout.page.width = width;
    }
    if let Some(height) = attr_length(e, b"page-height") {
        layout.page.height = height;
    }
    layout.page.orientation = match attr_in(e, b"style", b"print-orientation").as_deref() {
        Some("landscape") => Orientation::Landscape,
        _ => Orientation::Portrait,
    };
    layout.margins = PageMargins {
        top: attr_in(e, b"fo", b"margin-top")
            .as_deref()
            .and_then(crate::xml::length)
            .unwrap_or(Twips(1440)),
        bottom: attr_in(e, b"fo", b"margin-bottom")
            .as_deref()
            .and_then(crate::xml::length)
            .unwrap_or(Twips(1440)),
        start: attr_in(e, b"fo", b"margin-left")
            .as_deref()
            .and_then(crate::xml::length)
            .unwrap_or(Twips(1440)),
        end: attr_in(e, b"fo", b"margin-right")
            .as_deref()
            .and_then(crate::xml::length)
            .unwrap_or(Twips(1440)),
        header: Twips(0),
        footer: Twips(0),
        gutter: Twips(0),
    };
}

/// `<style:columns>`, which sits inside the layout's properties.
fn columns(reader: &mut Reader<&[u8]>, layout: &mut Layout) {
    while let Ok(event) = reader.read_event() {
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"columns" {
                    let num = attr_u32(&e, b"column-count").unwrap_or(1).max(1);
                    layout.columns = Columns {
                        num,
                        space: attr_length(&e, b"column-gap").unwrap_or(Twips(720)),
                        equal_width: true,
                        separator: false,
                        columns: Vec::new(),
                    };
                }
                if !empty {
                    skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == b"page-layout-properties" => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// `<style:header-style>` or `<style:footer-style>`, each holding one
/// `<style:header-footer-properties>`.
fn band(reader: &mut Reader<&[u8]>, end: &[u8]) -> Band {
    let mut band = Band::default();
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return band,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"header-footer-properties" {
                    band.height = attr_in(&e, b"fo", b"min-height")
                        .as_deref()
                        .and_then(crate::xml::length)
                        .unwrap_or(Twips(0));
                    // A header's gap is stated as its own bottom margin, and a
                    // footer's as its top one: the two bands face the text from
                    // opposite sides.
                    let gap = match end {
                        b"header-style" => attr_in(&e, b"fo", b"margin-bottom"),
                        _ => attr_in(&e, b"fo", b"margin-top"),
                    };
                    band.gap = gap
                        .as_deref()
                        .and_then(crate::xml::length)
                        .unwrap_or(Twips(0));
                }
                if !empty {
                    skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == end => return band,
            Event::Eof => return band,
            _ => {}
        }
    }
}

/// The section a master page stands for, with the bands it actually carries
/// taken into account.
///
/// `has_header` and `has_footer` are not the same question as whether the
/// layout reserved room for one: a layout states a header height whether or not
/// its master page draws a header, and a page with no header has its body at
/// the margin rather than an inch below it.
pub fn section(layout: &Layout, has_header: bool, has_footer: bool) -> SectionProps {
    let mut margins = layout.margins;
    // The band's *stated minimum*, and only that. What the band actually takes
    // up, and the clear space it keeps under itself, are settled at layout
    // time — the body begins below whichever is larger, and neither is known
    // to a reader. Adding the gap here as well would charge for it twice.
    margins.header = layout.margins.top;
    margins.footer = layout.margins.bottom;
    if has_header {
        margins.top = Twips(layout.margins.top.0 + layout.header.height.0);
    }
    if has_footer {
        margins.bottom = Twips(layout.margins.bottom.0 + layout.footer.height.0);
    }
    SectionProps {
        page: layout.page,
        margins,
        columns: layout.columns.clone(),
        header_gap: match has_header {
            true => layout.header.gap,
            false => Twips(0),
        },
        footer_gap: match has_footer {
            true => layout.footer.gap,
            false => Twips(0),
        },
        footer_min: match has_footer {
            true => layout.footer.height,
            false => Twips(0),
        },
        ..SectionProps::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> Layout {
        let mut layouts = HashMap::new();
        let mut reader = Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if local_name(&e) == b"page-layout" => {
                    layout(&mut reader, &e, &mut layouts)
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        layouts.remove("PL0").expect("the layout was read")
    }

    const SAMPLE: &str = concat!(
        r#"<style:page-layout style:name="PL0"><style:page-layout-properties "#,
        r#"fo:page-width="8.5in" fo:page-height="11in" style:print-orientation="portrait" "#,
        r#"fo:margin-top="0.5in" fo:margin-left="1in" fo:margin-bottom="0.5in" fo:margin-right="1in"/>"#,
        r#"<style:header-style><style:header-footer-properties fo:min-height="0.5in"/></style:header-style>"#,
        r#"<style:footer-style><style:header-footer-properties fo:min-height="0.5in"/></style:footer-style>"#,
        r#"</style:page-layout>"#
    );

    #[test]
    fn a_page_layout_is_the_paper_and_its_margins() {
        let layout = read(SAMPLE);
        assert_eq!(layout.page.width, Twips(12240));
        assert_eq!(layout.page.height, Twips(15840));
        assert_eq!(layout.margins.top, Twips(720));
        assert_eq!(layout.margins.start, Twips(1440));
        assert_eq!(layout.header.height, Twips(720));
    }

    /// The conversion that decides where every line of the body sits.
    #[test]
    fn the_body_begins_below_the_header_rather_than_at_the_margin() {
        let layout = read(SAMPLE);
        let with = section(&layout, true, true);
        assert_eq!(
            with.margins.header,
            Twips(720),
            "the header is at the margin"
        );
        assert_eq!(
            with.margins.top,
            Twips(1440),
            "and the body an inch down, past the header's own half inch"
        );
        assert_eq!(with.margins.bottom, Twips(1440));
        assert_eq!(with.header_gap, Twips(0), "this layout keeps none");

        // A master page that draws no header has its body at the margin, even
        // though the layout still states a height for one.
        let without = section(&layout, false, false);
        assert_eq!(without.margins.top, Twips(720));
        assert_eq!(without.margins.bottom, Twips(720));
    }

    #[test]
    fn columns_are_a_child_of_the_layouts_properties() {
        let layout = read(concat!(
            r#"<style:page-layout style:name="PL0"><style:page-layout-properties "#,
            r#"fo:page-width="8.5in" fo:page-height="11in">"#,
            r#"<style:columns fo:column-count="2" fo:column-gap="0.25in"/>"#,
            r#"</style:page-layout-properties></style:page-layout>"#
        ));
        assert_eq!(layout.columns.num, 2);
        assert_eq!(layout.columns.space, Twips(360));
    }
}
