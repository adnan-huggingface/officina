//! String interning for cell text.
//!
//! Spreadsheets repeat themselves: a status column with 200k rows holds maybe
//! four distinct values. Interning turns each cell's text into a 4-byte id, which
//! is what keeps `Cell` at 24 bytes and the chunked store cache-friendly.

use std::collections::HashMap;

/// Handle into a [`StringTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrId(pub u32);

impl StrId {
    /// The empty string, always present at index 0 so `StrId::default()` is valid.
    pub const EMPTY: StrId = StrId(0);
}

impl Default for StrId {
    fn default() -> Self {
        StrId::EMPTY
    }
}

#[derive(Debug, Clone)]
pub struct StringTable {
    values: Vec<String>,
    /// Reverse index for deduplication.
    ///
    /// This does duplicate the key bytes. Fixing it properly needs a
    /// self-referential arena or hash-keyed buckets; it is not worth the unsafe
    /// or the complexity until profiling on a real workbook says so.
    index: HashMap<String, StrId>,
}

impl Default for StringTable {
    fn default() -> Self {
        Self::new()
    }
}

impl StringTable {
    pub fn new() -> Self {
        let mut t = StringTable {
            values: Vec::new(),
            index: HashMap::new(),
        };
        let empty = t.intern("");
        debug_assert_eq!(empty, StrId::EMPTY);
        t
    }

    /// Returns the id for `s`, adding it if new.
    pub fn intern(&mut self, s: &str) -> StrId {
        if let Some(id) = self.index.get(s) {
            return *id;
        }
        let id = StrId(self.values.len() as u32);
        self.values.push(s.to_owned());
        self.index.insert(s.to_owned(), id);
        id
    }

    /// Resolves an id. `None` only for an id from a different table.
    pub fn get(&self, id: StrId) -> Option<&str> {
        self.values.get(id.0 as usize).map(String::as_str)
    }

    /// Resolves an id, falling back to the empty string.
    ///
    /// Preferred at display sites: a stale id is a bug, but rendering nothing is
    /// better than panicking in the middle of a paint.
    pub fn resolve(&self, id: StrId) -> &str {
        self.get(id).unwrap_or("")
    }

    /// Every distinct string in the table, in id order.
    ///
    /// For questions about the text of a whole workbook. A sheet holds far more
    /// cells than the table holds strings — a million rows of a status column
    /// are four strings — so asking here rather than cell by cell is both fewer
    /// answers and no pointer-chasing through the store.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.values.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        // Never actually empty — index 0 is the empty string.
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_stable_and_deduplicates() {
        let mut t = StringTable::new();
        let a = t.intern("hello");
        let b = t.intern("hello");
        let c = t.intern("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(t.get(a), Some("hello"));
        assert_eq!(t.get(c), Some("world"));
    }

    #[test]
    fn empty_string_is_id_zero_so_default_is_valid() {
        let t = StringTable::new();
        assert_eq!(t.get(StrId::EMPTY), Some(""));
        assert_eq!(t.resolve(StrId::default()), "");
    }

    #[test]
    fn repeated_values_cost_one_entry() {
        let mut t = StringTable::new();
        for _ in 0..1000 {
            t.intern("Active");
        }
        assert_eq!(t.len(), 2, "empty string plus one distinct value");
    }

    #[test]
    fn unknown_id_resolves_rather_than_panics() {
        let t = StringTable::new();
        assert_eq!(t.get(StrId(9999)), None);
        assert_eq!(t.resolve(StrId(9999)), "");
    }

    #[test]
    fn distinguishes_strings_that_differ_only_by_whitespace_or_case() {
        let mut t = StringTable::new();
        assert_ne!(t.intern("a"), t.intern("A"));
        assert_ne!(t.intern("a"), t.intern("a "));
    }
}
