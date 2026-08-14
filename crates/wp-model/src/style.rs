//! Styles, and the layers a paragraph's appearance is assembled from.
//!
//! Nothing in a Word document carries its own complete formatting. A run's size
//! comes from up to five places at once, and the answer to "how big is this
//! text?" is a walk:
//!
//! ```text
//! document defaults
//!   -> the paragraph style, and everything it is basedOn, root first
//!     -> the numbering level, if the paragraph is in a list
//!       -> the character style on the run, and everything it is basedOn
//!         -> the paragraph's own <w:pPr>
//!           -> the run's own <w:rPr>
//! ```
//!
//! Two things about that walk are easy to get wrong and expensive to find:
//!
//! **A paragraph with no `<w:pStyle>` is not unstyled.** It takes the style
//! marked `w:default="1"` — Normal, in every document Word writes. Skipping that
//! layer loses the document's body font, its line spacing and its spacing after,
//! and the result looks like a rendering bug rather than a resolution one.
//!
//! **`basedOn` is a chain, and the root applies first.** Walking it from the
//! named style outward and taking the first value found gives the right answer
//! for ordinary properties and the wrong one for every toggle, because XOR is
//! order-independent only in its result, not in which values participate.
//!
//! A document may also contain a `basedOn` cycle. Word tolerates one; so does
//! this, by visiting no style twice.

use std::collections::HashMap;
use std::sync::Arc;

use crate::prop::{Layer, ParaProps, RunProps};

/// Index into [`StyleTable`].
///
/// The file names styles by string (`w:styleId="Heading1"`), and that string is
/// the durable identity — it is what `<w:pStyle>` holds and what a writer must
/// put back. The index exists so a paragraph carries four bytes rather than an
/// `Arc<str>`, and so resolution is an array lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleId(pub u32);

/// What a style may be applied to. A `w:type` mismatch is not a detail: a
/// character style named in `<w:pStyle>` is ignored by Word entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    /// A style that exists only to hold a `<w:numPr>` — the mechanism behind
    /// "List Bullet" and friends. Rare, and read so that it is not lost.
    Numbering,
}

impl StyleKind {
    pub fn from_val(text: &str) -> Option<StyleKind> {
        Some(match text {
            "paragraph" => StyleKind::Paragraph,
            "character" => StyleKind::Character,
            "table" => StyleKind::Table,
            "numbering" => StyleKind::Numbering,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "paragraph",
            StyleKind::Character => "character",
            StyleKind::Table => "table",
            StyleKind::Numbering => "numbering",
        }
    }
}

/// One `<w:style>`.
#[derive(Debug, Clone)]
pub struct Style {
    /// `w:styleId` — the identity, and what every reference in the document
    /// holds. Never derived from the name: two styles may share a name.
    pub id: Arc<str>,
    pub kind: StyleKind,
    /// `<w:name>` — what the UI calls it, which for the built-ins is a
    /// lowercase form (`heading 1`) that Word title-cases on the way to the
    /// screen.
    pub name: Option<Arc<str>>,
    pub based_on: Option<StyleId>,
    /// `<w:next>` — the style the *following* paragraph gets when Enter is
    /// pressed at the end of this one. A heading's next is Normal, which is why
    /// typing after a heading is not a heading.
    pub next: Option<StyleId>,
    /// `<w:link>` — the paragraph style paired with this character style, or the
    /// other way round. Word's UI shows the pair as one "linked" style.
    pub link: Option<StyleId>,
    pub para: ParaProps,
    pub run: RunProps,
    /// `w:default="1"` — the style used when nothing names one. Exactly one per
    /// kind, in a well-formed document.
    pub default: bool,
    /// `<w:qFormat>` — offered in the styles gallery.
    pub quick: bool,
    pub semi_hidden: bool,
    pub unhide_when_used: bool,
    /// `<w:uiPriority>` — the gallery's sort order.
    pub priority: Option<i32>,
    /// `<w:autoRedefine>`, `<w:locked>` and the rest are not modelled. They ride
    /// through the writer untouched; nothing here interprets them.
    pub custom: bool,
}

impl Style {
    pub fn new(id: impl Into<Arc<str>>, kind: StyleKind) -> Style {
        Style {
            id: id.into(),
            kind,
            name: None,
            based_on: None,
            next: None,
            link: None,
            para: ParaProps::default(),
            run: RunProps::default(),
            default: false,
            quick: false,
            semi_hidden: false,
            unhide_when_used: false,
            priority: None,
            custom: false,
        }
    }
}

/// `<w:docDefaults>` — the bottom layer, and the only one that is never absent.
#[derive(Debug, Clone, Default)]
pub struct DocDefaults {
    pub para: ParaProps,
    pub run: RunProps,
}

/// Every style in the document, addressable by index and by `w:styleId`.
///
/// Append-only, like a workbook's style tables and for the same reason: a
/// [`StyleId`] is an index, and inserting one in the middle would re-letter
/// every reference past it.
#[derive(Debug, Clone, Default)]
pub struct StyleTable {
    styles: Vec<Style>,
    by_id: HashMap<Arc<str>, StyleId>,
    defaults: DocDefaults,
}

/// How deep a `basedOn` chain may go before it is treated as broken.
///
/// Word's own limit is smaller. The point is not the number; it is that a
/// document with a cycle in it must render rather than hang.
const MAX_CHAIN: usize = 32;

impl StyleTable {
    pub fn new() -> StyleTable {
        StyleTable::default()
    }

