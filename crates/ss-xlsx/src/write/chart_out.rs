//! Retitling a chart without disturbing anything else in it.
//!
//! `chartSpace` is the most elaborate part in a workbook — axis formatting,
//! gradients, 3-D rotation, trendlines, data labels, per-point overrides — and
//! almost none of it is modeled. So the title is spliced in place, exactly as a
//! cell is: the file's own bytes go back everywhere except inside `<c:tx>`.
//!
//! Adding a title to a chart that has none *does* mean authoring an element.
//! That is allowed for the same reason `sheet_out` may author a `<c:>`: it is a
//! structure we understand completely, and everything around it is retained.
//! The schema fixes `<c:title>` as the first child of `<c:chart>`, and
//! `<c:autoTitleDeleted val="1"/>` has to be cleared at the same time or Excel
//! hides the title we just wrote.
//!
//! The inspector's properties — a series' colour, a doughnut's hole, the
//! legend's place — go the same way, in [`restyle`]: each at the one place
//! the schema fixes for it, everything else byte for byte.

use quick_xml::events::Event;
use ss_model::chart::{Axis, ChartKind, LegendPosition, Plot, Symbol};

use crate::error::Result;
use crate::write::splice::{close, escape_text, open, prefix_of, retag, Set, Splicer};
use crate::xml::{end_local_name, local_name};

