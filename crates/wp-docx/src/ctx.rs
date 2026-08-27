//! What a part reader needs beyond the bytes in front of it.
//!
//! Two things, and both are shared across parts rather than per part. The style
//! table, because `<w:pStyle w:val="Heading1"/>` in the document and
//! `<w:style w:styleId="Heading1">` in `styles.xml` must land on the same
//! [`StyleId`] whichever is read first — and the document is read before the
//! styles in a package whose relationships happen to be ordered that way. And
//! the header index, because a header body is a part and a section refers to it
//! by relationship id, so the two have to agree on a numbering.
//!
//! [`StyleId`]: wp_model::StyleId

use std::collections::BTreeMap;

use wp_model::section::HeaderId;
use wp_model::StyleTable;

/// Relationship id -> header body, allocated on first sight.
///
/// Headers and footers share one numbering because a document holds one list of
/// bodies: they are the same kind of thing and only the reference says which is
/// which.
#[derive(Debug, Default)]
pub(crate) struct HeaderIndex {
    by_rel: BTreeMap<String, HeaderId>,
    /// By id: the relationship it came from, and whether it is a footer.
    order: Vec<(String, bool)>,
}

impl HeaderIndex {
    pub fn id(&mut self, rel: &str, footer: bool) -> HeaderId {
        if let Some(&id) = self.by_rel.get(rel) {
            return id;
        }
        let id = HeaderId(self.order.len() as u32);
        self.by_rel.insert(rel.to_owned(), id);
        self.order.push((rel.to_owned(), footer));
        id
    }

    /// Every body a section referred to, in the order the ids were handed out.
    pub fn referenced(&self) -> impl Iterator<Item = (HeaderId, &str, bool)> {
        self.order
            .iter()
            .enumerate()
            .map(|(index, (rel, footer))| (HeaderId(index as u32), rel.as_str(), *footer))
    }
}

pub(crate) struct Ctx<'a> {
    pub styles: &'a mut StyleTable,
    headers: &'a mut HeaderIndex,
    /// The bytes of the part being read.
    ///
    /// Needed so that an element the model does not fully understand — a
    /// `<w:drawing>`, an `<m:oMath>`, a VML picture — can be captured verbatim
    /// and written back by a writer that understands it no better. Empty when
    /// the reader was handed events rather than a part, which only happens in
    /// this crate's own tests.
    part: &'a [u8],
    /// Which part is being read, when it is not the document.
    ///
    /// A header names its own pictures `rId1` just as the document names its
    /// own `rId1`, so a relationship read out of one has to be filed under a
    /// key that says which part it came from — see [`crate::parts::qualified`].
    scope: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    pub fn new(styles: &'a mut StyleTable, headers: &'a mut HeaderIndex) -> Ctx<'a> {
        Ctx {
            styles,
            headers,
            part: &[],
            scope: None,
        }
    }

    pub fn of_part(
        styles: &'a mut StyleTable,
        headers: &'a mut HeaderIndex,
        part: &'a [u8],
    ) -> Ctx<'a> {
        Ctx {
            styles,
            headers,
            part,
            scope: None,
        }
    }

    /// The same, for a part that is not the document — a header or a footer,
    /// whose relationships are numbered from `rId1` all over again.
    pub fn of_named_part(
        styles: &'a mut StyleTable,
        headers: &'a mut HeaderIndex,
        part: &'a [u8],
        scope: &'a str,
    ) -> Ctx<'a> {
        Ctx {
            styles,
            headers,
            part,
            scope: Some(scope),
        }
    }

    /// The key a relationship named inside this part is looked up by.
    pub fn rel(&self, id: &str) -> String {
        match self.scope {
            Some(scope) => crate::parts::qualified(scope, id),
            None => id.to_owned(),
        }
    }

    /// The part's bytes between two reader positions.
    ///
    /// A UTF-8 byte-order mark is added back: quick-xml does not count it, so a
    /// position it reports is three bytes short in a part that has one.
    pub fn span(&self, from: usize, to: usize) -> std::sync::Arc<[u8]> {
        let offset = if self.part.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        let (from, to) = (from + offset, (to + offset).min(self.part.len()));
        self.part.get(from..to).unwrap_or(&[]).into()
    }

    pub fn header_id(&mut self, rel: &str, footer: bool) -> HeaderId {
        self.headers.id(rel, footer)
    }
}

#[cfg(test)]
pub(crate) fn test_ctx() -> (StyleTable, HeaderIndex) {
    (StyleTable::new(), HeaderIndex::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relationship_asked_for_twice_gets_the_same_body() {
        let mut index = HeaderIndex::default();
        let first = index.id("rId4", false);
        assert_eq!(index.id("rId4", false), first);
        let footer = index.id("rId5", true);
        assert_ne!(first, footer);
        let listed: Vec<_> = index.referenced().collect();
        assert_eq!(listed, [(first, "rId4", false), (footer, "rId5", true)]);
    }
}
