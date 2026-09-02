//! Automatic styles, and the minting of one for formatting a person applied by
//! hand.
//!
//! **ODF has nowhere else to put direct formatting.** A run made bold in the
//! application is not bold in the file: it names a `<style:style
//! style:family="text">` that is, and that style lives in
//! `<office:automatic-styles>` at the top of the part. So a writer for this
//! format cannot simply write the properties where the run is — it has to mint
//! a style, put it in a stylesheet that stands *before* the body, and name it
//! from the run. Which means the stylesheet cannot be written until the body is
//! known, and is spliced back in afterwards; `write::mod` does that.
//!
//! **A minted style's parent is a common style, never another automatic one.**
//! ODF 1.4 part 3 §16.4 has automatic styles standing outside the hierarchy a
//! person navigates, and a producer that chains one to another is asking every
//! consumer to resolve a name in a pool it does not keep. So where the run
//! already names an automatic style, that style's *own* properties are copied
//! into the new one and its parent is inherited with them. The result says the
//! same thing in one level that the file said in two.
//!
//! Nothing here is minted for a paragraph nobody edited: an unchanged paragraph
//! is copied byte for byte and never reaches this module, so a save with no
//! edits adds no styles at all.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use wp_model::prop::{
    Border, BorderStyle, Justify, Layer, LineSpacing, ParaBorders, ParaProps, RunProps, Shading,
    TabKind, TabLeader, Toggle, UnderlineKind, VertAlign,
};
use wp_model::style::{StyleId, StyleTable};
use wp_model::units::Twips;

use super::splice::escape_attr;

/// The stylesheet a save adds to, and everything it needs to name a style
/// without colliding with one the file already has.
pub(crate) struct Automatic {
    /// The model's own table, not the reader's: what a style id stands for is
    /// what the document says it stands for. Owned because the reading context
    /// it was copied from goes on being written to while the body is walked.
    styles: StyleTable,
    /// Which names stand for styles the file wrote in `<office:automatic-styles>`
    /// rather than in `<office:styles>`.
    automatic: HashSet<String>,
    /// `style:master-page-name` and `style:list-style-name` by the style that
    /// carries them, so that a paragraph whose style starts a section or numbers
    /// itself keeps doing both through an edit.
    master_of_style: HashMap<String, String>,
    list_of_style: HashMap<String, String>,
    /// Every name already spoken for, minted ones included.
    taken: HashSet<String>,
    /// Minted styles in the order they were asked for, and the name each set of
    /// properties was given, so that ten bold runs mint one style.
    minted: Vec<String>,
    named: HashMap<String, String>,
    next: u32,
}

impl Automatic {
    pub(crate) fn new(styles: StyleTable, read: &crate::styles::Styles) -> Automatic {
        let mut taken: HashSet<String> = read.by_name.keys().cloned().collect();
        taken.extend(read.tables.keys().cloned());
        taken.extend(read.rows.keys().cloned());
        taken.extend(read.columns.keys().cloned());
        taken.extend(read.cells.keys().cloned());
        taken.extend(read.graphics.keys().cloned());
        Automatic {
            styles,
            automatic: read.automatic.clone(),
            master_of_style: read.master_of_style.clone(),
            list_of_style: read.list_of_style.clone(),
            taken,
            minted: Vec::new(),
            named: HashMap::new(),
            next: 1,
        }
    }

    /// The `<style:style>` elements minted this save, ready to splice into
    /// `<office:automatic-styles>`.
    pub(crate) fn stylesheet(&self) -> String {
        self.minted.concat()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.minted.is_empty()
    }

    /// The name a style id is written as.
    pub(crate) fn name_of(&self, id: StyleId) -> Option<&str> {
        self.styles.get(id).map(|style| style.id.as_ref())
    }

    /// What a paragraph's `text:style-name` should say.
    ///
    /// The style it already names where there is no direct formatting to place,
    /// and a minted one where there is.
    pub(crate) fn paragraph_style(&mut self, props: &ParaProps) -> Option<String> {
        let base = props
            .style
            .and_then(|id| self.name_of(id))
            .map(str::to_owned);
        let direct = paragraph_residue(props, base.as_deref(), &self.master_of_style);
        if direct == ParaProps::default() {
            return base;
        }
        let (parent, mut para, run) = self.foundation(base.as_deref());
        para.layer(&direct, Layer::Direct);
        let mut body = String::new();
        paragraph_properties(&mut body, &para);
        text_properties(&mut body, &run);
        Some(self.mint("paragraph", 'P', parent.as_deref(), base.as_deref(), &body))
    }

