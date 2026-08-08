//! Writing a worksheet by editing the one that was read.
//!
//! A worksheet part contains far more than its cells: autofilters, conditional
//! formatting, data validation, hyperlinks, sheet protection, the anchor tying a
//! chart to the grid, page setup, and whatever a later version of Excel decided
//! to put in `extLst`. Reprinting the part from our model would delete all of
//! it. So the original bytes *are* the document, and this walks them replacing
//! only `<sheetData>` — and inside `<sheetData>`, only the cells that actually
//! differ from what the file already says.
//!
//! The consequence worth stating plainly: a cell nobody touched is written back
//! byte for byte, including a cached value we would have rounded differently and
//! attributes we do not understand. A cell that *was* touched keeps its own
//! start tag and gets a new body. That is why the no-edit round-trip check is
//! still exact now that a serializer exists — it is this same code path, having
//! found nothing to change.

use quick_xml::events::{BytesStart, Event};

use ss_model::formula::{Formula, FormulaKind};
use ss_model::{Cell, CellRange, CellRef, Sheet, StringTable};

use crate::error::Result;
use crate::write::cells::{number, Content, Val};
use crate::write::splice::{close, escape_text, open, prefix_of, raw_attr, retag, Set, Splicer};
use crate::write::strings_out::Sst;
use crate::xml::{end_local_name, local_name, parse_bool, parse_f64, parse_u32, push_text};

/// Everything the cell writer needs that is not the cell.
pub(crate) struct Context<'a> {
    pub sheet: &'a Sheet,
    pub strings: &'a StringTable,
    pub sst: &'a mut Sst,
    /// Write every cell from the model, even one that has not changed.
    ///
    /// A save never does this. The harness does, to ask the stronger question:
    /// not "did the bytes we copied survive" but "could we have written them".
    pub regenerate: bool,
}

/// Rewrites a worksheet part so its cells match `ctx.sheet`.
pub(crate) fn rewrite(part: &str, data: &[u8], ctx: &mut Context<'_>) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 8);
    let mut splicer = Splicer::new(part, data);
    let mut pending = Rows::new(ctx.sheet);
    let mut in_data = false;
    let mut prefix: Vec<u8> = Vec::new();
    let mut next_row = 0u32;

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) if local_name(e) == b"sheetData" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
                in_data = true;
                next_row = 0;
            }

            Event::Empty(e) if local_name(e) == b"sheetData" => {
                prefix = prefix_of(e);
                if pending.peek().is_none() {
                    out.extend_from_slice(splicer.bytes(span));
                } else {
                    // The sheet was empty and is not any more, so the element
                    // has to grow a body.
                    out.extend_from_slice(&retag(e, &[], false));
                    flush(&mut out, &mut pending, u32::MAX, &prefix, ctx);
                    close(&mut out, &prefix, b"sheetData");
                }
            }

            Event::Start(e) | Event::Empty(e) if in_data && local_name(e) == b"row" => {
                let index = row_index(e, next_row);
                next_row = index.saturating_add(1);
                let row = if matches!(event, Event::Empty(_)) {
                    FileRow::new(e.to_owned(), span, index)
                } else {
                    collect_row(&mut splicer, e.to_owned(), span, index, ctx.sst)?
                };
                // Model rows the file has no element for go in ahead of it.
                flush(&mut out, &mut pending, index, &prefix, ctx);
                write_row(&mut out, &splicer, &row, &mut pending, &prefix, ctx);
            }

            Event::End(e) if end_local_name(e) == b"sheetData" => {
                flush(&mut out, &mut pending, u32::MAX, &prefix, ctx);
                out.extend_from_slice(splicer.bytes(span));
                in_data = false;
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    Ok(out)
}

/// The model's cells, walked in the same row-major order the file is in.
struct Rows<'a> {
    iter: std::iter::Peekable<Box<dyn Iterator<Item = (CellRef, &'a Cell)> + 'a>>,
}

