//! The lines of the paragraphs that did not change.
//!
//! **A keystroke changes one paragraph and the layout re-lays eight thousand.**
//! That is what makes a long document feel slow: not the shaping of the word
//! being typed, but the seven thousand nine hundred and ninety-nine paragraphs
//! either side of it that will come out exactly as they went in. Eight thousand
//! ordinary paragraphs cost about a third of a second, and a third of a second
//! between a key going down and the letter appearing is the difference between
//! an editor and a form.
//!
//! So a paragraph's lines are kept and handed back when nothing that could
//! change them has changed. The whole of the design is in what "nothing" means,
//! because a cache that is wrong shows the user a document that is not theirs.
//!
//! **The key is everything [`crate::inline::layout`] reads about the
//! paragraph** — the paragraph itself, the style layers resolved for it, its
//! list label, the measure it is laid into, and any float standing beside it.
//! Not a summary of those and not a hash of them: the values, compared. A
//! fingerprint would be smaller and would one day collide, and what a collision
//! produces is a paragraph silently drawn as a different paragraph.
//!
//! **The guard is everything it reads that is not about the paragraph** — the
//! style table, the theme, the note marks, and the handful of settings that
//! change what a line comes out as. Those are compared once per layout rather
//! than once per paragraph, and any difference empties the whole cache.
//! Comparing them is what makes the cache impossible to forget to clear:
//! editing a style does not have to remember to say so.
//!
//! **What is never kept.** A paragraph holding a field, because what a field
//! draws depends on the page it lands on and that is decided after it is laid.
//! A header, a footer or a note, because their paragraphs are numbered from
//! zero in a flow of their own and would collide with the body's. Each of those
//! is a small part of a document, and none of them is what a long one is made
//! of.
//!
//! A paragraph is looked up by its own index, so inserting one would shift
//! every paragraph after it out of place. It is looked for a little either side
//! of where it should be, and the offset that worked is tried first next time —
//! which costs one comparison in the ordinary case and makes pressing Return as
//! cheap as typing a letter. A change too big for that window costs one slow
//! layout and is then remembered at its new place.

use std::cell::RefCell;

use wp_model::color::Theme;
use wp_model::doc::Paragraph;
use wp_model::style::{Layers, StyleTable};
use wp_model::units::Twips;

use crate::inline::{Context, LaidParagraph, ListLabel, Obstacle};
use crate::notes::NoteMarks;

/// How far either side of its own index a paragraph is looked for.
///
/// Two is what splitting or joining a paragraph moves everything by, which is
/// what Return and Backspace do and is the only structural edit that happens on
/// an ordinary keystroke.
const WINDOW: isize = 2;

/// The lines of the paragraphs that did not change.
///
/// Owned by whoever lays the same document out repeatedly — an editor, across
/// keystrokes — and handed to the layout through [`Context::memo`]. A layout
/// given no memo behaves exactly as it did before there was one.
#[derive(Default)]
pub struct Memo {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    guard: Option<Guard>,
    /// What the previous layout arrived at, by paragraph index. An entry is
    /// taken out as it is matched, so one laid paragraph is never claimed by
    /// two.
    previous: Vec<Option<Entry>>,
    /// What this layout is arriving at, and what `previous` becomes when it
    /// ends.
    current: Vec<Option<Entry>>,
    /// The offset that last found a paragraph where its own index did not.
    shift: isize,
    hits: usize,
    misses: usize,
}

struct Entry {
    paragraph: Paragraph,
    layers: Layers,
    label: Option<ListLabel>,
    width: f64,
    obstacle: Option<Obstacle>,
    /// Whether the paragraph counted as one of a table of contents' entries.
    /// This is the one thing about a paragraph's *position* that changes how it
    /// is laid out: a hyperlink inside a contents field keeps the entry's own
    /// style instead of the hyperlink one.
    in_contents: bool,
    laid: LaidParagraph,
}

/// Everything a laid paragraph depends on that is not the paragraph.
#[derive(PartialEq)]
struct Guard {
    styles: StyleTable,
    theme: Theme,
    notes: NoteMarks,
    default_tab: Twips,
    no_leading: bool,
    fallback_font: String,
    show_revisions: bool,
    show_hidden: bool,
}

