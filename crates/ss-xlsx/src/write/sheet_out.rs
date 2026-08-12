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
    // The columns the model has an opinion about that the file's own `<col>`
    // elements do not speak for. Worked out ahead of the rewrite, because a
    // `<col>` that has to be added belongs *in* the sequence and there is no
    // way to know where until every span in the file has been seen.
    let missing = missing_columns(ctx.sheet, &file_columns(part, data)?);
    let mut next_missing = 0usize;
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

    // Conditional formatting and data validations, on the filter's pattern:
    // if the model still says what the file says, the file's own bytes go
    // back — which is what keeps x14 extensions, icon-set detail, and rule
    // kinds we do not model alive across a save. Only an actual edit
    // rewrites them from the model, and losing what we cannot represent is
    // then a consequence of the edit rather than of the save.
    let (file_cf, file_dv) = file_rules(part, data)?;
    let cf_untouched = file_cf == ctx.sheet.conditional_formats;
    let dv_untouched = file_dv == ctx.sheet.validations;
    let mut wrote_cf = false;
    let mut wrote_dv = false;
    let mut skip_cf = 0usize;
    let mut skip_dv = 0usize;

    // `<sheetFormatPr>` states the deepest outline level per axis, and Excel
    // sizes the outline pane from it — group a row without correcting it and
    // the collapse buttons render clipped.
    let outline_rows = ctx.sheet.row_outlines.values().copied().max().unwrap_or(0);
    let outline_cols = ctx
        .sheet
        .column_outlines
        .values()
        .copied()
        .max()
        .unwrap_or(0);
    let mut wrote_fmt = false;

    // The tab colour, which lives in `<sheetPr>` — the first child of
    // `<worksheet>` — and had no writer until a menu could set it. Same rule as
    // the filter: if the model agrees with the file, nothing is touched, so a
    // themed colour spelled `theme="4" tint="-0.2"` is not silently flattened
    // to the rgb it happens to resolve to on this workbook's palette.
    let tab_wanted = ctx.sheet.view.tab_color;
    let tab_settled = file_tab_color(part, data)? == tab_wanted;
    let mut tab_done = tab_settled;
    let mut in_sheet_pr = false;

    // The pane division, which lives in `<sheetView>` and had no writer at all
    // until now — so freezing a sheet was a change that did not survive its own
    // save. Settled by comparing only the division itself: a `topLeftCell` the
    // user never moved is none of this writer's business, and rewriting it
    // would put a diff in every file that was merely opened.
    let pane_wanted = wanted_pane(ctx.sheet);
    let file_pane = file_pane(part, data)?;
    let pane_settled = file_pane.as_ref().map(PaneTag::division) == pane_wanted.as_ref().map(PaneTag::division);
    let mut views_done = pane_settled;
    let mut in_sheet_view = false;
    let mut skip_pane = 0usize;
    let mut skip_selection = 0usize;
    // The pane whose `<selection>` the file called active, so that the one kept
    // is the one the cursor was actually in.
    let old_active = file_pane.as_ref().map(|p| p.active.clone()).unwrap_or_default();
    let mut kept_selection = false;

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

        if !pane_settled {
            // A `<pane>` or a `<selection>` with children — neither has any in
            // practice, but a swallowed element has to be swallowed whole.
            if skip_pane > 0 {
                match &event {
                    Event::Start(e) if local_name(e) == b"pane" => skip_pane += 1,
                    Event::End(e) if end_local_name(e) == b"pane" => skip_pane -= 1,
                    _ => {}
                }
                continue;
            }
            if skip_selection > 0 {
                match &event {
                    Event::Start(e) if local_name(e) == b"selection" => skip_selection += 1,
                    Event::End(e) if end_local_name(e) == b"selection" => skip_selection -= 1,
                    _ => {}
                }
                continue;
            }
            match &event {
                Event::Start(e) if local_name(e) == b"sheetView" => {
                    out.extend_from_slice(splicer.bytes(span));
                    write_pane(&mut out, &prefix_of(e), pane_wanted.as_ref());
                    in_sheet_view = true;
                    views_done = true;
                    continue;
                }
                Event::Empty(e) if local_name(e) == b"sheetView" => {
                    if pane_wanted.is_some() {
                        // The view has to grow a body to hold the division.
                        let view_prefix = prefix_of(e);
                        out.extend_from_slice(&retag(e, &[], false));
                        write_pane(&mut out, &view_prefix, pane_wanted.as_ref());
                        close(&mut out, &view_prefix, b"sheetView");
                    } else {
                        out.extend_from_slice(splicer.bytes(span));
                    }
                    views_done = true;
                    continue;
                }
                Event::End(e) if end_local_name(e) == b"sheetView" => {
                    in_sheet_view = false;
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }
                // The file's own division, superseded by the one just written.
                Event::Start(e) | Event::Empty(e) if in_sheet_view && local_name(e) == b"pane" => {
                    if matches!(event, Event::Start(_)) {
                        skip_pane = 1;
                    }
                    continue;
                }
                // A sheet keeps one selection per pane, and the panes have just
                // changed underneath them. The one the cursor was in is kept
                // and re-labelled; the others named panes that no longer exist.
                Event::Start(e) | Event::Empty(e)
                    if in_sheet_view && local_name(e) == b"selection" =>
                {
                    let pane = raw_attr(e, b"pane")
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                        .unwrap_or_default();
                    let mine = pane == old_active;
                    if !kept_selection && mine {
                        kept_selection = true;
                        let label = pane_wanted.as_ref().map(|p| p.active.clone());
                        out.extend_from_slice(&retag(
                            e,
                            &[Set::maybe(b"pane", label)],
                            matches!(event, Event::Empty(_)),
                        ));
                        continue;
                    }
                    if matches!(event, Event::Start(_)) {
                        skip_selection = 1;
                    }
                    continue;
                }
                // A file with no `<sheetViews>` at all — an authored workbook —
                // gets one, in its schema slot before everything that follows.
                Event::Start(e) | Event::Empty(e)
                    if !views_done && !in_sheet_pr && after_views(local_name(e)) =>
                {
                    write_sheet_views(&mut out, &prefix_of(e), pane_wanted.as_ref());
                    views_done = true;
                }
                Event::End(e) if !views_done && end_local_name(e) == b"worksheet" => {
                    write_sheet_views(&mut out, &prefix, pane_wanted.as_ref());
                    views_done = true;
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

        if skip_cf > 0 {
            match &event {
                Event::Start(e) if local_name(e) == b"conditionalFormatting" => skip_cf += 1,
                Event::End(e) if end_local_name(e) == b"conditionalFormatting" => skip_cf -= 1,
                _ => {}
            }
            if cf_untouched {
                out.extend_from_slice(splicer.bytes(span));
            }
            continue;
        }
        if skip_dv > 0 {
            match &event {
                Event::Start(e) if local_name(e) == b"dataValidations" => skip_dv += 1,
                Event::End(e) if end_local_name(e) == b"dataValidations" => skip_dv -= 1,
                _ => {}
            }
            if dv_untouched {
                out.extend_from_slice(splicer.bytes(span));
            }
            continue;
        }

        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"sheetFormatPr" => {
                wrote_fmt = true;
                let stated_rows = raw_attr(e, b"outlineLevelRow")
                    .and_then(|a| parse_u32(&a.value))
                    .unwrap_or(0) as u8;
                let stated_cols = raw_attr(e, b"outlineLevelCol")
                    .and_then(|a| parse_u32(&a.value))
                    .unwrap_or(0) as u8;
                if stated_rows == outline_rows && stated_cols == outline_cols {
                    out.extend_from_slice(splicer.bytes(span));
                } else {
                    let sets = [
                        if outline_rows > 0 {
                            Set::to(b"outlineLevelRow", outline_rows.to_string())
                        } else {
                            Set::off(b"outlineLevelRow")
                        },
                        if outline_cols > 0 {
                            Set::to(b"outlineLevelCol", outline_cols.to_string())
                        } else {
                            Set::off(b"outlineLevelCol")
                        },
                    ];
                    out.extend_from_slice(&retag(e, &sets, matches!(event, Event::Empty(_))));
                }
                continue;
            }
            // A file with no `<sheetFormatPr>` gets one when the model has
            // outlines to declare; its schema slot is just before `<cols>`.
            Event::Start(e) | Event::Empty(e)
                if !wrote_fmt
                    && matches!(local_name(e), b"cols" | b"sheetData")
                    && (outline_rows > 0 || outline_cols > 0) =>
            {
                let fmt_prefix = prefix_of(e);
                let mut sets = Vec::new();
                if outline_rows > 0 {
                    sets.push(Set::to(b"outlineLevelRow", outline_rows.to_string()));
                }
                if outline_cols > 0 {
                    sets.push(Set::to(b"outlineLevelCol", outline_cols.to_string()));
                }
                open(&mut out, &fmt_prefix, b"sheetFormatPr", &sets, true);
                wrote_fmt = true;
                // No `continue`: the element itself still gets its own turn.
            }
            _ => {}
        }

        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"conditionalFormatting" => {
                // The first block the file had stands in for all of them:
                // the model's blocks are written there, once, and the rest
                // of the file's are swallowed.
                if cf_untouched {
                    out.extend_from_slice(splicer.bytes(span.clone()));
                } else if !wrote_cf {
                    write_conditional_formats(&mut out, &prefix_of(e), ctx.sheet);
                }
                wrote_cf = true;
                if matches!(event, Event::Start(_)) {
                    skip_cf = 1;
                }
                continue;
            }
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"dataValidations" => {
                if dv_untouched {
                    out.extend_from_slice(splicer.bytes(span.clone()));
                } else if !wrote_dv {
                    write_validations(&mut out, &prefix_of(e), ctx.sheet);
                }
                wrote_dv = true;
                if matches!(event, Event::Start(_)) {
                    skip_dv = 1;
                }
                continue;
            }
            // Blocks the file never had go in at their schema positions:
            // immediately before the first element that must follow them.
            Event::Start(e) | Event::Empty(e) => {
                if !cf_untouched && !wrote_cf && after_conditional(local_name(e)) {
                    write_conditional_formats(&mut out, &prefix_of(e), ctx.sheet);
                    wrote_cf = true;
                }
                if !dv_untouched && !wrote_dv && after_validations(local_name(e)) {
                    write_validations(&mut out, &prefix_of(e), ctx.sheet);
                    wrote_dv = true;
                }
            }
            Event::End(e) if end_local_name(e) == b"worksheet" => {
                if !cf_untouched && !wrote_cf {
                    write_conditional_formats(&mut out, &prefix, ctx.sheet);
                    wrote_cf = true;
                }
                if !dv_untouched && !wrote_dv {
                    write_validations(&mut out, &prefix, ctx.sheet);
                    wrote_dv = true;
                }
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
                    // columns to put in it.
                    if missing.is_empty() {
                        out.extend_from_slice(splicer.bytes(span));
                    } else {
                        out.extend_from_slice(&retag(e, &[], false));
                        write_columns(&mut out, &prefix, &missing, &mut next_missing, u32::MAX);
                        close(&mut out, &prefix, b"cols");
                    }
                    wrote_cols = true;
                }
            }
            Event::Start(e) | Event::Empty(e) if in_cols && local_name(e) == b"col" => {
                // Ahead of this element, not after the whole block: `<col>`
                // elements out of numerical order are legal and readers cope,
                // but no spreadsheet has ever written them that way.
                let min = raw_attr(e, b"min")
                    .and_then(|a| parse_u32(&a.value))
                    .unwrap_or(u32::MAX);
                write_columns(&mut out, &prefix, &missing, &mut next_missing, min);
                write_col(&mut out, splicer.bytes(span.clone()), e, ctx.sheet);
            }
            Event::End(e) if end_local_name(e) == b"cols" => {
                write_columns(&mut out, &prefix, &missing, &mut next_missing, u32::MAX);
                out.extend_from_slice(splicer.bytes(span));
                in_cols = false;
                wrote_cols = true;
            }

            Event::Start(e) if local_name(e) == b"sheetData" => {
                prefix = prefix_of(e);
                // The schema fixes `<cols>` immediately before `<sheetData>`,
                // so this is the last chance to add one the file left out.
                if !wrote_cols {
                    if !missing.is_empty() {
                        open(&mut out, &prefix, b"cols", &[], false);
                        write_columns(&mut out, &prefix, &missing, &mut next_missing, u32::MAX);
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
                    if !missing.is_empty() {
                        open(&mut out, &prefix, b"cols", &[], false);
                        write_columns(&mut out, &prefix, &missing, &mut next_missing, u32::MAX);
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

/// A `<pane>` as a file spells it.
///
/// Kept as the attributes rather than as the model's [`ss_model::Panes`],
/// because deciding whether the file already says what the model says is a
/// question about the file's own numbers — a split's position is in twips, and
/// converting the model's boundary back is the only way to compare them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PaneTag {
    frozen: bool,
    /// `xSplit`/`ySplit`: columns and rows when frozen, twips when split.
    x: u32,
    y: u32,
    top_left: String,
    active: String,
}

impl PaneTag {
    /// The part that decides whether the file needs rewriting: where the sheet
    /// is divided and whether the division pins. `topLeftCell` is where the
    /// user had scrolled to, which no edit here changes.
    fn division(&self) -> (bool, u32, u32) {
        (self.frozen, self.x, self.y)
    }

    fn pane_name(at: CellRef) -> String {
        match (at.col > 0, at.row > 0) {
            (true, true) => "bottomRight",
            (true, false) => "topRight",
            _ => "bottomLeft",
        }
        .to_string()
    }
}

/// The `<pane>` the model asks for, in the units the file writes.
fn wanted_pane(sheet: &Sheet) -> Option<PaneTag> {
    let panes = sheet.panes?;
    let (x, y) = if panes.frozen {
        (panes.at.col, panes.at.row)
    } else {
        (
            sheet.pane_offset(ss_model::Axis::Columns, panes.at.col),
            sheet.pane_offset(ss_model::Axis::Rows, panes.at.row),
        )
    };
    // The first cell of the *scrolling* pane. A sheet scrolled to A1 and then
    // frozen at C3 is showing C3 in its scrolling pane, not A1 — the frozen
    // bands are not part of that pane at all.
    let scrolled = sheet.view.top_left.unwrap_or(panes.at);
    let top_left = CellRef::new(
        scrolled.row.max(panes.at.row),
        scrolled.col.max(panes.at.col),
    );
    Some(PaneTag {
        frozen: panes.frozen,
        x,
        y,
        top_left: top_left.to_a1(),
        active: PaneTag::pane_name(panes.at),
    })
}

/// The `<pane>` the file already has, if it has one.
fn file_pane(part: &str, data: &[u8]) -> Result<Option<PaneTag>> {
    let mut splicer = Splicer::new(part, data);
    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"pane" => {
                let text = |name: &[u8]| {
                    raw_attr(e, name)
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                        .unwrap_or_default()
                };
                let number = |name: &[u8]| {
                    raw_attr(e, name)
                        .and_then(|a| parse_u32(&a.value))
                        .unwrap_or(0)
                };
                let state = text(b"state");
                return Ok(Some(PaneTag {
                    frozen: state == "frozen" || state == "frozenSplit",
                    x: number(b"xSplit"),
                    y: number(b"ySplit"),
                    top_left: text(b"topLeftCell"),
                    active: text(b"activePane"),
                }));
            }
            // `<sheetViews>` comes before the cells, so anything here is far
            // too late to hold a pane.
            Event::Start(e) if local_name(e) == b"sheetData" => break,
            _ => {}
        }
    }
    Ok(None)
}

fn write_pane(out: &mut Vec<u8>, prefix: &[u8], pane: Option<&PaneTag>) {
    let Some(pane) = pane else { return };
    let mut sets = Vec::new();
    if pane.x > 0 {
        sets.push(Set::to(b"xSplit", pane.x.to_string()));
    }
    if pane.y > 0 {
        sets.push(Set::to(b"ySplit", pane.y.to_string()));
    }
    sets.push(Set::to(b"topLeftCell", pane.top_left.clone()));
    sets.push(Set::to(b"activePane", pane.active.clone()));
    sets.push(Set::to(
        b"state",
        if pane.frozen { "frozen" } else { "split" },
    ));
    open(out, prefix, b"pane", &sets, true);
}

/// Authors the whole `<sheetViews>` block for a file that never had one.
///
/// `workbookViewId` is required by the schema and there is exactly one workbook
/// view in anything this writes, so zero is not a guess.
fn write_sheet_views(out: &mut Vec<u8>, prefix: &[u8], pane: Option<&PaneTag>) {
    if pane.is_none() {
        return;
    }
    open(out, prefix, b"sheetViews", &[], false);
    open(
        out,
        prefix,
        b"sheetView",
        &[Set::to(b"workbookViewId", "0")],
        false,
    );
    write_pane(out, prefix, pane);
    close(out, prefix, b"sheetView");
    close(out, prefix, b"sheetViews");
}

/// The worksheet children the schema puts *after* `<sheetViews>`.
fn after_views(name: &[u8]) -> bool {
    matches!(
        name,
        b"sheetFormatPr"
            | b"cols"
            | b"sheetData"
            | b"sheetCalcPr"
            | b"sheetProtection"
            | b"protectedRanges"
            | b"scenarios"
            | b"autoFilter"
    ) || after_filter(name)
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

/// The conditional formats and validations as the file's own bytes read
/// back, which is what an edit is detected against.
fn file_rules(
    part: &str,
    data: &[u8],
) -> Result<(
    Vec<ss_model::cond::ConditionalFormat>,
    Vec<ss_model::cond::DataValidation>,
)> {
    let mut scratch = Sheet::new("scratch");
    let mut strings = StringTable::default();
    crate::sheet::parse(part, data, &mut scratch, &[], &mut strings)?;
    Ok((scratch.conditional_formats, scratch.validations))
}

/// The worksheet children `<conditionalFormatting>` must precede.
fn after_conditional(name: &[u8]) -> bool {
    name == b"dataValidations" || after_validations(name)
}

/// And the ones `<dataValidations>` must precede.
fn after_validations(name: &[u8]) -> bool {
    matches!(
        name,
        b"hyperlinks"
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
            | b"legacyDrawing"
            | b"legacyDrawingHF"
            | b"picture"
            | b"oleObjects"
            | b"controls"
            | b"webPublishItems"
            | b"tableParts"
            | b"extLst"
    )
}

/// Writes every `<conditionalFormatting>` block the model holds.
fn write_conditional_formats(out: &mut Vec<u8>, prefix: &[u8], sheet: &Sheet) {
    for block in &sheet.conditional_formats {
        if block.ranges.is_empty() || block.rules.is_empty() {
            continue;
        }
        let sqref = block
            .ranges
            .iter()
            .map(|r| range_ref(*r))
            .collect::<Vec<_>>()
            .join(" ");
        open(
            out,
            prefix,
            b"conditionalFormatting",
            &[Set::to(b"sqref", sqref)],
            false,
        );
        for rule in &block.rules {
            write_cf_rule(out, prefix, rule, block.anchor());
        }
        close(out, prefix, b"conditionalFormatting");
    }
}

fn write_cf_rule(
    out: &mut Vec<u8>,
    prefix: &[u8],
    rule: &ss_model::cond::CfRule,
    anchor: Option<ss_model::CellRef>,
) {
    use ss_model::cond::{CfKind, TextOp};
    let home = anchor.map_or_else(|| "A1".to_string(), |a| a.to_a1());
    let mut sets: Vec<Set> = Vec::new();
    let mut formulas: Vec<String> = Vec::new();
    let ty: String = match &rule.kind {
        CfKind::CellIs {
            operator,
            formulas: bounds,
        } => {
            sets.push(Set::to(b"operator", operator.as_xml()));
            formulas = bounds.clone();
            "cellIs".into()
        }
        CfKind::Expression { formula } => {
            formulas = vec![formula.clone()];
            "expression".into()
        }
        // A text rule carries its answer twice: our reader takes the `text`
        // attribute, Excel evaluates the derived formula, so both go out.
        CfKind::Text { op, text } => {
            sets.push(Set::to(b"text", text.clone()));
            let quoted = format!("\"{}\"", text.replace('"', "\"\""));
            formulas = vec![match op {
                TextOp::Contains => format!("NOT(ISERROR(SEARCH({quoted},{home})))"),
                TextOp::NotContains => format!("ISERROR(SEARCH({quoted},{home}))"),
                TextOp::BeginsWith => format!("LEFT({home},LEN({quoted}))={quoted}"),
                TextOp::EndsWith => format!("RIGHT({home},LEN({quoted}))={quoted}"),
            }];
            op.rule_type().into()
        }
        CfKind::TimePeriod { period } => {
            sets.push(Set::to(b"timePeriod", period.clone()));
            "timePeriod".into()
        }
        CfKind::Duplicates { unique } => if *unique {
            "uniqueValues"
        } else {
            "duplicateValues"
        }
        .into(),
        CfKind::Presence { blanks, negated } => match (blanks, negated) {
            (true, false) => "containsBlanks",
            (true, true) => "notContainsBlanks",
            (false, false) => "containsErrors",
            (false, true) => "notContainsErrors",
        }
        .into(),
        CfKind::Top10 {
            rank,
            percent,
            bottom,
        } => {
            sets.push(Set::to(b"rank", rank.to_string()));
            if *percent {
                sets.push(Set::to(b"percent", "1"));
            }
            if *bottom {
                sets.push(Set::to(b"bottom", "1"));
            }
            "top10".into()
        }
        CfKind::AboveAverage {
            above,
            equal_average,
            std_dev,
        } => {
            if !*above {
                sets.push(Set::to(b"aboveAverage", "0"));
            }
            if *equal_average {
                sets.push(Set::to(b"equalAverage", "1"));
            }
            if let Some(sd) = std_dev {
                sets.push(Set::to(b"stdDev", sd.to_string()));
            }
            "aboveAverage".into()
        }
        CfKind::ColorScale { .. } => "colorScale".into(),
        CfKind::DataBar { .. } => "dataBar".into(),
        CfKind::IconSet { .. } => "iconSet".into(),
        // A kind we never modelled keeps its name; its body is gone, which
        // is the price of editing the rules beside it. The untouched path
        // is what protects the sheet nobody edited.
        CfKind::Other(name) => name.clone(),
    };
    let mut all = vec![Set::to(b"type", ty)];
    if let Some(dxf) = rule.dxf {
        all.push(Set::to(b"dxfId", dxf.to_string()));
    }
    all.push(Set::to(b"priority", rule.priority.to_string()));
    if rule.stop_if_true {
        all.push(Set::to(b"stopIfTrue", "1"));
    }
    all.append(&mut sets);
    let visual = matches!(
        &rule.kind,
        CfKind::ColorScale { .. } | CfKind::DataBar { .. } | CfKind::IconSet { .. }
    );
    let childless = formulas.is_empty() && !visual;
    open(out, prefix, b"cfRule", &all, childless);
    for formula in &formulas {
        open(out, prefix, b"formula", &[], false);
        out.extend_from_slice(escape_text(formula).as_bytes());
        close(out, prefix, b"formula");
    }
    match &rule.kind {
        CfKind::ColorScale { stops, colors } => {
            open(out, prefix, b"colorScale", &[], false);
            for stop in stops {
                write_cfvo(out, prefix, stop);
            }
            for color in colors {
                write_cf_color(out, prefix, b"color", *color);
            }
            close(out, prefix, b"colorScale");
        }
        CfKind::DataBar {
            min,
            max,
            color,
            show_value,
        } => {
            let bar: Vec<Set> = if *show_value {
                Vec::new()
            } else {
                vec![Set::to(b"showValue", "0")]
            };
            open(out, prefix, b"dataBar", &bar, false);
            write_cfvo(out, prefix, min);
            write_cfvo(out, prefix, max);
            write_cf_color(out, prefix, b"color", *color);
            close(out, prefix, b"dataBar");
        }
        CfKind::IconSet {
            set,
            stops,
            show_value,
            reverse,
        } => {
            let mut icon = vec![Set::to(b"iconSet", set.clone())];
            if !*show_value {
                icon.push(Set::to(b"showValue", "0"));
            }
            if *reverse {
                icon.push(Set::to(b"reverse", "1"));
            }
            open(out, prefix, b"iconSet", &icon, false);
            for stop in stops {
                write_cfvo(out, prefix, stop);
            }
            close(out, prefix, b"iconSet");
        }
        _ => {}
    }
    if !childless {
        close(out, prefix, b"cfRule");
    }
}

fn write_cfvo(out: &mut Vec<u8>, prefix: &[u8], value: &ss_model::cond::CfValue) {
    let mut sets = vec![Set::to(b"type", value.kind.as_xml())];
    if !value.value.is_empty() {
        sets.push(Set::to(b"val", value.value.clone()));
    }
    open(out, prefix, b"cfvo", &sets, true);
}

/// A rule-owned colour, spelt the way the styles writer spells one.
fn write_cf_color(out: &mut Vec<u8>, prefix: &[u8], name: &[u8], color: ss_model::Color) {
    use ss_model::Color;
    let sets = match color {
        Color::Auto => vec![Set::to(b"auto", "1")],
        Color::Rgb(_) => vec![Set::to(b"rgb", color.to_hex().unwrap_or_default())],
        Color::Indexed(i) => vec![Set::to(b"indexed", i.to_string())],
        Color::Theme { index, tint } => {
            let mut sets = vec![Set::to(b"theme", index.to_string())];
            if tint != 0.0 {
                sets.push(Set::to(b"tint", ss_model::format_general(tint)));
            }
            sets
        }
    };
    open(out, prefix, name, &sets, true);
}

/// Writes the `<dataValidations>` wrapper and every rule in it, or nothing
/// at all when no rule survives the reader's own droppings.
fn write_validations(out: &mut Vec<u8>, prefix: &[u8], sheet: &Sheet) {
    use ss_model::cond::{DvKind, DvOperator, DvSeverity};
    let rules: Vec<_> = sheet
        .validations
        .iter()
        .filter(|v| !v.ranges.is_empty() && v.kind != DvKind::None)
        .collect();
    if rules.is_empty() {
        return;
    }
    open(
        out,
        prefix,
        b"dataValidations",
        &[Set::to(b"count", rules.len().to_string())],
        false,
    );
    for v in rules {
        let mut sets = vec![Set::to(b"type", v.kind.as_xml())];
        if v.operator != DvOperator::Between {
            sets.push(Set::to(b"operator", v.operator.as_xml()));
        }
        if v.severity != DvSeverity::Stop {
            sets.push(Set::to(b"errorStyle", v.severity.as_xml()));
        }
        if v.allow_blank {
            sets.push(Set::to(b"allowBlank", "1"));
        }
        // Inverted in the file: `showDropDown="1"` *hides* the arrow.
        if !v.show_dropdown {
            sets.push(Set::to(b"showDropDown", "1"));
        }
        // The model does not carry these two booleans, and Excel keys the
        // popups off them: the error alert is always armed, the prompt
        // exactly when there is something to say.
        sets.push(Set::to(b"showErrorMessage", "1"));
        if !v.prompt_title.is_empty() || !v.prompt_message.is_empty() {
            sets.push(Set::to(b"showInputMessage", "1"));
        }
        if !v.error_title.is_empty() {
            sets.push(Set::to(b"errorTitle", v.error_title.clone()));
        }
        if !v.error_message.is_empty() {
            sets.push(Set::to(b"error", v.error_message.clone()));
        }
        if !v.prompt_title.is_empty() {
            sets.push(Set::to(b"promptTitle", v.prompt_title.clone()));
        }
        if !v.prompt_message.is_empty() {
            sets.push(Set::to(b"prompt", v.prompt_message.clone()));
        }
        let sqref = v
            .ranges
            .iter()
            .map(|r| range_ref(*r))
            .collect::<Vec<_>>()
            .join(" ");
        sets.push(Set::to(b"sqref", sqref));
        let childless = v.formula1.is_empty() && v.formula2.is_empty();
        open(out, prefix, b"dataValidation", &sets, childless);
        for (name, formula) in [
            (b"formula1".as_slice(), &v.formula1),
            (b"formula2".as_slice(), &v.formula2),
        ] {
            if !formula.is_empty() {
                open(out, prefix, name, &[], false);
                out.extend_from_slice(escape_text(formula).as_bytes());
                close(out, prefix, name);
            }
        }
        if !childless {
            close(out, prefix, b"dataValidation");
        }
    }
    close(out, prefix, b"dataValidations");
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
            b"outlineLevel",
            b"collapsed",
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
    if let Some(level) = sheet.row_outlines.get(&row) {
        sets.push(Set::to(b"outlineLevel", level.to_string()));
    }
    if sheet.row_collapsed.contains(&row) {
        sets.push(Set::to(b"collapsed", "1"));
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
    /// Outline depth and the collapse marker, so a Group that changes
    /// nothing *but* these is still seen as a change — the fast path would
    /// otherwise splice the ungrouped bytes back and discard it silently.
    outline: u8,
    collapsed: bool,
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
        let outline = raw_attr(&start, b"outlineLevel")
            .and_then(|a| parse_u32(&a.value))
            .unwrap_or(0)
            .min(7) as u8;
        let collapsed = raw_attr(&start, b"collapsed")
            .and_then(|a| parse_bool(&a.value))
            .unwrap_or(false);
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
            outline,
            collapsed,
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
            && self.outline == sheet.row_outlines.get(&self.index).copied().unwrap_or(0)
            && self.collapsed == sheet.row_collapsed.contains(&self.index)
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

/// Everything a `<col>` says about one column that the model also holds.
///
/// Style and width travel together because they are written together: they are
/// two attributes of one element, and a run of columns can only be spelled as
/// one `<col>` when they agree about both.
#[derive(Clone, Copy, PartialEq)]
struct ColumnLook {
    style: u32,
    /// `None` where the column has no width of its own — which is what a
    /// `<col>` carrying nothing but a style means, and it is not the same as a
    /// width that happens to equal the default.
    width: Option<f64>,
    /// Outline depth and collapse marker. In the look so a Group that
    /// changes nothing else still compares unequal to the file's bytes —
    /// the early-out would otherwise splice the ungrouped element back.
    outline: u8,
    collapsed: bool,
}

impl ColumnLook {
    fn of(sheet: &Sheet, column: u32) -> Self {
        ColumnLook {
            style: sheet
                .column_styles
                .get(&column)
                .map(|s| s.0)
                .unwrap_or_default(),
            width: sheet.column_widths.get(&column).copied(),
            outline: sheet.column_outlines.get(&column).copied().unwrap_or(0),
            collapsed: sheet.column_collapsed.contains(&column),
        }
    }

    fn is_default(self) -> bool {
        self.style == 0 && self.width.is_none() && self.outline == 0 && !self.collapsed
    }

    /// The look a `<col>` element itself states.
    ///
    /// `hidden` reads back as a width of zero, which is how the reader records
    /// it and how a hidden *row* is already modelled. Keeping the two spellings
    /// apart here would make an untouched hidden column compare unequal to
    /// itself, and the writer would rebuild every one of them on every save.
    fn stated(e: &BytesStart<'_>) -> Self {
        let hidden = raw_attr(e, b"hidden")
            .and_then(|a| parse_bool(&a.value))
            .unwrap_or(false);
        ColumnLook {
            style: raw_attr(e, b"style")
                .and_then(|a| parse_u32(&a.value))
                .unwrap_or(0),
            width: if hidden {
                Some(0.0)
            } else {
                raw_attr(e, b"width").and_then(|a| parse_f64(&a.value))
            },
            outline: raw_attr(e, b"outlineLevel")
                .and_then(|a| parse_u32(&a.value))
                .unwrap_or(0)
                .min(7) as u8,
            collapsed: raw_attr(e, b"collapsed")
                .and_then(|a| parse_bool(&a.value))
                .unwrap_or(false),
        }
    }

    /// The attributes it takes to say this, over whatever the element holds.
    ///
    /// `customWidth` rides with the width because the pair is what Excel reads
    /// as "somebody chose this": a bare `width` is a producer's own measurement
    /// and Excel is free to re-fit it, which would quietly undo the drag that
    /// put it there.
    ///
    /// A hidden column writes `hidden` and leaves `width` alone, exactly as a
    /// hidden row writes `hidden` and leaves `ht` alone. `width="0"` would hide
    /// it just as well on screen and would restore it to nothing when somebody
    /// unhid it, which is not what unhiding is for.
    fn sets(self) -> Vec<Set<'static>> {
        let mut sets = vec![Set::maybe(
            b"style",
            (self.style != 0).then(|| self.style.to_string()),
        )];
        match self.width {
            Some(0.0) => sets.push(Set::to(b"hidden", "1")),
            Some(width) => {
                sets.push(Set::to(b"width", ss_model::format_general(width)));
                sets.push(Set::to(b"customWidth", "1"));
                sets.push(Set::off(b"hidden"));
            }
            // `width` is deliberately left alone rather than removed. The only
            // thing that takes a width out of the model is *unhiding* a
            // column — hiding overwrote it with a zero and nothing remembers
            // what it was — and the width still sitting in the file is exactly
            // the size the column should come back at.
            None => sets.push(Set::off(b"hidden")),
        }
        if self.outline > 0 {
            sets.push(Set::to(b"outlineLevel", self.outline.to_string()));
        } else {
            sets.push(Set::off(b"outlineLevel"));
        }
        if self.collapsed {
            sets.push(Set::to(b"collapsed", "1"));
        } else {
            sets.push(Set::off(b"collapsed"));
        }
        sets
    }
}

/// The default column width, which is what Excel writes when it has to write
/// something. A `<col>` with no width at all is repaired on open.
const DEFAULT_WIDTH: f64 = 8.43;

/// A `<col>` from the file, re-emitted so its span carries the model's widths
/// and styles.
///
/// A span whose columns no longer agree is *split* rather than replaced: each
/// piece is a retag of the original element, so every attribute nobody here
/// models — `outlineLevel`, `collapsed`, `bestFit`, `phonetic` — goes back
/// unchanged on each piece.
fn write_col(out: &mut Vec<u8>, original: &[u8], e: &BytesStart<'_>, sheet: &Sheet) {
    let Some((min, max)) = col_span(e) else {
        out.extend_from_slice(original);
        return;
    };
    let file = ColumnLook::stated(e);
    // Beyond the limit the reader materializes, the model knows nothing and the
    // file's own value is the only truth there is.
    let horizon = max.min(min.saturating_add(COLS_SPAN_LIMIT));

    let wanted = |column: u32| ColumnLook::of(sheet, column - 1);
    if (min..=horizon).all(|c| wanted(c) == file) && horizon == max {
        out.extend_from_slice(original);
        return;
    }

    // Split into runs that agree.
    let mut start = min;
    while start <= max {
        let look = if start <= horizon {
            wanted(start)
        } else {
            file
        };
        let mut end = start;
        while end < max {
            let next = end + 1;
            let next_look = if next <= horizon { wanted(next) } else { file };
            if next_look != look {
                break;
            }
            end = next;
        }
        let mut sets = vec![
            Set::to(b"min", start.to_string()),
            Set::to(b"max", end.to_string()),
        ];
        sets.extend(look.sets());
        out.extend_from_slice(&retag(e, &sets, true));
        start = end + 1;
    }
}

/// `min` and `max` off a `<col>`, if it states a span that makes sense.
fn col_span(e: &BytesStart<'_>) -> Option<(u32, u32)> {
    let min = raw_attr(e, b"min").and_then(|a| parse_u32(&a.value))?;
    let max = raw_attr(e, b"max").and_then(|a| parse_u32(&a.value))?;
    (min > 0 && min <= max).then_some((min, max))
}

/// How far into a `<col>` span the reader materializes columns.
///
/// The same number the reader uses, and it has to be: past it the model holds
/// nothing, so the file's own value is the only truth there is.
const COLS_SPAN_LIMIT: u32 = 1024;

/// The columns the file's own `<col>` elements speak for, in one pass ahead of
/// the rewrite.
fn file_columns(part: &str, data: &[u8]) -> Result<std::collections::BTreeSet<u32>> {
    let mut covered = std::collections::BTreeSet::new();
    let mut splicer = Splicer::new(part, data);
    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"col" => {
                if let Some((min, max)) = col_span(e) {
                    for column in min..=max.min(min.saturating_add(COLS_SPAN_LIMIT)) {
                        covered.insert(column - 1);
                    }
                }
            }
            // The schema puts `<cols>` immediately before `<sheetData>`, so a
            // `<col>` after this point belongs to something else.
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"sheetData" => break,
            _ => {}
        }
    }
    Ok(covered)
}

/// Columns the model has an opinion about that no `<col>` in the file speaks
/// for, gathered into runs that agree.
fn missing_columns(
    sheet: &Sheet,
    covered: &std::collections::BTreeSet<u32>,
) -> Vec<(u32, u32, ColumnLook)> {
    let mut runs: Vec<(u32, u32, ColumnLook)> = Vec::new();
    // Every map, because a column may carry any one of these alone — a width,
    // a style, or an outline level — and each on its own is worth a `<col>`.
    // A BTreeSet walks them in order, so a run only ever grows at its end.
    let named: std::collections::BTreeSet<u32> = sheet
        .column_styles
        .keys()
        .chain(sheet.column_widths.keys())
        .chain(sheet.column_outlines.keys())
        .chain(sheet.column_collapsed.iter())
        .copied()
        .collect();
    for column in named {
        if covered.contains(&column) {
            continue;
        }
        let look = ColumnLook::of(sheet, column);
        if look.is_default() {
            continue;
        }
        match runs.last_mut() {
            Some((_, last, existing)) if *last + 1 == column && *existing == look => *last = column,
            _ => runs.push((column, column, look)),
        }
    }
    runs
}

/// Emits the runs `missing_columns` found that come before `before`.
///
/// `next` is where the last call stopped, so the runs go out in order and each
/// one goes out once. `u32::MAX` means "whatever is left".
fn write_columns(
    out: &mut Vec<u8>,
    prefix: &[u8],
    runs: &[(u32, u32, ColumnLook)],
    next: &mut usize,
    before: u32,
) {
    while let Some((first, last, look)) = runs.get(*next) {
        // `min` and `max` are one-based; `first` and `last` are not.
        if first + 1 >= before {
            return;
        }
        *next += 1;
        let mut sets = vec![
            Set::to(b"min", (first + 1).to_string()),
            Set::to(b"max", (last + 1).to_string()),
        ];
        if look.style != 0 {
            sets.push(Set::to(b"style", look.style.to_string()));
        }
        // Written out in full rather than through `ColumnLook::sets`, because
        // that one edits an element that exists and this one builds one that
        // does not: there is no `width` here to leave alone. Excel repairs a
        // `<col>` with none at all, so every branch carries one.
        match look.width {
            Some(0.0) => {
                sets.push(Set::to(b"width", ss_model::format_general(DEFAULT_WIDTH)));
                sets.push(Set::to(b"hidden", "1"));
            }
            Some(width) => {
                sets.push(Set::to(b"width", ss_model::format_general(width)));
                sets.push(Set::to(b"customWidth", "1"));
            }
            None => sets.push(Set::to(b"width", ss_model::format_general(DEFAULT_WIDTH))),
        }
        if look.outline > 0 {
            sets.push(Set::to(b"outlineLevel", look.outline.to_string()));
        }
        if look.collapsed {
            sets.push(Set::to(b"collapsed", "1"));
        }
        open(out, prefix, b"col", &sets, true);
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
    // The `<sheetFormatPr>` rides along because Excel always writes one
    // beside an outline — a file with levels and no declaration is a file
    // the writer would correct, and this fixture stands in for untouched.
    const WITH_COLS: &str = concat!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://x">"#,
        r#"<sheetFormatPr outlineLevelCol="1"/>"#,
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
        // The file's span covers A to C; only B is styled now. Splitting has
        // to carry `customWidth` and `outlineLevel` onto every piece — the
        // width from the file's own spelling, the outline from the model,
        // which learnt it when grouping did.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        with_cols_widths(&mut sheet);
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

    /// The widths and outlines `WITH_COLS` states, as the reader would have
    /// left them.
    ///
    /// A model with no width against a file that has one is not a sheet nobody
    /// touched — it is a sheet whose widths were just cleared, and the writer
    /// is right to take them out. So a fixture standing in for an untouched
    /// sheet has to say what the file says. The outline levels joined the
    /// model when grouping did, and the same rule now covers them.
    fn with_cols_widths(sheet: &mut Sheet) {
        for column in 0..3 {
            sheet.column_widths.insert(column, 12.5);
            sheet.column_outlines.insert(column, 1);
        }
    }

    #[test]
    fn a_column_nobody_restyled_comes_back_byte_for_byte() {
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        with_cols_widths(&mut sheet);
        assert_eq!(written_from(WITH_COLS, &sheet), WITH_COLS);
    }

    #[test]
    fn a_dragged_column_edge_reaches_the_file() {
        // The bug this exists for: `<cols>` had a writer for styles and none
        // for widths, so dragging a column wider showed on screen, survived
        // undo, and was gone the moment the file was read back.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        with_cols_widths(&mut sheet);
        sheet.column_widths.insert(1, 30.0);

        let out = written_from(WITH_COLS, &sheet);
        assert_eq!(out.matches("<col ").count(), 3, "B split out: {out}");
        assert!(
            out.contains(r#"min="2" max="2" width="30" customWidth="1""#),
            "{out}"
        );
        assert_eq!(out.matches(r#"width="12.5""#).count(), 2, "A and C: {out}");
    }

    #[test]
    fn a_width_on_a_column_the_file_never_mentioned_grows_a_col_element() {
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.column_widths.insert(4, 22.0);

        let out = written_from(
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
            &sheet,
        );
        assert!(
            out.contains(r#"<cols><col min="5" max="5" width="22" customWidth="1"/></cols>"#),
            "{out}"
        );
    }

    #[test]
    fn a_column_narrowed_to_nothing_is_hidden_and_keeps_the_width_it_had() {
        // Zero is how the model spells hidden, and `hidden` is the attribute
        // Excel's own Unhide looks for. The width beside it is left alone on
        // purpose: it is the size the column comes back at, and `width="0"`
        // would bring it back to nothing.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        with_cols_widths(&mut sheet);
        sheet.column_widths.insert(0, 0.0);

        let out = written_from(WITH_COLS, &sheet);
        assert!(
            out.contains(
                r#"<col min="1" max="1" width="12.5" customWidth="1" outlineLevel="1" hidden="1"/>"#
            ),
            "{out}"
        );
    }

    #[test]
    fn a_column_excel_hid_reads_as_hidden_and_writes_back_unchanged() {
        // `hidden="1"` with no width is all Excel writes. Read as "no width at
        // all" it comes back visible; read as a width of zero — which is what
        // a hidden row already does — it round-trips.
        const HIDDEN: &str = concat!(
            r#"<?xml version="1.0"?><worksheet xmlns="http://x">"#,
            r#"<cols><col min="2" max="2" width="9.14" hidden="1"/></cols>"#,
            r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        );
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.column_widths.insert(1, 0.0);
        assert_eq!(written_from(HIDDEN, &sheet), HIDDEN);

        // And unhiding it takes the attribute back off and leaves the width,
        // which is the size the column comes back at. Nothing in the model
        // remembers that width — hiding overwrote it with a zero — so taking
        // the file's word for it is the only way the column returns to the
        // size it was rather than to the default.
        sheet.column_widths.remove(&1);
        let out = written_from(HIDDEN, &sheet);
        assert!(!out.contains("hidden"), "{out}");
        assert!(out.contains(r#"width="9.14""#), "{out}");
    }

    #[test]
    fn an_added_column_goes_in_where_it_belongs_rather_than_at_the_end() {
        // `<col>` elements out of numerical order are legal, and no spreadsheet
        // has ever written them that way. The file speaks for B and C; A and D
        // have to be threaded either side of it.
        let mut sheet = Sheet::new("Data");
        set(&mut sheet, "A1", CellValue::Number(1.0), 0);
        sheet.column_widths.insert(0, 20.0);
        sheet.column_widths.insert(1, 9.14);
        sheet.column_widths.insert(2, 9.14);
        sheet.column_widths.insert(3, 30.0);

        let out = written_from(
            concat!(
                r#"<worksheet><cols><col min="2" max="3" width="9.14" customWidth="1"/></cols>"#,
                r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
            ),
            &sheet,
        );
        let order: Vec<&str> = out
            .match_indices("min=")
            .map(|(i, _)| &out[i..i + 7])
            .collect();
        assert_eq!(
            order,
            vec![r#"min="1""#, r#"min="2""#, r#"min="4""#],
            "{out}"
        );
        // The file's own span is untouched in passing.
        assert!(
            out.contains(r#"<col min="2" max="3" width="9.14" customWidth="1"/>"#),
            "{out}"
        );
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
    fn a_freeze_reaches_the_file_it_was_made_in() {
        let mut sheet = Sheet::new("Data");
        sheet.panes = Some(ss_model::Panes::frozen(CellRef::new(1, 0)));

        // A file with no view at all — an authored workbook — grows one.
        let out = written_from(r#"<worksheet><sheetData/></worksheet>"#, &sheet);
        assert_eq!(
            out,
            r#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><sheetData/></worksheet>"#
        );

        // A view that exists keeps everything it says about itself.
        let out = written_from(
            r#"<worksheet><sheetViews><sheetView tabSelected="1" zoomScale="90" workbookViewId="0"><selection activeCell="C7" sqref="C7"/></sheetView></sheetViews><sheetData/></worksheet>"#,
            &sheet,
        );
        assert!(out.contains(r#"tabSelected="1" zoomScale="90""#), "{out}");
        assert!(
            out.contains(r#"<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection activeCell="C7" sqref="C7" pane="bottomLeft"/>"#),
            "the pane goes before the selection, and the selection moves into it: {out}"
        );
    }

    #[test]
    fn unfreezing_takes_the_pane_out_rather_than_leaving_an_empty_one() {
        let sheet = Sheet::new("Data");
        let out = written_from(
            r#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane ySplit="3" topLeftCell="A4" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="B9" sqref="B9"/></sheetView></sheetViews><sheetData/></worksheet>"#,
            &sheet,
        );
        assert!(!out.contains("<pane"), "{out}");
        assert!(
            out.contains(r#"<selection activeCell="B9" sqref="B9"/>"#),
            "the cursor stays where it was, in a sheet that now has one pane: {out}"
        );
    }

    #[test]
    fn a_sheet_nobody_split_is_not_touched_by_the_pane_writer() {
        // The no-edit round trip is the whole contract: a `topLeftCell` we did
        // not move, and a `state` spelled `frozenSplit`, both survive a save.
        let mut sheet = Sheet::new("Data");
        sheet.panes = Some(ss_model::Panes::frozen(CellRef::new(3, 5)));
        sheet.view.top_left = Some(CellRef::from_a1("F4").unwrap());
        let original = r#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane xSplit="5" ySplit="3" topLeftCell="F4" activePane="bottomRight" state="frozenSplit"/><selection pane="topRight" activeCell="G1" sqref="G1"/><selection pane="bottomRight" activeCell="B5" sqref="B5"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        assert_eq!(written_from(original, &sheet), original);
    }

    #[test]
    fn a_split_is_written_as_the_distance_excel_measures_it_in() {
        let mut sheet = Sheet::new("Data");
        sheet.panes = Some(ss_model::Panes::split(CellRef::new(0, 2)));
        let out = written_from(r#"<worksheet><sheetData/></worksheet>"#, &sheet);
        // Two default columns: 64 pixels each, and a pixel is fifteen twips.
        assert!(
            out.contains(r#"<pane xSplit="1920" topLeftCell="C1" activePane="topRight" state="split"/>"#),
            "{out}"
        );
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
