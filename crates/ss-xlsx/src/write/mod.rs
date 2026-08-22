//! Writing a workbook back out.
//!
//! The rule from `DESIGN.md` §1 — *never write a part we did not either author
//! or faithfully retain* — decides the shape of this entirely. There is no
//! serializer here in the usual sense, because a serializer would need to
//! understand every part it emits, and this crate understands four of the
//! twenty-odd parts in a real workbook. What there is instead is a set of
//! *edits*: the original bytes go out unless a specific, named piece of them
//! disagrees with the model, and then only that piece is replaced.
//!
//! Three parts are edited. Worksheets get their changed cells replaced.
//! `sharedStrings.xml` gets the text those cells introduced appended to it.
//! `styles.xml` gets any number format they needed. Everything else in the
//! package — including the parts of those three files outside the edited
//! regions — is the producer's bytes, untouched.
//!
//! Every part goes through the edit on every save, even one with no changes to
//! make. That is deliberate: it means the no-edit fidelity check is a test of
//! this code rather than a test of code that skipped it.

mod blank;
mod cells;
mod chart_out;
mod drawing_out;
pub(crate) mod insert;
mod sheet_out;
mod splice;
mod strings_out;
mod styles_out;
mod workbook_out;

use std::collections::BTreeMap;
use std::io::{Seek, Write};
use std::path::Path;

use ooxml::{PartName, Relationship, TargetMode};
use ss_model::{SheetKind, Workbook};

use crate::error::Result;
use crate::{parts, workbook_part, XlsxDocument};

const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";

impl XlsxDocument {
    /// A document for a workbook that has never been in a file.
    ///
    /// The package is authored here rather than copied from a template, so
    /// every byte in it is one this crate is answerable for.
    pub fn new(mut workbook: Workbook) -> Result<Self> {
        let package = blank::package_for(&mut workbook)?;
        Ok(XlsxDocument { workbook, package })
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.flush()?;
        self.package.save(path)?;
        Ok(())
    }

    pub fn write_to<W: Write + Seek>(&mut self, writer: W) -> Result<()> {
        self.flush()?;
        self.package.write(writer)?;
        Ok(())
    }

    /// Brings the package's edited parts in line with the model.
    ///
    /// Separate from writing so that a caller can inspect the result — the
    /// fidelity harness compares packages, not files.
    pub fn flush(&mut self) -> Result<()> {
        self.flush_with(false)
    }

    /// Flushes, writing every cell out of the model rather than only the ones
    /// that changed.
    ///
    /// No save should call this: the whole point of the writer is that it leaves
    /// untouched cells alone, cached values and unknown attributes included, and
    /// regenerating them throws that protection away for nothing.
    ///
    /// It exists because it asks the harness a stronger question than a save
    /// does. A save proves the bytes we copied survived being copied. This
    /// proves we could have written them ourselves — that our idea of a
    /// worksheet and Excel's agree everywhere in the corpus, not just in the
    /// places an edit happened to land. When those two answers diverge, the
    /// difference is a feature of real files we have not understood yet, and it
    /// is much better to hear about it from the harness than from a user whose
    /// edit landed on that row.
    pub fn flush_regenerating(&mut self) -> Result<()> {
        self.flush_with(true)
    }

