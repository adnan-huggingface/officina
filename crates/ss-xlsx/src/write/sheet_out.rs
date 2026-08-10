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
    // Column styles live in `<cols>`, which is a sibling of `<sheetData>` and
    // has to be dealt with before it is passed.
    let mut wrote_cols = false;
    let mut in_cols = false;
    // Which columns the file's own `<col>` elements already speak for.
    let mut covered: std::collections::BTreeSet<u32> = Default::default();
    // The autofilter, which sits between `</sheetData>` and `<mergeCells>` and
    // may have to be inserted into a file that never had one.
    //
    // `untouched` is the whole reason this is not simply "replace the element":
    // a `<filterColumn>` can hold a `<top10>`, a `<dynamicFilter>`, a colour or
    // icon filter, or a date grouping, none of which this crate models. If the
    // model's filter is what the file already says, the file's own bytes go
    // back and every one of those survives. Only a filter the user actually
    // changed is rewritten from the model, and losing what we cannot represent
    // is then a consequence of the edit rather than of the save.
    let untouched = crate::autofilter::present(data)
        && crate::autofilter::parse(part, data)? == ctx.sheet.filter;
    let mut wrote_filter = false;
    let mut skip_filter = 0usize;

    // The tab colour, which lives in `<sheetPr>` — the first child of
    // `<worksheet>` — and had no writer until a menu could set it. Same rule as
    // the filter: if the model agrees with the file, nothing is touched, so a
    // themed colour spelled `theme="4" tint="-0.2"` is not silently flattened
    // to the rgb it happens to resolve to on this workbook's palette.
    let tab_wanted = ctx.sheet.view.tab_color;
    let tab_settled = file_tab_color(part, data)? == tab_wanted;
    let mut tab_done = tab_settled;
    let mut in_sheet_pr = false;

    while let Some((event, span)) = splicer.next()? {
        if !tab_settled {
            match &event {
                Event::Start(e) if local_name(e) == b"sheetPr" => {
                    out.extend_from_slice(splicer.bytes(span));
                    write_tab_color(&mut out, &prefix_of(e), tab_wanted);
                    tab_done = true;
                    in_sheet_pr = true;
                    continue;
                }
                Event::Empty(e) if local_name(e) == b"sheetPr" => {
                    if tab_wanted.is_some() {
                        // The element has to grow a body to hold the colour.
                        let prefix = prefix_of(e);
                        out.extend_from_slice(&retag(e, &[], false));
                        write_tab_color(&mut out, &prefix, tab_wanted);
                        close(&mut out, &prefix, b"sheetPr");
                    } else {
                        out.extend_from_slice(splicer.bytes(span));
                    }
                    tab_done = true;
                    continue;
                }
                Event::End(e) if end_local_name(e) == b"sheetPr" => {
                    in_sheet_pr = false;
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }
                // The file's own colour, superseded by the one just written.
                Event::Start(e) | Event::Empty(e)
                    if in_sheet_pr && local_name(e) == b"tabColor" =>
                {
                    continue;
                }
                Event::End(e) if end_local_name(e) == b"tabColor" => continue,
                // Any other child of `<worksheet>`: `<sheetPr>` comes before
                // all of them, so this is the last moment one can be added.
                // The root itself is not a child of itself.
                Event::Start(e) | Event::Empty(e) if !tab_done && local_name(e) != b"worksheet" => {
                    let prefix = prefix_of(e);
                    open(&mut out, &prefix, b"sheetPr", &[], false);
                    write_tab_color(&mut out, &prefix, tab_wanted);
                    close(&mut out, &prefix, b"sheetPr");
                    tab_done = true;
                }
                _ => {}
            }
        }

        if skip_filter > 0 {
            match &event {
                Event::Start(e) if local_name(e) == b"autoFilter" => skip_filter += 1,
                Event::End(e) if end_local_name(e) == b"autoFilter" => skip_filter -= 1,
                _ => {}
            }
            if untouched {
                out.extend_from_slice(splicer.bytes(span));
            }
            continue;
        }

        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"autoFilter" => {
                if untouched {
                    out.extend_from_slice(splicer.bytes(span.clone()));
                } else {
                    write_filter(&mut out, &prefix_of(e), ctx.sheet);
                }
                wrote_filter = true;
                if matches!(event, Event::Start(_)) {
                    skip_filter = 1;
                }
                continue;
            }
            // The schema fixes the order of a worksheet's children, so a filter
            // the file did not have goes in immediately before the first
            // element that must follow it — not simply after `</sheetData>`,
            // which would put it ahead of `<sheetProtection>` and make the part
            // invalid.
            Event::Start(e) | Event::Empty(e) if !wrote_filter && after_filter(local_name(e)) => {
                write_filter(&mut out, &prefix_of(e), ctx.sheet);
                wrote_filter = true;
            }
            Event::End(e) if !wrote_filter && end_local_name(e) == b"worksheet" => {
                write_filter(&mut out, &prefix, ctx.sheet);
                wrote_filter = true;
            }
            _ => {}
        }

        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"cols" => {
                prefix = prefix_of(e);
                in_cols = !matches!(event, Event::Empty(_));
                if in_cols {
                    out.extend_from_slice(splicer.bytes(span));
                } else {
                    // An empty `<cols/>` grows a body when the model has
                    // column styles to put in it.
                    let extra = missing_columns(ctx.sheet, &Default::default());
                    if extra.is_empty() {
                        out.extend_from_slice(splicer.bytes(span));
                    } else {
                        out.extend_from_slice(&retag(e, &[], false));
                        write_columns(&mut out, &prefix, &extra);
                        close(&mut out, &prefix, b"cols");
                    }
                    wrote_cols = true;
                }
                covered.clear();
            }
            Event::Start(e) | Event::Empty(e) if in_cols && local_name(e) == b"col" => {
                write_col(
                    &mut out,
                    splicer.bytes(span.clone()),
                    e,
                    ctx.sheet,
                    &mut covered,
                );
            }
            Event::End(e) if end_local_name(e) == b"cols" => {
                let extra = missing_columns(ctx.sheet, &covered);
                write_columns(&mut out, &prefix, &extra);
                out.extend_from_slice(splicer.bytes(span));
                in_cols = false;
                wrote_cols = true;
            }

            Event::Start(e) if local_name(e) == b"sheetData" => {
                prefix = prefix_of(e);
                // The schema fixes `<cols>` immediately before `<sheetData>`,
                // so this is the last chance to add one the file left out.
                if !wrote_cols {
                    let extra = missing_columns(ctx.sheet, &covered);
                    if !extra.is_empty() {
                        open(&mut out, &prefix, b"cols", &[], false);
                        write_columns(&mut out, &prefix, &extra);
                        close(&mut out, &prefix, b"cols");
                    }
                    wrote_cols = true;
                }
                out.extend_from_slice(splicer.bytes(span));
                in_data = true;
                next_row = 0;
            }

            Event::Empty(e) if local_name(e) == b"sheetData" => {
                prefix = prefix_of(e);
                if !wrote_cols {
                    let extra = missing_columns(ctx.sheet, &covered);
                    if !extra.is_empty() {
                        open(&mut out, &prefix, b"cols", &[], false);
                        write_columns(&mut out, &prefix, &extra);
                        close(&mut out, &prefix, b"cols");
                    }
                    wrote_cols = true;
                }
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

/// The tab colour the file currently states, if any.
///
/// Guarded by a substring test so a worksheet with no `<sheetPr>` — the common
/// case — costs one pass over the bytes and no XML parsing.
fn file_tab_color(part: &str, data: &[u8]) -> Result<Option<ss_model::Color>> {
    const TAG: &[u8] = b"tabColor";
    if !data.windows(TAG.len()).any(|w| w == TAG) {
        return Ok(None);
    }
    let mut splicer = Splicer::new(part, data);
    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"tabColor" => {
                let color = crate::styles::read_color(e);
                return Ok((color != ss_model::Color::Auto).then_some(color));
            }
            // `<sheetPr>` is the first child of `<worksheet>`, so anything
            // inside `<sheetData>` is far too late to be the sheet's own.
            Event::Start(e) if local_name(e) == b"sheetData" => break,
            _ => {}
        }
    }
    Ok(None)
}