/// Replaces the chart's title text, or removes it.
///
/// A title whose text comes from a cell (`<c:tx><c:strRef>`) is left alone:
/// changing it would mean writing to the cell, which is the user's to do.
pub(crate) fn retitle(part: &str, data: &[u8], title: Option<&str>) -> Result<Vec<u8>> {
    let found = scan(part, data)?;
    if found.from_cell {
        return Ok(data.to_vec());
    }
    // Three cases, not two. A chart may carry a `<c:title>` with no text in it
    // at all — Excel writes one holding only formatting when the automatic
    // title has been switched off — and there is then no run to write into. So
    // an existing title is *replaced wholesale* unless it has runs to edit.
    let author = title.is_some() && !found.has_runs;
    let discard = title.is_none() || author;

    let mut out = Vec::with_capacity(data.len() + 128);
    let mut splicer = Splicer::new(part, data);
    let mut prefix = Vec::new();
    // A guess, used only when the chart has no DrawingML element to learn the
    // real prefix from — which happens exactly when it has no title text yet.
    let mut drawing_prefix: Vec<u8> = b"a:".to_vec();
    // Nesting inside the title, so that a `<a:t>` belonging to a data label or
    // an axis is never mistaken for the title's.
    let mut in_title = 0usize;
    let mut in_run = 0usize;
    let mut written = false;

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) if local_name(e) == b"chartSpace" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if local_name(e) == b"chart" => {
                out.extend_from_slice(splicer.bytes(span));
                // The schema fixes the position: first child of `<c:chart>`.
                if let Some(text) = title.filter(|_| author) {
                    write_title(&mut out, &prefix, &drawing_prefix, text);
                }
            }
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"title" => {
                if matches!(event, Event::Empty(_)) {
                    if !discard {
                        out.extend_from_slice(splicer.bytes(span));
                    }
                    continue;
                }
                in_title += 1;
                if !discard {
                    out.extend_from_slice(splicer.bytes(span));
                }
            }
            Event::End(e) if end_local_name(e) == b"title" => {
                in_title = in_title.saturating_sub(1);
                if !discard {
                    out.extend_from_slice(splicer.bytes(span));
                }
            }

            _ if in_title > 0 && discard => {}

            Event::Start(e) if in_title > 0 && local_name(e) == b"r" => {
                in_run += 1;
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::End(e) if in_title > 0 && end_local_name(e) == b"r" => {
                in_run = in_run.saturating_sub(1);
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if in_run > 0 && local_name(e) == b"t" => {
                out.extend_from_slice(splicer.bytes(span));
                // The first run gets the whole title; the rest are emptied,
                // because a title split across three runs must not come back as
                // the new text three times.
                if let (Some(text), false) = (title, written) {
                    out.extend_from_slice(escape_text(text).as_bytes());
                    written = true;
                }
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if in_run > 0 => {
                // Dropped: the run's text is whatever was just written.
            }

            // Excel writes this when the user deletes a chart's automatic
            // title. Left at 1, the title we just wrote would not be shown.
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"autoTitleDeleted" => {
                let value = if title.is_some() { "0" } else { "1" };
                out.extend_from_slice(&retag(
                    e,
                    &[Set::to(b"val", value)],
                    matches!(event, Event::Empty(_)),
                ));
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
        // The drawing prefix is learnt from any DrawingML element in the file,
        // wherever it is: a data label's run will do just as well as a title's.
        if drawing_prefix == b"a:" {
            if let Event::Start(e) | Event::Empty(e) = &event {
                if matches!(local_name(e), b"bodyPr" | b"p" | b"r" | b"t") {
                    let seen = prefix_of(e);
                    if !seen.is_empty() {
                        drawing_prefix = seen;
                    }
                }
            }
        }
    }

    Ok(out)
}

fn write_title(out: &mut Vec<u8>, prefix: &[u8], drawing: &[u8], text: &str) {
    open(out, prefix, b"title", &[], false);
    open(out, prefix, b"tx", &[], false);
    open(out, prefix, b"rich", &[], false);
    open(out, drawing, b"bodyPr", &[], true);
    open(out, drawing, b"p", &[], false);
    open(out, drawing, b"r", &[], false);
    open(out, drawing, b"t", &[], false);
    out.extend_from_slice(escape_text(text).as_bytes());
    close(out, drawing, b"t");
    close(out, drawing, b"r");
    close(out, drawing, b"p");
    close(out, prefix, b"rich");
    close(out, prefix, b"tx");
    open(out, prefix, b"overlay", &[Set::to(b"val", "0")], true);
    close(out, prefix, b"title");
}

struct Found {
    has_title: bool,
    /// The title has at least one text run to write into. A `<c:title>` holding
    /// only formatting has none.
    has_runs: bool,
    /// The title's text comes from a cell rather than being typed.
    from_cell: bool,
}

fn scan(part: &str, data: &[u8]) -> Result<Found> {
    let mut splicer = Splicer::new(part, data);
    let mut out = Found {
        has_title: false,
        has_runs: false,
        from_cell: false,
    };
    let mut in_title = 0usize;
    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"title" => {
                    out.has_title = true;
                    if !matches!(event, Event::Empty(_)) {
                        in_title += 1;
                    }
                }
                b"r" if in_title > 0 => out.has_runs = true,
                b"strRef" | b"numRef" if in_title > 0 => out.from_cell = true,
                _ => {}
            },
            Event::End(e) if end_local_name(e) == b"title" => {
                in_title = in_title.saturating_sub(1);
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Carries the inspector's changes into a chart part, and nothing else.
///
/// `stored` is the plot as the file has it and `wanted` the plot as the model
/// has it; only what differs between them is touched. Each property lives at
/// one place the schema fixes, so each is spliced there — a `val` rewritten
/// where the element exists, the element authored in its place where it does
/// not, an element dropped whole where the model has none. Excel's own parts
/// state everything, so authoring is the rare case; it exists for the parts
/// of other producers, which say less.
///
/// A chart whose *kind* changed is not a case for this: every one of its
/// elements moves, and the part is written afresh from the model instead.
pub(crate) fn restyle(part: &str, data: &[u8], stored: &Plot, wanted: &Plot) -> Result<Vec<u8>> {
    let diff = Diff::between(stored, wanted);
    if diff.is_empty() {
        return Ok(data.to_vec());
    }

    let mut out = Vec::with_capacity(data.len() + 256);
    let mut splicer = Splicer::new(part, data);
    let mut prefix: Vec<u8> = b"c:".to_vec();
    let mut drawing: Vec<u8> = b"a:".to_vec();
    let mut seen_drawing = false;

    // Where we are. The plot-type element is the one the kind names;
    // everything the inspector sets that is not a series' lives directly in
    // it, or in an axis, or in the legend.
    let mut in_plot_type = false;
    let mut axis: Option<bool> = None; // Some(true) = value axis
    let mut in_legend = false;
    // Inside a series: its index, and how deep below `<c:ser>` we are, so that
    // a direct child can be told from a data label's or a point's.
    let mut series: Option<usize> = None;
    let mut next_series = 0usize;
    let mut level = 0usize;
    let mut wrote_props = false;
    let mut wrote_marker = false;
    let mut wrote_smooth = false;
    let mut in_marker = false;
    // Chart-level elements seen, so the missing ones can be authored where
    // they belong.
    let mut saw_vary = false;
    let mut saw_gap = false;
    let mut saw_overlap = false;
    let mut saw_hole = false;
    let mut saw_line_marker = false;
    let mut wrote_legend_pos = false;
    // Dropping an element whole: how many of its descendants are open.
    let mut skipping = 0usize;

    while let Some((event, span)) = splicer.next()? {
        if skipping > 0 {
            match &event {
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                _ => {}
            }
            continue;
        }
        if !seen_drawing {
            if let Event::Start(e) | Event::Empty(e) = &event {
                if matches!(
                    local_name(e),
                    b"bodyPr" | b"p" | b"r" | b"t" | b"solidFill" | b"ln" | b"noFill"
                ) {
                    let seen = prefix_of(e);
                    if !seen.is_empty() {
                        drawing = seen;
                    }
                    seen_drawing = true;
                }
            }
        }

        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = local_name(e);
                if name == b"chartSpace" {
                    prefix = prefix_of(e);
                }

                // ---- inside a series ----
                if let Some(index) = series {
                    let direct = level == 0;
                    let want = diff.series.get(index);
                    if direct {
                        // Anything a series' own properties stand before.
                        if !wrote_props && AFTER_PROPS.contains(&name) {
                            if let Some(Some(color)) = want.map(|w| w.color) {
                                write_props(&mut out, &prefix, &drawing, color, wanted);
                            }
                            wrote_props = true;
                        }
                        if !wrote_marker && AFTER_MARKER.contains(&name) {
                            if let Some(Some(symbol)) = want.map(|w| w.symbol) {
                                write_marker(&mut out, &prefix, symbol);
                            }
                            wrote_marker = true;
                        }
                        if !wrote_smooth && name == b"extLst" {
                            if let Some(Some(smooth)) = want.map(|w| w.smooth) {
                                write_val(
                                    &mut out,
                                    &prefix,
                                    b"smooth",
                                    if smooth { "1" } else { "0" },
                                );
                            }
                            wrote_smooth = true;
                        }
                        match name {
                            b"spPr" => {
                                if let Some(Some(color)) = want.map(|w| w.color) {
                                    write_props(&mut out, &prefix, &drawing, color, wanted);
                                    wrote_props = true;
                                    if !empty {
                                        skipping = 1;
                                    }
                                    continue;
                                }
                            }
                            b"marker" => {
                                if let Some(Some(symbol)) = want.map(|w| w.symbol) {
                                    write_marker_open(&mut out, &prefix, symbol);
                                    wrote_marker = true;
                                    if empty {
                                        close(&mut out, &prefix, b"marker");
                                    } else {
                                        in_marker = true;
                                        level += 1;
                                    }
                                    continue;
                                }
                            }
                            b"smooth" => {
                                if let Some(Some(smooth)) = want.map(|w| w.smooth) {
                                    out.extend_from_slice(&retag(
                                        e,
                                        &[Set::to(b"val", if smooth { "1" } else { "0" })],
                                        empty,
                                    ));
                                    wrote_smooth = true;
                                    if !empty {
                                        level += 1;
                                    }
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    } else if in_marker && level == 1 && name == b"symbol" {
                        // Ours went first; the file's is dropped.
                        if !empty {
                            skipping = 1;
                        }
                        continue;
                    }
                    out.extend_from_slice(splicer.bytes(span));
                    if !empty {
                        level += 1;
                    }
                    continue;
                }

                // ---- the plot-type element and its direct children ----
                if in_plot_type {
                    match name {
                        b"ser" => {
                            if !saw_vary {
                                if let Some(vary) = diff.vary_colors {
                                    write_val(
                                        &mut out,
                                        &prefix,
                                        b"varyColors",
                                        if vary { "1" } else { "0" },
                                    );
                                }
                                saw_vary = true;
                            }
                            out.extend_from_slice(splicer.bytes(span));
                            series = Some(next_series);
                            next_series += 1;
                            level = 0;
                            wrote_props = false;
                            wrote_marker = false;
                            wrote_smooth = false;
                            in_marker = false;
                            continue;
                        }
                        b"varyColors" => {
                            saw_vary = true;
                            if let Some(vary) = diff.vary_colors {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", if vary { "1" } else { "0" })],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"gapWidth" => {
                            saw_gap = true;
                            if let Some(gap) = diff.gap {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", whole(gap))],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"overlap" => {
                            saw_overlap = true;
                            if let Some(overlap) = diff.overlap {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", whole(overlap))],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"holeSize" => {
                            saw_hole = true;
                            if let Some(hole) = diff.hole {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", whole(hole))],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"marker" => {
                            saw_line_marker = true;
                            if let Some(markers) = diff.markers {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", if markers { "1" } else { "0" })],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"scatterStyle" => {
                            if let Some(lines) = diff.scatter_lines {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", if lines { "lineMarker" } else { "marker" })],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"axId" | b"smooth" | b"serLines" | b"extLst" => {
                            // The tail of the element: whatever was not
                            // stated goes in here, in the schema's order.
                            if !saw_gap {
                                if let Some(gap) = diff.gap {
                                    write_val(&mut out, &prefix, b"gapWidth", &whole(gap));
                                }
                                saw_gap = true;
                            }
                            if !saw_overlap {
                                if let Some(overlap) = diff.overlap {
                                    write_val(&mut out, &prefix, b"overlap", &whole(overlap));
                                }
                                saw_overlap = true;
                            }
                            if !saw_line_marker && name != b"serLines" {
                                if let Some(markers) = diff.markers {
                                    write_val(
                                        &mut out,
                                        &prefix,
                                        b"marker",
                                        if markers { "1" } else { "0" },
                                    );
                                }
                                saw_line_marker = true;
                            }
                            if !saw_hole && name == b"extLst" {
                                if let Some(hole) = diff.hole {
                                    write_val(&mut out, &prefix, b"holeSize", &whole(hole));
                                }
                                saw_hole = true;
                            }
                        }
                        _ => {}
                    }
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }

                // ---- an axis ----
                if let Some(value) = axis {
                    let want = if value {
                        &diff.val_axis
                    } else {
                        &diff.cat_axis
                    };
                    match name {
                        b"delete" => {
                            if let Some(gone) = want.deleted {
                                out.extend_from_slice(&retag(
                                    e,
                                    &[Set::to(b"val", if gone { "1" } else { "0" })],
                                    empty,
                                ));
                                continue;
                            }
                        }
                        b"axPos" => {
                            out.extend_from_slice(splicer.bytes(span));
                            if want.gridlines == Some(true) {
                                open(&mut out, &prefix, b"majorGridlines", &[], true);
                            }
                            continue;
                        }
                        b"majorGridlines" => {
                            if want.gridlines == Some(false) {
                                if !empty {
                                    skipping = 1;
                                }
                                continue;
                            }
                            if want.gridlines == Some(true) {
                                // Authored after `axPos` already.
                                if !empty {
                                    skipping = 1;
                                }
                                continue;
                            }
                        }
                        _ => {}
                    }
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }

                // ---- the legend ----
                if in_legend {
                    if name == b"legendPos" && diff.legend.is_some() {
                        // Ours went first.
                        if !empty {
                            skipping = 1;
                        }
                        continue;
                    }
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }

                // ---- structure ----
                match name {
                    _ if is_plot_type(name) && !empty => {
                        in_plot_type = true;
                        saw_vary = false;
                        saw_gap = false;
                        saw_overlap = false;
                        saw_hole = false;
                        saw_line_marker = false;
                    }
                    b"catAx" | b"dateAx" | b"serAx" if !empty => axis = Some(false),
                    b"valAx" if !empty => axis = Some(true),
                    b"legend" => {
                        match diff.legend {
                            Some(None) => {
                                if !empty {
                                    skipping = 1;
                                }
                                continue;
                            }
                            Some(Some(position)) => {
                                if empty {
                                    // `<c:legend/>` said nothing; it now does.
                                    open(&mut out, &prefix, b"legend", &[], false);
                                    write_val(
                                        &mut out,
                                        &prefix,
                                        b"legendPos",
                                        legend_val(position),
                                    );
                                    close(&mut out, &prefix, b"legend");
                                } else {
                                    out.extend_from_slice(splicer.bytes(span));
                                    write_val(
                                        &mut out,
                                        &prefix,
                                        b"legendPos",
                                        legend_val(position),
                                    );
                                    in_legend = true;
                                    wrote_legend_pos = true;
                                }
                                continue;
                            }
                            None => {
                                if !empty {
                                    in_legend = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::End(e) => {
                let name = end_local_name(e);
                if let Some(index) = series {
                    if level == 0 && name == b"ser" {
                        let want = diff.series.get(index);
                        if !wrote_props {
                            if let Some(Some(color)) = want.map(|w| w.color) {
                                write_props(&mut out, &prefix, &drawing, color, wanted);
                            }
                        }
                        if !wrote_marker {
                            if let Some(Some(symbol)) = want.map(|w| w.symbol) {
                                write_marker(&mut out, &prefix, symbol);
                            }
                        }
                        if !wrote_smooth {
                            if let Some(Some(smooth)) = want.map(|w| w.smooth) {
                                write_val(
                                    &mut out,
                                    &prefix,
                                    b"smooth",
                                    if smooth { "1" } else { "0" },
                                );
                            }
                        }
                        series = None;
                    } else {
                        level = level.saturating_sub(1);
                        if in_marker && level == 0 && name == b"marker" {
                            in_marker = false;
                        }
                    }
                    out.extend_from_slice(splicer.bytes(span));
                    continue;
                }
                if in_plot_type && is_plot_type(name) {
                    if !saw_hole {
                        if let Some(hole) = diff.hole {
                            write_val(&mut out, &prefix, b"holeSize", &whole(hole));
                        }
                    }
                    in_plot_type = false;
                }
                if axis.is_some() && matches!(name, b"catAx" | b"dateAx" | b"serAx" | b"valAx") {
                    axis = None;
                }
                if in_legend && name == b"legend" {
                    in_legend = false;
                    let _ = wrote_legend_pos;
                }
                out.extend_from_slice(splicer.bytes(span));
                if name == b"plotArea" {
                    if let (Some(Some(position)), false) = (diff.legend, stored.legend.is_some()) {
                        open(&mut out, &prefix, b"legend", &[], false);
                        write_val(&mut out, &prefix, b"legendPos", legend_val(position));
                        write_val(&mut out, &prefix, b"overlay", "0");
                        close(&mut out, &prefix, b"legend");
                    }
                }
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }
    Ok(out)
}

/// The direct children of `<c:ser>` that the schema puts after `spPr`.
const AFTER_PROPS: &[&[u8]] = &[
    b"invertIfNegative",
    b"pictureOptions",
    b"marker",
    b"dPt",
    b"dLbls",
    b"trendline",
    b"errBars",
    b"cat",
    b"val",
    b"xVal",
    b"yVal",
    b"smooth",
    b"bubbleSize",
    b"bubble3D",
    b"shape",
    b"extLst",
];

/// And after `marker`.
const AFTER_MARKER: &[&[u8]] = &[
    b"dPt",
    b"dLbls",
    b"trendline",
    b"errBars",
    b"cat",
    b"val",
    b"xVal",
    b"yVal",
    b"smooth",
    b"bubbleSize",
    b"bubble3D",
    b"extLst",
];

fn is_plot_type(name: &[u8]) -> bool {
    ChartKind::from_element(&String::from_utf8_lossy(name)).is_some()
}

fn whole(value: f64) -> String {
    (value.round() as i64).to_string()
}

fn legend_val(position: LegendPosition) -> &'static str {
    match position {
        LegendPosition::Top => "t",
        LegendPosition::Bottom => "b",
        LegendPosition::Left => "l",
        LegendPosition::TopRight => "tr",
        LegendPosition::Right => "r",
    }
}

fn symbol_val(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::Auto | Symbol::Circle => "circle",
        Symbol::None => "none",
        Symbol::Square => "square",
        Symbol::Diamond => "diamond",
        Symbol::Triangle => "triangle",
        Symbol::X => "x",
        Symbol::Star => "star",
        Symbol::Plus => "plus",
        Symbol::Dash => "dash",
        Symbol::Dot => "dot",
    }
}

fn write_val(out: &mut Vec<u8>, prefix: &[u8], name: &[u8], value: &str) {
    open(out, prefix, name, &[Set::to(b"val", value)], true);
}

/// A series' shape properties for a colour: the fill of a bar or a slice,
/// the ink of a line. Or, for no colour at all, properties that say nothing
/// — which hands the series back to the theme, as Excel's *Automatic* does.
fn write_props(
    out: &mut Vec<u8>,
    prefix: &[u8],
    drawing: &[u8],
    color: Option<[u8; 3]>,
    plot: &Plot,
) {
    let Some(rgb) = color else {
        open(out, prefix, b"spPr", &[], true);
        return;
    };
    let lines = matches!(plot.kind, ChartKind::Line | ChartKind::Radar)
        || (plot.kind == ChartKind::Scatter && plot.scatter_lines);
    let hex = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
    open(out, prefix, b"spPr", &[], false);
    if lines {
        open(out, drawing, b"ln", &[], false);
    }
    open(out, drawing, b"solidFill", &[], false);
    open(out, drawing, b"srgbClr", &[Set::to(b"val", hex)], true);
    close(out, drawing, b"solidFill");
    if lines {
        close(out, drawing, b"ln");
    }
    close(out, prefix, b"spPr");
}

fn write_marker_open(out: &mut Vec<u8>, prefix: &[u8], symbol: Symbol) {
    open(out, prefix, b"marker", &[], false);
    write_val(out, prefix, b"symbol", symbol_val(symbol));
}

fn write_marker(out: &mut Vec<u8>, prefix: &[u8], symbol: Symbol) {
    write_marker_open(out, prefix, symbol);
    close(out, prefix, b"marker");
}

/// What the inspector changed, property by property: `None` is untouched.
#[derive(Default)]
struct Diff {
    series: Vec<SeriesDiff>,
    vary_colors: Option<bool>,
    gap: Option<f64>,
    overlap: Option<f64>,
    hole: Option<f64>,
    markers: Option<bool>,
    scatter_lines: Option<bool>,
    /// `Some(None)` takes the legend away.
    legend: Option<Option<LegendPosition>>,
    cat_axis: AxisDiff,
    val_axis: AxisDiff,
}

#[derive(Default, Clone, Copy)]
struct SeriesDiff {
    /// `Some(None)` hands the colour back to the theme.
    color: Option<Option<[u8; 3]>>,
    symbol: Option<Symbol>,
    smooth: Option<bool>,
}

#[derive(Default, Clone, Copy)]
struct AxisDiff {
    deleted: Option<bool>,
    gridlines: Option<bool>,
}

impl Diff {
    fn between(stored: &Plot, wanted: &Plot) -> Diff {
        fn changed<T: PartialEq + Copy>(a: T, b: T) -> Option<T> {
            (a != b).then_some(b)
        }
        let axis = |a: &Axis, b: &Axis| AxisDiff {
            deleted: changed(a.deleted, b.deleted),
            gridlines: changed(a.gridlines, b.gridlines),
        };
        Diff {
            series: stored
                .series
                .iter()
                .zip(&wanted.series)
                .map(|(a, b)| SeriesDiff {
                    color: changed(a.color, b.color),
                    symbol: changed(a.symbol, b.symbol),
                    smooth: changed(a.smooth, b.smooth).flatten(),
                })
                .collect(),
            vary_colors: changed(stored.vary_colors, wanted.vary_colors),
            gap: changed(stored.gap, wanted.gap),
            overlap: changed(stored.overlap, wanted.overlap),
            hole: changed(stored.hole, wanted.hole),
            markers: changed(stored.markers, wanted.markers),
            scatter_lines: changed(stored.scatter_lines, wanted.scatter_lines),
            legend: changed(stored.legend, wanted.legend),
            cat_axis: axis(&stored.cat_axis, &wanted.cat_axis),
            val_axis: axis(&stored.val_axis, &wanted.val_axis),
        }
    }

    fn is_empty(&self) -> bool {
        let axis = |a: &AxisDiff| a.deleted.is_none() && a.gridlines.is_none();
        self.series
            .iter()
            .all(|s| s.color.is_none() && s.symbol.is_none() && s.smooth.is_none())
            && self.vary_colors.is_none()
            && self.gap.is_none()
            && self.overlap.is_none()
            && self.hole.is_none()
            && self.markers.is_none()
            && self.scatter_lines.is_none()
            && self.legend.is_none()
            && axis(&self.cat_axis)
            && axis(&self.val_axis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- restyle ----

    /// A bar chart as Excel writes one, trimmed to what the splicer looks at.
    const EXCEL_BAR: &str = concat!(
        r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/>"#,
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>"#,
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f></c:strRef></c:tx>"#,
        r#"<c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr>"#,
        r#"<c:invertIfNegative val="0"/><c:dLbls><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr></c:dLbls>"#,
        r#"<c:cat><c:strRef><c:f>Sheet1!$A$2:$A$3</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f></c:numRef></c:val></c:ser>"#,
        r#"<c:ser><c:idx val="1"/><c:order val="1"/><c:val><c:numRef><c:f>Sheet1!$C$2:$C$3</c:f></c:numRef></c:val></c:ser>"#,
        r#"<c:dLbls><c:showVal val="0"/></c:dLbls><c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        r#"<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:numFmt formatCode="General" sourceLinked="1"/><c:crossAx val="2"/></c:catAx>"#,
        r#"<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/>"#,
        r#"<c:majorGridlines><c:spPr><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>"#,
        r#"<c:numFmt formatCode="General" sourceLinked="1"/><c:crossAx val="1"/></c:valAx>"#,
        r#"</c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/><c:spPr><a:noFill/></c:spPr></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>"#,
    );

    fn restyled(source: &str, change: impl FnOnce(&mut Plot)) -> String {
        let stored =
            ss_model::chart::read::plot(source.as_bytes()).expect("a plot the reader draws");
        let mut wanted = stored.clone();
        change(&mut wanted);
        let out = restyle("chart1.xml", source.as_bytes(), &stored, &wanted).expect("writes");
        let out = String::from_utf8(out).expect("utf-8");
        // Whatever was done, the result reads back as what was asked for.
        let back = ss_model::chart::read::plot(out.as_bytes()).expect("still a plot");
        assert_eq!(back, wanted, "the part reads back as the model:\n{out}");
        out
    }

    #[test]
    fn a_plot_nobody_changed_is_not_rewritten() {
        let stored = ss_model::chart::read::plot(EXCEL_BAR.as_bytes()).expect("a plot");
        let out = restyle("chart1.xml", EXCEL_BAR.as_bytes(), &stored, &stored).expect("writes");
        assert_eq!(out, EXCEL_BAR.as_bytes());
    }

    #[test]
    fn a_series_colour_replaces_its_properties_and_touches_no_other_series() {
        let out = restyled(EXCEL_BAR, |p| p.series[0].color = Some([0x1E, 0x6F, 0x5C]));
        assert!(out.contains(r#"<c:spPr><a:solidFill><a:srgbClr val="1E6F5C"/></a:solidFill></c:spPr><c:invertIfNegative"#), "{out}");
        assert!(!out.contains("accent1"), "the theme colour is gone: {out}");
        assert!(
            out.contains(r#"<c:dLbls><c:spPr><a:solidFill><a:srgbClr val="FF0000"/>"#),
            "a data label's colour is not the series': {out}"
        );
        assert!(
            out.contains(r#"<c:ser><c:idx val="1"/><c:order val="1"/><c:val>"#),
            "the second series is untouched: {out}"
        );
        assert!(out.contains(r#"<c:gapWidth val="219"/>"#), "{out}");
    }

    #[test]
    fn a_series_without_properties_gets_them_where_the_schema_puts_them() {
        let out = restyled(EXCEL_BAR, |p| p.series[1].color = Some([0, 0, 0xFF]));
        assert!(out.contains(r#"<c:order val="1"/><c:spPr><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></c:spPr><c:val>"#), "{out}");
    }

    #[test]
    fn a_colour_handed_back_leaves_properties_that_say_nothing() {
        let coloured = restyled(EXCEL_BAR, |p| p.series[0].color = Some([1, 2, 3]));
        let out = restyled(&coloured, |p| p.series[0].color = None);
        assert!(out.contains(r#"<c:spPr/><c:invertIfNegative"#), "{out}");
    }

    #[test]
    fn the_bars_spacing_and_the_legend_are_rewritten_in_place() {
        let out = restyled(EXCEL_BAR, |p| {
            p.gap = 80.0;
            p.overlap = 100.0;
            p.legend = Some(LegendPosition::Right);
            p.vary_colors = true;
        });
        assert!(
            out.contains(r#"<c:gapWidth val="80"/><c:overlap val="100"/><c:axId"#),
            "{out}"
        );
        assert!(out.contains(r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/><c:spPr><a:noFill/></c:spPr></c:legend>"#), "{out}");
        assert!(out.contains(r#"<c:varyColors val="1"/>"#), "{out}");
    }

    #[test]
    fn a_legend_taken_away_goes_whole_and_one_wanted_is_authored_after_the_plot_area() {
        let gone = restyled(EXCEL_BAR, |p| p.legend = None);
        assert!(!gone.contains("legend"), "{gone}");
        assert!(
            gone.contains(r#"</c:plotArea><c:plotVisOnly val="1"/>"#),
            "{gone}"
        );
        let back = restyled(&gone, |p| p.legend = Some(LegendPosition::Top));
        assert!(back.contains(r#"</c:plotArea><c:legend><c:legendPos val="t"/><c:overlay val="0"/></c:legend><c:plotVisOnly"#), "{back}");
    }

    #[test]
    fn an_axis_is_hidden_and_its_gridlines_come_and_go_with_their_formatting() {
        let out = restyled(EXCEL_BAR, |p| {
            p.val_axis.gridlines = false;
            p.cat_axis.gridlines = true;
            p.cat_axis.deleted = true;
        });
        assert!(
            !out.contains("tx1"),
            "the gridlines went with their line formatting: {out}"
        );
        assert!(out.contains(r#"<c:axPos val="l"/><c:numFmt"#), "{out}");
        assert!(
            out.contains(r#"<c:delete val="1"/><c:axPos val="b"/><c:majorGridlines/><c:numFmt"#),
            "{out}"
        );
    }

    #[test]
    fn a_doughnuts_hole_and_a_lines_markers_are_set_where_they_stand() {
        let doughnut = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart><c:plotArea>"#,
            r#"<c:doughnutChart><c:varyColors val="1"/><c:ser><c:idx val="0"/><c:order val="0"/></c:ser>"#,
            r#"<c:firstSliceAng val="0"/><c:holeSize val="75"/></c:doughnutChart></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = restyled(doughnut, |p| p.hole = 40.0);
        assert!(
            out.contains(r#"<c:firstSliceAng val="0"/><c:holeSize val="40"/></c:doughnutChart>"#),
            "{out}"
        );

        // A second producer's doughnut that states no hole gets one.
        let bare = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart><c:plotArea>"#,
            r#"<c:doughnutChart><c:varyColors val="1"/><c:ser><c:idx val="0"/><c:order val="0"/></c:ser>"#,
            r#"</c:doughnutChart></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = restyled(bare, |p| p.hole = 40.0);
        assert!(
            out.contains(r#"</c:ser><c:holeSize val="40"/></c:doughnutChart>"#),
            "{out}"
        );

        let line = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart><c:plotArea>"#,
            r#"<c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>"#,
            r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:spPr><a:ln w="28575"><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:ln></c:spPr>"#,
            r#"<c:marker><c:symbol val="none"/></c:marker><c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f></c:numRef></c:val><c:smooth val="0"/></c:ser>"#,
            r#"<c:marker val="1"/><c:axId val="1"/><c:axId val="2"/></c:lineChart>"#,
            r#"<c:catAx><c:axId val="1"/><c:delete val="0"/><c:axPos val="b"/></c:catAx><c:valAx><c:axId val="2"/><c:delete val="0"/><c:axPos val="l"/></c:valAx>"#,
            r#"</c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = restyled(line, |p| {
            p.series[0].symbol = Symbol::Diamond;
            p.series[0].smooth = Some(true);
            p.series[0].color = Some([0xFF, 0, 0]);
            p.markers = false;
        });
        assert!(out.contains(r#"<c:spPr><a:ln><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val="diamond"/></c:marker><c:val>"#), "{out}");
        assert!(
            out.contains(r#"<c:smooth val="1"/></c:ser><c:marker val="0"/><c:axId"#),
            "{out}"
        );
    }

    const CHART: &str = concat!(
        r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Old </a:t></a:r>"#,
        r#"<a:r><a:t>name</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
        r#"<c:autoTitleDeleted val="1"/>"#,
        r#"<c:plotArea><c:barChart><c:ser><c:dLbls><c:tx><c:rich><a:p><a:r><a:t>label</a:t>"#,
        r#"</a:r></a:p></c:rich></c:tx></c:dLbls></c:ser></c:barChart></c:plotArea>"#,
        r#"</c:chart></c:chartSpace>"#,
    );

    fn rewritten(source: &str, title: Option<&str>) -> String {
        String::from_utf8(retitle("chart1.xml", source.as_bytes(), title).expect("writes"))
            .expect("utf-8")
    }

    #[test]
    fn a_new_title_replaces_every_run_of_the_old_one() {
        // The old title is two runs. Writing the new text into each would give
        // "NewNew"; leaving the second alone would give "Newname".
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains("<a:t>New name</a:t>"), "{out}");
        assert!(!out.contains("Old "), "{out}");
        assert!(!out.contains(">name<"), "{out}");
    }

    #[test]
    fn a_data_label_is_not_a_title() {
        // `<c:tx><c:rich><a:t>` appears inside `<c:dLbls>` too, further down
        // the same file.
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains("<a:t>label</a:t>"), "{out}");
    }

    #[test]
    fn writing_a_title_clears_the_flag_that_would_hide_it() {
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains(r#"<c:autoTitleDeleted val="0"/>"#), "{out}");
    }

    #[test]
    fn a_title_element_with_no_text_in_it_is_replaced_rather_than_edited() {
        // Excel writes one of these when the automatic title is switched off:
        // formatting and nothing to write into. Editing the runs would find
        // none and silently do nothing.
        let empty = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
            r#"<c:title><c:overlay val="0"/></c:title><c:autoTitleDeleted val="1"/>"#,
            r#"<c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = rewritten(empty, Some("Added"));
        assert!(out.contains("<a:t>Added</a:t>"), "{out}");
        assert_eq!(out.matches("<c:title>").count(), 1, "{out}");
        assert!(out.contains(r#"<c:autoTitleDeleted val="0"/>"#), "{out}");
    }

    #[test]
    fn a_chart_with_no_title_gets_one_in_the_position_the_schema_fixes() {
        let bare = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
            r#"<c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = rewritten(bare, Some("Added"));
        let title = out.find("<c:title>").expect("added");
        let plot = out.find("<c:plotArea>").expect("still there");
        assert!(title < plot, "first child of <c:chart>: {out}");
        assert!(out.contains("<a:t>Added</a:t>"), "{out}");
    }

    #[test]
    fn removing_a_title_removes_the_element_and_nothing_else() {
        let out = rewritten(CHART, None);
        assert!(!out.contains("<c:title>"), "{out}");
        assert!(out.contains(r#"<c:autoTitleDeleted val="1"/>"#), "{out}");
        assert!(
            out.contains("<a:t>label</a:t>"),
            "the data label stays: {out}"
        );
    }

    #[test]
    fn a_title_that_comes_from_a_cell_is_left_alone() {
        // Rewriting it would mean writing to the cell, which is the user's to do.
        let linked = concat!(
            r#"<c:chartSpace xmlns:c="http://c"><c:chart><c:title><c:tx><c:strRef>"#,
            r#"<c:f>Sheet1!$A$1</c:f></c:strRef></c:tx></c:title></c:chart></c:chartSpace>"#,
        );
        assert_eq!(rewritten(linked, Some("Nope")), linked);
    }

    #[test]
    fn a_title_split_across_runs_comes_back_as_one_run() {
        // Which is why the caller compares before it writes: retitling to the
        // same text is not a no-op, it is a normalization. Everything outside
        // `<c:tx>` is still byte for byte.
        let out = rewritten(CHART, Some("Old name"));
        assert!(out.contains("<a:t>Old name</a:t>"), "{out}");
        assert!(
            out.contains("<c:plotArea><c:barChart>"),
            "the rest is untouched: {out}"
        );
    }
}
