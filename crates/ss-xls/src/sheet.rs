//! One worksheet substream: the cells, and the geometry around them.

use std::collections::BTreeMap;

use ss_model::style::StyleId;
use ss_model::{Cell, CellRange, CellRef, CellValue, Formula, FormulaKind, Sheet, StringTable};

use crate::error::Result;
use crate::formula::Parsed;
use crate::globals::{BoundSheet, Globals};
use crate::record::{f64_at, kind, rk, u16_at, Records};

/// A shared or array formula waiting for the cells that use it.
#[derive(Default)]
struct Group {
    /// Anchor cell to decompiled text.
    shared: BTreeMap<CellRef, String>,
    /// Anchor cell to the range the array covers.
    arrays: BTreeMap<CellRef, (String, CellRange)>,
    /// Cells whose own expression was only a pointer at an anchor.
    followers: Vec<(CellRef, CellRef)>,
}

pub(crate) fn read(
    stream: &[u8],
    meta: &BoundSheet,
    globals: &Globals,
    strings: &mut StringTable,
) -> Result<Sheet> {
    let mut sheet = Sheet::new(meta.name.clone());
    sheet.kind = meta.kind;
    sheet.hidden = meta.hidden;

    // A chart sheet has no cell grid; its substream holds drawing records this
    // reader does not model. The tab still has to exist, because a defined
    // name's sheet index counts it.
    if !meta.kind.has_grid() {
        return Ok(sheet);
    }

    let sheet_names: Vec<String> = globals.sheets.iter().map(|s| s.name.clone()).collect();
    let context = crate::formula::Context {
        sheets: &sheet_names,
        xti: &globals.xti,
        internal: &globals.internal,
        names: &globals.names,
    };

    let mut records = Records::at(stream, meta.offset);
    let mut group = Group::default();
    // The cell a `STRING` record would belong to: a formula whose result is
    // text stores the text in a record of its own, afterwards.
    let mut awaiting_string: Option<CellRef> = None;
    let mut frozen: Option<CellRef> = None;
    let mut is_frozen = false;

    while let Some(record) = records.pop() {
        let body = record.body();
        if record.kind == kind::EOF {
            break;
        }
        // Only the record *immediately* after a formula carries its result, so
        // the note of which cell is waiting lasts exactly one record.
        let waiting = awaiting_string.take();

        match record.kind {
            kind::BLANK => {
                if let (Some(row), Some(col), Some(xf)) =
                    (u16_at(&body, 0), u16_at(&body, 2), u16_at(&body, 4))
                {
                    put(&mut sheet, row, col, CellValue::Blank, xf);
                }
            }
            kind::MULBLANK => {
                let (Some(row), Some(first)) = (u16_at(&body, 0), u16_at(&body, 2)) else {
                    continue;
                };
                for i in 0..body.len().saturating_sub(6) / 2 {
                    if let Some(xf) = u16_at(&body, 4 + i * 2) {
                        put(&mut sheet, row, first + i as u16, CellValue::Blank, xf);
                    }
                }
            }
            kind::NUMBER => {
                if let (Some(row), Some(col), Some(xf), Some(value)) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    u16_at(&body, 4),
                    f64_at(&body, 6),
                ) {
                    put(&mut sheet, row, col, CellValue::Number(value), xf);
                }
            }
            kind::RK => {
                if let (Some(row), Some(col), Some(xf), Some(value)) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    u16_at(&body, 4),
                    crate::record::u32_at(&body, 6),
                ) {
                    put(&mut sheet, row, col, CellValue::Number(rk(value)), xf);
                }
            }
            kind::MULRK => {
                let (Some(row), Some(first)) = (u16_at(&body, 0), u16_at(&body, 2)) else {
                    continue;
                };
                for i in 0..body.len().saturating_sub(6) / 6 {
                    let at = 4 + i * 6;
                    if let (Some(xf), Some(value)) =
                        (u16_at(&body, at), crate::record::u32_at(&body, at + 2))
                    {
                        put(
                            &mut sheet,
                            row,
                            first + i as u16,
                            CellValue::Number(rk(value)),
                            xf,
                        );
                    }
                }
            }
            kind::LABELSST => {
                if let (Some(row), Some(col), Some(xf), Some(index)) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    u16_at(&body, 4),
                    crate::record::u32_at(&body, 6),
                ) {
                    let text = globals
                        .strings
                        .get(index as usize)
                        .map(String::as_str)
                        .unwrap_or_default();
                    let value = CellValue::Text(strings.intern(text));
                    put(&mut sheet, row, col, value, xf);
                }
            }
            // A string stored in the cell record rather than in the shared
            // table. Excel 97 writes these for rich text.
            kind::LABEL | kind::RSTRING => {
                if let (Some(row), Some(col), Some(xf), Some((text, _))) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    u16_at(&body, 4),
                    crate::string::long(&body, 6),
                ) {
                    let value = CellValue::Text(strings.intern(&text));
                    put(&mut sheet, row, col, value, xf);
                }
            }
            kind::BOOLERR => {
                if let (Some(row), Some(col), Some(xf), Some(value), Some(is_error)) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    u16_at(&body, 4),
                    body.get(6).copied(),
                    body.get(7).copied(),
                ) {
                    let value = if is_error != 0 {
                        CellValue::Error(
                            ss_model::CellError::from_code(crate::formula::error_code(value))
                                .unwrap_or(ss_model::CellError::Value),
                        )
                    } else {
                        CellValue::Bool(value != 0)
                    };
                    put(&mut sheet, row, col, value, xf);
                }
            }
            kind::FORMULA => {
                awaiting_string = formula(&mut sheet, &body, &context, &mut group, strings);
            }
            kind::STRING => {
                if let (Some(at), Some((text, _))) = (waiting, crate::string::long(&body, 0)) {
                    if let Some(cell) = sheet.cells.get(at).copied() {
                        sheet.set(
                            at,
                            Cell {
                                value: CellValue::Text(strings.intern(&text)),
                                ..cell
                            },
                        );
                    }
                }
            }
            kind::SHRFMLA => {
                // The header is ten bytes and the anchor is its top-left corner:
                // the offsets inside are relative to whichever cell is using it.
                if let (Some(row), Some(col), Some(cce)) =
                    (u16_at(&body, 0), body.get(4).copied(), u16_at(&body, 8))
                {
                    let anchor = CellRef::new(row as u32, col as u32);
                    if let Some(rgce) = body.get(10..10 + cce as usize) {
                        if let Parsed::Text(text) = crate::formula::parse(rgce, anchor, &context) {
                            group.shared.insert(anchor, text);
                        }
                    }
                }
            }
            kind::ARRAY => {
                if let (
                    Some(first_row),
                    Some(last_row),
                    Some(first_col),
                    Some(last_col),
                    Some(cce),
                ) = (
                    u16_at(&body, 0),
                    u16_at(&body, 2),
                    body.get(4).copied(),
                    body.get(5).copied(),
                    u16_at(&body, 12),
                ) {
                    let anchor = CellRef::new(first_row as u32, first_col as u32);
                    let range =
                        CellRange::new(anchor, CellRef::new(last_row as u32, last_col as u32));
                    if let Some(rgce) = body.get(14..14 + cce as usize) {
                        if let Parsed::Text(text) = crate::formula::parse(rgce, anchor, &context) {
                            group.arrays.insert(anchor, (text, range));
                        }
                    }
                }
            }
            kind::MERGEDCELLS => {
                let count = u16_at(&body, 0).unwrap_or(0) as usize;
                for i in 0..count {
                    let at = 2 + i * 8;
                    if let (Some(r1), Some(r2), Some(c1), Some(c2)) = (
                        u16_at(&body, at),
                        u16_at(&body, at + 2),
                        u16_at(&body, at + 4),
                        u16_at(&body, at + 6),
                    ) {
                        sheet.merges.push(CellRange::new(
                            CellRef::new(r1 as u32, c1 as u32),
                            CellRef::new(r2 as u32, c2 as u32),
                        ));
                    }
                }
            }
            kind::ROW => row_geometry(&mut sheet, &body),
            kind::COLINFO => column_geometry(&mut sheet, &body),
            kind::WINDOW2 => {
                let flags = u16_at(&body, 0).unwrap_or(0);
                sheet.view.gridlines = flags & 0x0002 != 0;
                sheet.view.headings = flags & 0x0004 != 0;
                is_frozen = flags & 0x0008 != 0;
                if let (Some(row), Some(col)) = (u16_at(&body, 2), u16_at(&body, 4)) {
                    sheet.view.top_left = Some(CellRef::new(row as u32, col as u32));
                }
            }
            kind::PANE => {
                // The split is in cells when the pane is frozen and in twips
                // when it is only split, and the two are not interchangeable.
                if let (Some(cols), Some(rows)) = (u16_at(&body, 0), u16_at(&body, 2)) {
                    frozen = Some(CellRef::new(rows as u32, cols as u32));
                }
            }
            kind::SCL => {
                if let Some(zoom) = crate::style::zoom(&body) {
                    sheet.view.zoom = zoom;
                }
            }
            kind::SELECTION => {
                // The first reference in the list is the active cell.
                if let (Some(row), Some(col)) = (u16_at(&body, 1), body.get(3).copied()) {
                    sheet.view.selection = Some(CellRef::new(row as u32, col as u32));
                }
            }
            _ => {}
        }
    }

    if is_frozen {
        sheet.frozen = frozen.filter(|at| at.row > 0 || at.col > 0);
    }
    resolve(&mut sheet, group);
    Ok(sheet)
}

