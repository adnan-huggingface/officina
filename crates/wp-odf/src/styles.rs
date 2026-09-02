//! `<office:styles>`, `<office:automatic-styles>` and `<style:default-style>`.
//!
//! ODF has two stylesheets and a document uses both at once. The **named**
//! styles live in `styles.xml` and are what a person picks from a list;
//! the **automatic** styles live at the top of whichever part uses them and are
//! what direct formatting is written as — a paragraph that a person made bold
//! by hand does not carry bold, it carries `text:style-name="P7"`, and `P7` is
//! an automatic style whose parent is the named one.
//!
//! That is a real difference from WordprocessingML, where direct formatting is
//! written into the run. It is also, conveniently, no difference at all to the
//! model: an automatic style is interned like any other, the paragraph points
//! at it, and `StyleTable::resolve_paragraph` walks the chain it was already
//! built to walk. **Nothing here flattens an automatic style into its
//! paragraph**, because doing so would lose the distinction between a run that
//! states twelve points and a run that inherits them, which is the distinction
//! the whole model exists to keep.
//!
//! Not every family goes into the style table. A paragraph and a text style
//! become `Style`s; a table, column, row or cell style becomes properties in a
//! map of its own, because the model has no `StyleId` for a column and inventing
//! one would put a thing in the stylesheet that no paragraph can ever be in.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::style::{DocDefaults, Style, StyleId, StyleKind, StyleTable};
use wp_model::table::{CellProps, RowProps, TableProps, Width};

use crate::fonts::FontFaces;
use crate::props::{self, Props};
use crate::xml::{attr_in, end_local_name, local_name, skip_element};

/// Everything the two stylesheets said, in the shapes the rest of the reader
/// wants it in.
#[derive(Debug, Default)]
pub struct Styles {
    pub faces: FontFaces,
    /// Paragraph and text styles, by the name a `text:style-name` uses.
    pub by_name: HashMap<String, StyleId>,
    /// Which of those names came from `<office:automatic-styles>` rather than
    /// from `<office:styles>`.
    ///
    /// Nothing in reading a document turns on the difference — an automatic
    /// style is interned and inherited like any other — but writing one does:
    /// an automatic style is not a thing to name as a parent, so a writer
    /// placing new direct formatting has to know which kind it is standing on.
    /// See `write::auto`.
    pub automatic: std::collections::HashSet<String>,
    pub tables: HashMap<String, TableProps>,
    pub rows: HashMap<String, RowProps>,
    pub columns: HashMap<String, Width>,
    pub cells: HashMap<String, CellProps>,
    /// Graphic styles, kept whole because a frame wants its wrap and its
    /// position rather than any one property of them.
    pub graphics: HashMap<String, Props>,
    /// Which list style a paragraph style is numbered by.
    pub list_of_style: HashMap<String, String>,
    /// Which master page a paragraph style starts. ODF spells a section break
    /// as a property of the paragraph after it.
    pub master_of_style: HashMap<String, String>,
    /// Where each `<text:list-style>` ended up in the numbering table.
    pub lists: crate::list::Lists,
    /// `<text:outline-style>`, the numbering of the headings, if it numbers
    /// them at all.
    pub outline: Option<u32>,
}

impl Styles {
    /// The id a `text:style-name` stands for, interning it if the style itself
    /// has not been read yet.
    ///
    /// A body may legally be read before the stylesheet that defines what it
    /// points at — and in ODF it usually is, because `content.xml` carries its
    /// own automatic styles ahead of the body while `styles.xml` is a separate
    /// part read at another time. Interning is what makes the order not matter.
    pub fn id(&mut self, table: &mut StyleTable, name: &str, kind: StyleKind) -> StyleId {
        if let Some(id) = self.by_name.get(name) {
            return *id;
        }
        let id = table.intern(name, kind);
        self.by_name.insert(name.to_string(), id);
        id
    }
}

