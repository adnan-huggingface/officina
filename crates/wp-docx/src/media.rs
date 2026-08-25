//! Putting an image *into* a package, which is three things and not one.
//!
//! A picture in a `.docx` is a part holding the bytes, a relationship from
//! `document.xml` naming that part, and a `<w:drawing>` in the text naming the
//! relationship. Miss any of the three and Word offers to repair the file
//! rather than open it — an `r:embed` pointing at nothing is not a missing
//! picture to Word, it is a broken document.
//!
//! The bytes go in verbatim. Everything else this crate writes is a splice into
//! something Word authored; a media part is the one kind that is wholly ours,
//! and there is nothing in a PNG to preserve or to rewrite.

use ooxml::{Package, PartName, Relationship, TargetMode};

use crate::error::{Error, Result};
use crate::parts;

const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// Adds an image to the package and relates it to the document part.
///
/// Answers the relationship id — what a `<a:blip r:embed>` names it by, and the
/// only handle the model keeps: [`wp_model::doc::Drawing::rel`].
pub fn embed(package: &mut Package, data: &[u8], content_type: &str) -> Result<String> {
    let located = parts::locate(package)?;
    let name = free_name(package, extension_for(content_type))?;
    package.put_part(name.clone(), content_type, data.to_vec());

    let mut rels = package.relationships(&located.document)?;
    let id = rels.next_id();
    rels.insert(Relationship {
        id: id.clone(),
        rel_type: format!("{REL_BASE}/image"),
        // Relative, because `word/media/image1.png` beside `word/document.xml`
        // is what Word writes and what keeps the package movable.
        target: relative_to(&located.document, &name),
        mode: TargetMode::Internal,
    });
    package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    Ok(id)
}

/// Adds a chart part from its bytes and relates it to the document part.
///
/// For a chart that arrives from outside the package — copied in Calx, whose
/// clipboard payload is the `<c:chartSpace>` itself. The part carries no
/// relationships of its own: an authored chart has no embedded workbook, no
/// colour part and no style part, and its caches are what a reader draws.
/// Answers the relationship id the drawing will name it by.
pub fn embed_chart(package: &mut Package, data: &[u8]) -> Result<String> {
    let located = parts::locate(package)?;
    let name = free_chart_name(package, &located.document)?;
    package.put_part(
        name.clone(),
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        data.to_vec(),
    );

    let mut rels = package.relationships(&located.document)?;
    let id = rels.next_id();
    rels.insert(Relationship {
        id: id.clone(),
        rel_type: format!("{REL_BASE}/chart"),
        target: relative_to(&located.document, &name),
        mode: TargetMode::Internal,
    });
    package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    Ok(id)
}

/// The first `charts/chartN.xml` beside the document part that nothing has
/// taken. Beside the document rather than at a fixed `/word/`, for the same
/// reason as [`relative_to`]: the document part is where Word put it.
fn free_chart_name(package: &Package, document: &PartName) -> Result<PartName> {
    let dir = match document.parent() {
        "/" => String::new(),
        dir => dir.to_owned(),
    };
    for n in 1..10_000 {
        let candidate =
            PartName::new(&format!("{dir}/charts/chart{n}.xml")).map_err(Error::Package)?;
        if package.part(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(Error::Package(ooxml::Error::BadPartName {
        raw: format!("{dir}/charts/chartN.xml"),
        reason: "the package already holds ten thousand of them",
    }))
}

/// Clones a chart part and answers a fresh relationship naming the clone.
///
/// A picture part can be shown by two drawings at once; a chart part cannot.
/// Word refuses to *open* a document where two `<c:chart r:id>` name one part
/// — probed on `file-sample_500kB.docx`: the shared part gets "Word
/// experienced an error trying to open the file", and the same bytes under a
/// second part name open as two charts. So a duplicated chart is a duplicated
/// part. The clone keeps the original's own relationships — colours, style,
/// an embedded workbook — pointing at the same targets; only the chart part
/// itself must not be shared.
pub fn clone_chart(package: &mut Package, rel: &str) -> Result<String> {
    let located = parts::locate(package)?;
    let dangling = || {
        Error::Package(ooxml::Error::BadPartName {
            raw: rel.to_owned(),
            reason: "this relationship names no chart part",
        })
    };
    let source = located.target(rel).cloned().ok_or_else(dangling)?;
    let bytes = package.part(&source).ok_or_else(dangling)?.data().to_vec();
    let content_type = package
        .content_types()
        .get(&source)
        .unwrap_or("application/vnd.openxmlformats-officedocument.drawingml.chart+xml")
        .to_owned();
    let name = free_beside(package, &source)?;
    package.put_part(name.clone(), &content_type, bytes);
    // Relative targets stay valid because the clone lives beside the original.
    if let Some(chart_rels) = package
        .part(&source.rels_part())
        .map(|part| part.data().to_vec())
    {
        package.put_part(name.rels_part(), RELS_TYPE, chart_rels);
    }

    let mut rels = package.relationships(&located.document)?;
    let rel_type = rels
        .get(rel)
        .map(|original| original.rel_type.clone())
        .unwrap_or_else(|| format!("{REL_BASE}/chart"));
    let id = rels.next_id();
    rels.insert(Relationship {
        id: id.clone(),
        rel_type,
        target: relative_to(&located.document, &name),
        mode: TargetMode::Internal,
    });
    package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    Ok(id)
}

/// The first `chartN.xml` beside the original that nothing else has taken.
fn free_beside(package: &Package, beside: &PartName) -> Result<PartName> {
    let dir = beside.parent();
    for n in 1..10_000 {
        let candidate = PartName::new(&format!("{dir}/chart{n}.xml")).map_err(Error::Package)?;
        if package.part(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(Error::Package(ooxml::Error::BadPartName {
        raw: format!("{dir}/chartN.xml"),
        reason: "the package already holds ten thousand of them",
    }))
}

/// The first `word/media/imageN` nothing else has taken.
///
/// By name and not by count: a document that has had a picture deleted has a
/// gap in the numbering, and a package where two parts share a name is a
/// package with one of them missing.
fn free_name(package: &Package, extension: &str) -> Result<PartName> {
    for n in 1..10_000 {
        let candidate =
            PartName::new(&format!("/word/media/image{n}.{extension}")).map_err(Error::Package)?;
        if package.part(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(Error::Package(ooxml::Error::BadPartName {
        raw: format!("/word/media/imageN.{extension}"),
        reason: "the package already holds ten thousand of them",
    }))
}

/// The extension a content type is written with. Word decides what a part is
/// from `[Content_Types].xml`, but a reader that goes by the name — and there
/// are several — needs the two to agree.
fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/jpeg" => "jpeg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        // A metafile is a picture as far as a package is concerned, and Word
        // takes one under its own extension.
        "image/x-emf" => "emf",
        "image/x-wmf" => "wmf",
        _ => "png",
    }
}

/// A part name as a relationship target relative to the part that names it.
fn relative_to(from: &PartName, target: &PartName) -> String {
    let dir = from.parent();
    let prefix = match dir {
        "/" => "/".to_string(),
        dir => format!("{dir}/"),
    };
    match target.as_str().strip_prefix(&prefix) {
        Some(rest) if !rest.contains("../") => rest.to_string(),
        _ => target.as_str().to_string(),
    }
}
