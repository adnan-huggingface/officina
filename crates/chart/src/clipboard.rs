//! The chart clipboard format the two applications exchange.
//!
//! A chart copied in Calx and pasted in Scriva crosses a process boundary, so
//! it travels on the OS clipboard under a registered format of our own. The
//! payload is the `<c:chartSpace>` part itself — the one element a workbook
//! and a document hold in exactly the same shape — plus the size the chart had
//! where it was copied, because a document has no cells to anchor to and must
//! be told how big the thing is.
//!
//! Defined here, in the crate both applications already share, so that there
//! is one statement of the format rather than two that agree today.

/// The registered clipboard format's name. Not an Office name: nothing else
/// reads this, and nothing else should mistake it for something it reads.
pub const FORMAT: &str = "Officina Chart";

/// The bytes that open a payload. Versioned so that a change to the layout is
/// a new magic, not a silent misread of an old board by a new build.
const MAGIC: &[u8; 8] = b"OFCHART1";

/// Wraps a chart part and its size for the board.
///
/// `cx` and `cy` are EMUs, because that is the unit the drawing that receives
/// them states its extent in.
pub fn pack(cx: i64, cy: i64, chart_space: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + chart_space.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&cx.to_le_bytes());
    out.extend_from_slice(&cy.to_le_bytes());
    out.extend_from_slice(chart_space);
    out
}

/// Reads a payload back, or nothing for bytes this format did not write.
///
/// The board is shared with every program on the machine; a registered format
/// name is a claim, not a guarantee, so the magic is checked rather than
/// trusted.
pub fn unpack(bytes: &[u8]) -> Option<(i64, i64, &[u8])> {
    let rest = bytes.strip_prefix(MAGIC.as_slice())?;
    if rest.len() < 16 {
        return None;
    }
    let cx = i64::from_le_bytes(rest[..8].try_into().ok()?);
    let cy = i64::from_le_bytes(rest[8..16].try_into().ok()?);
    if cx <= 0 || cy <= 0 {
        return None;
    }
    Some((cx, cy, &rest[16..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_comes_back_as_it_went_on() {
        let xml = br#"<c:chartSpace xmlns:c="c"/>"#;
        let packed = pack(914_400, 457_200, xml);
        let (cx, cy, bytes) = unpack(&packed).expect("our own payload");
        assert_eq!((cx, cy), (914_400, 457_200));
        assert_eq!(bytes, xml);
    }

    #[test]
    fn bytes_from_anywhere_else_are_refused() {
        assert_eq!(unpack(b""), None);
        assert_eq!(unpack(b"OFCHART1"), None);
        assert_eq!(
            unpack(b"not a chart payload at all, whatever it claims"),
            None
        );
        // A well-formed header stating an impossible size is a payload nothing
        // sensible wrote.
        let hollow = pack(0, 457_200, b"<c:chartSpace/>");
        assert_eq!(unpack(&hollow), None);
    }
}