/// Reads one stylesheet — `<office:styles>` or `<office:automatic-styles>` —
/// whose start tag the caller has just seen.
pub fn read(
    reader: &mut Reader<&[u8]>,
    end: &[u8],
    table: &mut StyleTable,
    styles: &mut Styles,
    numbering: &mut wp_model::Numbering,
    layouts: &mut HashMap<String, crate::page::Layout>,
) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                // A style with nothing to say is written empty, and it is
                // still a style: `<style:style style:name="Quote"
                // style:parent-style-name="Standard"/>` says the whole of what
                // it means in its attributes. Reading only the start tags would
                // lose it, and lose every reference to it with it.
                match name.as_slice() {
                    b"style" => one(reader, &e, empty, table, styles, end == b"automatic-styles"),
                    b"default-style" if !empty => default(reader, &e, table, styles),
                    b"list-style" if !empty => {
                        crate::list::list_style(reader, &e, styles, numbering)
                    }
                    b"outline-style" if !empty => {
                        styles.outline = crate::list::outline_style(reader, styles, numbering)
                    }
                    // A page layout is an automatic style like any other, and
                    // this is the one place every stylesheet is read from.
                    b"page-layout" if !empty => crate::page::layout(reader, &e, layouts),
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// `<style:default-style>` — what everything of a family starts from.
fn default(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    table: &mut StyleTable,
    styles: &mut Styles,
) {
    let family = attr_in(e, b"style", b"family").unwrap_or_default();
    let mut props = Props::default();
    props::properties(reader, b"default-style", &styles.faces, &mut props);
    // Only the paragraph family reaches the document defaults. The others are
    // defaults for tables and drawings, and the model keeps no equivalent —
    // folding them in here would make every paragraph inherit a table's
    // background.
    if family == "paragraph" {
        table.set_doc_defaults(DocDefaults {
            para: props.para,
            run: props.run,
        });
    }
}

/// One `<style:style>`, of whichever family.
fn one(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    empty: bool,
    table: &mut StyleTable,
    styles: &mut Styles,
    automatic: bool,
) {
    let Some(name) = attr_in(e, b"style", b"name") else {
        if !empty {
            skip_element(reader, b"style");
        }
        return;
    };
    let family = attr_in(e, b"style", b"family").unwrap_or_default();
    let mut props = Props {
        parent: attr_in(e, b"style", b"parent-style-name"),
        next: attr_in(e, b"style", b"next-style-name"),
        list_style: attr_in(e, b"style", b"list-style-name"),
        master_page: attr_in(e, b"style", b"master-page-name"),
        ..Props::default()
    };
    if !empty {
        props::properties(reader, b"style", &styles.faces, &mut props);
    }
    if automatic {
        styles.automatic.insert(name.clone());
    }

    match family.as_str() {
        "paragraph" | "text" => {
            let kind = match family.as_str() {
                "text" => StyleKind::Character,
                _ => StyleKind::Paragraph,
            };
            let id = styles.id(table, &name, kind);
            let based_on = props
                .parent
                .as_deref()
                .map(|parent| styles.id(table, parent, kind));
            let next = props
                .next
                .as_deref()
                .map(|next| styles.id(table, next, StyleKind::Paragraph));
            if let Some(list) = props.list_style.clone() {
                styles.list_of_style.insert(name.clone(), list);
            }
            if let Some(master) = props.master_page.clone() {
                // An empty name is how ODF says "no break here", and it is not
                // the same as the attribute being absent.
                styles.master_of_style.insert(name.clone(), master);
            }
            let display = attr_in(e, b"style", b"display-name").unwrap_or_else(|| spelled(&name));
            let mut style = Style::new(name.clone(), kind);
            style.name = Some(display.into());
            style.based_on = based_on;
            style.next = next;
            style.para = props.para;
            style.run = props.run;
            style.custom = true;
            // `insert` replaces by style id and keeps the index, so the
            // placeholder a forward reference interned is filled in rather
            // than shadowed — every id already handed out still points here.
            let filled = table.insert(style);
            debug_assert_eq!(filled, id);
        }
        "table" => {
            if let Some(table_props) = props.table {
                styles.tables.insert(name, table_props);
            }
        }
        "table-row" => {
            if let Some(row) = props.row {
                styles.rows.insert(name, row);
            }
        }
        "table-column" => {
            if let Some(width) = props.column {
                styles.columns.insert(name, width);
            }
        }
        "table-cell" => {
            if let Some(cell) = props.cell {
                styles.cells.insert(name, cell);
            }
        }
        "graphic" => {
            styles.graphics.insert(name, props);
        }
        _ => {}
    }
}

/// ODF escapes a style name's spaces and punctuation into `_20_`-style
/// sequences, so `Heading_20_1` is `Heading 1`.
///
/// The display name usually says so outright and is preferred where it does.
/// Where it does not — hand-written ODF often omits it — a heading style whose
/// name still reads `List_20_Paragraph` is one that nothing looking for
/// `List Paragraph` will find.
fn spelled(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'_' {
            if let Some(end) = name[i + 1..].find('_').map(|at| i + 1 + at) {
                let digits = &name[i + 1..end];
                if !digits.is_empty()
                    && digits.len() <= 4
                    && digits.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    if let Some(ch) = u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        out.push(ch);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        let ch = name[i..].chars().next().unwrap_or('_');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::prop::Toggle;

    /// What the model makes of a paragraph in that style and nothing else.
    fn resolved(table: &StyleTable, style: StyleId) -> wp_model::Layers {
        let direct = wp_model::prop::ParaProps {
            style: Some(style),
            ..Default::default()
        };
        table.resolve_paragraph(&direct, None)
    }

    fn read_styles(xml: &str) -> (StyleTable, Styles, wp_model::Numbering) {
        let mut table = StyleTable::new();
        let mut styles = Styles::default();
        let mut numbering = wp_model::Numbering::new();
        let mut reader = Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if local_name(&e) == b"styles" => read(
                    &mut reader,
                    b"styles",
                    &mut table,
                    &mut styles,
                    &mut numbering,
                    &mut HashMap::new(),
                ),
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        (table, styles, numbering)
    }

    #[test]
    fn a_style_names_its_parent_and_the_chain_is_left_for_the_model_to_walk() {
        let (table, styles, _) = read_styles(concat!(
            r#"<office:styles>"#,
            r#"<style:style style:name="Standard" style:family="paragraph">"#,
            r#"<style:text-properties fo:font-size="11pt"/></style:style>"#,
            r#"<style:style style:name="Heading_20_1" style:family="paragraph" "#,
            r#"style:parent-style-name="Standard" style:display-name="Heading 1">"#,
            r#"<style:text-properties fo:font-weight="bold"/></style:style>"#,
            r#"</office:styles>"#
        ));
        let heading = styles.by_name["Heading_20_1"];
        let style = table.get(heading).expect("the style was read");
        assert_eq!(style.name.as_deref(), Some("Heading 1"));
        assert_eq!(style.based_on, Some(styles.by_name["Standard"]));
        assert_eq!(style.run.toggles.get(Toggle::Bold), Some(true));
        // Not flattened: the heading says nothing about size, and resolving is
        // the model's job rather than the reader's.
        assert_eq!(style.run.size, None);
        assert_eq!(
            resolved(&table, heading).run.size,
            Some(wp_model::HalfPoint(22))
        );
    }

    /// The order the two stylesheets arrive in is not the order they refer to
    /// each other in, and interning is what makes that survivable.
    #[test]
    fn a_style_may_name_a_parent_that_stands_later_in_the_file() {
        let (table, styles, _) = read_styles(concat!(
            r#"<office:styles>"#,
            r#"<style:style style:name="Quote" style:family="paragraph" style:parent-style-name="Standard"/>"#,
            r#"<style:style style:name="Standard" style:family="paragraph">"#,
            r#"<style:text-properties fo:font-size="10pt"/></style:style>"#,
            r#"</office:styles>"#
        ));
        let quote = styles.by_name["Quote"];
        assert_eq!(
            table.get(quote).and_then(|s| s.based_on),
            Some(styles.by_name["Standard"]),
            "the forward reference resolved to the same id the definition filled"
        );
        assert_eq!(
            resolved(&table, quote).run.size,
            Some(wp_model::HalfPoint(20))
        );
    }

    #[test]
    fn a_default_style_is_the_documents_defaults_and_only_for_paragraphs() {
        let (table, _, _) = read_styles(concat!(
            r#"<office:styles>"#,
            r#"<style:default-style style:family="paragraph">"#,
            r#"<style:text-properties fo:font-size="12pt"/></style:default-style>"#,
            r#"<style:default-style style:family="table">"#,
            r##"<style:table-properties fo:background-color="#ff0000"/></style:default-style>"##,
            r#"</office:styles>"#
        ));
        assert_eq!(table.doc_defaults().run.size, Some(wp_model::HalfPoint(24)));
    }

    #[test]
    fn a_style_name_carries_its_spaces_as_escapes() {
        assert_eq!(spelled("Heading_20_1"), "Heading 1");
        assert_eq!(spelled("List_20_Paragraph"), "List Paragraph");
        assert_eq!(spelled("Standard"), "Standard");
        assert_eq!(spelled("Table_20_Contents"), "Table Contents");
        // Not every underscore is an escape, and one that is not stays put.
        assert_eq!(spelled("my_style"), "my_style");
    }

    #[test]
    fn a_family_the_style_table_has_no_place_for_goes_into_a_map_of_its_own() {
        let (table, styles, _) = read_styles(concat!(
            r#"<office:styles>"#,
            r#"<style:style style:name="Table1.A1" style:family="table-cell">"#,
            r##"<style:table-cell-properties fo:background-color="#eeeeee"/></style:style>"##,
            r#"<style:style style:name="Table1.A" style:family="table-column">"#,
            r#"<style:table-column-properties style:column-width="2in"/></style:style>"#,
            r#"</office:styles>"#
        ));
        assert!(styles.cells.contains_key("Table1.A1"));
        assert_eq!(
            styles.columns.get("Table1.A"),
            Some(&Width::Fixed(wp_model::Twips(2880)))
        );
        assert_eq!(table.len(), 0, "and not into the style table");
    }
}
