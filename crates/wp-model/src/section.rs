//! Sections: page size, margins, columns, and the headers and footers that
//! belong to them.
//!
//! A section is not a container. It is a *terminator*: `<w:sectPr>` inside a
//! paragraph's `<w:pPr>` says "the section ends with this paragraph, and these
//! are its properties", and the one `<w:sectPr>` at the end of `<w:body>`
//! governs everything after the last of those. A document with one page setup
//! throughout has exactly one, at the end.
//!
//! Reading it as a container — properties that apply *forwards* — puts every
//! page setup one section too late, which in a document with a landscape
//! appendix means the appendix is portrait and the page before it is landscape.

use std::sync::Arc;

use crate::prop::{Border, BorderStyle};
use crate::units::{Twips, POINTS_PER_INCH};

/// `<w:type>` — where the new section starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionStart {
    #[default]
    NextPage,
    /// No break at all: the sections share a page. What a two-column pull-quote
    /// in the middle of a page is made of.
    Continuous,
    NextColumn,
    EvenPage,
    OddPage,
}

impl SectionStart {
    pub fn from_val(text: &str) -> Option<SectionStart> {
        Some(match text {
            "nextPage" => SectionStart::NextPage,
            "continuous" => SectionStart::Continuous,
            "nextColumn" => SectionStart::NextColumn,
            "evenPage" => SectionStart::EvenPage,
            "oddPage" => SectionStart::OddPage,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            SectionStart::NextPage => "nextPage",
            SectionStart::Continuous => "continuous",
            SectionStart::NextColumn => "nextColumn",
            SectionStart::EvenPage => "evenPage",
            SectionStart::OddPage => "oddPage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// `<w:pgSz>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize {
    /// **Already the printed width.** Word writes `w:w` and `w:h` swapped for a
    /// landscape page *and* writes `w:orient="landscape"` beside them, so a
    /// reader that swaps again on seeing the attribute turns the page back to
    /// portrait. The orientation is carried for the writer and for the page
    /// setup dialog; it is not an instruction to this struct.
    pub width: Twips,
    pub height: Twips,
    pub orientation: Orientation,
    /// `w:code` — the printer's paper-size number. Never interpreted, always
    /// written back, because it is what makes a document print on the right tray.
    pub code: Option<u32>,
}

impl Default for PageSize {
    /// US Letter, portrait — what Word starts a blank document at on this
    /// machine's locale. A document always states its own; this is for one we
    /// author.
    fn default() -> Self {
        PageSize {
            width: Twips::LETTER_WIDTH,
            height: Twips::LETTER_HEIGHT,
            orientation: Orientation::Portrait,
            code: None,
        }
    }
}

impl PageSize {
    pub const A4: PageSize = PageSize {
        width: Twips::A4_WIDTH,
        height: Twips::A4_HEIGHT,
        orientation: Orientation::Portrait,
        code: Some(9),
    };

    /// Turns the page, swapping the measurements as Word does.
    pub fn rotated(self) -> PageSize {
        PageSize {
            width: self.height,
            height: self.width,
            orientation: match self.orientation {
                Orientation::Portrait => Orientation::Landscape,
                Orientation::Landscape => Orientation::Portrait,
            },
            code: self.code,
        }
    }
}

/// `<w:pgMar>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMargins {
    /// May be negative, and legitimately is in documents whose header sits in
    /// the margin.
    pub top: Twips,
    pub bottom: Twips,
    pub start: Twips,
    pub end: Twips,
    /// Distance from the *page edge* to the header, not from the top margin.
    /// The body starts at `top` regardless, so a header taller than
    /// `top - header` pushes the body down.
    pub header: Twips,
    pub footer: Twips,
    /// Extra binding margin, added to `start` — or to the top, if
    /// `gutter_at_top`.
    pub gutter: Twips,
}

impl Default for PageMargins {
    fn default() -> Self {
        PageMargins {
            top: Twips::INCH,
            bottom: Twips::INCH,
            start: Twips::INCH,
            end: Twips::INCH,
            header: Twips(720),
            footer: Twips(720),
            gutter: Twips(0),
        }
    }
}

/// One column of a multi-column section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub width: Twips,
    /// Space after this column. The last column's is ignored.
    pub space: Twips,
}

/// `<w:cols>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columns {
    /// `w:num`. **May disagree with the number of `<w:col>` children**, and Word
    /// believes the children. [`Columns::count`] is the one that answers.
    pub num: u32,
    /// `w:space` — the gap when the columns are equal.
    pub space: Twips,
    /// `w:equalWidth`, whose default is *true*, so a `<w:cols w:num="3"/>` with
    /// no children is three equal columns and not an error.
    pub equal_width: bool,
    /// `w:sep` — draw a line between them.
    pub separator: bool,
    /// Present only when the columns are not equal.
    pub columns: Vec<Column>,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            num: 1,
            space: Twips(720),
            equal_width: true,
            separator: false,
            columns: Vec::new(),
        }
    }
}

