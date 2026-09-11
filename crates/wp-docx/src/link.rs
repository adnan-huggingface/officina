//! Relating a hyperlink to where it goes, which in a `.docx` is not where the
//! link is.
//!
//! A `<w:hyperlink>` to somewhere outside the document names a relationship,
//! and the relationship holds the address with `TargetMode="External"`. That is
//! the only spelling Word accepts: an `r:id` holding an address is not a link
//! to that address but a relationship that does not exist, and Word refuses the
//! file rather than the link. ODF states the address on the link itself, so a
//! document read out of an `.odt` arrives holding addresses where this format
//! wants names, and each one is related here on its way in.

use ooxml::{Package, Relationship, TargetMode};

use crate::error::Result;
use crate::media::{RELS_TYPE, REL_BASE};
use crate::parts;

/// Relates `address` to the document part as an external hyperlink, and
/// answers the relationship id a `<w:hyperlink r:id>` names it by.
///
/// An address already related is answered with the relationship it already
/// has, so a document saved twice does not collect two of them.
pub fn relate(package: &mut Package, address: &str) -> Result<String> {
    let located = parts::locate(package)?;
    let mut rels = package.relationships(&located.document)?;
    let kind = format!("{REL_BASE}/hyperlink");
    if let Some(existing) = rels
        .by_type(&kind)
        .find(|rel| rel.mode == TargetMode::External && rel.target == address)
    {
        return Ok(existing.id.clone());
    }
    let id = rels.next_id();
    rels.insert(Relationship {
        id: id.clone(),
        rel_type: kind,
        target: address.to_owned(),
        mode: TargetMode::External,
    });
    package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_related_once_and_named_by_its_relationship() {
        let document = wp_model::Document::default();
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        let first = relate(&mut package, "https://example.invalid/a").expect("related");
        let again = relate(&mut package, "https://example.invalid/a").expect("related");
        let other = relate(&mut package, "https://example.invalid/b").expect("related");
        assert_eq!(first, again, "the same address is not related twice");
        assert_ne!(first, other);

        let located = parts::locate(&package).expect("a document part");
        let rels = package
            .relationships(&located.document)
            .expect("its relationships");
        let rel = rels.get(&first).expect("the id names a relationship");
        assert_eq!(rel.target, "https://example.invalid/a");
        assert_eq!(rel.mode, TargetMode::External);
    }
}