impl<'a> Rows<'a> {
    fn new(sheet: &'a Sheet) -> Self {
        let iter: Box<dyn Iterator<Item = (CellRef, &'a Cell)> + 'a> = Box::new(sheet.cells.iter());
        Rows {
            iter: iter.peekable(),
        }
    }

    fn peek(&mut self) -> Option<u32> {
        self.iter.peek().map(|(at, _)| at.row)
    }

    /// The model's cells in `row`, in column order and already lowered.
    fn take(&mut self, row: u32, ctx: &Context<'_>) -> Vec<(u32, Content)> {
        let mut out = Vec::new();
        while self.peek() == Some(row) {
            let (at, cell) = self.iter.next().expect("just peeked");
            out.push((at.col, content_of(cell, ctx)));
        }
        out
    }
}

fn content_of(cell: &Cell, ctx: &Context<'_>) -> Content {
    Content {
        value: Val::of(cell, ctx.strings),
        style: cell.style.0,
        formula: cell.formula.and_then(|id| ctx.sheet.formula(id)).cloned(),
    }
}

/// Emits every model row before `limit` that the file has no element for.
fn flush(
    out: &mut Vec<u8>,
    pending: &mut Rows<'_>,
    limit: u32,
    prefix: &[u8],
    ctx: &mut Context<'_>,
) {
    while let Some(row) = pending.peek() {
        if row >= limit {
            return;
        }
        let cells = pending.take(row, ctx);
        write_new_row(out, row, &cells, prefix, ctx);
    }
}

fn write_new_row(
    out: &mut Vec<u8>,
    row: u32,
    cells: &[(u32, Content)],
    prefix: &[u8],
    ctx: &mut Context<'_>,
) {
    let mut sets = vec![
        Set::to(b"r", (row + 1).to_string()),
        Set::maybe(b"spans", spans_of(cells.iter().map(|(c, _)| *c))),
    ];
    sets.extend(geometry_sets(ctx.sheet, row));
    open(out, prefix, b"row", &sets, false);
    for (col, content) in cells {
        write_cell(out, CellRef::new(row, *col), content, None, prefix, ctx);
    }
    close(out, prefix, b"row");
}

/// Emits a row the file already has, changing only what differs.
fn write_row(
    out: &mut Vec<u8>,
    splicer: &Splicer<'_>,
    row: &FileRow,
    pending: &mut Rows<'_>,
    prefix: &[u8],
    ctx: &mut Context<'_>,
) {
    let model = pending.take(row.index, ctx);
    let merged = merge(&row.cells, &model);

    let unchanged = !ctx.regenerate
        && row.geometry_matches(ctx.sheet)
        && merged.iter().all(|slot| match (slot.file, slot.model) {
            (Some(f), Some(m)) => f.content.same(m),
            (None, None) => true,
            _ => false,
        });
    if unchanged {
        out.extend_from_slice(splicer.bytes(row.span.clone()));
        return;
    }

    let mut sets = vec![
        Set::to(b"r", (row.index + 1).to_string()),
        Set::maybe(
            b"spans",
            widen(
                row.spans,
                spans_of(merged.iter().filter(|s| s.model.is_some()).map(|s| s.col)),
            ),
        ),
    ];
    // Only when the model disagrees with the file: a `ht` with no
    // `customHeight` is an auto-fit measurement the producer made, which the
    // model does not carry and must not delete.
    if !row.geometry_matches(ctx.sheet) {
        sets.extend(geometry_sets(ctx.sheet, row.index));
        for name in [b"ht".as_slice(), b"customHeight", b"hidden"] {
            if !sets.iter().any(|s| s.name == name) {
                sets.push(Set::off(name));
            }
        }
    }
    out.extend_from_slice(&retag(&row.start, &sets, false));

    for slot in &merged {
        let Some(content) = slot.model else {
            // Deleted, and its leading whitespace goes with it so a row that
            // empties out does not fill up with blanks.
            continue;
        };
        if let Some(file) = slot.file {
            out.extend_from_slice(splicer.bytes(file.lead.clone()));
            if !ctx.regenerate && file.content.same(content) {
                out.extend_from_slice(splicer.bytes(file.span.clone()));
                continue;
            }
        }
        write_cell(
            out,
            CellRef::new(row.index, slot.col),
            content,
            slot.file.map(|f| (&f.start, f.formula_start.as_ref())),
            prefix,
            ctx,
        );
    }
    out.extend_from_slice(splicer.bytes(row.tail.clone()));
    close(out, prefix, b"row");
}

/// A column, and what each side has at it.
struct Slot<'a> {
    col: u32,
    file: Option<&'a FileCell>,
    model: Option<&'a Content>,
}

