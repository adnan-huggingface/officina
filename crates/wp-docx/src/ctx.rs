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
}

impl<'a> Ctx<'a> {
    pub fn new(styles: &'a mut StyleTable, headers: &'a mut HeaderIndex) -> Ctx<'a> {
        Ctx { styles, headers }
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
