//! Writing a document back to its package.
//!
//! **The writer edits `content.xml`; it does not reprint it.** The part is
//! walked with a [`splice::Splicer`], and each `<text:p>`, `<text:h>` and
//! block-level `<draw:frame>` is compared against the model by *re-reading it*
//! and asking whether the result differs. One that does not is copied byte for
//! byte — its change-tracking marks, its `<text:soft-page-break>`s, its form
//! controls, its annotations and everything else this crate does not model
//! included. One that does is emitted from the model.
//!
//! Comparing by re-reading rather than by remembering is the same decision
//! `wp_docx::write` states at length, and for the same reason: the writer never
//! has to trust what a reader believed some time earlier, and "changed" means
//! exactly "would read back differently" — the only definition that cannot
//! drift away from the reader.
//!
//! **Two things here are not what the other writer does, and both are the
//! format's doing.**
//!
//! A table is *not* paired as a unit and re-emitted when something in it
//! changes. WordprocessingML puts a table's geometry inside the table — the
//! grid, each cell's width, its borders and its shading are elements of
//! `<w:tbl>` — so re-emitting one from the model loses nothing the model does
//! not hold. ODF puts all of it in automatic styles named from every column,
//! row and cell, so re-emitting a table means minting that whole family of
//! styles and hoping they say what the originals said. The splice goes *into*
//! the table instead, down to the paragraphs in its cells, and every width and
//! border in it is copied rather than restated. The cost is that a change to a
//! table's *structure* — a row added, a column removed — is not written; the
//! benefit is that a typo fixed in a cell does not resize the table.
//!
//! Direct formatting cannot be written where the run is. ODF states it as an
//! automatic style, which stands in `<office:automatic-styles>` *before* the
//! body — so the stylesheet cannot be written until the body is known, and is
//! spliced back into the part afterwards. See [`auto`].
//!
//! `styles.xml` is rewritten too, and only ever for the same reason: a header
//! and a footer are paragraphs inside a master page, and an application that
//! lets a person edit one in place must be able to save it.

mod auto;
pub mod blank;
mod emit;
mod splice;

use std::collections::HashMap;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use wp_model::doc::{Block, Document};

use crate::container::Container;
use crate::content::{self, Lists, Which};
use crate::xml::{end_local_name, local_name};
use crate::{Ctx, Error, Result};
use auto::Automatic;
use emit::Out;
use splice::Splicer;

/// What a part this crate rewrites is declared as in the manifest.
const XML_MEDIA_TYPE: &str = "text/xml";

/// Rewrites the modelled parts of `container` from `document`, and saves it.
///
/// Beside the target first, then renamed over it — [`Container::save`] does
/// that, and it is what stops a refusal or a crash halfway through from leaving
/// the user with neither the old document nor the new one.
///
/// The signature is `wp_docx::save`'s, down to taking the document by `&mut`,
/// so that the application can hold either format behind the same call. Nothing
/// here needs the mutability: ODF has no relationship ids to hand out.
pub fn save(
    document: &mut Document,
    container: &mut Container,
    path: impl AsRef<Path>,
) -> Result<()> {
    flush(document, container)?;
    container.save(path)?;
    Ok(())
}

/// Puts the rewritten parts back into the package without saving.
pub fn flush(document: &mut Document, container: &mut Container) -> Result<()> {
    let (content, styles) = rewritten(document, container)?;
    // Put a part back only where it actually changed. A package whose
    // `styles.xml` is byte for byte what it was is one where nothing has to say
    // so, and check 2 of the fidelity harness asks exactly that question.
    if container.data("content.xml") != Some(&content) {
        let media = media_type(container, "content.xml");
        container.put_part("content.xml", &media, content)?;
    }
    if let Some(styles) = styles {
        if container.data("styles.xml") != Some(&styles) {
            let media = media_type(container, "styles.xml");
            container.put_part("styles.xml", &media, styles)?;
        }
    }
    Ok(())
}

fn media_type(container: &Container, name: &str) -> String {
    container
        .part(name)
        .map(|part| part.media_type().to_owned())
        .unwrap_or_else(|| XML_MEDIA_TYPE.to_owned())
}

