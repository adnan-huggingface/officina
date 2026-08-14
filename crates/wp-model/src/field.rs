//! Field codes: the little programs embedded in a document's text.
//!
//! A field is `{ INSTRUCTION }` and a cached result. Word recomputes the result
//! when it opens the document; until then the result is what was true when it
//! was last saved, which is why a page number in a document nobody has opened
//! since March is March's page number.
//!
//! **The instruction arrives in fragments.** `<w:instrText>` is split wherever
//! Word felt like splitting it, so ` PAGE \* MERGEFORMAT ` may come through as
//! ` PA`, `GE \* MERGEFOR`, `MAT `. Joining before parsing is not an
//! optimisation; a parser fed the fragments finds no field at all.
//!
//! **Switches are not arguments.** `\* MERGEFORMAT`, `\h`, `\o "1-3"` are
//! modifiers, and the argument is what is left. Treating the first token after
//! the name as the argument makes ` TOC \o "1-3" ` a table of contents of a
//! chapter called `\o`.

use std::sync::Arc;

/// A parsed field instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Upper-cased, because `Page` and `PAGE` are the same field and documents
    /// contain both.
    pub name: Arc<str>,
    /// The arguments, in order, with their quotes removed.
    pub arguments: Vec<Arc<str>>,
    /// Switches, each with its own argument where it had one: `\o "1-3"` is
    /// `('o', Some("1-3"))`.
    pub switches: Vec<(char, Option<Arc<str>>)>,
}

impl Field {
    /// Parses a joined instruction.
    ///
    /// `None` for an instruction with no name at all, which is what an empty or
    /// whitespace-only `<w:instrText>` is.
    pub fn parse(instruction: &str) -> Option<Field> {
        let tokens = tokens(instruction);
        let mut tokens = tokens.into_iter();
        let name: Arc<str> = tokens.next()?.to_uppercase().into();
        let mut arguments = Vec::new();
        let mut switches: Vec<(char, Option<Arc<str>>)> = Vec::new();
        for token in tokens {
            if let Some(letter) = token.strip_prefix('\\').and_then(|s| s.chars().next()) {
                switches.push((letter, None));
            } else if let Some(last) = switches.last_mut().filter(|(_, arg)| arg.is_none()) {
                // The token after a switch belongs to it — that is what makes
                // `\o "1-3"` one thing rather than two.
                last.1 = Some(token.into());
            } else {
                arguments.push(token.into());
            }
        }
        Some(Field {
            name,
            arguments,
            switches,
        })
    }

    pub fn switch(&self, letter: char) -> Option<Option<&str>> {
        self.switches
            .iter()
            .find(|(name, _)| *name == letter)
            .map(|(_, argument)| argument.as_deref())
    }

    pub fn has_switch(&self, letter: char) -> bool {
        self.switch(letter).is_some()
    }

    pub fn argument(&self, index: usize) -> Option<&str> {
        self.arguments.get(index).map(|a| a.as_ref())
    }