/// Walks both sides in column order — the order they are already in.
fn merge<'a>(file: &'a [FileCell], model: &'a [(u32, Content)]) -> Vec<Slot<'a>> {
    let mut out = Vec::with_capacity(file.len().max(model.len()));
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        let left = file.get(i).map(|c| c.at.col);
        let right = model.get(j).map(|(c, _)| *c);
        match (left, right) {
            (None, None) => return out,
            (Some(a), Some(b)) if a == b => {
                out.push(Slot {
                    col: a,
                    file: Some(&file[i]),
                    model: Some(&model[j].1),
                });
                i += 1;
                j += 1;
            }
            (Some(a), right) if right.is_none_or(|b| a < b) => {
                out.push(Slot {
                    col: a,
                    file: Some(&file[i]),
                    model: None,
                });
                i += 1;
            }
            (_, Some(b)) => {
                out.push(Slot {
                    col: b,
                    file: None,
                    model: Some(&model[j].1),
                });
                j += 1;
            }
            (Some(_), None) => unreachable!("covered by the arm above"),
        }
    }
}

/// `spans` says which columns a row occupies, as an optimization hint.
fn spans_of(mut columns: impl Iterator<Item = u32>) -> Option<String> {
    let first = columns.next()?;
    let last = columns.fold(first, u32::max);
    Some(format!("{}:{}", first + 1, last + 1))
}

/// The file's span widened to cover ours, rather than replaced by ours.
///
/// Excel does not write the span of the row. It writes the span of the
/// *sixteen-row block* the row belongs to, so a row holding A and B alone comes
/// back as `1:4` if some other row in its block reaches column D. Recomputing
/// the attribute would therefore change a row nobody edited — and there is no
/// gain to weigh against that, because the hint's only requirement is that it
/// covers the cells. Widening satisfies it and touches nothing else.
fn widen(file: Option<(u32, u32)>, ours: Option<String>) -> Option<String> {
    let Some((from, to)) = file else { return ours };
    let Some(ours) = ours else {
        return Some(format!("{from}:{to}")); // the row emptied out; leave the hint
    };
    let (a, b) = parse_spans(&ours).expect("we wrote it");
    Some(format!("{}:{}", from.min(a), to.max(b)))
}

/// `"1:4"` as the pair it names, one-based as the file writes it.
fn parse_spans(text: &str) -> Option<(u32, u32)> {
    let (a, b) = text.split_once(':')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// The height attributes the model implies for `row`.
fn geometry_sets(sheet: &Sheet, row: u32) -> Vec<Set<'static>> {
    match sheet.row_heights.get(&row) {
        // Zero is how the reader records a hidden row: a hidden row has no
        // height to speak of.
        Some(h) if *h == 0.0 => vec![Set::to(b"hidden", "1")],
        Some(h) => vec![Set::to(b"ht", h.to_string()), Set::to(b"customHeight", "1")],
        None => Vec::new(),
    }
}

/// One `<c>` element of the file, with the bytes it came from.
struct FileCell {
    at: CellRef,
    /// Whitespace and comments between the previous cell and this one.
    lead: std::ops::Range<usize>,
    /// The whole element, start tag through end tag.
    span: std::ops::Range<usize>,
    start: BytesStart<'static>,
    /// The `<f>` start tag, if the cell had one.
    ///
    /// Kept for the same reason `start` is: `ca="1"` marks a formula Excel
    /// cannot tell is volatile by looking at it, and a cell that lost the
    /// attribute is a cell Excel quietly stops recalculating.
    formula_start: Option<BytesStart<'static>>,
    content: Content,
}