impl Columns {
    /// How many columns there actually are.
    pub fn count(&self) -> usize {
        if self.columns.is_empty() {
            self.num.max(1) as usize
        } else {
            self.columns.len()
        }
    }

    /// The columns laid out across `text_width`, whether the file spelled them
    /// out or left them equal.
    pub fn resolve(&self, text_width: Twips) -> Vec<Column> {
        if !self.columns.is_empty() {
            return self.columns.clone();
        }
        let count = self.count() as i32;
        if count <= 1 {
            return vec![Column {
                width: text_width,
                space: Twips(0),
            }];
        }
        let gaps = self.space.0 * (count - 1);
        let each = (text_width.0 - gaps) / count;
        (0..count)
            .map(|index| Column {
                width: Twips(each),
                space: if index == count - 1 {
                    Twips(0)
                } else {
                    self.space
                },
            })
            .collect()
    }
}

/// Which of a section's three headers or footers a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeaderKind {
    /// Every page that is not covered by one of the other two.
    Default,
    /// Used only when `<w:titlePg>` is on.
    First,
    /// Used only when the document's settings turn on even/odd headers, which
    /// is a *document* setting rather than a section one — so a section can
    /// carry an even-page header that is never drawn.
    Even,
}

impl HeaderKind {
    pub fn from_val(text: &str) -> Option<HeaderKind> {
        Some(match text {
            "default" => HeaderKind::Default,
            "first" => HeaderKind::First,
            "even" => HeaderKind::Even,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            HeaderKind::Default => "default",
            HeaderKind::First => "first",
            HeaderKind::Even => "even",
        }
    }
}

/// Index into the document's list of header and footer bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeaderId(pub u32);

/// One `<w:headerReference>` or `<w:footerReference>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRef {
    pub kind: HeaderKind,
    pub body: HeaderId,
}

/// `<w:lnNumType>` — line numbers down the margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineNumbers {
    /// Print every nth number. 1 numbers every line.
    pub count_by: u32,
    pub start: u32,
    /// Distance from the text. `None` is Word's automatic placement.
    pub distance: Option<Twips>,
    pub restart: LineNumberRestart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineNumberRestart {
    #[default]
    NewPage,
    NewSection,
    Continuous,
}

/// `<w:pgNumType>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageNumbering {
    /// `w:start` — restart the page number here. Absent means carry on, which
    /// is a different thing from starting at 1.
    pub start: Option<u32>,
    /// `w:fmt` — a [`crate::numbering::NumFormat`] name, so a preface can be
    /// numbered in lowercase Roman.
    pub format: Option<Arc<str>>,
    /// `w:chapStyle` and `w:chapSep` — chapter-prefixed page numbers (`3-12`).
    pub chapter_style: Option<u8>,
    pub chapter_separator: Option<char>,
}

/// `<w:pgBorders>` — a border around the page rather than around anything on it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageBorders {
    pub top: Option<Border>,
    pub start: Option<Border>,
    pub bottom: Option<Border>,
    pub end: Option<Border>,
    /// `w:offsetFrom` — measured from the page edge or from the text.
    pub from_text: bool,
    /// `w:display` — all pages, first only, or all but first.
    pub display: PageBorderDisplay,
    /// `w:zOrder="back"` puts it behind the text.
    pub behind_text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageBorderDisplay {
    #[default]
    AllPages,
    FirstPage,
    NotFirstPage,
}

impl PageBorders {
    pub fn is_empty(&self) -> bool {
        [self.top, self.start, self.bottom, self.end]
            .iter()
            .all(|edge| edge.is_none_or(|b| !b.style.draws()))
    }
}

/// `<w:docGrid>` — the East Asian character grid.
///
/// Not decoration: with `w:type="lines"` the grid's `linePitch` *is* the line
/// height for the whole section, so a document that sets one and a renderer that
/// ignores it disagree about where every line sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocGrid {
    pub kind: DocGridKind,
    /// Line height, in twips.
    pub line_pitch: Twips,
    /// Character advance, in twips.
    pub char_space: Twips,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocGridKind {
    /// No grid. What every Latin-script document says.
    #[default]
    Default,
    /// Lines are snapped to the grid; characters are not.
    Lines,
    /// Both are.
    LinesAndChars,
    /// Characters are snapped and lines are not.
    SnapToChars,
}