    /// What a run's `text:style-name` should say, or nothing where the run has
    /// no formatting of its own and needs no `<text:span>` at all.
    pub(crate) fn run_style(&mut self, props: &RunProps) -> Option<String> {
        let base = props
            .style
            .and_then(|id| self.name_of(id))
            .map(str::to_owned);
        let direct = RunProps {
            style: None,
            ..props.clone()
        };
        if direct == RunProps::default() {
            return base;
        }
        let (parent, _, mut run) = self.foundation(base.as_deref());
        run.layer(&direct, Layer::Direct);
        let mut body = String::new();
        text_properties(&mut body, &run);
        Some(self.mint("text", 'T', parent.as_deref(), None, &body))
    }

    /// The style a minted one stands on, and the properties it starts from.
    ///
    /// A common style is stood on by name and nothing is copied out of it. An
    /// automatic style is not something to stand on — see the note at the top —
    /// so its own properties are copied and its parent is taken over.
    fn foundation(&self, base: Option<&str>) -> (Option<String>, ParaProps, RunProps) {
        let Some(base) = base else {
            return (None, ParaProps::default(), RunProps::default());
        };
        if !self.automatic.contains(base) {
            return (
                Some(base.to_owned()),
                ParaProps::default(),
                RunProps::default(),
            );
        }
        let Some(style) = self
            .styles
            .iter()
            .map(|(_, s)| s)
            .find(|s| s.id.as_ref() == base)
        else {
            return (None, ParaProps::default(), RunProps::default());
        };
        let parent = style
            .based_on
            .and_then(|id| self.name_of(id))
            .map(str::to_owned);
        (parent, style.para.clone(), style.run.clone())
    }

