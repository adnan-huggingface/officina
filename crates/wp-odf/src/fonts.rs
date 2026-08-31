//! `<office:font-face-decls>` — the faces a document names, and the ones it
//! carries.
//!
//! Everything that names a face in ODF names it *twice removed*: a run says
//! `style:font-name="F1"`, and `F1` is a declaration elsewhere in the file
//! whose `svg:font-family` is the family a system would look up. A reader that
//! took the declaration's own name for the family would set every run of a Word
//! ODF export in a face called `F1`, which is not installed on any machine.
//!
//! A document may also *hold* its faces, under `<svg:font-face-src>`, and those
//! matter for more than completeness: a page measured in whatever the machine
//! substitutes reads as a layout fault when it is really an instrument fault.
//! Unlike `.docx`, an embedded ODF face is stored plainly — there is no
//! obfuscation key to undo, and so no null one to refuse.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::xml::{attr_in, end_local_name, local_name, skip_element};

/// What each declared name stands for.
#[derive(Debug, Default, Clone)]
pub struct FontFaces {
    families: HashMap<String, Arc<str>>,
    embedded: Vec<Embedded>,
}

/// A face the package carries, and where in it the bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embedded {
    pub family: Arc<str>,
    pub bold: bool,
    pub italic: bool,
    /// The path inside the package, as `content.xml` spells it.
    pub href: String,
}

impl FontFaces {
    /// The family a declared name stands for, or nothing if the document never
    /// declared it.
    ///
    /// Nothing, rather than the name itself: a name that was never declared is
    /// not a family, and passing it on would put `F1` into the model where the
    /// truthful answer is that the file did not say.
    pub fn family(&self, name: &str) -> Option<Arc<str>> {
        self.families.get(name).cloned()
    }

    pub fn embedded(&self) -> &[Embedded] {
        &self.embedded
    }

    fn declare(&mut self, name: String, family: Arc<str>) {
        self.families.insert(name, family);
    }
}

/// Reads `<office:font-face-decls>`, whose start tag the caller has just seen.
pub fn declarations(reader: &mut Reader<&[u8]>, faces: &mut FontFaces) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if local_name(&e) != b"font-face" {
                    if !empty {
                        let name = local_name(&e).to_vec();
                        skip_element(reader, &name);
                    }
                    continue;
                }
                let Some(name) = attr_in(&e, b"style", b"name") else {
                    if !empty {
                        skip_element(reader, b"font-face");
                    }
                    continue;
                };
                let family: Arc<str> = attr_in(&e, b"svg", b"font-family")
                    .map(|f| unquoted(&f))
                    .unwrap_or_else(|| name.clone())
                    .into();
                let bold = attr_in(&e, b"svg", b"font-weight")
                    .is_some_and(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600));
                let italic = attr_in(&e, b"svg", b"font-style")
                    .is_some_and(|s| s == "italic" || s == "oblique");
                faces.declare(name, family.clone());
                if !empty {
                    for href in sources(reader) {
                        faces.embedded.push(Embedded {
                            family: family.clone(),
                            bold,
                            italic,
                            href,
                        });
                    }
                }
            }
            Event::End(e) if end_local_name(&e) == b"font-face-decls" => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The `<svg:font-face-uri>` hrefs inside one declaration.
fn sources(reader: &mut Reader<&[u8]>) -> Vec<String> {
    let mut hrefs = Vec::new();
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return hrefs,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"font-face-uri" {
                    if let Some(href) = attr_in(&e, b"xlink", b"href") {
                        hrefs.push(href);
                    }
                    if !empty {
                        skip_element(reader, &name);
                    }
                } else if !empty && name != b"font-face-src" {
                    skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == b"font-face" => return hrefs,
            Event::Eof => return hrefs,
            _ => {}
        }
    }
}

fn unquoted(family: &str) -> String {
    family
        .split(',')
        .next()
        .unwrap_or(family)
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> FontFaces {
        let mut reader = Reader::from_str(xml);
        let mut faces = FontFaces::default();
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if local_name(&e) == b"font-face-decls" => {
                    declarations(&mut reader, &mut faces)
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        faces
    }

    /// The indirection, and what goes wrong without it: a run says `F1`, and
    /// `F1` is not a face on anybody's machine.
    #[test]
    fn a_declared_name_stands_for_a_family_rather_than_being_one() {
        let faces = read(concat!(
            r#"<office:font-face-decls>"#,
            r#"<style:font-face style:name="F1" svg:font-family="&apos;Cambria&apos;"/>"#,
            r#"<style:font-face style:name="Courier New" svg:font-family="Courier New"/>"#,
            r#"</office:font-face-decls>"#
        ));
        assert_eq!(faces.family("F1").as_deref(), Some("Cambria"));
        assert_eq!(faces.family("Courier New").as_deref(), Some("Courier New"));
        assert_eq!(
            faces.family("F9"),
            None,
            "a name the document never declared is not a family"
        );
    }

    #[test]
    fn a_face_the_package_carries_says_where_its_bytes_are() {
        let faces = read(concat!(
            r#"<office:font-face-decls>"#,
            r#"<style:font-face style:name="F2" svg:font-family="Ubuntu Mono" svg:font-weight="bold">"#,
            r#"<svg:font-face-src><svg:font-face-uri xlink:href="Fonts/ubuntu-b.ttf"/></svg:font-face-src>"#,
            r#"</style:font-face>"#,
            r#"</office:font-face-decls>"#
        ));
        assert_eq!(faces.family("F2").as_deref(), Some("Ubuntu Mono"));
        assert_eq!(
            faces.embedded(),
            [Embedded {
                family: "Ubuntu Mono".into(),
                bold: true,
                italic: false,
                href: "Fonts/ubuntu-b.ttf".into(),
            }]
        );
    }
}