    fn flush_with(&mut self, regenerate: bool) -> Result<()> {
        let located = parts::locate(&self.package)?;

        let mut sst = match &located.shared_strings {
            Some(name) => match self.package.part(name) {
                Some(part) => strings_out::Sst::read(name.as_str(), part.data())?,
                None => strings_out::Sst::absent(),
            },
            None => strings_out::Sst::absent(),
        };

        // Sheets first: they are what discovers new text and new styles.
        let meta = {
            let part =
                self.package
                    .part(&located.workbook)
                    .ok_or_else(|| crate::Error::MissingPart {
                        referenced_by: "/_rels/.rels".to_owned(),
                        rel_id: "officeDocument".to_owned(),
                    })?;
            workbook_part::parse(located.workbook.as_str(), part.data())?
        };

        // The sheet list, reconciled against the model: sheets added, removed,
        // renamed, hidden or reordered all land here, and every one of them has
        // to be settled before the cells are written, because this is what
        // decides which part each sheet's cells go into.
        let targets = self.reconcile_sheets(&located, &meta)?;

        let mut written: Vec<(PartName, String, Vec<u8>)> = Vec::new();
        for (index, name) in targets.iter().enumerate() {
            let Some(sheet) = self.workbook.sheet(index) else {
                continue;
            };
            if !matches!(sheet.kind, SheetKind::Worksheet) {
                continue;
            }
            let Some(name) = name.clone() else {
                continue;
            };
            if self.package.part(&name).is_none() {
                continue;
            }
            // Charts and pictures the application made, which have no parts
            // yet. Done before the worksheet is written, because authoring a
            // drawing for a sheet that had none is the one case where the
            // worksheet itself gains an element — and the sheet has to be
            // re-borrowed afterwards, since materializing writes into it.
            let drawing_rel =
                insert::materialize(&mut self.package, &mut self.workbook, index, &name)?;
            // Notes, which live in a part of their own and need a VML shape
            // beside them for Excel to draw the box.
            let legacy_rel = insert::comments(&mut self.package, &self.workbook, index, &name)?;
            let Some(sheet) = self.workbook.sheet(index) else {
                continue;
            };
            let Some(part) = self.package.part(&name) else {
                continue;
            };
            let content_type = part.content_type.clone();
            let mut ctx = sheet_out::Context {
                sheet,
                strings: &self.workbook.strings,
                sst: &mut sst,
                regenerate,
                drawing_rel: drawing_rel.as_deref(),
                legacy_rel: legacy_rel.as_deref(),
            };
            let data = sheet_out::rewrite(name.as_str(), part.data(), &mut ctx)?;
            written.push((name.clone(), content_type, data));

            // Pictures and charts the user moved, resized or deleted.
            // Compared against the *file*, re-read, rather than against a
            // flag set when the drag ended: an object dragged and dragged
            // back has not changed, and a part we rewrite for nothing is a
            // part we could get wrong for nothing.
            if let Some(drawing) = crate::drawing_of(&self.package, &name) {
                let original = crate::object_anchors(&self.package, &drawing)?;
                let current: std::collections::BTreeMap<usize, &ss_model::Anchor> = sheet
                    .pictures
                    .iter()
                    .filter(|p| p.drawing_part == drawing.as_str())
                    .map(|p| (p.anchor_index, &p.anchor))
                    .chain(
                        sheet
                            .charts
                            .iter()
                            .filter(|c| c.drawing_part == drawing.as_str())
                            .map(|c| (c.anchor_index, &c.anchor)),
                    )
                    .collect();

                let mut wanted = drawing_out::Wanted::new();
                for (index, was) in &original {
                    match current.get(index) {
                        None => {
                            wanted.insert(*index, None);
                        }
                        Some(now) if *now != was => {
                            wanted.insert(*index, Some((*now).clone()));
                        }
                        Some(_) => {}
                    }
                }
                if !wanted.is_empty() {
                    if let Some(part) = self.package.part(&drawing) {
                        let content_type = part.content_type.clone();
                        let data = drawing_out::rewrite(drawing.as_str(), part.data(), &wanted)?;
                        written.push((drawing, content_type, data));
                    }
                }
            }
        }

        // Charts the user formatted. A chart is otherwise entirely preserved,
        // so what the inspector sets is spliced into the part property by
        // property — and the comparison is against the *file*, re-read,
        // rather than against a flag set when the user clicked. A chart
        // whose kind changed is the exception: every element in it moves, so
        // the part is written afresh from the model, as a new chart's is.
        for sheet in &self.workbook.sheets {
            for chart in &sheet.charts {
                let Ok(name) = PartName::new(&chart.part) else {
                    continue;
                };
                let Some(part) = self.package.part(&name) else {
                    continue;
                };
                let Some(stored) = ss_model::chart::read::plot(part.data()) else {
                    continue;
                };
                let content_type = part.content_type.clone();
                let retyped = stored.kind != chart.plot.kind
                    || stored.grouping != chart.plot.grouping
                    || stored.horizontal != chart.plot.horizontal;
                if retyped {
                    written.push((name, content_type, insert::chart_part(chart)));
                    continue;
                }
                let mut data =
                    chart_out::restyle(name.as_str(), part.data(), &stored, &chart.plot)?;
                if stored.title != chart.plot.title {
                    data = chart_out::retitle(name.as_str(), &data, chart.plot.title.as_deref())?;
                }
                if data != part.data() {
                    written.push((name, content_type, data));
                }
            }
        }

        if let Some(name) = &located.shared_strings {
            if let Some(part) = self.package.part(name) {
                let content_type = part.content_type.clone();
                let data = strings_out::rewrite(name.as_str(), part.data(), &sst)?;
                written.push((name.clone(), content_type, data));
            }
        }

        if let Some(name) = &located.styles {
            if let Some(part) = self.package.part(name) {
                let content_type = part.content_type.clone();
                let additions =
                    styles_out::additions(name.as_str(), part.data(), &self.workbook.styles)?;
                let data = styles_out::rewrite(name.as_str(), part.data(), &additions)?;
                written.push((name.clone(), content_type, data));
            }
        }

        for (name, content_type, data) in written {
            self.package.put_part(name, &content_type, data);
        }
        Ok(())
    }