impl DocGridKind {
    pub fn from_val(text: &str) -> Option<DocGridKind> {
        Some(match text {
            "default" => DocGridKind::Default,
            "lines" => DocGridKind::Lines,
            "linesAndChars" => DocGridKind::LinesAndChars,
            "snapToChars" => DocGridKind::SnapToChars,
            _ => return None,
        })
    }
}

/// Vertical alignment of a page's text within the text area — `<w:vAlign>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageVAlign {
    #[default]
    Top,
    Center,
    /// Stretches the paragraph spacing to fill the page. A title page uses it.
    Both,
    Bottom,
}

impl PageVAlign {
    pub fn from_val(text: &str) -> Option<PageVAlign> {
        Some(match text {
            "top" => PageVAlign::Top,
            "center" => PageVAlign::Center,
            "both" => PageVAlign::Both,
            "bottom" => PageVAlign::Bottom,
            _ => return None,
        })
    }
}

/// `<w:sectPr>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SectionProps {
    pub start: SectionStart,
    pub page: PageSize,
    pub margins: PageMargins,
    pub columns: Columns,
    /// Every header and footer this section references. A section may name up to
    /// three of each, and naming none means it inherits the previous section's —
    /// which is what "Link to Previous" is, and why an empty list is not the
    /// same as a reference to an empty header.
    pub headers: Vec<HeaderRef>,
    pub footers: Vec<HeaderRef>,
    /// `<w:titlePg>` — the first page uses the `first` header and footer.
    pub title_page: bool,
    pub v_align: PageVAlign,
    pub line_numbers: Option<LineNumbers>,
    pub page_numbering: PageNumbering,
    pub borders: Option<Box<PageBorders>>,
    /// Right-to-left section: the columns run the other way, and so does the
    /// gutter.
    pub bidi: bool,
    pub rtl_gutter: bool,
    pub doc_grid: Option<DocGrid>,
    /// `<w:paperSrc>`, `<w:footnotePr>`, `<w:endnotePr>` and the rest are not
    /// modelled and ride through the writer untouched.
    pub gutter_at_top: bool,
}

impl SectionProps {
    pub fn new() -> SectionProps {
        SectionProps::default()
    }

    /// The width available to text: the page less its side margins and gutter.
    pub fn text_width(&self) -> Twips {
        let gutter = if self.gutter_at_top {
            Twips(0)
        } else {
            self.margins.gutter
        };
        Twips(self.page.width.0 - self.margins.start.0 - self.margins.end.0 - gutter.0)
    }

    /// The height available to text.
    ///
    /// Headers and footers are *not* subtracted: they sit in the margins, and a
    /// header only pushes the body down when it grows past the space between the
    /// page edge and the top margin. That is a layout decision rather than a
    /// page-setup one.
    pub fn text_height(&self) -> Twips {
        let gutter = if self.gutter_at_top {
            self.margins.gutter
        } else {
            Twips(0)
        };
        Twips(self.page.height.0 - self.margins.top.0 - self.margins.bottom.0 - gutter.0)
    }

    pub fn header(&self, kind: HeaderKind) -> Option<HeaderId> {
        self.headers
            .iter()
            .find(|reference| reference.kind == kind)
            .map(|reference| reference.body)
    }

    pub fn footer(&self, kind: HeaderKind) -> Option<HeaderId> {
        self.footers
            .iter()
            .find(|reference| reference.kind == kind)
            .map(|reference| reference.body)
    }

    /// Which header a page uses, given its one-based number and whether the
    /// document distinguishes odd from even pages.
    ///
    /// Falls back the way Word does: a missing `first` header on a title page
    /// means the page has *no* header, not the default one. The reference being
    /// absent is the instruction.
    pub fn header_for_page(&self, page: u32, even_and_odd: bool) -> Option<HeaderKind> {
        if self.title_page && page == 1 {
            return Some(HeaderKind::First);
        }
        if even_and_odd && page.is_multiple_of(2) {
            return Some(HeaderKind::Even);
        }
        Some(HeaderKind::Default)
    }
}

/// A whole page's geometry in points, for a layout engine that would rather not
/// think in twips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageBox {
    pub width: f64,
    pub height: f64,
    pub top: f64,
    pub bottom: f64,
    pub start: f64,
    pub end: f64,
}

impl PageBox {
    pub fn of(section: &SectionProps) -> PageBox {
        PageBox {
            width: section.page.width.points(),
            height: section.page.height.points(),
            top: section.margins.top.points(),
            bottom: section.margins.bottom.points(),
            start: section.margins.start.points(),
            end: section.margins.end.points(),
        }
    }