    pub fn doc_defaults(&self) -> &DocDefaults {
        &self.defaults
    }

    pub fn set_doc_defaults(&mut self, defaults: DocDefaults) {
        self.defaults = defaults;
    }

    /// Adds a style, or replaces one already carrying the same `w:styleId`.
    ///
    /// Replacing keeps the index, because a document being re-read must not
    /// invalidate ids handed out from the previous read.
    pub fn insert(&mut self, style: Style) -> StyleId {
        if let Some(&id) = self.by_id.get(&style.id) {
            self.styles[id.0 as usize] = style;
            return id;
        }
        let id = StyleId(self.styles.len() as u32);
        self.by_id.insert(style.id.clone(), id);
        self.styles.push(style);
        id
    }

    /// The id for a `w:styleId`, creating a placeholder if the document refers
    /// to a style it never defines.
    ///
    /// Word does this too — a `<w:pStyle w:val="Quote"/>` with no matching
    /// `<w:style>` is not an error, it simply resolves to nothing. Making it an
    /// id anyway is what lets the writer put the reference back exactly as it
    /// found it, rather than silently unstyling the paragraph on save.
    pub fn intern(&mut self, style_id: &str, kind: StyleKind) -> StyleId {
        if let Some(&id) = self.by_id.get(style_id) {
            return id;
        }
        self.insert(Style::new(style_id, kind))
    }

    pub fn lookup(&self, style_id: &str) -> Option<StyleId> {
        self.by_id.get(style_id).copied()
    }

    pub fn get(&self, id: StyleId) -> Option<&Style> {
        self.styles.get(id.0 as usize)
    }

    pub fn get_mut(&mut self, id: StyleId) -> Option<&mut Style> {
        self.styles.get_mut(id.0 as usize)
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (StyleId, &Style)> {
        self.styles
            .iter()
            .enumerate()
            .map(|(i, style)| (StyleId(i as u32), style))
    }

    /// The style a paragraph gets when it names none — `w:default="1"` on a
    /// style of the given kind.
    ///
    /// Falling back to nothing here is the bug that renders a whole document in
    /// the wrong font.
    pub fn default_style(&self, kind: StyleKind) -> Option<StyleId> {
        self.iter()
            .find(|(_, style)| style.default && style.kind == kind)
            .map(|(id, _)| id)
    }

    /// The `basedOn` chain from the root down to `id`, with cycles broken.
    ///
    /// Root first, because that is the order the layers apply in and because
    /// which toggle values participate depends on it.
    pub fn chain(&self, id: StyleId) -> Vec<StyleId> {
        let mut chain = Vec::new();
        let mut seen = Vec::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            if seen.contains(&current) || seen.len() >= MAX_CHAIN {
                break;
            }
            seen.push(current);
            chain.push(current);
            cursor = self.get(current).and_then(|style| style.based_on);
        }
        chain.reverse();
        chain
    }

    /// Lays a whole `basedOn` chain onto an accumulator, root first.
    ///
    /// `styles` selects which half of each style contributes, so the same walk
    /// serves paragraphs and runs.
    fn layer_chain(&self, into: &mut Layers, id: StyleId) {
        for step in self.chain(id) {
            if let Some(style) = self.get(step) {
                into.para.layer(&style.para, Layer::Style);
                into.run.layer(&style.run, Layer::Style);
            }
        }
    }

    /// Resolves a paragraph's formatting, without its runs.
    ///
    /// `direct` is the paragraph's own `<w:pPr>`; the style it names is read out
    /// of it. `numbering` is the list level's own `<w:pPr>`/`<w:rPr>` when the
    /// paragraph is in a list, which the caller looks up because this crate's
    /// numbering lives in its own table.
    pub fn resolve_paragraph(&self, direct: &ParaProps, numbering: Option<&Layers>) -> Layers {
        let mut layers = Layers {
            para: self.defaults.para.clone(),
            run: self.defaults.run.clone(),
        };

        // A paragraph naming no style still gets the default one.
        let style = direct
            .style
            .or_else(|| self.default_style(StyleKind::Paragraph));
        if let Some(style) = style {
            self.layer_chain(&mut layers, style);
        }

        // The list level sits above the paragraph style and below the
        // paragraph's own properties. ECMA's §17.7.2 ordering puts numbering
        // *below* paragraph styles; Word plainly does not, or every bulleted
        // list would sit at its style's indent rather than at the list's. This
        // is a deliberate, stated divergence from the text of the spec in favour
        // of the behaviour, and it is what `a_list_level_moves_a_paragraph_its_
        // style_did_not` pins.
        if let Some(numbering) = numbering {
            layers.para.layer(&numbering.para, Layer::Style);
        }

        layers.para.layer(direct, Layer::Direct);
        layers
    }