/// Both parts, rewritten — `content.xml` always, `styles.xml` where there is
/// one.
fn rewritten(document: &Document, container: &Container) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
    let content = container
        .data("content.xml")
        .ok_or(Error::MissingPart("content.xml"))?;
    let styles = container.data("styles.xml");

    // A first reading of the whole package, for the two things a walk cannot
    // work out from the bytes in front of it: the path inside the package each
    // minted picture name stands for, and the name each bookmark id came from.
    let (pictures, bookmarks) = {
        let mut ctx = Ctx::new(container);
        if let Some(styles) = styles {
            content::part(styles, &mut ctx, Which::Styles)?;
        }
        content::part(content, &mut ctx, Which::Content)?;
        (
            ctx.minted
                .iter()
                .map(|(href, rel)| (rel.to_string(), href.clone()))
                .collect::<HashMap<_, _>>(),
            ctx.bookmarks
                .iter()
                .map(|(name, id)| (*id, name.clone()))
                .collect::<HashMap<_, _>>(),
        )
    };

    // The body. Its context is put back in the state the reader was in at the
    // moment it began reading `<office:text>` — the stylesheets of both parts
    // and the master pages read, nothing of the body — because the walk reads
    // every paragraph again in document order and each note number, bookmark id
    // and picture name it hands out has to land where the first reading put it.
    let content = {
        let mut ctx = Ctx::new(container);
        if let Some(styles) = styles {
            content::part(styles, &mut ctx, Which::Styles)?;
        }
        content::part(content, &mut ctx, Which::Stylesheet)?;
        let mut w = Out {
            document,
            auto: Automatic::new(document.styles.clone(), &ctx.styles),
            pictures: pictures.clone(),
            bookmarks: bookmarks.clone(),
        };
        content_out(content, &mut ctx, &mut w)
    };

    // The bands. The master pages stand after the stylesheets in `styles.xml`
    // and are read after them, so a context holding only the stylesheets is the
    // state the reader was in when it reached the first one.
    let styles = match styles {
        Some(bytes) => {
            let mut ctx = Ctx::new(container);
            content::part(bytes, &mut ctx, Which::Stylesheet)?;
            let mut w = Out {
                document,
                auto: Automatic::new(document.styles.clone(), &ctx.styles),
                pictures,
                bookmarks,
            };
            Some(styles_out(bytes, &mut ctx, &mut w))
        }
        None => None,
    };

    Ok((content, styles))
}

/// Which of the two parts is being walked, and so what its body is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Scope {
    /// `content.xml`: one `<office:text>`, holding the document's own blocks.
    Body,
    /// `styles.xml`: the headers and footers of every master page, in the order
    /// the reader numbered them.
    Bands,
}

/// Rewrites `content.xml`: every byte of it except the blocks that read back
/// differently from the model.
fn content_out<'a>(original: &'a [u8], ctx: &mut Ctx<'a>, w: &mut Out<'_>) -> Vec<u8> {
    part_out(original, Scope::Body, ctx, w)
}

/// Rewrites `styles.xml`, whose body is the headers and footers of its master
/// pages. Everything else in it — the named styles, the page layouts, the list
/// definitions — is copied.
fn styles_out<'a>(original: &'a [u8], ctx: &mut Ctx<'a>, w: &mut Out<'_>) -> Vec<u8> {
    part_out(original, Scope::Bands, ctx, w)
}