    pub fn text_width(&self) -> f64 {
        self.width - self.start - self.end
    }

    pub fn text_height(&self) -> f64 {
        self.height - self.top - self.bottom
    }

    /// The page in inches, which is the unit a page setup dialog talks in.
    pub fn inches(&self) -> (f64, f64) {
        (self.width / POINTS_PER_INCH, self.height / POINTS_PER_INCH)
    }
}

/// A drawn page border is one that has a style and a width.
pub fn border_draws(border: Option<Border>) -> bool {
    border.is_some_and(|border| {
        border.style.draws()
            && border.style != BorderStyle::None
            && border.size.is_none_or(|s| s.0 > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_landscape_page_is_already_the_right_way_round() {
        // Word writes the swapped measurements *and* the attribute. Swapping
        // again on seeing `orient` turns the page back to portrait, and the
        // document prints off the edge of the paper.
        let landscape = PageSize::default().rotated();
        assert_eq!(landscape.orientation, Orientation::Landscape);
        assert_eq!(landscape.width, Twips::LETTER_HEIGHT);
        assert_eq!(landscape.height, Twips::LETTER_WIDTH);
        assert!(landscape.width > landscape.height);
    }

    #[test]
    fn the_text_area_is_the_page_less_its_margins() {
        let section = SectionProps::new();
        // 8.5in less two 1in margins.
        assert_eq!(section.text_width(), Twips(12240 - 2880));
        assert_eq!(section.text_height(), Twips(15840 - 2880));
        assert_eq!(PageBox::of(&section).text_width(), 6.5 * 72.0);
    }

    #[test]
    fn a_gutter_comes_off_the_side_unless_it_is_at_the_top() {
        let mut section = SectionProps::new();
        section.margins.gutter = Twips(720);
        assert_eq!(section.text_width(), Twips(12240 - 2880 - 720));
        assert_eq!(section.text_height(), Twips(15840 - 2880));

        section.gutter_at_top = true;
        assert_eq!(section.text_width(), Twips(12240 - 2880));
        assert_eq!(section.text_height(), Twips(15840 - 2880 - 720));
    }

    #[test]
    fn three_equal_columns_need_no_col_elements() {
        // `<w:cols w:num="3"/>` with no children is legal and common, because
        // equalWidth defaults to true.
        let columns = Columns {
            num: 3,
            space: Twips(720),
            ..Columns::default()
        };
        assert_eq!(columns.count(), 3);
        let laid = columns.resolve(Twips(9360));
        assert_eq!(laid.len(), 3);
        assert_eq!(laid[0].width, Twips((9360 - 1440) / 3));
        assert_eq!(laid[0].space, Twips(720));
        assert_eq!(laid[2].space, Twips(0), "nothing follows the last column");
    }

    #[test]
    fn the_col_elements_win_over_the_count_beside_them() {
        // Word believes the children when `w:num` disagrees, and files where
        // they disagree exist.
        let columns = Columns {
            num: 5,
            columns: vec![
                Column {
                    width: Twips(4000),
                    space: Twips(360),
                },
                Column {
                    width: Twips(5000),
                    space: Twips(0),
                },
            ],
            ..Columns::default()
        };
        assert_eq!(columns.count(), 2);
        assert_eq!(columns.resolve(Twips(9360)).len(), 2);
    }

    #[test]
    fn a_title_page_takes_the_first_header_and_the_rest_take_the_default() {
        let mut section = SectionProps::new();
        section.title_page = true;
        assert_eq!(section.header_for_page(1, false), Some(HeaderKind::First));
        assert_eq!(section.header_for_page(2, false), Some(HeaderKind::Default));
        assert_eq!(section.header_for_page(2, true), Some(HeaderKind::Even));
        assert_eq!(section.header_for_page(3, true), Some(HeaderKind::Default));

        section.title_page = false;
        assert_eq!(section.header_for_page(1, false), Some(HeaderKind::Default));
    }

    #[test]
    fn a_section_naming_no_header_is_not_a_section_with_an_empty_one() {
        // "Link to Previous" is spelled by leaving the reference out, so an
        // empty list has to stay distinguishable from a reference to a header
        // with nothing in it.
        let section = SectionProps::new();
        assert_eq!(section.header(HeaderKind::Default), None);

        let linked = SectionProps {
            headers: vec![HeaderRef {
                kind: HeaderKind::Default,
                body: HeaderId(3),
            }],
            ..SectionProps::new()
        };
        assert_eq!(linked.header(HeaderKind::Default), Some(HeaderId(3)));
        assert_eq!(linked.footer(HeaderKind::Default), None);
    }
}