    /// Brings the file's `<sheets>` list in line with the model's, and returns
    /// the part each model sheet's cells belong in.
    ///
    /// The correspondence is by **part**, never by position: a sheet the user
    /// dragged is the same sheet in a different place, and pairing by index
    /// would write its cells into its neighbour. `Sheet::part` is what carries
    /// that identity, and a sheet without one has never been written.
    ///
    /// Nothing is rewritten unless something actually differs. A save with no
    /// structural change must leave `workbook.xml` and the relationship part
    /// exactly as they were — the no-edit fidelity check is precisely that
    /// question, and the cheapest way to keep passing it honestly is not to
    /// touch what has not changed.
    fn reconcile_sheets(
        &mut self,
        located: &parts::WorkbookParts,
        meta: &crate::workbook_part::WorkbookMeta,
    ) -> Result<Vec<Option<PartName>>> {
        // What the file says today: part name -> its entry.
        let mut by_part: BTreeMap<String, &crate::workbook_part::SheetEntry> = BTreeMap::new();
        for entry in &meta.sheets {
            let Some(name) = entry
                .rel_id
                .as_deref()
                .and_then(|id| located.sheet_target(id))
                .map(|(_, name)| name.as_str().to_string())
            else {
                continue;
            };
            by_part.insert(name, entry);
        }

        let mut rels = self.package.relationships(&located.workbook)?;
        let mut next_sheet_id = meta.sheets.iter().map(|e| e.sheet_id).max().unwrap_or(0) + 1;
        let existing_parts: Vec<PartName> = self.package.parts().map(|p| p.name.clone()).collect();
        let directory = by_part
            .keys()
            .next()
            .and_then(|p| PartName::new(p).ok())
            .map_or_else(
                || format!("{}/worksheets", trim_root(located.workbook.parent())),
                |p| p.parent().to_string(),
            );

        let mut entries: Vec<workbook_out::Entry> = Vec::new();
        let mut targets: Vec<Option<PartName>> = Vec::new();
        let mut claimed: Vec<String> = Vec::new();
        let mut authored: Vec<(PartName, Vec<u8>)> = Vec::new();
        let mut taken = existing_parts.clone();

        for index in 0..self.workbook.sheets.len() {
            let known = self.workbook.sheets[index]
                .part
                .as_deref()
                .and_then(|p| by_part.get(p).map(|e| (p.to_string(), *e)));

            match known {
                Some((part, entry)) => {
                    entries.push(workbook_out::Entry {
                        name: self.workbook.sheets[index].name.clone(),
                        rel_id: entry.rel_id.clone().unwrap_or_default(),
                        sheet_id: if entry.sheet_id == 0 {
                            next_sheet_id
                        } else {
                            entry.sheet_id
                        },
                        hidden: self.workbook.sheets[index].hidden,
                    });
                    if entry.sheet_id == 0 {
                        next_sheet_id += 1;
                    }
                    targets.push(PartName::new(&part).ok());
                    claimed.push(part);
                }
                // A sheet the file has never seen. It gets a part of its own,
                // authored empty; the cells go in through the same splice
                // writer that edits a worksheet Excel wrote.
                None => {
                    let name = workbook_out::free_part_name(&taken, &directory)?;
                    taken.push(name.clone());
                    let rel_id = rels.next_id();
                    rels.insert(Relationship {
                        id: rel_id.clone(),
                        rel_type: format!("{REL_BASE}/worksheet"),
                        target: relative_to(&located.workbook, &name),
                        mode: TargetMode::Internal,
                    });
                    entries.push(workbook_out::Entry {
                        name: self.workbook.sheets[index].name.clone(),
                        rel_id,
                        sheet_id: next_sheet_id,
                        hidden: self.workbook.sheets[index].hidden,
                    });
                    next_sheet_id += 1;
                    authored.push((name.clone(), workbook_out::empty_worksheet()));
                    self.workbook.sheets[index].part = Some(name.as_str().to_string());
                    claimed.push(name.as_str().to_string());
                    targets.push(Some(name));
                }
            }
        }

        // Whatever the file still lists and the model no longer holds.
        let dropped: Vec<(String, Option<String>)> = by_part
            .iter()
            .filter(|(part, _)| !claimed.contains(part))
            .map(|(part, entry)| (part.clone(), entry.rel_id.clone()))
            .collect();

        let unchanged = authored.is_empty()
            && dropped.is_empty()
            && entries.len() == meta.sheets.len()
            && entries.iter().zip(&meta.sheets).all(|(now, was)| {
                now.name == was.name
                    && now.hidden == was.hidden
                    && now.rel_id.as_str() == was.rel_id.as_deref().unwrap_or("")
            })
            && self.workbook.defined_names == meta.defined_names;
        if unchanged {
            return Ok(targets);
        }

        for (part, rel_id) in dropped {
            if let Some(id) = rel_id {
                rels.remove(&id);
            }
            if let Ok(name) = PartName::new(&part) {
                self.package.remove_part(&name);
            }
        }
        for (name, data) in authored {
            self.package.put_part(name, WORKSHEET_TYPE, data);
        }

        let rels_name = located.workbook.rels_part();
        let rels_type = self
            .package
            .part(&rels_name)
            .map(|p| p.content_type.clone())
            .unwrap_or_else(|| {
                "application/vnd.openxmlformats-package.relationships+xml".to_string()
            });
        self.package.put_part(rels_name, &rels_type, rels.to_xml());

        if let Some(part) = self.package.part(&located.workbook) {
            let content_type = part.content_type.clone();
            let data = workbook_out::rewrite(
                located.workbook.as_str(),
                part.data(),
                &entries,
                &self.workbook,
            )?;
            self.package
                .put_part(located.workbook.clone(), &content_type, data);
        }

        Ok(targets)
    }
}

/// A part name as a relationship target relative to `from`.
///
/// Relative where it can be, because that is what Excel writes and what keeps a
/// package movable; absolute otherwise, which is equally legal and is what a
/// part outside the owner's directory needs.
fn relative_to(from: &PartName, target: &PartName) -> String {
    let dir = from.parent();
    let prefix = if dir == "/" {
        "/".to_string()
    } else {
        format!("{dir}/")
    };
    match target.as_str().strip_prefix(&prefix) {
        Some(rest) if !rest.contains("../") => rest.to_string(),
        _ => target.as_str().to_string(),
    }
}

/// `/` becomes the empty string so a directory can be built by concatenation.
fn trim_root(dir: &str) -> &str {
    if dir == "/" {
        ""
    } else {
        dir
    }
}