fn part_out<'a>(original: &'a [u8], scope: Scope, ctx: &mut Ctx<'a>, w: &mut Out<'_>) -> Vec<u8> {
    // The document is behind a shared reference, so it can be held beside the
    // writer rather than through it: the blocks being paired must not borrow
    // the thing that emits them.
    let document = w.document;
    let mut out = Vec::with_capacity(original.len() + 64);
    let mut splicer = Splicer::new(original);
    out.extend_from_slice(splicer.preamble());

    // Where a minted automatic style would have to go. Three answers, in the
    // order they are preferred: inside the `<office:automatic-styles>` the file
    // has, in place of the empty one it has, or in a new one before the body.
    let mut before_end: Option<usize> = None;
    let mut instead_of: Option<std::ops::Range<usize>> = None;
    let mut before_body: Option<usize> = None;
    let mut band = 0usize;

    while let Some((event, span)) = splicer.next() {
        match &event {
            Event::Empty(e) if local_name(e) == b"automatic-styles" => {
                instead_of = Some(out.len()..out.len() + span.len());
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::End(e) if end_local_name(e) == b"automatic-styles" => {
                before_end = Some(out.len());
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if local_name(e) == b"body" && before_body.is_none() => {
                before_body = Some(out.len());
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if scope == Scope::Body && local_name(e) == b"text" => {
                out.extend_from_slice(splicer.bytes(span));
                blocks_out(
                    &mut splicer,
                    &mut out,
                    b"text",
                    &document.body,
                    &mut Source::Read(ctx),
                    w,
                    &mut Lists::new(),
                    &mut Vec::new(),
                    true,
                );
            }
            Event::Start(e) if scope == Scope::Bands && is_band(local_name(e)) => {
                out.extend_from_slice(splicer.bytes(span));
                let name = local_name(e).to_vec();
                let content = document
                    .headers
                    .get(band)
                    .map(|band| band.content.as_slice())
                    .unwrap_or_default();
                band += 1;
                blocks_out(
                    &mut splicer,
                    &mut out,
                    &name,
                    content,
                    &mut Source::Read(ctx),
                    w,
                    &mut Lists::new(),
                    &mut Vec::new(),
                    true,
                );
            }
            Event::Empty(e) if scope == Scope::Bands && is_band(local_name(e)) => {
                band += 1;
                out.extend_from_slice(splicer.bytes(span));
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    if w.auto.is_empty() {
        return out;
    }
    let minted = w.auto.stylesheet();
    if let Some(at) = before_end {
        out.splice(at..at, minted.bytes());
    } else if let Some(range) = instead_of {
        // `<office:automatic-styles/>` has to be opened before anything can go
        // in it, and its own attributes come along by keeping every byte but
        // the closing slash.
        let opened = String::from_utf8_lossy(&out[range.clone()])
            .trim_end()
            .trim_end_matches("/>")
            .to_owned();
        let replacement = format!("{opened}>{minted}</office:automatic-styles>");
        out.splice(range, replacement.bytes());
    } else if let Some(at) = before_body {
        let sheet = format!("<office:automatic-styles>{minted}</office:automatic-styles>");
        out.splice(at..at, sheet.bytes());
    }
    out
}

/// The six elements a master page draws its running content in.
fn is_band(name: &[u8]) -> bool {
    matches!(
        name,
        b"header" | b"header-left" | b"header-first" | b"footer" | b"footer-left" | b"footer-first"
    )
}

/// One block of the file: where its bytes belong in the output, what those
/// bytes are, and what the reader makes of them now.
struct Slot<'a> {
    at: usize,
    bytes: &'a [u8],
    read: Option<Block>,
}

/// Where the file's own version of a block comes from.
///
/// Reading it out of its bytes is the usual answer and the one that defines
/// "changed". A table is different: it is read whole, cells and all, and its
/// cell paragraphs must not be read a second time — every note number and
/// picture name the reading context hands out is in document order, and reading
/// anything twice moves all of them.
enum Source<'c, 'a> {
    Read(&'c mut Ctx<'a>),
    Known { blocks: &'c [Block], at: usize },
}

impl<'a> Source<'_, 'a> {
    fn block(&mut self, bytes: &'a [u8], lists: &Lists) -> Option<Block> {
        match self {
            Source::Read(ctx) => content::block_of(bytes, ctx, lists),
            Source::Known { blocks, at } => {
                let block = blocks.get(*at).cloned();
                *at += 1;
                block
            }
        }
    }
}

/// Everything between here and `end` that the reader treats as a block, paired
/// with the model.
///
/// Two passes, and the first one writes nothing where a block stands. The file's
/// blocks are read in document order — which is what keeps the reading context
/// in step — and set aside with the offset their bytes would have gone to;
/// [`resolve`] then works out which of the model's blocks each of them *is* and
/// puts the answer at that offset. It has to be that way round: which block a
/// paragraph pairs with cannot be decided by counting, because a paragraph
/// deleted in the middle of a document would then shift every one after it and
/// rewrite the lot.
///
/// `owns` says whether this scope is the one the model's list belongs to. A
/// `<text:list>` or a `<text:section>` is a wrapper the reader flattens, so it
/// shares its parent's blocks and must not decide anything about them; a cell,
/// a band and the body each own theirs.
#[allow(clippy::too_many_arguments)]
fn blocks_out<'a>(
    splicer: &mut Splicer<'a>,
    out: &mut Vec<u8>,
    end: &[u8],
    model: &[Block],
    source: &mut Source<'_, 'a>,
    w: &mut Out<'_>,
    lists: &mut Lists,
    slots: &mut Vec<Slot<'a>>,
    owns: bool,
) {
    while let Some((event, span)) = splicer.next() {
        let empty = matches!(event, Event::Empty(_));
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(e).to_vec();
                match name.as_slice() {
                    b"p" | b"h" => {
                        let whole = match empty {
                            true => span,
                            false => splicer.element(&name, span),
                        };
                        let bytes = splicer.bytes(whole);
                        let read = source.block(bytes, lists);
                        slots.push(Slot {
                            at: out.len(),
                            bytes,
                            read,
                        });
                    }
                    // A frame standing where a block belongs is a paragraph in
                    // the model, and pairs like one. A frame inside a paragraph
                    // was consumed with it.
                    b"frame" | b"table" if !empty => {
                        let whole = splicer.element(&name, span);
                        let bytes = splicer.bytes(whole);
                        let read = source.block(bytes, lists);
                        slots.push(Slot {
                            at: out.len(),
                            bytes,
                            read,
                        });
                    }
                    b"list" if !empty => {
                        let level = match source {
                            Source::Read(ctx) => content::list_level(e, ctx, lists),
                            // Inside a table's cell, where the blocks are
                            // already read and their numbering settled.
                            Source::Known { .. } => None,
                        };
                        lists.push(level);
                        out.extend_from_slice(splicer.bytes(span));
                        blocks_out(splicer, out, b"list", model, source, w, lists, slots, false);
                        lists.pop();
                    }
                    // The wrappers the reader walks through: their blocks are
                    // the document's, at the same level as everything around
                    // them.
                    _ if !empty && descends(&name) => {
                        out.extend_from_slice(splicer.bytes(span));
                        blocks_out(splicer, out, &name, model, source, w, lists, slots, false);
                    }
                    // Anything else the reader steps over — change tracking,
                    // forms, a sequence declaration — is copied whole, and the
                    // paragraphs inside it are none of the model's business.
                    _ if !empty => {
                        let whole = splicer.element(&name, span);
                        out.extend_from_slice(splicer.bytes(whole));
                    }
                    _ => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::End(e) if end_local_name(e) == end => {
                if owns {
                    resolve(out, slots, model, w);
                }
                out.extend_from_slice(splicer.bytes(span));
                return;
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }
    if owns {
        resolve(out, slots, model, w);
    }
}

/// The wrappers `content::blocks_into` descends into, and nothing else.
///
/// Kept in step with the reader by hand, because the two lists mean the same
/// thing: a `<text:p>` inside one of these is a paragraph of the document, and
/// a `<text:p>` inside anything not on this list — a deletion held in
/// `<text:tracked-changes>`, say — is not, and pairing one against the model
/// would put an edit where no edit was made.
fn descends(name: &[u8]) -> bool {
    matches!(
        name,
        b"list-item"
            | b"list-header"
            | b"section"
            | b"index-body"
            | b"index-title"
            | b"table-of-content"
            | b"illustration-index"
            | b"table-index"
            | b"object-index"
            | b"user-index"
            | b"alphabetical-index"
            | b"bibliography"
    )
}

/// What becomes of one slot.
enum Step<'m> {
    /// It is the model's block unchanged: keep the producer's bytes.
    Keep,
    /// It is this model block, edited: write that instead.
    From(&'m Block),
    /// The model has not got it: the bytes go with it.
    Drop,
}

/// Which of the model's blocks each of the file's blocks is.
struct Plan<'m> {
    before: Vec<Vec<&'m Block>>,
    steps: Vec<Step<'m>>,
    tail: Vec<&'m Block>,
}

/// How far ahead a block may have moved and still be recognised as itself.
///
/// A bound rather than a search of the whole document, because the cost of
/// looking is the length of the window times the length of the file and the
/// gain past a few paragraphs is nothing: an edit that moves a paragraph
/// thirty-two blocks is a rewrite of that stretch either way.
const WINDOW: usize = 32;

/// Pairs the file's blocks with the model's, by what they say rather than by
/// where they stand.
///
/// **Counting will not do it.** Delete the second paragraph of a document and
/// every paragraph after it pairs with the one that used to follow it: all of
/// them read back "changed", all of them are rewritten, and everything in them
/// this crate does not model is lost — for one deletion. So a block that does
/// not match is looked for a little way ahead on each side. Finding it further
/// on in the *model* means blocks were inserted before it; finding the model's
/// block further on in the *file* means blocks were deleted. Only when neither
/// is true is it what it looks like: the same block, edited.
fn align<'m>(slots: &[Slot<'_>], model: &'m [Block]) -> Plan<'m> {
    let mut plan = Plan {
        before: (0..slots.len()).map(|_| Vec::new()).collect(),
        steps: (0..slots.len()).map(|_| Step::Drop).collect(),
        tail: Vec::new(),
    };
    let (mut i, mut j) = (0usize, 0usize);
    while i < slots.len() && j < model.len() {
        let read = slots[i].read.as_ref();
        if read == Some(&model[j]) {
            plan.steps[i] = Step::Keep;
            i += 1;
            j += 1;
            continue;
        }
        if let Some(ahead) = (1..=WINDOW)
            .take_while(|ahead| j + ahead < model.len())
            .find(|&ahead| read == Some(&model[j + ahead]))
        {
            plan.before[i].extend(model[j..j + ahead].iter());
            plan.steps[i] = Step::Keep;
            i += 1;
            j += ahead + 1;
            continue;
        }
        if let Some(ahead) = (1..=WINDOW)
            .take_while(|ahead| i + ahead < slots.len())
            .find(|&ahead| slots[i + ahead].read.as_ref() == Some(&model[j]))
        {
            i += ahead; // the ones passed over stay `Drop`
            continue;
        }
        // A table cannot be an edited paragraph and a paragraph cannot be an
        // edited table, so a disagreement about *kind* is one side having
        // something the other has not. The model's is written where it stands
        // and the file's block is asked again.
        if !same_kind(read, &model[j]) {
            plan.before[i].push(&model[j]);
            j += 1;
            continue;
        }
        plan.steps[i] = Step::From(&model[j]);
        i += 1;
        j += 1;
    }
    plan.tail.extend(model[j.min(model.len())..].iter());
    plan
}

fn same_kind(read: Option<&Block>, model: &Block) -> bool {
    matches!(
        (read, model),
        (Some(Block::Paragraph(_)), Block::Paragraph(_)) | (Some(Block::Table(_)), Block::Table(_))
    )
}

/// Writes the plan into the gaps the scan left.
///
/// Front to back, because a minted automatic style is named in the order it is
/// asked for; then spliced in back to front, so that an earlier offset is still
/// where the scan left it.
fn resolve(out: &mut Vec<u8>, slots: &[Slot<'_>], model: &[Block], w: &mut Out<'_>) {
    let plan = align(slots, model);
    let mut writes: Vec<(usize, Vec<u8>)> = Vec::with_capacity(slots.len() + 1);
    for (i, slot) in slots.iter().enumerate() {
        let mut text = Vec::new();
        for block in &plan.before[i] {
            emitted(&mut text, block, w);
        }
        match plan.steps[i] {
            Step::Keep => text.extend_from_slice(slot.bytes),
            Step::Drop => {}
            // A table that changed is spliced into rather than rewritten: its
            // widths, borders and shading are in styles named from outside it,
            // and re-emitting one would mean minting the lot.
            Step::From(Block::Table(model)) => match &slot.read {
                Some(Block::Table(read)) => {
                    text.extend_from_slice(&table_spliced(slot.bytes, read, model, w))
                }
                _ => emitted(&mut text, &Block::Table(model.clone()), w),
            },
            Step::From(block) => emitted(&mut text, block, w),
        }
        writes.push((slot.at, text));
    }
    let mut tail = Vec::new();
    for block in &plan.tail {
        emitted(&mut tail, block, w);
    }
    writes.push((out.len(), tail));
    for (at, text) in writes.into_iter().rev() {
        out.splice(at..at, text);
    }
}

fn emitted(out: &mut Vec<u8>, block: &Block, w: &mut Out<'_>) {
    let mut text = String::new();
    emit::block(&mut text, block, w);
    out.extend_from_slice(text.as_bytes());
}

/// A `<table:table>` that changed, with everything but its changed paragraphs
/// copied.
fn table_spliced(
    bytes: &[u8],
    read: &wp_model::table::Table,
    model: &wp_model::table::Table,
    w: &mut Out<'_>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut splicer = Splicer::new(bytes);
    out.extend_from_slice(splicer.preamble());
    let Some((_, span)) = splicer.next() else {
        return bytes.to_vec();
    };
    out.extend_from_slice(splicer.bytes(span));
    let mut row = 0usize;
    rows_out(&mut splicer, &mut out, b"table", read, model, &mut row, w);
    out
}

/// The rows of a table, following the header and group wrappers that may stand
/// between, and counting the repeats the reader counted.
fn rows_out<'a>(
    splicer: &mut Splicer<'a>,
    out: &mut Vec<u8>,
    end: &[u8],
    read: &wp_model::table::Table,
    model: &wp_model::table::Table,
    row: &mut usize,
    w: &mut Out<'_>,
) {
    while let Some((event, span)) = splicer.next() {
        let empty = matches!(event, Event::Empty(_));
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(e).to_vec();
                match name.as_slice() {
                    b"table-row" => {
                        // `table:number-rows-repeated` says "and this many more
                        // just like it", and the reader made that many rows out
                        // of one element. They are copies of each other, so the
                        // first is what this one pairs against.
                        let times = repeat(e, b"number-rows-repeated") as usize;
                        let at = *row;
                        *row += times;
                        out.extend_from_slice(splicer.bytes(span));
                        if !empty {
                            cells_out(splicer, out, read.rows.get(at), model.rows.get(at), w);
                        }
                    }
                    b"table-header-rows"
                    | b"table-rows"
                    | b"table-columns"
                    | b"table-header-columns"
                    | b"table-column-group"
                    | b"table-row-group"
                        if !empty =>
                    {
                        out.extend_from_slice(splicer.bytes(span));
                        rows_out(splicer, out, &name, read, model, row, w);
                    }
                    _ if !empty => {
                        let whole = splicer.element(&name, span);
                        out.extend_from_slice(splicer.bytes(whole));
                    }
                    _ => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::End(e) if end_local_name(e) == end => {
                out.extend_from_slice(splicer.bytes(span));
                return;
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }
}

/// The cells of one row, with the covered-cell bookkeeping the reader does.
fn cells_out<'a>(
    splicer: &mut Splicer<'a>,
    out: &mut Vec<u8>,
    read: Option<&wp_model::table::Row>,
    model: Option<&wp_model::table::Row>,
    w: &mut Out<'_>,
) {
    let mut cell = 0usize;
    // How many more positions the cell most recently seen covers to its right.
    let mut covered = 0u32;
    while let Some((event, span)) = splicer.next() {
        let empty = matches!(event, Event::Empty(_));
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(e).to_vec();
                match name.as_slice() {
                    b"table-cell" => {
                        let times = repeat(e, b"number-columns-repeated");
                        let span_of = repeat(e, b"number-columns-spanned");
                        let at = cell;
                        cell += times as usize;
                        covered = (span_of - 1) * times;
                        out.extend_from_slice(splicer.bytes(span));
                        if empty {
                            continue;
                        }
                        let known = read
                            .and_then(|row| row.cells.get(at))
                            .map(|cell| cell.content.as_slice())
                            .unwrap_or_default();
                        let wanted = model
                            .and_then(|row| row.cells.get(at))
                            .map(|cell| cell.content.as_slice())
                            .unwrap_or_default();
                        let mut source = Source::Known {
                            blocks: known,
                            at: 0,
                        };
                        let mut lists = Lists::new();
                        let mut slots = Vec::new();
                        blocks_out(
                            splicer,
                            out,
                            b"table-cell",
                            wanted,
                            &mut source,
                            w,
                            &mut lists,
                            &mut slots,
                            true,
                        );
                    }
                    b"covered-table-cell" => {
                        for _ in 0..repeat(e, b"number-columns-repeated") {
                            // Covered from the left is inside the span of the
                            // cell that covers it and is no cell of its own;
                            // covered from above is a cell the reader made.
                            match covered > 0 {
                                true => covered -= 1,
                                false => cell += 1,
                            }
                        }
                        match empty {
                            true => out.extend_from_slice(splicer.bytes(span)),
                            false => {
                                let whole = splicer.element(&name, span);
                                out.extend_from_slice(splicer.bytes(whole));
                            }
                        }
                    }
                    _ if !empty => {
                        let whole = splicer.element(&name, span);
                        out.extend_from_slice(splicer.bytes(whole));
                    }
                    _ => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::End(e) if end_local_name(e) == b"table-row" => {
                out.extend_from_slice(splicer.bytes(span));
                return;
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }
}

/// A repeat count, which is one where the attribute is absent — the reader's
/// own rule, cap and all, because the two have to agree about how many cells a
/// row has or every pairing after it is wrong.
fn repeat(e: &BytesStart<'_>, want: &[u8]) -> u32 {
    const MOST: u32 = 1024;
    crate::xml::attr_in(e, b"table", want)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, MOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use wp_model::doc::{Inline, Paragraph, Run};

    /// A package with the shape a real one has: two stylesheets, an automatic
    /// style, a body with a heading, a list and a table, and an entry the
    /// manifest gives no media type at all.
    const CONTENT: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document-content xmlns:office="urn:office" xmlns:text="urn:text" xmlns:table="urn:table" xmlns:style="urn:style" xmlns:fo="urn:fo">"#,
        r#"<office:automatic-styles>"#,
        r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"/>"#,
        r#"</office:automatic-styles>"#,
        r#"<office:body><office:text>"#,
        r#"<text:sequence-decls><text:sequence-decl text:name="Illustration"/></text:sequence-decls>"#,
        r#"<text:h text:style-name="Heading_20_1" text:outline-level="1">Title</text:h>"#,
        r#"<text:p text:style-name="P1">first</text:p>"#,
        r#"<text:p text:style-name="P1">second<text:s text:c="2"/>space</text:p>"#,
        r#"<table:table table:name="T1"><table:table-column table:number-columns-repeated="2"/>"#,
        r#"<table:table-row><table:table-cell><text:p>alpha</text:p></table:table-cell>"#,
        r#"<table:table-cell><text:p>beta</text:p></table:table-cell></table:table-row>"#,
        r#"</table:table>"#,
        r#"<text:list text:style-name="L1"><text:list-item><text:p>bullet</text:p></text:list-item></text:list>"#,
        r#"</office:text></office:body></office:document-content>"#
    );

    const STYLES: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document-styles xmlns:office="urn:office" xmlns:text="urn:text" xmlns:style="urn:style" xmlns:fo="urn:fo">"#,
        r#"<office:styles>"#,
        r#"<style:style style:name="Standard" style:family="paragraph"/>"#,
        r#"<style:style style:name="Heading_20_1" style:family="paragraph" style:parent-style-name="Standard"/>"#,
        r#"</office:styles>"#,
        r#"<office:automatic-styles/>"#,
        r#"<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">"#,
        r#"<style:header><text:p>the header</text:p></style:header>"#,
        r#"</style:master-page></office:master-styles>"#,
        r#"</office:document-styles>"#
    );

    fn package() -> Container {
        let mut container = Container::empty(crate::container::TEXT_MIMETYPE);
        container
            .put_part("content.xml", XML_MEDIA_TYPE, CONTENT.as_bytes().to_vec())
            .expect("a part name");
        container
            .put_part("styles.xml", XML_MEDIA_TYPE, STYLES.as_bytes().to_vec())
            .expect("a part name");
        container
    }

    fn opened() -> (Document, Container) {
        let container = package();
        let (document, _) = crate::read(&container).expect("it reads");
        (document, container)
    }

    fn text_of(container: &Container) -> String {
        String::from_utf8(container.data("content.xml").expect("the part").to_vec()).expect("utf-8")
    }

    /// The guarantee the whole design exists for. Not "equivalent XML" —
    /// identical bytes, the sequence declaration and the table's own columns
    /// and all.
    #[test]
    fn an_untouched_save_reproduces_the_bytes_exactly() {
        let (mut document, mut container) = opened();
        flush(&mut document, &mut container).expect("it writes");
        assert_eq!(text_of(&container), CONTENT);
        assert_eq!(
            String::from_utf8(container.data("styles.xml").unwrap().to_vec()).unwrap(),
            STYLES
        );
    }

    /// And the whole way out to a file and back, which is what the fidelity
    /// harness asks of every package in the corpus.
    #[test]
    fn an_untouched_save_leaves_every_entry_of_the_package_alone() {
        let (mut document, mut container) = opened();
        let before = container.clone();
        flush(&mut document, &mut container).expect("it writes");
        let mut buffer = Vec::new();
        container.write(Cursor::new(&mut buffer)).expect("written");
        let after = Container::read(Cursor::new(buffer)).expect("it reads back");
        assert!(
            ooxml::compare::diff_entries(&before.entries(), &after.entries()).is_empty(),
            "an untouched save moved a byte"
        );
    }

    #[test]
    fn an_edited_paragraph_is_rewritten_and_its_neighbours_are_not() {
        let (mut document, mut container) = opened();
        let Block::Paragraph(paragraph) = &mut document.body[1] else {
            panic!("the second block is a paragraph");
        };
        paragraph.content = vec![Inline::Run(Run::of("changed"))];

        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        assert!(text.contains(">changed</text:p>"), "{text}");
        assert!(!text.contains(">first</text:p>"), "{text}");
        // The paragraph nobody touched keeps every byte, its `<text:s>` and all,
        // and so does everything that is not a paragraph.
        assert!(
            text.contains(
                r#"<text:p text:style-name="P1">second<text:s text:c="2"/>space</text:p>"#
            ),
            "{text}"
        );
        assert!(text.contains("<text:sequence-decls>"), "{text}");
        // And the rewritten one keeps the style it wore.
        assert!(
            text.contains(r#"<text:p text:style-name="P1">changed</text:p>"#),
            "{text}"
        );
    }

    #[test]
    fn an_edit_survives_being_read_back() {
        let (mut document, mut container) = opened();
        let Block::Paragraph(paragraph) = &mut document.body[1] else {
            panic!("a paragraph");
        };
        paragraph.content = vec![Inline::Run(Run::of("rewritten"))];
        flush(&mut document, &mut container).expect("it writes");

        let (reopened, _) = crate::read(&container).expect("it reads again");
        assert_eq!(
            reopened.text(),
            document.text(),
            "the document came back as it went in"
        );
    }

    /// The reason a table is spliced into rather than re-emitted: its geometry
    /// is in styles named from the outside, and a cell edit must not disturb
    /// any of it.
    #[test]
    fn an_edited_cell_rewrites_the_paragraph_and_not_the_table() {
        let (mut document, mut container) = opened();
        let Block::Table(table) = &mut document.body[3] else {
            panic!("the fourth block is a table");
        };
        let Block::Paragraph(paragraph) = &mut table.rows[0].cells[1].content[0] else {
            panic!("a paragraph in the cell");
        };
        paragraph.content = vec![Inline::Run(Run::of("gamma"))];

        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        assert!(text.contains("<text:p>gamma</text:p>"), "{text}");
        assert!(!text.contains("beta"), "{text}");
        assert!(
            text.contains(r#"<table:table-column table:number-columns-repeated="2"/>"#),
            "the table's own columns are untouched: {text}"
        );
        assert!(
            text.contains("<text:p>alpha</text:p>"),
            "and so is the cell beside it: {text}"
        );
    }

    /// A header is edited where it is drawn, so a save has to be able to write
    /// one — and it lives in the other part.
    #[test]
    fn an_edited_header_is_written_back_to_styles_xml() {
        let (mut document, mut container) = opened();
        let Some(Block::Paragraph(paragraph)) = document.headers[0].content.first_mut() else {
            panic!("the header is a paragraph");
        };
        paragraph.content = vec![Inline::Run(Run::of("a new header"))];

        flush(&mut document, &mut container).expect("it writes");
        let styles = String::from_utf8(container.data("styles.xml").unwrap().to_vec()).unwrap();
        assert!(styles.contains("a new header"), "{styles}");
        assert!(!styles.contains("the header"), "{styles}");
        assert_eq!(
            text_of(&container),
            CONTENT,
            "and the body was not touched for it"
        );
    }

    /// The one thing ODF makes a writer do that the other format does not.
    #[test]
    fn direct_formatting_mints_an_automatic_style_in_the_stylesheet_before_the_body() {
        let (mut document, mut container) = opened();
        let Block::Paragraph(paragraph) = &mut document.body[1] else {
            panic!("a paragraph");
        };
        let mut run = Run::of("bold now");
        run.props.toggles.set(wp_model::prop::Toggle::Bold, true);
        paragraph.content = vec![Inline::Run(run)];

        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        let minted = text
            .find(r#"style:family="text""#)
            .expect("a text style was minted");
        let body = text.find("<office:body>").expect("the body is still there");
        assert!(minted < body, "an automatic style stands before the body");
        assert!(text.contains(r#"fo:font-weight="bold""#), "{text}");
        // And the run names it.
        assert!(
            text.contains("<text:span text:style-name=\"T1\">bold now</text:span>"),
            "{text}"
        );
    }

    #[test]
    fn a_paragraph_added_at_the_end_lands_inside_the_body() {
        let (mut document, mut container) = opened();
        document
            .body
            .push(Block::Paragraph(Paragraph::of("appended")));
        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        let added = text.find("appended").expect("the new paragraph is there");
        let close = text.find("</office:text>").expect("the body still closes");
        assert!(added < close, "{text}");

        let (reopened, _) = crate::read(&container).expect("it reads again");
        assert_eq!(reopened.body.len(), document.body.len());
    }

    /// **The reason [`align`] looks either way instead of counting.** Delete
    /// the second paragraph of a document and a writer that pairs by position
    /// has every paragraph after it pairing with the one that used to follow
    /// it: all of them read back "changed", all of them are rewritten, and
    /// everything in them this crate does not model goes with the rewrite —
    /// for one deletion.
    #[test]
    fn content_out_pairs_a_block_by_what_it_says_rather_than_by_where_it_stands() {
        let (mut document, mut container) = opened();
        document.body.remove(1);
        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        assert!(!text.contains(">first</text:p>"), "{text}");
        assert!(
            text.contains(
                r#"<text:p text:style-name="P1">second<text:s text:c="2"/>space</text:p>"#
            ),
            "the paragraph after the deletion is byte for byte what it was: {text}"
        );
        assert!(
            text.contains(r#"<table:table table:name="T1"><table:table-column table:number-columns-repeated="2"/>"#),
            "and so is the table after that: {text}"
        );
    }

    /// The same question asked from the other side: a paragraph inserted in the
    /// middle does not disturb the one it was inserted before.
    #[test]
    fn content_out_copies_everything_it_did_not_change_around_an_insertion() {
        let (mut document, mut container) = opened();
        document
            .body
            .insert(1, Block::Paragraph(Paragraph::of("inserted")));
        flush(&mut document, &mut container).expect("it writes");
        let text = text_of(&container);
        assert!(text.contains("<text:p>inserted</text:p>"), "{text}");
        assert!(
            text.contains(r#"<text:p text:style-name="P1">first</text:p>"#),
            "{text}"
        );
        assert!(
            text.contains(
                r#"<text:p text:style-name="P1">second<text:s text:c="2"/>space</text:p>"#
            ),
            "{text}"
        );
        let at = text.find("inserted").expect("the new paragraph");
        let after = text.find(">first<").expect("the one it went before");
        assert!(at < after, "{text}");
    }
}