/// A `FORMULA` record: the cached result, then the expression behind it.
///
/// Returns the cell if a `STRING` record should follow with its result.
fn formula(
    sheet: &mut Sheet,
    body: &[u8],
    context: &crate::formula::Context<'_>,
    group: &mut Group,
    strings: &mut StringTable,
) -> Option<CellRef> {
    let (row, col, xf) = (u16_at(body, 0)?, u16_at(body, 2)?, u16_at(body, 4)?);
    let at = CellRef::new(row as u32, col as u32);
    let cached = body.get(6..14)?;

    // Sentinel `FFFF` in the last two bytes means the eight bytes are not a
    // double at all — a real double with that exponent would be a NaN, which is
    // how the format gets away with it.
    let mut wants_string = false;
    let value = if cached[6] == 0xFF && cached[7] == 0xFF {
        match cached[0] {
            0 => {
                wants_string = true;
                CellValue::Text(strings.intern(""))
            }
            1 => CellValue::Bool(cached[2] != 0),
            2 => CellValue::Error(
                ss_model::CellError::from_code(crate::formula::error_code(cached[2]))
                    .unwrap_or(ss_model::CellError::Value),
            ),
            _ => CellValue::Text(strings.intern("")),
        }
    } else {
        CellValue::Number(f64_at(body, 6)?)
    };

    let cce = u16_at(body, 20)? as usize;
    let rgce = body.get(22..22 + cce).unwrap_or(&[]);
    let parsed = crate::formula::parse(rgce, at, context);

    let mut cell = Cell {
        value,
        style: StyleId(xf as u32),
        formula: None,
    };
    match parsed {
        Parsed::Text(text) => {
            cell.formula = Some(sheet.push_formula(Formula::normal(text)));
        }
        Parsed::Follows(anchor) => group.followers.push((at, anchor)),
        // The value is real and cached; only the expression is lost.
        Parsed::Unreadable => {}
    }
    sheet.set(at, cell);
    wants_string.then_some(at)
}