/// One `<row>` element of the file.
struct FileRow {
    index: u32,
    start: BytesStart<'static>,
    /// The whole element, for the common case where nothing in it changed.
    span: std::ops::Range<usize>,
    cells: Vec<FileCell>,
    /// Whitespace between the last cell and `</row>`.
    tail: std::ops::Range<usize>,
    height: Option<f64>,
    custom_height: bool,
    hidden: bool,
    /// The `spans` hint as the file wrote it, one-based.
    spans: Option<(u32, u32)>,
}

impl FileRow {
    fn new(start: BytesStart<'static>, span: std::ops::Range<usize>, index: u32) -> Self {
        let height = raw_attr(&start, b"ht").and_then(|a| parse_f64(&a.value));
        let custom_height = raw_attr(&start, b"customHeight")
            .and_then(|a| parse_bool(&a.value))
            .unwrap_or(false);
        let hidden = raw_attr(&start, b"hidden")
            .and_then(|a| parse_bool(&a.value))
            .unwrap_or(false);
        let spans = raw_attr(&start, b"spans")
            .and_then(|a| parse_spans(&String::from_utf8_lossy(&a.value)));
        let tail = span.end..span.end;
        FileRow {
            index,
            start,
            span,
            cells: Vec::new(),
            tail,
            height,
            custom_height,
            hidden,
            spans,
        }
    }

    /// Whether the model's height for this row is what the file already says.
    fn geometry_matches(&self, sheet: &Sheet) -> bool {
        let stated = if self.hidden {
            Some(0.0)
        } else if self.custom_height {
            self.height
        } else {
            None
        };
        stated == sheet.row_heights.get(&self.index).copied()
    }
}

fn row_index(e: &BytesStart<'_>, fallback: u32) -> u32 {
    // `r` is optional; without it the position is implied by document order.
    raw_attr(e, b"r")
        .and_then(|a| parse_u32(&a.value))
        .filter(|n| *n >= 1)
        .map(|n| n - 1)
        .unwrap_or(fallback)
}

/// Reads a `<row>` element through its end tag, keeping every byte.
fn collect_row(
    splicer: &mut Splicer<'_>,
    start: BytesStart<'static>,
    span: std::ops::Range<usize>,
    index: u32,
    sst: &Sst,
) -> Result<FileRow> {
    let from = span.start;
    let mut row = FileRow::new(start, span.clone(), index);
    let mut gap = span.end..span.end;
    let mut col = 0u32;
    let mut end = span.end;

    while let Some((event, span)) = splicer.next()? {
        end = span.end;
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"c" => {
                let empty = matches!(event, Event::Empty(_));
                let mut cell = collect_cell(splicer, e.to_owned(), span, empty, &mut col, sst)?;
                cell.lead = std::mem::replace(&mut gap, 0..0);
                end = cell.span.end;
                gap = end..end;
                row.cells.push(cell);
            }
            Event::End(e) if end_local_name(e) == b"row" => {
                row.tail = gap;
                row.span = from..span.end;
                return Ok(row);
            }
            _ => gap.end = span.end,
        }
    }

    // No `</row>`: the part is truncated. Keep what there was.
    row.tail = gap;
    row.span = from..end;
    Ok(row)
}