    /// Adds a style with the given properties, or hands back the name of one
    /// minted earlier that says exactly the same thing.
    fn mint(
        &mut self,
        family: &str,
        letter: char,
        parent: Option<&str>,
        carry: Option<&str>,
        body: &str,
    ) -> String {
        let master = carry
            .and_then(|name| self.master_of_style.get(name))
            .cloned();
        let list = carry.and_then(|name| self.list_of_style.get(name)).cloned();
        let key = format!(
            "{family}\u{1}{}\u{1}{}\u{1}{}\u{1}{body}",
            parent.unwrap_or_default(),
            master.as_deref().unwrap_or_default(),
            list.as_deref().unwrap_or_default()
        );
        if let Some(name) = self.named.get(&key) {
            return name.clone();
        }
        let name = loop {
            let candidate = format!("{letter}{}", self.next);
            self.next += 1;
            if self.taken.insert(candidate.clone()) {
                break candidate;
            }
        };
        let mut element = String::new();
        let _ = write!(
            element,
            r#"<style:style style:name="{}" style:family="{family}""#,
            escape_attr(&name)
        );
        if let Some(parent) = parent {
            let _ = write!(
                element,
                r#" style:parent-style-name="{}""#,
                escape_attr(parent)
            );
        }
        if let Some(master) = &master {
            let _ = write!(
                element,
                r#" style:master-page-name="{}""#,
                escape_attr(master)
            );
        }
        if let Some(list) = &list {
            let _ = write!(element, r#" style:list-style-name="{}""#, escape_attr(list));
        }
        element.push('>');
        element.push_str(body);
        element.push_str("</style:style>");
        self.minted.push(element);
        self.named.insert(key, name.clone());
        name
    }
}

/// What a paragraph states beyond the style it names.
///
/// Three of its fields are structure rather than formatting and are written
/// elsewhere: the style itself, the numbering — which is the `<text:list>` the
/// paragraph sits inside and is spliced through untouched — and the outline
/// level, which is an attribute of `<text:h>`. A fourth is a trap: the reader
/// gives a paragraph `page_break_before` when its style names a master page,
/// because that is how ODF spells a section break, and minting a style stating
/// `fo:break-before` for it would turn the section break into a page break. The
/// minted style carries the master page instead.
fn paragraph_residue(
    props: &ParaProps,
    base: Option<&str>,
    master_of_style: &HashMap<String, String>,
) -> ParaProps {
    let from_master = base
        .and_then(|name| master_of_style.get(name))
        .is_some_and(|master| !master.is_empty());
    ParaProps {
        style: None,
        numbering: None,
        outline_level: None,
        page_break_before: match from_master {
            true => None,
            false => props.page_break_before,
        },
        ..props.clone()
    }
}

/// `<style:text-properties>`, in the vocabulary `props.rs` reads back.
pub(crate) fn text_properties(out: &mut String, run: &RunProps) {
    let mut attrs = String::new();
    if let Some(family) = &run.fonts.ascii {
        let _ = write!(attrs, r#" fo:font-family="{}""#, escape_attr(family));
    }
    if let Some(size) = run.size {
        let _ = write!(attrs, r#" fo:font-size="{}""#, points(size.0 as f64 / 2.0));
    }
    if let Some(bold) = run.toggles.get(Toggle::Bold) {
        let _ = write!(attrs, r#" fo:font-weight="{}""#, weight(bold));
    }
    if let Some(italic) = run.toggles.get(Toggle::Italic) {
        let _ = write!(attrs, r#" fo:font-style="{}""#, slant(italic));
    }
    if let Some(caps) = run.toggles.get(Toggle::Caps) {
        let value = if caps { "uppercase" } else { "none" };
        let _ = write!(attrs, r#" fo:text-transform="{value}""#);
    }
    if let Some(small) = run.toggles.get(Toggle::SmallCaps) {
        let value = if small { "small-caps" } else { "normal" };
        let _ = write!(attrs, r#" fo:font-variant="{value}""#);
    }
    // One attribute pair says both of the model's two, so they are written
    // together or not at all: a run that is struck through once and a run that
    // is struck through twice differ only in `style:text-line-through-type`.
    let struck = run.toggles.get(Toggle::Strike);
    let twice = run.toggles.get(Toggle::DoubleStrike);
    if struck.is_some() || twice.is_some() {
        let on = struck == Some(true) || twice == Some(true);
        let style = if on { "solid" } else { "none" };
        let _ = write!(attrs, r#" style:text-line-through-style="{style}""#);
        if twice == Some(true) {
            attrs.push_str(r#" style:text-line-through-type="double""#);
        }
    }
    if let Some(hidden) = run.toggles.get(Toggle::Vanish) {
        let value = if hidden { "none" } else { "true" };
        let _ = write!(attrs, r#" text:display="{value}""#);
    }
    if let Some(underline) = run.underline {
        let (style, kind, width) = underline_words(underline.kind);
        let _ = write!(attrs, r#" style:text-underline-style="{style}""#);
        if let Some(kind) = kind {
            let _ = write!(attrs, r#" style:text-underline-type="{kind}""#);
        }
        if let Some(width) = width {
            let _ = write!(attrs, r#" style:text-underline-width="{width}""#);
        }
        if let Some(color) = underline.color {
            let _ = write!(attrs, r#" style:text-underline-color="{}""#, hex(color));
        }
    }
    if let Some(color) = run.color {
        let _ = write!(attrs, r#" fo:color="{}""#, hex(color));
    }
    if let Some(fill) = run.shading.and_then(shading_fill) {
        let _ = write!(attrs, r#" fo:background-color="{}""#, hex(fill));
    }
    match run.vert_align {
        Some(VertAlign::Superscript) => attrs.push_str(r#" style:text-position="super 58%""#),
        Some(VertAlign::Subscript) => attrs.push_str(r#" style:text-position="sub 58%""#),
        Some(VertAlign::Baseline) => attrs.push_str(r#" style:text-position="0% 100%""#),
        None => {}
    }
    if let Some(spacing) = run.letter_spacing {
        let _ = write!(attrs, r#" fo:letter-spacing="{}""#, twips(spacing));
    }
    if let Some(scale) = run.scale {
        let _ = write!(attrs, r#" style:text-scale="{scale}%""#);
    }
    if let Some(lang) = &run.lang {
        if let Some(tag) = &lang.value {
            let (language, country) = split_tag(tag);
            let _ = write!(attrs, r#" fo:language="{}""#, escape_attr(language));
            if let Some(country) = country {
                let _ = write!(attrs, r#" fo:country="{}""#, escape_attr(country));
            }
        }
    }
    if !attrs.is_empty() {
        let _ = write!(out, "<style:text-properties{attrs}/>");
    }
}

/// `<style:paragraph-properties>`, tab stops included — they are a child
/// element rather than an attribute, which is why this closes its own tag.
pub(crate) fn paragraph_properties(out: &mut String, para: &ParaProps) {
    let mut attrs = String::new();
    if let Some(justify) = para.justify {
        let value = match justify {
            Justify::Center => "center",
            Justify::End => "end",
            Justify::Both | Justify::Distribute => "justify",
            _ => "start",
        };
        let _ = write!(attrs, r#" fo:text-align="{value}""#);
    }
    if let Some(start) = para.indent.start {
        let _ = write!(attrs, r#" fo:margin-left="{}""#, twips(start));
    }
    if let Some(end) = para.indent.end {
        let _ = write!(attrs, r#" fo:margin-right="{}""#, twips(end));
    }
    // One attribute where the model keeps two: a hanging indent is a negative
    // first-line one, and stating both would be stating it twice.
    if let Some(hanging) = para.indent.hanging {
        let _ = write!(attrs, r#" fo:text-indent="{}""#, twips(Twips(-hanging.0)));
    } else if let Some(first) = para.indent.first_line {
        let _ = write!(attrs, r#" fo:text-indent="{}""#, twips(first));
    }
    if let Some(before) = para.spacing.before {
        let _ = write!(attrs, r#" fo:margin-top="{}""#, twips(before));
    }
    if let Some(after) = para.spacing.after {
        let _ = write!(attrs, r#" fo:margin-bottom="{}""#, twips(after));
    }
    match para.spacing.line {
        Some(LineSpacing::Multiple(line)) => {
            let _ = write!(
                attrs,
                r#" fo:line-height="{}%""#,
                trim(line.0 as f64 / 240.0 * 100.0)
            );
        }
        Some(LineSpacing::Exact(height)) => {
            let _ = write!(attrs, r#" fo:line-height="{}""#, twips(height));
        }
        Some(LineSpacing::AtLeast(height)) => {
            let _ = write!(attrs, r#" style:line-height-at-least="{}""#, twips(height));
        }
        None => {}
    }
    if let Some(keep) = para.keep_next {
        let _ = write!(attrs, r#" fo:keep-with-next="{}""#, always(keep));
    }
    if let Some(keep) = para.keep_lines {
        let _ = write!(attrs, r#" fo:keep-together="{}""#, always(keep));
    }
    if para.page_break_before == Some(true) {
        attrs.push_str(r#" fo:break-before="page""#);
    }
    if para.widow_control == Some(true) {
        attrs.push_str(r#" fo:orphans="2" fo:widows="2""#);
    }
    if let Some(suppressed) = para.suppress_line_numbers {
        let _ = write!(attrs, r#" text:number-lines="{}""#, !suppressed);
    }
    if let Some(bidi) = para.bidi {
        let value = if bidi { "rl-tb" } else { "lr-tb" };
        let _ = write!(attrs, r#" style:writing-mode="{value}""#);
    }
    if let Some(fill) = para.shading.and_then(shading_fill) {
        let _ = write!(attrs, r#" fo:background-color="{}""#, hex(fill));
    }
    if let Some(borders) = &para.borders {
        borders_out(&mut attrs, borders);
    }
    let stops = para.tabs.as_deref().unwrap_or_default();
    if attrs.is_empty() && stops.is_empty() {
        return;
    }
    let _ = write!(out, "<style:paragraph-properties{attrs}");
    if stops.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push_str("><style:tab-stops>");
    for stop in stops {
        let _ = write!(
            out,
            r#"<style:tab-stop style:position="{}""#,
            twips(stop.position)
        );
        let kind = match stop.kind {
            TabKind::Center => Some("center"),
            TabKind::End => Some("right"),
            TabKind::Decimal => Some("char"),
            _ => None,
        };
        if let Some(kind) = kind {
            let _ = write!(out, r#" style:type="{kind}""#);
        }
        let leader = match stop.leader {
            TabLeader::None => None,
            TabLeader::Dot => Some("dotted"),
            TabLeader::Hyphen => Some("dash"),
            TabLeader::Underscore | TabLeader::MiddleDot => Some("solid"),
            _ => Some("dotted"),
        };
        if let Some(leader) = leader {
            let _ = write!(out, r#" style:leader-style="{leader}""#);
        }
        if stop.leader == TabLeader::MiddleDot {
            out.push_str(r#" style:leader-text="&#183;""#);
        }
        out.push_str("/>");
    }
    out.push_str("</style:tab-stops></style:paragraph-properties>");
}

fn borders_out(attrs: &mut String, borders: &ParaBorders) {
    for (edge, pad, border) in [
        ("border-top", "padding-top", borders.top),
        ("border-left", "padding-left", borders.start),
        ("border-bottom", "padding-bottom", borders.bottom),
        ("border-right", "padding-right", borders.end),
    ] {
        let Some(border) = border else {
            continue;
        };
        let _ = write!(attrs, r#" fo:{edge}="{}""#, border_words(&border));
        if let Some(space) = border.space {
            let _ = write!(attrs, r#" fo:{pad}="{}pt""#, space);
        }
    }
}

/// `fo:border` is one string: a width, a style and a colour.
fn border_words(border: &Border) -> String {
    let width = border
        .size
        .map(|size| points(size.points()))
        .unwrap_or_else(|| "0.5pt".to_owned());
    let style = match border.style {
        BorderStyle::None => return "none".to_owned(),
        BorderStyle::Dotted => "dotted",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Double | BorderStyle::Triple | BorderStyle::DoubleWave => "double",
        _ => "solid",
    };
    let color = border
        .color
        .map(hex)
        .unwrap_or_else(|| "#000000".to_owned());
    format!("{width} {style} {color}")
}

fn underline_words(
    kind: UnderlineKind,
) -> (&'static str, Option<&'static str>, Option<&'static str>) {
    match kind {
        UnderlineKind::None => ("none", None, None),
        UnderlineKind::Double => ("solid", Some("double"), None),
        UnderlineKind::Thick => ("solid", None, Some("bold")),
        UnderlineKind::Dotted => ("dotted", None, None),
        UnderlineKind::DottedHeavy => ("dotted", None, Some("bold")),
        UnderlineKind::Dash => ("dash", None, None),
        UnderlineKind::DashedHeavy => ("dash", None, Some("bold")),
        UnderlineKind::DashLong | UnderlineKind::DashLongHeavy => ("long-dash", None, None),
        UnderlineKind::DotDash | UnderlineKind::DashDotHeavy => ("dot-dash", None, None),
        UnderlineKind::DotDotDash | UnderlineKind::DashDotDotHeavy => ("dot-dot-dash", None, None),
        UnderlineKind::Wave => ("wave", None, None),
        UnderlineKind::WavyHeavy => ("wave", None, Some("bold")),
        UnderlineKind::WavyDouble => ("wave", Some("double"), None),
        _ => ("solid", None, None),
    }
}

fn shading_fill(shading: Shading) -> Option<wp_model::Color> {
    shading.fill
}

fn weight(bold: bool) -> &'static str {
    match bold {
        true => "bold",
        false => "normal",
    }
}

fn slant(italic: bool) -> &'static str {
    match italic {
        true => "italic",
        false => "normal",
    }
}

fn always(on: bool) -> &'static str {
    match on {
        true => "always",
        false => "auto",
    }
}

fn hex(color: wp_model::Color) -> String {
    match color {
        wp_model::Color::Rgb([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
        // `auto` is "whatever the reader thinks contrasts", which ODF has no
        // word for. Black is what every consumer resolves it to on white paper.
        _ => "#000000".to_owned(),
    }
}

/// A measurement, in points.
///
/// Points rather than inches because a twip *is* a twentieth of a point: every
/// value the model holds comes out with at most two decimal places and reads
/// back as exactly the number it went out as, where an inch would round.
fn twips(value: Twips) -> String {
    points(value.0 as f64 / 20.0)
}

fn points(value: f64) -> String {
    format!("{}pt", trim(value))
}

fn trim(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text.is_empty() || text == "-" {
        true => "0".to_owned(),
        false => text.to_owned(),
    }
}

/// `en-GB` is two attributes here, and a tag with no region is one.
fn split_tag(tag: &str) -> (&str, Option<&str>) {
    match tag.split_once('-') {
        Some((language, country)) => (language, Some(country)),
        None => (tag, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::style::{Style, StyleKind};

    fn read_styles(automatic: &[&str]) -> crate::styles::Styles {
        crate::styles::Styles {
            automatic: automatic.iter().map(|n| (*n).to_string()).collect(),
            ..crate::styles::Styles::default()
        }
    }

    /// The whole reason this module exists: bold applied by hand is not bold in
    /// the file, it is a style that is.
    #[test]
    fn a_run_made_bold_by_hand_mints_an_automatic_style() {
        let table = StyleTable::new();
        let read = read_styles(&[]);
        let mut auto = Automatic::new(table, &read);
        let mut props = RunProps::default();
        props.toggles.set(Toggle::Bold, true);

        let name = auto.run_style(&props).expect("a style was minted");
        let sheet = auto.stylesheet();
        assert!(
            sheet.contains(&format!(r#"style:name="{name}""#)),
            "{sheet}"
        );
        assert!(sheet.contains(r#"style:family="text""#), "{sheet}");
        assert!(sheet.contains(r#"fo:font-weight="bold""#), "{sheet}");
    }

    /// A save with nothing to place adds nothing, which is what keeps an
    /// untouched save byte-identical.
    #[test]
    fn a_run_with_no_formatting_of_its_own_mints_no_automatic_style() {
        let table = StyleTable::new();
        let read = read_styles(&[]);
        let mut auto = Automatic::new(table, &read);
        assert_eq!(auto.run_style(&RunProps::default()), None);
        assert!(auto.is_empty());
    }

    #[test]
    fn the_same_formatting_twice_mints_one_automatic_style() {
        let table = StyleTable::new();
        let read = read_styles(&[]);
        let mut auto = Automatic::new(table, &read);
        let mut props = RunProps::default();
        props.toggles.set(Toggle::Italic, true);
        let first = auto.run_style(&props);
        let second = auto.run_style(&props);
        assert_eq!(first, second);
        assert_eq!(auto.stylesheet().matches("<style:style").count(), 1);
    }

    /// The rule at the top of the module, and the one a consumer depends on: a
    /// minted style stands on a common style, and an automatic one is flattened
    /// into it rather than named as its parent.
    #[test]
    fn an_automatic_style_is_flattened_rather_than_stood_on() {
        let mut table = StyleTable::new();
        let standard = table.intern("Standard", StyleKind::Character);
        let mut inner = Style::new("T3", StyleKind::Character);
        inner.based_on = Some(standard);
        inner.run.size = Some(wp_model::HalfPoint(28));
        let inner = table.insert(inner);

        let read = read_styles(&["T3"]);
        let mut auto = Automatic::new(table, &read);
        let mut props = RunProps {
            style: Some(inner),
            ..RunProps::default()
        };
        props.toggles.set(Toggle::Bold, true);
        let _ = auto.run_style(&props).expect("a style was minted");
        let sheet = auto.stylesheet();
        assert!(
            sheet.contains(r#"style:parent-style-name="Standard""#),
            "the automatic style's own parent is taken over: {sheet}"
        );
        assert!(
            sheet.contains(r#"fo:font-size="14pt""#),
            "and its properties are copied rather than inherited: {sheet}"
        );
        assert!(sheet.contains(r#"fo:font-weight="bold""#), "{sheet}");
    }

    #[test]
    fn a_common_style_is_stood_on_by_name() {
        let mut table = StyleTable::new();
        let emphasis = table.intern("Emphasis", StyleKind::Character);
        let read = read_styles(&[]);
        let mut auto = Automatic::new(table, &read);
        let mut props = RunProps {
            style: Some(emphasis),
            ..RunProps::default()
        };
        props.toggles.set(Toggle::Bold, true);
        let _ = auto.run_style(&props);
        assert!(
            auto.stylesheet()
                .contains(r#"style:parent-style-name="Emphasis""#),
            "{}",
            auto.stylesheet()
        );
    }

    /// A name the file already uses is not one to mint, and a producer that
    /// reused `T1` would silently reformat every run wearing the file's own.
    #[test]
    fn a_minted_name_does_not_collide_with_one_the_file_already_has() {
        let table = StyleTable::new();
        let mut read = crate::styles::Styles::default();
        let mut names = StyleTable::new();
        read.by_name
            .insert("T1".into(), names.intern("T1", StyleKind::Character));
        read.by_name
            .insert("T2".into(), names.intern("T2", StyleKind::Character));
        let mut auto = Automatic::new(table, &read);
        let mut props = RunProps::default();
        props.toggles.set(Toggle::Bold, true);
        assert_eq!(auto.run_style(&props).as_deref(), Some("T3"));
    }

    #[test]
    fn a_measurement_is_written_in_points_and_reads_back_as_itself() {
        assert_eq!(twips(Twips(355)), "17.75pt");
        assert_eq!(twips(Twips(1440)), "72pt");
        assert_eq!(twips(Twips(0)), "0pt");
        assert_eq!(
            crate::xml::length(&twips(Twips(355))),
            Some(Twips(355)),
            "and the reader gets the same number back"
        );
    }
}