    /// What kind of field this is, for the ones whose result changes what is
    /// drawn.
    pub fn kind(&self) -> Kind {
        match self.name.as_ref() {
            "PAGE" => Kind::Page,
            "NUMPAGES" => Kind::NumPages,
            "SECTIONPAGES" => Kind::SectionPages,
            "DATE" => Kind::Date,
            "TIME" => Kind::Time,
            "TOC" => Kind::Toc,
            "REF" | "PAGEREF" | "NOTEREF" => Kind::Ref,
            "SEQ" => Kind::Seq,
            "HYPERLINK" => Kind::Hyperlink,
            "FILENAME" => Kind::FileName,
            "AUTHOR" => Kind::Author,
            "TITLE" => Kind::Title,
            "STYLEREF" => Kind::StyleRef,
            _ => Kind::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Page,
    NumPages,
    SectionPages,
    Date,
    Time,
    Toc,
    Ref,
    Seq,
    Hyperlink,
    FileName,
    Author,
    Title,
    StyleRef,
    /// One of the several hundred others. Its cached result is drawn, which is
    /// what Word shows for a field it cannot recompute either.
    Other,
}

/// Splits an instruction into tokens, honouring quotes.
///
/// `TOC \o "1-3" \h` is five tokens and the quoted one keeps its space.
fn tokens(instruction: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in instruction.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                // A quote *closes* a token even when what is inside is empty:
                // `REF ""` has one empty argument, not none.
                if !quoted {
                    out.push(std::mem::take(&mut current));
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Joins the fragments of an instruction before it is parsed.
pub fn join(fragments: &[Arc<str>]) -> String {
    fragments.concat()
}

/// Which heading levels a `TOC` field collects.
///
/// `\o "1-3"` is the usual form. `\o` with nothing after it means every level,
/// and a `TOC` with no `\o` at all is built from `\t` style names instead —
/// which is not implemented and is reported as an empty range so a caller can
/// say so rather than silently producing nothing.
pub fn toc_levels(field: &Field) -> Option<std::ops::RangeInclusive<u8>> {
    match field.switch('o')? {
        Some(range) => {
            let (from, to) = range.split_once('-')?;
            Some(from.trim().parse().ok()?..=to.trim().parse().ok()?)
        }
        None => Some(1..=9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(instruction: &str) -> Field {
        Field::parse(instruction).expect("a field")
    }

    #[test]
    fn an_instruction_split_into_fragments_is_joined_before_it_is_parsed() {
        // Word splits `<w:instrText>` wherever it likes, and a parser fed the
        // fragments finds no field at all.
        let fragments: Vec<Arc<str>> = vec![" PA".into(), "GE \\* MERGEFOR".into(), "MAT ".into()];
        let joined = join(&fragments);
        assert_eq!(joined, " PAGE \\* MERGEFORMAT ");
        let field = field(&joined);
        assert_eq!(field.kind(), Kind::Page);
        assert!(field.has_switch('*'));
    }

    #[test]
    fn a_field_name_is_matched_whatever_its_case() {
        assert_eq!(field(" Page ").kind(), Kind::Page);
        assert_eq!(field("numpages").kind(), Kind::NumPages);
    }

    #[test]
    fn a_switch_takes_the_token_after_it_and_an_argument_does_not() {
        // ` TOC \o "1-3" ` is a table of contents of levels one to three, not a
        // table of contents of a chapter called `\o`.
        let toc = field(r#" TOC \o "1-3" \h \z \u "#);
        assert!(toc.arguments.is_empty());
        assert_eq!(toc.switch('o'), Some(Some("1-3")));
        assert!(toc.has_switch('h'));
        assert_eq!(toc_levels(&toc), Some(1..=3));

        let reference = field(r#" REF _Ref12345 \h "#);
        assert_eq!(reference.argument(0), Some("_Ref12345"));
        assert!(reference.has_switch('h'));
    }

    #[test]
    fn a_quoted_argument_keeps_its_spaces() {
        let link = field(r#" HYPERLINK "https://example.com/a b" \o "A tooltip" "#);
        assert_eq!(link.argument(0), Some("https://example.com/a b"));
        assert_eq!(link.switch('o'), Some(Some("A tooltip")));
        assert_eq!(link.kind(), Kind::Hyperlink);
    }

    #[test]
    fn an_empty_quoted_argument_is_an_argument() {
        let reference = field(r#" REF "" "#);
        assert_eq!(reference.argument(0), Some(""));
    }

    #[test]
    fn a_toc_with_no_level_switch_says_so_rather_than_guessing() {
        let by_style = field(r#" TOC \t "Caption,1" "#);
        assert_eq!(toc_levels(&by_style), None);
        // And a bare `\o` is every level.
        assert_eq!(toc_levels(&field(r" TOC \o ")), Some(1..=9));
    }

    #[test]
    fn a_field_nobody_models_still_parses() {
        let odd = field(" MERGEFIELD Surname ");
        assert_eq!(odd.kind(), Kind::Other);
        assert_eq!(odd.argument(0), Some("Surname"));
    }

    #[test]
    fn an_empty_instruction_is_not_a_field() {
        assert_eq!(Field::parse("   "), None);
        assert_eq!(Field::parse(""), None);
    }
}