/// Reads one `<c>` element and everything inside it.
fn collect_cell(
    splicer: &mut Splicer<'_>,
    start: BytesStart<'static>,
    span: std::ops::Range<usize>,
    empty: bool,
    col: &mut u32,
    sst: &Sst,
) -> Result<FileCell> {
    let at = raw_attr(&start, b"r")
        .and_then(|a| CellRef::from_a1(&String::from_utf8_lossy(&a.value)))
        .unwrap_or(CellRef::new(0, *col));
    *col = at.col.saturating_add(1);

    let style = raw_attr(&start, b"s")
        .and_then(|a| parse_u32(&a.value))
        .unwrap_or(0);
    let kind = raw_attr(&start, b"t").map(|a| a.value.into_owned());

    let mut cell = FileCell {
        at,
        lead: 0..0,
        span: span.clone(),
        start,
        formula_start: None,
        content: Content {
            value: Val::Blank,
            style,
            formula: None,
        },
    };
    if empty {
        return Ok(cell);
    }

    let from = span.start;
    let mut value = String::new();
    let mut formula_text = String::new();
    let mut formula_kind: Option<FormulaKind> = None;
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_phonetic = 0usize;
    let mut end = span.end;

    while let Some((event, span)) = splicer.next()? {
        end = span.end;
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"v" => in_value = true,
                // `<t>` inside `<is>`, an inline string. Phonetic guides carry
                // one too, and it is pronunciation rather than content.
                b"t" if in_phonetic == 0 => in_value = true,
                b"rPh" => in_phonetic += 1,
                b"f" => {
                    formula_kind = Some(formula_kind_of(e));
                    cell.formula_start = Some(e.to_owned());
                    in_formula = !matches!(event, Event::Empty(_));
                }
                _ => {}
            },
            Event::End(e) => match end_local_name(e) {
                b"c" => {
                    cell.span = from..span.end;
                    cell.content.value = decode(kind.as_deref(), &value, sst);
                    cell.content.formula =
                        formula_kind.map(|kind| build_formula(kind, formula_text));
                    return Ok(cell);
                }
                b"v" | b"t" => in_value = false,
                b"f" => in_formula = false,
                b"rPh" => in_phonetic = in_phonetic.saturating_sub(1),
                _ => {}
            },
            other => {
                if in_formula {
                    push_text(&mut formula_text, other)?;
                } else if in_value && in_phonetic == 0 {
                    push_text(&mut value, other)?;
                }
            }
        }
    }

    cell.span = from..end;
    Ok(cell)
}

/// What a `<c>` element's `t` attribute and text amount to.
///
/// This repeats the reader's decoding rather than calling it, because the two
/// want different things: the reader interns text into the workbook, and a save
/// must not mutate the workbook to find out whether it needs saving.
fn decode(kind: Option<&[u8]>, text: &str, sst: &Sst) -> Val {
    match kind {
        Some(b"s") => match text.trim().parse::<usize>() {
            Ok(i) => sst
                .entry(i)
                .map(|s| Val::Text(s.to_string()))
                .unwrap_or(Val::Blank),
            Err(_) => Val::Blank,
        },
        Some(b"str") | Some(b"inlineStr") | Some(b"d") => {
            if text.is_empty() {
                Val::Blank
            } else {
                Val::Text(text.to_string())
            }
        }
        Some(b"b") => match text.trim() {
            "1" | "true" | "TRUE" => Val::Bool(true),
            "0" | "false" | "FALSE" => Val::Bool(false),
            "" => Val::Blank,
            other => Val::Text(other.to_string()),
        },
        Some(b"e") => match text.trim() {
            "" => Val::Blank,
            code => Val::Error(code.to_string()),
        },
        // Absent or unrecognized: the schema's default is a number.
        _ => match text.trim() {
            "" => Val::Blank,
            n => n
                .parse::<f64>()
                .map(Val::Number)
                .unwrap_or_else(|_| Val::Text(n.to_string())),
        },
    }
}

fn formula_kind_of(e: &BytesStart<'_>) -> FormulaKind {
    let range = raw_attr(e, b"ref").and_then(|a| parse_range(&String::from_utf8_lossy(&a.value)));
    let si = raw_attr(e, b"si")
        .and_then(|a| parse_u32(&a.value))
        .unwrap_or(0);
    match raw_attr(e, b"t").as_ref().map(|a| a.value.as_ref()) {
        Some(b"array") => match range {
            Some(range) => FormulaKind::Array { range },
            None => FormulaKind::Normal,
        },
        Some(b"shared") => match range {
            Some(range) => FormulaKind::Shared {
                index: si,
                range: Some(range),
            },
            None => FormulaKind::SharedFollower { index: si },
        },
        Some(b"dataTable") => FormulaKind::DataTable,
        _ => FormulaKind::Normal,
    }
}

fn build_formula(kind: FormulaKind, text: String) -> Formula {
    let kind = match kind {
        FormulaKind::SharedFollower { index } if !text.is_empty() => {
            FormulaKind::Shared { index, range: None }
        }
        other => other,
    };
    Formula { text, kind }
}

