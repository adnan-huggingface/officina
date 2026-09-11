//! Putting an image *into* an OpenDocument package, which is one thing where a
//! `.docx` makes it three.
//!
//! ODF has no relationships. A `<draw:image>` names its picture by the path of
//! an entry in the package — `xlink:href="Pictures/…"` — and the manifest gives
//! that entry a media type, which a save regenerates from the parts that are
//! there. So a picture put in here is a part and nothing else, and the name
//! [`embed`] answers is the path itself: it is what a drawing's
//! [`wp_model::doc::Drawing::rel`] holds for a picture the application added
//! rather than read, and the writer resolves such a name as its own answer.
//!
//! The bytes go in verbatim, as they do for a `.docx`: a picture is the one kind
//! of part that is wholly ours, with nothing in it to preserve or to rewrite.

use crate::container::Container;
use crate::Result;

/// Where every producer puts pictures, and so where a reader looks first.
pub const PICTURES: &str = "Pictures/";

/// Adds an image to the package and answers the path a drawing names it by.
///
/// The name is chosen not to collide with anything already in the package, so
/// that a picture added to a document LibreOffice wrote never replaces one of
/// its own.
pub fn embed(container: &mut Container, data: &[u8], media_type: &str) -> Result<String> {
    let extension = extension_for(media_type);
    let name = (1..)
        .map(|n| format!("{PICTURES}scriva-{n}.{extension}"))
        .find(|name| container.data(name).is_none())
        .expect("an unbounded range runs out of neither numbers nor names");
    container.put_part(&name, media_type, data.to_vec())?;
    Ok(name)
}

/// The extension a media type is written with. The manifest is what says what
/// an entry is, but a reader that goes by the name — and there are several —
/// needs the two to agree.
fn extension_for(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpeg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/svg+xml" => "svg",
        "image/x-emf" => "emf",
        "image/x-wmf" => "wmf",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::TEXT_MIMETYPE;

    #[test]
    fn an_embedded_picture_is_a_part_named_by_its_path() {
        let mut container = Container::empty(TEXT_MIMETYPE);
        let first = embed(&mut container, b"\x89PNG one", "image/png").expect("embedded");
        let second = embed(&mut container, b"\x89PNG two", "image/png").expect("embedded");
        assert_eq!(first, "Pictures/scriva-1.png");
        assert_eq!(
            second, "Pictures/scriva-2.png",
            "a second picture does not take the first one's name"
        );
        assert_eq!(container.data(&second), Some(&b"\x89PNG two"[..]));
        assert_eq!(container.data(&first), Some(&b"\x89PNG one"[..]));
        assert_eq!(
            container.part(&first).map(|part| part.media_type()),
            Some("image/png"),
            "the manifest is told what the entry is"
        );
    }
}