impl Guard {
    fn of(ctx: &Context<'_>) -> Guard {
        Guard {
            styles: ctx.styles.clone(),
            theme: ctx.theme.clone(),
            notes: ctx.notes.clone(),
            default_tab: ctx.default_tab,
            no_leading: ctx.no_leading,
            fallback_font: ctx.fallback_font.to_owned(),
            show_revisions: ctx.show_revisions,
            show_hidden: ctx.show_hidden,
        }
    }
}

impl Memo {
    pub fn new() -> Memo {
        Memo::default()
    }

    /// Throws everything away — for a caller that has changed the document
    /// wholesale rather than edited it.
    pub fn forget(&mut self) {
        *self.inner.borrow_mut() = Inner::default();
    }

    /// How many paragraphs the last layout recalled, and how many it laid.
    ///
    /// For the tests and the probe. An editor never asks.
    pub fn tally(&self) -> (usize, usize) {
        let inner = self.inner.borrow();
        (inner.hits, inner.misses)
    }

    /// Starts a layout, emptying the cache if anything outside the paragraphs
    /// has changed since the last one.
    pub(crate) fn begin(&self, ctx: &Context<'_>) {
        let guard = Guard::of(ctx);
        let mut inner = self.inner.borrow_mut();
        if inner.guard.as_ref() != Some(&guard) {
            inner.previous.clear();
            inner.shift = 0;
            inner.guard = Some(guard);
        }
        inner.current.clear();
        inner.hits = 0;
        inner.misses = 0;
    }

    /// Ends a layout: what it arrived at is what the next one starts from.
    pub(crate) fn commit(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.previous = std::mem::take(&mut inner.current);
    }

    /// The paragraph's lines, if this paragraph was laid this way before.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn recall(
        &self,
        index: usize,
        paragraph: &Paragraph,
        layers: &Layers,
        label: Option<&ListLabel>,
        width: f64,
        obstacle: Option<Obstacle>,
        in_contents: bool,
    ) -> Option<LaidParagraph> {
        let mut inner = self.inner.borrow_mut();
        let same = |entry: &Entry| {
            entry.width == width
                && entry.obstacle == obstacle
                && entry.in_contents == in_contents
                && entry.label.as_ref() == label
                && entry.layers == *layers
                && entry.paragraph == *paragraph
        };
        // A paragraph laid twice in one layout — the second pass a page number
        // or a half-point debt asks for — is answered from what this pass has
        // already settled rather than from the one before it.
        if let Some(entry) = inner.current.get(index).and_then(Option::as_ref) {
            if same(entry) {
                let laid = entry.laid.clone();
                inner.hits += 1;
                return Some(laid);
            }
        }
        let offsets = [inner.shift, 0, -1, 1, -WINDOW, WINDOW];
        for (at, offset) in offsets.into_iter().enumerate() {
            if offsets[..at].contains(&offset) {
                continue;
            }
            let Ok(probe) = usize::try_from(index as isize + offset) else {
                continue;
            };
            let matched = inner
                .previous
                .get(probe)
                .and_then(Option::as_ref)
                .is_some_and(&same);
            if !matched {
                continue;
            }
            let entry = inner.previous[probe].take().expect("just matched");
            let laid = entry.laid.clone();
            inner.shift = offset;
            inner.hits += 1;
            inner.put(index, entry);
            return Some(laid);
        }
        inner.misses += 1;
        None
    }

    /// Keeps a paragraph's lines against the next layout.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remember(
        &self,
        index: usize,
        paragraph: &Paragraph,
        layers: &Layers,
        label: Option<&ListLabel>,
        width: f64,
        obstacle: Option<Obstacle>,
        in_contents: bool,
        laid: &LaidParagraph,
    ) {
        // What a field draws is settled by the page it lands on, which is not
        // known while it is being laid. Keeping it would draw the number of
        // wherever it used to be.
        let holds_a_field = laid.lines.iter().any(|line| {
            line.fragments
                .iter()
                .any(|fragment| fragment.field.is_some())
        });
        if holds_a_field {
            return;
        }
        self.inner.borrow_mut().put(
            index,
            Entry {
                paragraph: paragraph.clone(),
                layers: layers.clone(),
                label: label.cloned(),
                width,
                obstacle,
                in_contents,
                laid: laid.clone(),
            },
        );
    }
}

impl Inner {
    fn put(&mut self, index: usize, entry: Entry) {
        if self.current.len() <= index {
            self.current.resize_with(index + 1, || None);
        }
        self.current[index] = Some(entry);
    }
}
