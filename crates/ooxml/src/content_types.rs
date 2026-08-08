//! `[Content_Types].xml` — the map from part to MIME type.
//!
//! This part is *derived*: it is regenerated from the package's actual contents on
//! every save rather than carried over, because an edit that adds or removes a part
//! must be reflected here or the package is invalid.

use std::collections::BTreeMap;

use quick_xml::events::{BytesDecl, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::error::{Error, Result};
use crate::name::PartName;
use crate::xml::{attr, local_name};

pub const CONTENT_TYPES_PATH: &str = "[Content_Types].xml";
const NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

/// Content type applied to a retained part whose type we could not determine.
/// Chosen over guessing: a wrong declared type is worse than an opaque one.
pub const FALLBACK_CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Debug, Clone, Default)]
pub struct ContentTypes {
    /// Extension (lowercased, no dot) -> content type.
    defaults: BTreeMap<String, String>,
    /// Part name -> content type. Takes precedence over `defaults`.
    overrides: BTreeMap<PartName, String>,
}

impl ContentTypes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves the content type for a part: override first, then extension default.
    pub fn get(&self, part: &PartName) -> Option<&str> {
        if let Some(ct) = self.overrides.get(part) {
            return Some(ct.as_str());
        }
        let ext = part.extension()?;
        self.defaults.get(&ext).map(String::as_str)
    }

    pub fn set_default(&mut self, extension: &str, content_type: &str) {
        self.defaults
            .insert(extension.to_ascii_lowercase(), content_type.to_string());
    }

    pub fn set_override(&mut self, part: PartName, content_type: &str) {
        self.overrides.insert(part, content_type.to_string());
    }

    pub fn defaults(&self) -> impl Iterator<Item = (&str, &str)> {
        self.defaults.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn overrides(&self) -> impl Iterator<Item = (&PartName, &str)> {
        self.overrides.iter().map(|(k, v)| (k, v.as_str()))
    }

    /// Records `content_type` for `part` in whichever form keeps the file smallest:
    /// an extension default if it agrees with what is already there, else an override.
    pub fn declare(&mut self, part: &PartName, content_type: &str) {
        if let Some(ext) = part.extension() {
            match self.defaults.get(&ext) {
                Some(existing) if existing == content_type => return,
                None => {
                    self.defaults.insert(ext, content_type.to_string());
                    return;
                }
                Some(_) => {} // conflicts with the default; needs an override
            }
        }
        self.overrides
            .insert(part.clone(), content_type.to_string());
    }

    pub fn parse(xml: &[u8]) -> Result<Self> {
        let part = PartName::new("/[Content_Types].xml")
            .unwrap_or_else(|_| unreachable!("literal is a valid part name"));
        let fail = |e: quick_xml::Error| Error::Xml {
            part: part.clone(),
            source: e.to_string(),
        };

        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);

        let mut out = ContentTypes::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).map_err(fail)? {
                Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                    b"Default" => {
                        let ext = attr(&e, b"Extension");
                        let ct = attr(&e, b"ContentType");
                        if let (Some(ext), Some(ct)) = (ext, ct) {
                            out.set_default(&ext, &ct);
                        }
                    }
                    b"Override" => {
                        let pn = attr(&e, b"PartName");
                        let ct = attr(&e, b"ContentType");
                        if let (Some(pn), Some(ct)) = (pn, ct) {
                            // A malformed part name here is the producer's bug, not
                            // a reason to refuse the whole document.
                            if let Ok(pn) = PartName::new(&pn) {
                                out.set_override(pn, &ct);
                            }
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    pub fn to_xml(&self) -> Vec<u8> {
        let mut w = Writer::new(Vec::new());
        // Word writes a standalone declaration; matching it keeps diffs against
        // real-world packages small.
        let _ = w.write_event(Event::Decl(BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            Some("yes"),
        )));

        let mut root = BytesStart::new("Types");
        root.push_attribute(("xmlns", NS));
        let _ = w.write_event(Event::Start(root));

        for (ext, ct) in &self.defaults {
            let mut e = BytesStart::new("Default");
            e.push_attribute(("Extension", ext.as_str()));
            e.push_attribute(("ContentType", ct.as_str()));
            let _ = w.write_event(Event::Empty(e));
        }
        for (part, ct) in &self.overrides {
            let mut e = BytesStart::new("Override");
            e.push_attribute(("PartName", part.as_str()));
            e.push_attribute(("ContentType", ct.as_str()));
            let _ = w.write_event(Event::Empty(e));
        }

        let _ = w.write_event(Event::End(quick_xml::events::BytesEnd::new("Types")));
        w.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;

    #[test]
    fn resolves_override_before_extension_default() {
        let ct = ContentTypes::parse(SAMPLE).unwrap();
        let doc = PartName::new("/word/document.xml").unwrap();
        assert_eq!(
            ct.get(&doc).unwrap(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        );

        let styles = PartName::new("/word/styles.xml").unwrap();
        assert_eq!(ct.get(&styles).unwrap(), "application/xml");
    }

    #[test]
    fn unknown_extension_resolves_to_nothing() {
        let ct = ContentTypes::parse(SAMPLE).unwrap();
        let img = PartName::new("/word/media/image1.png").unwrap();
        assert!(ct.get(&img).is_none());
    }

    #[test]
    fn round_trips_through_serialization() {
        let a = ContentTypes::parse(SAMPLE).unwrap();
        let b = ContentTypes::parse(&a.to_xml()).unwrap();

        let doc = PartName::new("/word/document.xml").unwrap();
        assert_eq!(a.get(&doc), b.get(&doc));
        assert_eq!(a.defaults().count(), b.defaults().count());
        assert_eq!(a.overrides().count(), b.overrides().count());
    }

    #[test]
    fn declare_prefers_a_default_and_falls_back_to_override() {
        let mut ct = ContentTypes::new();
        let a = PartName::new("/word/media/a.png").unwrap();
        let b = PartName::new("/word/media/b.png").unwrap();

        ct.declare(&a, "image/png");
        assert_eq!(ct.defaults().count(), 1);
        assert_eq!(ct.overrides().count(), 0);

        // Same type as the established default: nothing new needed.
        ct.declare(&b, "image/png");
        assert_eq!(ct.overrides().count(), 0);

        // Conflicting type for the same extension: must become an override.
        let c = PartName::new("/word/media/c.png").unwrap();
        ct.declare(&c, "image/x-weird");
        assert_eq!(ct.overrides().count(), 1);
        assert_eq!(ct.get(&c).unwrap(), "image/x-weird");
        assert_eq!(ct.get(&a).unwrap(), "image/png");
    }

    #[test]
    fn part_name_matching_ignores_case() {
        let ct = ContentTypes::parse(SAMPLE).unwrap();
        let shouty = PartName::new("/WORD/DOCUMENT.XML").unwrap();
        assert!(ct.get(&shouty).unwrap().ends_with("main+xml"));
    }
}