    /// Resolves one run inside an already-resolved paragraph.
    ///
    /// Takes the paragraph's layers rather than recomputing them, because a
    /// paragraph has many runs and the walk above is the expensive half.
    pub fn resolve_run(&self, paragraph: &Layers, direct: &RunProps) -> RunProps {
        let mut run = paragraph.run.clone();
        if let Some(style) = direct.style {
            let mut chain = Layers {
                para: ParaProps::default(),
                run,
            };
            self.layer_chain(&mut chain, style);
            run = chain.run;
        }
        run.layer(direct, Layer::Direct);
        run.style = direct.style;
        run
    }
}

/// A paragraph's resolved formatting: everything the layers agreed on.
///
/// Still `Option`-shaped, because "nobody said" survives resolution and is a
/// real answer — a renderer's fallback is not the same as a document's choice,
/// and a writer must not author a value the document never stated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
    pub para: ParaProps,
    pub run: RunProps,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::prop::{Indent, Toggle};
    use crate::units::{HalfPoint, Twips};

    /// Normal (11pt, Calibri) with Heading1 based on it (16pt, bold), plus a
    /// Strong character style that is also bold.
    fn table() -> (StyleTable, StyleId, StyleId, StyleId) {
        let mut styles = StyleTable::new();
        styles.set_doc_defaults(DocDefaults {
            run: RunProps {
                size: Some(HalfPoint(20)),
                ..RunProps::default()
            },
            ..DocDefaults::default()
        });

        let mut normal = Style::new("Normal", StyleKind::Paragraph);
        normal.default = true;
        normal.run.size = Some(HalfPoint(22));
        normal.run.fonts.ascii = Some("Calibri".into());
        let normal = styles.insert(normal);

        let mut heading = Style::new("Heading1", StyleKind::Paragraph);
        heading.based_on = Some(normal);
        heading.run.size = Some(HalfPoint(32));
        heading.run.toggles.set(Toggle::Bold, true);
        heading.run.color = Some(Color::Rgb([0x2F, 0x54, 0x96]));
        heading.para.outline_level = Some(0);
        let heading = styles.insert(heading);

        let mut strong = Style::new("Strong", StyleKind::Character);
        strong.run.toggles.set(Toggle::Bold, true);
        let strong = styles.insert(strong);

        (styles, normal, heading, strong)
    }

    #[test]
    fn a_paragraph_naming_no_style_still_gets_the_default_one() {
        let (styles, _, _, _) = table();
        let resolved = styles.resolve_paragraph(&ParaProps::default(), None);
        // Not the doc default's 10pt: Normal is a layer, and skipping it loses
        // the document's body font.
        assert_eq!(resolved.run.size, Some(HalfPoint(22)));
        assert_eq!(resolved.run.fonts.ascii.as_deref(), Some("Calibri"));
    }