fn write_tab_color(out: &mut Vec<u8>, prefix: &[u8], color: Option<ss_model::Color>) {
    let sets = match color {
        None | Some(ss_model::Color::Auto) => return,
        Some(ss_model::Color::Rgb(_)) => vec![Set::to(
            b"rgb",
            color.and_then(|c| c.to_hex()).unwrap_or_default(),
        )],
        Some(ss_model::Color::Indexed(i)) => vec![Set::to(b"indexed", i.to_string())],
        Some(ss_model::Color::Theme { index, tint }) => {
            let mut sets = vec![Set::to(b"theme", index.to_string())];
            if tint != 0.0 {
                sets.push(Set::to(b"tint", ss_model::format_general(tint)));
            }
            sets
        }
    };
    open(out, prefix, b"tabColor", &sets, true);
}

/// The worksheet children the schema puts *after* `<autoFilter>`.
///
/// Listed rather than derived, because the sequence is a fixed list in the
/// schema and there is nothing in the file that says where an element belongs.
/// Everything before `autoFilter` — `sheetData`, `sheetCalcPr`,
/// `sheetProtection`, `protectedRanges`, `scenarios` — is deliberately absent.
fn after_filter(name: &[u8]) -> bool {
    matches!(
        name,
        b"sortState"
            | b"dataConsolidate"
            | b"customSheetViews"
            | b"mergeCells"
            | b"phoneticPr"
            | b"conditionalFormatting"
            | b"dataValidations"
            | b"hyperlinks"
            | b"printOptions"
            | b"pageMargins"
            | b"pageSetup"
            | b"headerFooter"
            | b"rowBreaks"
            | b"colBreaks"
            | b"customProperties"
            | b"cellWatches"
            | b"ignoredErrors"
            | b"smartTags"
            | b"drawing"
            | b"drawingHF"
            | b"picture"
            | b"oleObjects"
            | b"controls"
            | b"webPublishItems"
            | b"tableParts"
            | b"extLst"
    )
}