/// Attach the shared and array formulas once every record that describes them
/// has been seen. A `SHRFMLA` follows the first cell that uses it, and the rest
/// of its cells come later still.
fn resolve(sheet: &mut Sheet, group: Group) {
    let mut index_of: BTreeMap<CellRef, u32> = BTreeMap::new();
    for (n, (anchor, text)) in group.shared.iter().enumerate() {
        let index = n as u32;
        index_of.insert(*anchor, index);
        let id = sheet.push_formula(Formula {
            text: text.clone(),
            kind: FormulaKind::Shared { index, range: None },
        });
        if let Some(cell) = sheet.cells.get(*anchor).copied() {
            sheet.set(
                *anchor,
                Cell {
                    formula: Some(id),
                    ..cell
                },
            );
        }
    }

    for (anchor, (text, range)) in &group.arrays {
        let id = sheet.push_formula(Formula {
            text: text.clone(),
            kind: FormulaKind::Array { range: *range },
        });
        if let Some(cell) = sheet.cells.get(*anchor).copied() {
            sheet.set(
                *anchor,
                Cell {
                    formula: Some(id),
                    ..cell
                },
            );
        }
    }

    for (at, anchor) in group.followers {
        // The anchor of an array formula is the only cell that carries text;
        // the cells it covers keep their values and nothing else.
        let Some(index) = index_of.get(&anchor).copied() else {
            continue;
        };
        if at == anchor {
            continue;
        }
        let id = sheet.push_formula(Formula {
            text: String::new(),
            kind: FormulaKind::SharedFollower { index },
        });
        if let Some(cell) = sheet.cells.get(at).copied() {
            sheet.set(
                at,
                Cell {
                    formula: Some(id),
                    ..cell
                },
            );
        }
    }
}