    #[test]
    fn based_on_applies_from_the_root_down() {
        let (styles, normal, heading, _) = table();
        assert_eq!(styles.chain(heading), vec![normal, heading]);

        let resolved = styles.resolve_paragraph(
            &ParaProps {
                style: Some(heading),
                ..ParaProps::default()
            },
            None,
        );
        // Heading1's own size wins over Normal's, and Normal's font survives
        // because Heading1 says nothing about it.
        assert_eq!(resolved.run.size, Some(HalfPoint(32)));
        assert_eq!(resolved.run.fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(resolved.para.outline_level, Some(0));
        assert!(resolved.run.bold());
    }

    #[test]
    fn strong_on_a_heading_un_bolds_it() {
        // The end-to-end version of the toggle rule: two bold styles, and the
        // text comes out plain. This is Word, not a bug.
        let (styles, _, heading, strong) = table();
        let paragraph = styles.resolve_paragraph(
            &ParaProps {
                style: Some(heading),
                ..ParaProps::default()
            },
            None,
        );
        let run = styles.resolve_run(
            &paragraph,
            &RunProps {
                style: Some(strong),
                ..RunProps::default()
            },
        );
        assert!(!run.bold());
        assert_eq!(run.size, Some(HalfPoint(32)), "the heading's size survives");
    }

    #[test]
    fn a_run_of_its_own_overrides_the_style_that_bolded_it() {
        let (styles, _, heading, _) = table();
        let paragraph = styles.resolve_paragraph(
            &ParaProps {
                style: Some(heading),
                ..ParaProps::default()
            },
            None,
        );
        let mut direct = RunProps::default();
        direct.toggles.set(Toggle::Bold, false);
        let run = styles.resolve_run(&paragraph, &direct);
        assert!(!run.bold(), "direct formatting is absolute");
    }

    #[test]
    fn a_list_level_moves_a_paragraph_its_style_did_not() {
        let (mut styles, _, _, _) = table();
        let mut body = Style::new("ListParagraph", StyleKind::Paragraph);
        body.para.indent = Indent {
            start: Some(Twips(0)),
            ..Indent::default()
        };
        let body = styles.insert(body);

        let level = Layers {
            para: ParaProps {
                indent: Indent {
                    start: Some(Twips(720)),
                    hanging: Some(Twips(360)),
                    ..Indent::default()
                },
                ..ParaProps::default()
            },
            run: RunProps::default(),
        };
        let resolved = styles.resolve_paragraph(
            &ParaProps {
                style: Some(body),
                ..ParaProps::default()
            },
            Some(&level),
        );
        assert_eq!(resolved.para.indent.start, Some(Twips(720)));
        assert_eq!(resolved.para.indent.first_line_offset(), Twips(-360));

        // And the paragraph's own indent still beats the list's.
        let overridden = styles.resolve_paragraph(
            &ParaProps {
                style: Some(body),
                indent: Indent {
                    start: Some(Twips(1440)),
                    ..Indent::default()
                },
                ..ParaProps::default()
            },
            Some(&level),
        );
        assert_eq!(overridden.para.indent.start, Some(Twips(1440)));
    }

    #[test]
    fn a_based_on_cycle_resolves_rather_than_hanging() {
        let mut styles = StyleTable::new();
        let a = styles.insert(Style::new("A", StyleKind::Paragraph));
        let b = styles.insert(Style::new("B", StyleKind::Paragraph));
        styles.get_mut(a).unwrap().based_on = Some(b);
        styles.get_mut(b).unwrap().based_on = Some(a);

        let chain = styles.chain(a);
        assert_eq!(chain.len(), 2);
        assert!(chain.contains(&a) && chain.contains(&b));
        let _ = styles.resolve_paragraph(
            &ParaProps {
                style: Some(a),
                ..ParaProps::default()
            },
            None,
        );
    }

    #[test]
    fn a_reference_to_a_style_that_does_not_exist_keeps_its_name() {
        // Word does not treat this as an error, and a writer has to put the
        // reference back — so it becomes a real id with nothing in it rather
        // than being dropped.
        let mut styles = StyleTable::new();
        let quote = styles.intern("Quote", StyleKind::Paragraph);
        assert_eq!(styles.get(quote).unwrap().id.as_ref(), "Quote");
        assert!(styles.get(quote).unwrap().para.is_empty());
        assert_eq!(styles.intern("Quote", StyleKind::Paragraph), quote);
    }

    #[test]
    fn re_reading_a_style_keeps_its_index() {
        // A document read twice must not invalidate ids, or an undo stack built
        // before the re-read points at the wrong styles.
        let (mut styles, normal, _, _) = table();
        let mut replacement = Style::new("Normal", StyleKind::Paragraph);
        replacement.run.size = Some(HalfPoint(24));
        assert_eq!(styles.insert(replacement), normal);
        assert_eq!(styles.get(normal).unwrap().run.size, Some(HalfPoint(24)));
    }

    #[test]
    fn a_character_style_chain_applies_under_the_runs_own_properties() {
        let mut styles = StyleTable::new();
        let mut base = Style::new("Base", StyleKind::Character);
        base.run.size = Some(HalfPoint(18));
        base.run.color = Some(Color::BLACK);
        let base = styles.insert(base);

        let mut emphasis = Style::new("Emphasis", StyleKind::Character);
        emphasis.based_on = Some(base);
        emphasis.run.toggles.set(Toggle::Italic, true);
        let emphasis = styles.insert(emphasis);

        let paragraph = Layers::default();
        let run = styles.resolve_run(
            &paragraph,
            &RunProps {
                style: Some(emphasis),
                color: Some(Color::Rgb([0xFF, 0, 0])),
                ..RunProps::default()
            },
        );
        assert_eq!(run.size, Some(HalfPoint(18)), "from the chain's root");
        assert!(run.italic());
        assert_eq!(run.color, Some(Color::Rgb([0xFF, 0, 0])), "the run's own");
        assert_eq!(
            run.style,
            Some(emphasis),
            "the reference is kept for the writer"
        );
    }
}