/// Writes the sheet's autofilter, or nothing at all when it has none.
///
/// Nothing at all is the important half: clearing a filter has to *remove* the
/// element, and a writer that only ever replaced it would leave the arrows on
/// a sheet the user just un-filtered.
fn write_filter(out: &mut Vec<u8>, prefix: &[u8], sheet: &Sheet) {
    let Some(filter) = &sheet.filter else {
        return;
    };
    let range = range_ref(filter.range);
    if filter.columns.is_empty() {
        open(out, prefix, b"autoFilter", &[Set::to(b"ref", range)], true);
        return;
    }
    open(out, prefix, b"autoFilter", &[Set::to(b"ref", range)], false);
    for column in &filter.columns {
        open(
            out,
            prefix,
            b"filterColumn",
            &[Set::to(b"colId", column.col.to_string())],
            false,
        );
        match &column.kind {
            ss_model::FilterKind::Values { values, blanks } => {
                let blank = blanks.then(|| "1".to_string());
                open(
                    out,
                    prefix,
                    b"filters",
                    &[Set::maybe(b"blank", blank)],
                    false,
                );
                for value in values {
                    open(
                        out,
                        prefix,
                        b"filter",
                        &[Set::to(b"val", value.clone())],
                        true,
                    );
                }
                close(out, prefix, b"filters");
            }
            ss_model::FilterKind::Custom { first, second } => {
                let and = second
                    .as_ref()
                    .and_then(|(and, _)| and.then(|| "1".to_string()));
                open(
                    out,
                    prefix,
                    b"customFilters",
                    &[Set::maybe(b"and", and)],
                    false,
                );
                for criterion in std::iter::once(first).chain(second.iter().map(|(_, c)| c)) {
                    open(
                        out,
                        prefix,
                        b"customFilter",
                        &[
                            Set::to(b"operator", criterion.op.code()),
                            Set::to(b"val", criterion.value.clone()),
                        ],
                        true,
                    );
                }
                close(out, prefix, b"customFilters");
            }
        }
        close(out, prefix, b"filterColumn");
    }
    close(out, prefix, b"autoFilter");
}