fn parse_range(text: &str) -> Option<CellRange> {
    let (a, b) = text.split_once(':').unwrap_or((text, text));
    Some(CellRange::new(CellRef::from_a1(a)?, CellRef::from_a1(b)?))
}

/// Writes one cell, keeping the start tag the file gave it if there was one.
///
/// Keeping the tag matters for `cm` and `vm`, the metadata indices behind
/// dynamic arrays and rich values: editing a cell is not a reason to drop them.
fn write_cell(
    out: &mut Vec<u8>,
    at: CellRef,
    content: &Content,
    original: Option<(&BytesStart<'_>, Option<&BytesStart<'static>>)>,
    prefix: &[u8],
    ctx: &mut Context<'_>,
) {
    // A data table's formula is entirely attributes we do not model, so there is
    // nothing to write for one we did not read verbatim.
    let formula = content
        .formula
        .as_ref()
        .filter(|f| !matches!(f.kind, FormulaKind::DataTable));

    let mut body = Vec::new();
    if let Some(f) = formula {
        write_formula(&mut body, f, prefix, original.and_then(|(_, f)| f));
    }

    let kind = match (&content.value, formula.is_some()) {
        (Val::Blank | Val::Number(_), _) => None,
        (Val::Bool(_), _) => Some("b"),
        (Val::Error(_), _) => Some("e"),
        // A formula's text result is stored in the cell. The shared table is for
        // literal text only: Excel never points an `<f>` cell at it.
        (Val::Text(_), true) => Some("str"),
        (Val::Text(_), false) => Some(ctx.sst.cell_type()),
    };

    match &content.value {
        Val::Blank => {}
        Val::Number(n) if n.is_finite() => push_v(&mut body, &number(*n), prefix),
        // Not a number any file can hold. `#NUM!` is what Excel shows for one.
        Val::Number(_) => push_v(&mut body, "#NUM!", prefix),
        Val::Bool(b) => push_v(&mut body, if *b { "1" } else { "0" }, prefix),
        Val::Error(e) => push_v(&mut body, &escape_text(e), prefix),
        Val::Text(text) if formula.is_some() => push_v(&mut body, &escape_text(text), prefix),
        Val::Text(text) => match ctx.sst.intern(text) {
            Some(index) => push_v(&mut body, &index.to_string(), prefix),
            None => push_inline(&mut body, text, prefix),
        },
    }

    let sets = [
        Set::to(b"r", at.to_a1()),
        Set::maybe(
            b"s",
            (content.style != 0).then(|| content.style.to_string()),
        ),
        Set::maybe(b"t", kind.map(str::to_string)),
    ];
    match original {
        Some((e, _)) => out.extend_from_slice(&retag(e, &sets, body.is_empty())),
        None => open(out, prefix, b"c", &sets, body.is_empty()),
    }
    if body.is_empty() {
        return;
    }
    out.extend_from_slice(&body);
    close(out, prefix, b"c");
}

fn write_formula(
    out: &mut Vec<u8>,
    formula: &Formula,
    prefix: &[u8],
    original: Option<&BytesStart<'static>>,
) {
    // Every attribute the kind implies, stated either way round, so that
    // retagging an existing `<f>` clears the ones that no longer apply.
    let (kind, range, index) = match &formula.kind {
        FormulaKind::Normal | FormulaKind::DataTable => (None, None, None),
        FormulaKind::Array { range } => (Some("array"), Some(*range), None),
        FormulaKind::Shared { index, range } => (Some("shared"), *range, Some(*index)),
        FormulaKind::SharedFollower { index } => (Some("shared"), None, Some(*index)),
    };
    let sets = [
        Set::maybe(b"t", kind.map(str::to_string)),
        Set::maybe(b"ref", range.map(a1_range)),
        Set::maybe(b"si", index.map(|i| i.to_string())),
    ];

    let bare = formula.text.is_empty();
    match original {
        Some(e) => out.extend_from_slice(&retag(e, &sets, bare)),
        None => open(out, prefix, b"f", &sets, bare),
    }
    if bare {
        return;
    }
    out.extend_from_slice(escape_text(&formula.text).as_bytes());
    close(out, prefix, b"f");
}