fn put(sheet: &mut Sheet, row: u16, col: u16, value: CellValue, xf: u16) {
    sheet.set(
        CellRef::new(row as u32, col as u32),
        Cell {
            value,
            style: StyleId(xf as u32),
            formula: None,
        },
    );
}

fn row_geometry(sheet: &mut Sheet, body: &[u8]) {
    let Some(row) = u16_at(body, 0) else { return };
    let row = row as u32;
    let height = u16_at(body, 6).unwrap_or(0) & 0x7FFF;
    let flags = crate::record::u32_at(body, 12).unwrap_or(0);

    // Only a height the user set is kept. Storing the automatic ones would
    // freeze every row at whatever the version of Excel that wrote the file
    // measured its default font to be.
    if flags & 0x0040 != 0 {
        sheet.row_heights.insert(row, height as f64 / 20.0);
    }
    if flags & 0x0020 != 0 {
        sheet.row_heights.insert(row, 0.0);
    }
    // A row-wide style only counts when the file says the row was formatted.
    if flags & 0x0080 != 0 {
        let style = StyleId((flags >> 16) & 0x0FFF);
        if style != StyleId::DEFAULT {
            sheet.row_styles.insert(row, style);
        }
    }
}

fn column_geometry(sheet: &mut Sheet, body: &[u8]) {
    let (Some(first), Some(last), Some(width), Some(xf), Some(flags)) = (
        u16_at(body, 0),
        u16_at(body, 2),
        u16_at(body, 4),
        u16_at(body, 6),
        u16_at(body, 8),
    ) else {
        return;
    };
    // 0x00FF is the "to the last column" marker; the model stores columns one
    // at a time and would otherwise write sixteen thousand entries.
    let last = last.min(255);
    let style = StyleId(xf as u32);
    for col in first..=last {
        let col = col as u32;
        sheet.column_widths.insert(
            col,
            if flags & 0x0001 != 0 {
                0.0
            } else {
                width as f64 / 256.0
            },
        );
        if style != StyleId::DEFAULT {
            sheet.column_styles.insert(col, style);
        }
    }
}