/// A range as `ref` spells it.
fn range_ref(range: CellRange) -> String {
    format!("{}:{}", range.start.to_a1(), range.end.to_a1())
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
        for name in [
            b"ht".as_slice(),
            b"customHeight",
            b"hidden",
            b"s",
            b"customFormat",
        ] {
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
    let mut sets = match sheet.row_heights.get(&row) {
        // Zero is how the reader records a hidden row: a hidden row has no
        // height to speak of.
        Some(h) if *h == 0.0 => vec![Set::to(b"hidden", "1")],
        Some(h) => vec![Set::to(b"ht", h.to_string()), Set::to(b"customHeight", "1")],
        None => Vec::new(),
    };
    // A row style needs `customFormat` beside it or Excel ignores the `s`
    // entirely, which makes a formatted row come back unformatted.
    if let Some(style) = sheet.row_styles.get(&row) {
        sets.push(Set::to(b"s", style.0.to_string()));
        sets.push(Set::to(b"customFormat", "1"));
    }
    sets
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
    /// `s` on the row, but only when `customFormat` says it counts. Without
    /// that attribute Excel ignores the style, so recording it would make the
    /// model and the file disagree about a row nobody formatted.
    style: Option<ss_model::StyleId>,
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
        let style = raw_attr(&start, b"customFormat")
            .and_then(|a| parse_bool(&a.value))
            .unwrap_or(false)
            .then(|| raw_attr(&start, b"s").and_then(|a| parse_u32(&a.value)))
            .flatten()
            .map(ss_model::StyleId)
            .filter(|s| *s != ss_model::StyleId::DEFAULT);
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
            style,
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
            && self.style == sheet.row_styles.get(&self.index).copied()
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

/// A `<col>` from the file, re-emitted so its span carries the model's styles.
///
/// A span whose columns no longer agree is *split* rather than replaced: each
/// piece is a retag of the original element, so every attribute nobody here
/// models — `outlineLevel`, `collapsed`, `bestFit`, `phonetic` — goes back
/// unchanged on each piece.
fn write_col(
    out: &mut Vec<u8>,
    original: &[u8],
    e: &BytesStart<'_>,
    sheet: &Sheet,
    covered: &mut std::collections::BTreeSet<u32>,
) {
    let (Some(min), Some(max)) = (
        raw_attr(e, b"min").and_then(|a| parse_u32(&a.value)),
        raw_attr(e, b"max").and_then(|a| parse_u32(&a.value)),
    ) else {
        out.extend_from_slice(original);
        return;
    };
    if min == 0 || min > max {
        out.extend_from_slice(original);
        return;
    }
    let file_style = raw_attr(e, b"style")
        .and_then(|a| parse_u32(&a.value))
        .unwrap_or(0);
    // Beyond the limit the reader materializes, the model knows nothing and the
    // file's own value is the only truth there is.
    let horizon = max.min(min.saturating_add(1024));
    for column in min..=horizon {
        covered.insert(column - 1);
    }

    let wanted = |column: u32| {
        sheet
            .column_styles
            .get(&(column - 1))
            .map(|s| s.0)
            .unwrap_or(0)
    };
    if (min..=horizon).all(|c| wanted(c) == file_style) && horizon == max {
        out.extend_from_slice(original);
        return;
    }

    // Split into runs that agree.
    let mut start = min;
    while start <= max {
        let style = if start <= horizon {
            wanted(start)
        } else {
            file_style
        };
        let mut end = start;
        while end < max {
            let next = end + 1;
            let next_style = if next <= horizon {
                wanted(next)
            } else {
                file_style
            };
            if next_style != style {
                break;
            }
            end = next;
        }
        let sets = [
            Set::to(b"min", start.to_string()),
            Set::to(b"max", end.to_string()),
            Set::maybe(b"style", (style != 0).then(|| style.to_string())),
        ];
        out.extend_from_slice(&retag(e, &sets, true));
        start = end + 1;
    }
}

/// Column styles the model has that no `<col>` in the file speaks for,
/// gathered into runs of `(first, last, style)`.
fn missing_columns(
    sheet: &Sheet,
    covered: &std::collections::BTreeSet<u32>,
) -> Vec<(u32, u32, u32)> {
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    for (column, style) in sheet
        .column_styles
        .iter()
        .filter(|(c, s)| !covered.contains(c) && **s != ss_model::StyleId::DEFAULT)
        .map(|(c, s)| (*c, s.0))
    {
        match runs.last_mut() {
            // `column_styles` is a BTreeMap, so this walks in order and a run
            // only ever grows at its end.
            Some((_, last, existing)) if *last + 1 == column && *existing == style => {
                *last = column
            }
            _ => runs.push((column, column, style)),
        }
    }
    runs
}

/// Emits the `<col>` elements for the runs `missing_columns` found.
fn write_columns(out: &mut Vec<u8>, prefix: &[u8], runs: &[(u32, u32, u32)]) {
    for (first, last, style) in runs {
        open(
            out,
            prefix,
            b"col",
            &[
                // `min` and `max` are one-based and inclusive at both ends.
                Set::to(b"min", (first + 1).to_string()),
                Set::to(b"max", (last + 1).to_string()),
                Set::to(b"style", style.to_string()),
                // Excel repairs a `<col>` with no width, so the workbook
                // default goes out with it.
                Set::to(b"width", "8.43"),
            ],
            true,
        );
    }
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
        // As the reader would have left it. The writer compares the model's
        // filter against the file's, so a fixture that claimed the sheet had
        // none would be describing a filter the user had just cleared.
        sheet.filter = crate::autofilter::parse("sheet1.xml", SHEET.as_bytes()).expect("parses");

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

    /// The same worksheet, but with a `<cols>` block already in it.
    const WITH_COLS: &str = concat!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://x">"#,
        r#"<cols><col min="1" max="3" width="12.5" customWidth="1" outlineLevel="1"/></cols>"#,
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
    );

    fn written_from(source: &str, sheet: &Sheet) -> String {
        let strings = StringTable::new();
        let mut sst = Sst::absent();
        let mut ctx = Context {
            sheet,
            strings: &strings,
            sst: &mut sst,
            regenerate: false,
        };
        String::from_utf8(rewrite("sheet1.xml", source.as_bytes(), &mut ctx).expect("writes"))
            .expect("utf-8")
    }

    #[test]
    fn a_column_style_reaches_the_file_without_materializing_a_column_of_cells() {
        // Shading a column is one attribute. Writing it as cells would turn a
        // sparse sheet into a million-row one, and writing it nowhere — which
        // is what happened before `<cols>` had a writer — loses it on save.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.column_styles.insert(3, StyleId(2));
        sheet.column_styles.insert(4, StyleId(2));

        let out = written_from(
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
            &sheet,
        );
        assert!(
            out.contains(r#"<cols><col min="4" max="5" style="2" width="8.43"/></cols>"#),
            "adjacent columns share one span: {out}"
        );
        // And it goes before `<sheetData>`, which the schema fixes.
        assert!(out.find("<cols>") < out.find("<sheetData>"), "{out}");
    }

    #[test]
    fn a_span_whose_columns_stop_agreeing_is_split_and_keeps_its_attributes() {
        // The file's span covers A to C; only B is styled now. Splitting has to
        // carry `customWidth` and `outlineLevel` onto every piece, because this
        // writer does not model either and must not drop them.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.column_styles.insert(1, StyleId(4));

        let out = written_from(WITH_COLS, &sheet);
        assert_eq!(out.matches("<col ").count(), 3, "{out}");
        assert!(out.contains(r#"min="2" max="2""#), "{out}");
        assert!(out.contains(r#"style="4""#), "{out}");
        assert_eq!(
            out.matches("outlineLevel=\"1\"").count(),
            3,
            "every piece keeps what nobody here models: {out}"
        );
        assert_eq!(out.matches("width=\"12.5\"").count(), 3, "{out}");
    }

    #[test]
    fn a_column_nobody_restyled_comes_back_byte_for_byte() {
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        assert_eq!(written_from(WITH_COLS, &sheet), WITH_COLS);
    }

    #[test]
    fn a_row_style_is_written_with_the_flag_that_makes_excel_honour_it() {
        // `s` alone on a `<row>` is advisory and Excel ignores it. Without
        // `customFormat` beside it, a formatted row comes back unformatted.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.row_styles.insert(0, StyleId(3));

        let out = written_from(
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
            &sheet,
        );
        assert!(out.contains(r#"s="3""#), "{out}");
        assert!(out.contains(r#"customFormat="1""#), "{out}");
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
    fn a_tab_colour_is_written_where_the_schema_puts_it() {
        let mut sheet = Sheet::new("Data");
        sheet.view.tab_color = Some(ss_model::Color::rgb(0, 0xB0, 0x50));

        // No `<sheetPr>` at all: one is authored, and it goes first.
        let out = written_from(r#"<worksheet><sheetData/></worksheet>"#, &sheet);
        assert_eq!(
            out,
            r#"<worksheet><sheetPr><tabColor rgb="FF00B050"/></sheetPr><sheetData/></worksheet>"#
        );

        // A `<sheetPr>` that exists but says nothing about colour keeps what it
        // does say.
        let out = written_from(
            r#"<worksheet><sheetPr codeName="Sheet3"><outlinePr summaryBelow="1"/></sheetPr><sheetData/></worksheet>"#,
            &sheet,
        );
        assert_eq!(
            out,
            r#"<worksheet><sheetPr codeName="Sheet3"><tabColor rgb="FF00B050"/><outlinePr summaryBelow="1"/></sheetPr><sheetData/></worksheet>"#
        );

        // And one that already carries a different colour is corrected.
        let out = written_from(
            r#"<worksheet><sheetPr><tabColor rgb="FFC00000"/></sheetPr><sheetData/></worksheet>"#,
            &sheet,
        );
        assert!(out.contains(r#"<tabColor rgb="FF00B050"/>"#), "{out}");
        assert!(!out.contains("FFC00000"), "{out}");
    }

    #[test]
    fn a_themed_tab_colour_nobody_changed_is_not_flattened_to_rgb() {
        // `theme="4" tint="-0.2"` resolves to an rgb, and writing that rgb back
        // would be a different document: it stops following the theme.
        const THEMED: &str = r#"<worksheet><sheetPr><tabColor theme="4" tint="-0.2"/></sheetPr><sheetData/></worksheet>"#;
        let mut sheet = Sheet::new("Data");
        sheet.view.tab_color = Some(ss_model::Color::Theme {
            index: 4,
            tint: -0.2,
        });
        assert_eq!(written_from(THEMED, &sheet), THEMED);

        // Removing it takes the element and nothing else.
        let mut cleared = Sheet::new("Data");
        cleared.view.tab_color = None;
        assert_eq!(
            written_from(THEMED, &cleared),
            r#"<worksheet><sheetPr></sheetPr><sheetData/></worksheet>"#
        );
    }

    #[test]
    fn a_filter_the_user_did_not_touch_keeps_what_we_do_not_model() {
        // `<top10>` is a filter kind this crate has no representation for. A
        // writer that rebuilt the element from the model would delete it on a
        // save that had nothing to do with filtering.
        const RICH: &str = concat!(
            r#"<worksheet><sheetData/><autoFilter ref="A1:C9">"#,
            r#"<filterColumn colId="0" hiddenButton="1"><top10 val="10" percent="1"/></filterColumn>"#,
            r#"</autoFilter></worksheet>"#,
        );
        let mut sheet = Sheet::new("Data");
        sheet.filter = crate::autofilter::parse("s.xml", RICH.as_bytes()).expect("parses");
        assert!(
            !sheet.filter.as_ref().expect("read").is_filtering(),
            "a top-10 filter is not one we can represent"
        );
        assert_eq!(written_from(RICH, &sheet), RICH);
    }

    #[test]
    fn clearing_a_filter_removes_the_element_rather_than_emptying_it() {
        let sheet = Sheet::new("Data");
        let out = written_from(
            r#"<worksheet><sheetData/><autoFilter ref="A1:C9"/></worksheet>"#,
            &sheet,
        );
        assert_eq!(out, r#"<worksheet><sheetData/></worksheet>"#);
    }

    #[test]
    fn a_filter_added_to_a_sheet_that_had_none_lands_in_schema_order() {
        // Between `<sheetProtection>` and `<mergeCells>`. Anywhere else and
        // Excel reports the file as damaged rather than as oddly ordered.
        let mut sheet = Sheet::new("Data");
        sheet.filter = Some(ss_model::AutoFilter::over(CellRange::new(
            CellRef::from_a1("A1").expect("valid"),
            CellRef::from_a1("B4").expect("valid"),
        )));
        let out = written_from(
            r#"<worksheet><sheetData/><sheetProtection sheet="1"/><mergeCells count="1"><mergeCell ref="D1:E1"/></mergeCells></worksheet>"#,
            &sheet,
        );
        assert_eq!(
            out,
            r#"<worksheet><sheetData/><sheetProtection sheet="1"/><autoFilter ref="A1:B4"/><mergeCells count="1"><mergeCell ref="D1:E1"/></mergeCells></worksheet>"#
        );
    }

    #[test]
    fn a_filter_with_criteria_writes_them_and_reads_back_as_itself() {
        let mut sheet = Sheet::new("Data");
        let mut filter = ss_model::AutoFilter::over(CellRange::new(
            CellRef::from_a1("A1").expect("valid"),
            CellRef::from_a1("C9").expect("valid"),
        ));
        filter.set(
            1,
            Some(ss_model::FilterKind::Values {
                values: ["North".to_string(), "South".to_string()]
                    .into_iter()
                    .collect(),
                blanks: true,
            }),
        );
        filter.set(
            2,
            Some(ss_model::FilterKind::Custom {
                first: ss_model::Criterion {
                    op: ss_model::Compare::Greater,
                    value: "10".into(),
                },
                second: Some((
                    true,
                    ss_model::Criterion {
                        op: ss_model::Compare::LessEqual,
                        value: "99".into(),
                    },
                )),
            }),
        );
        sheet.filter = Some(filter);

        let out = written_from(r#"<worksheet><sheetData/></worksheet>"#, &sheet);
        let back = crate::autofilter::parse("s.xml", out.as_bytes()).expect("parses");
        assert_eq!(back, sheet.filter, "what was written reads back as itself");
        assert!(out.contains(r#"<filters blank="1">"#), "{out}");
        assert!(
            out.contains(r#"<customFilter operator="greaterThan" val="10"/>"#),
            "{out}"
        );
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
            out.contains(r#"<autoFilter ref="A1:C3"/><pageSetup orientation="landscape"/>"#),
            "the fixture's filter goes in ahead of pageSetup, and pageSetup is \
             untouched: {out}"
        );
    }
}