/// A range as a `ref` attribute spells it.
///
/// One cell is written as one address. A single-cell array formula — which is
/// most of them, since `{=SUM(A1:A9*B1:B9)}` occupies one cell — comes out of
/// Excel as `ref="D1"`, never `ref="D1:D1"`.
fn a1_range(range: CellRange) -> String {
    if range.start == range.end {
        return range.start.to_a1();
    }
    format!("{}:{}", range.start.to_a1(), range.end.to_a1())
}

fn push_v(out: &mut Vec<u8>, text: &str, prefix: &[u8]) {
    open(out, prefix, b"v", &[], false);
    out.extend_from_slice(text.as_bytes());
    close(out, prefix, b"v");
}

/// Text carried in the cell rather than in the shared table.
///
/// Only reached when the workbook has no shared-string part to append to.
fn push_inline(out: &mut Vec<u8>, text: &str, prefix: &[u8]) {
    open(out, prefix, b"is", &[], false);
    let space = (text.trim() != text).then(|| "preserve".to_string());
    open(out, prefix, b"t", &[Set::maybe(b"xml:space", space)], false);
    out.extend_from_slice(escape_text(text).as_bytes());
    close(out, prefix, b"t");
    close(out, prefix, b"is");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{CellValue, StyleId};

    /// A sheet whose cells came from `SHEET`, so a test can change one thing.
    const SHEET: &str = concat!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://x"><sheetData>"#,
        r#"<row r="1" spans="1:3" x14ac:dyDescent="0.3">"#,
        r#"<c r="A1" t="s"><v>0</v></c>"#,
        r#"<c r="B1" s="2" cm="1"><v>0.42709999999999998</v></c>"#,
        r#"<c r="C1"><f ca="1">RAND()</f><v>0.5</v></c></row>"#,
        r#"<row r="3" spans="1:1"><c r="A3"><v>7</v></c></row>"#,
        r#"</sheetData><autoFilter ref="A1:C3"/></worksheet>"#,
    );

    struct Fixture {
        sheet: Sheet,
        strings: StringTable,
        sst: Sst,
    }

    fn fixture() -> Fixture {
        let mut strings = StringTable::new();
        let mut sheet = Sheet::new("Data");
        let total = strings.intern("Total");
        set(&mut sheet, "A1", CellValue::Text(total), 0);
        set(&mut sheet, "B1", CellValue::Number(0.4271), 2);
        let c1 = CellRef::from_a1("C1").expect("valid");
        let id = sheet.push_formula(Formula::normal("RAND()"));
        sheet.set(
            c1,
            Cell {
                value: CellValue::Number(0.5),
                style: StyleId(0),
                formula: Some(id),
            },
        );
        set(&mut sheet, "A3", CellValue::Number(7.0), 0);

        let sst = Sst::read(
            "sharedStrings.xml",
            br#"<sst count="1" uniqueCount="1"><si><t>Total</t></si></sst>"#,
        )
        .expect("parses");
        Fixture {
            sheet,
            strings,
            sst,
        }
    }

    fn set(sheet: &mut Sheet, a1: &str, value: CellValue, style: u32) {
        sheet.set(
            CellRef::from_a1(a1).expect("valid"),
            Cell {
                value,
                style: StyleId(style),
                formula: None,
            },
        );
    }

    fn written(fixture: &mut Fixture) -> String {
        let mut ctx = Context {
            sheet: &fixture.sheet,
            strings: &fixture.strings,
            sst: &mut fixture.sst,
            regenerate: false,
        };
        let out = rewrite("sheet1.xml", SHEET.as_bytes(), &mut ctx).expect("writes");
        String::from_utf8(out).expect("utf-8")
    }

    #[test]
    fn a_sheet_nobody_edited_comes_back_byte_for_byte() {
        let mut f = fixture();
        assert_eq!(written(&mut f), SHEET);
    }

    #[test]
    fn editing_one_cell_leaves_its_neighbours_exactly_as_they_were() {
        let mut f = fixture();
        set(&mut f.sheet, "A1", CellValue::Number(42.0), 0);
        let out = written(&mut f);

        assert!(out.contains(r#"<c r="A1"><v>42</v></c>"#), "{out}");
        assert!(
            out.contains(r#"<c r="B1" s="2" cm="1"><v>0.42709999999999998</v></c>"#),
            "the cached value we would have spelled `0.4271`, and a `cm` we do \
             not model, both survive: {out}"
        );
        assert!(
            out.contains(r#"<c r="C1"><f ca="1">RAND()</f><v>0.5</v></c>"#),
            "{out}"
        );
        assert!(out.contains(r#"<autoFilter ref="A1:C3"/>"#), "{out}");
    }

    #[test]
    fn a_volatile_formula_keeps_saying_so_when_its_text_changes() {
        // `ca="1"` is how Excel is told to recalculate a formula it cannot see
        // is volatile. Losing it on an edit means the cell silently goes stale.
        let mut f = fixture();
        let c1 = CellRef::from_a1("C1").expect("valid");
        let id = f.sheet.push_formula(Formula::normal("RAND()*2"));
        f.sheet.set(
            c1,
            Cell {
                value: CellValue::Number(1.0),
                style: StyleId(0),
                formula: Some(id),
            },
        );
        let out = written(&mut f);
        assert!(out.contains(r#"<f ca="1">RAND()*2</f>"#), "{out}");
    }

    #[test]
    fn a_new_row_lands_in_document_order() {
        let mut f = fixture();
        set(&mut f.sheet, "A2", CellValue::Number(5.0), 0);
        let out = written(&mut f);
        let row2 = out.find(r#"<row r="2""#).expect("row 2 written");
        let row3 = out.find(r#"<row r="3""#).expect("row 3 still there");
        assert!(row2 < row3, "{out}");
        assert!(
            out.contains(r#"<row r="2" spans="1:1"><c r="A2"><v>5</v></c></row>"#),
            "{out}"
        );
    }

    #[test]
    fn a_deleted_cell_leaves_the_row_and_takes_nothing_else_with_it() {
        let mut f = fixture();
        f.sheet.cells.clear(CellRef::from_a1("B1").expect("valid"));
        let out = written(&mut f);
        assert!(!out.contains(r#"r="B1""#), "{out}");
        assert!(out.contains(r#"<c r="A1" t="s"><v>0</v></c>"#), "{out}");
        assert!(out.contains(r#"<c r="C1">"#), "{out}");
    }

    #[test]
    fn a_cell_past_the_span_widens_the_hint_rather_than_replacing_it() {
        // Excel's `spans` covers a sixteen-row block, not the row, so ours must
        // never be *narrower* than what the file said.
        let mut f = fixture();
        set(&mut f.sheet, "E1", CellValue::Number(1.0), 0);
        let out = written(&mut f);
        assert!(out.contains(r#"<row r="1" spans="1:5""#), "{out}");
    }

    #[test]
    fn new_text_goes_into_the_shared_table_and_the_cell_points_at_it() {
        let mut f = fixture();
        let id = f.strings.intern("Subtotal");
        set(&mut f.sheet, "A3", CellValue::Text(id), 0);
        let out = written(&mut f);

        assert!(out.contains(r#"<c r="A3" t="s"><v>1</v></c>"#), "{out}");
        assert_eq!(f.sst.added(), ["Subtotal"]);
    }

    #[test]
    fn a_sheet_that_was_empty_grows_a_body() {
        let empty = r#"<worksheet><sheetData/><pageSetup orientation="landscape"/></worksheet>"#;
        let mut f = fixture();
        let mut ctx = Context {
            sheet: &f.sheet,
            strings: &f.strings,
            sst: &mut f.sst,
            regenerate: false,
        };
        let out = String::from_utf8(rewrite("s.xml", empty.as_bytes(), &mut ctx).expect("writes"))
            .expect("utf-8");
        assert!(
            out.starts_with(r#"<worksheet><sheetData><row r="1""#),
            "{out}"
        );
        assert!(
            out.contains(r#"</sheetData><pageSetup orientation="landscape"/>"#),
            "{out}"
        );
    }
}
