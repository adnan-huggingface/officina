//! The sparse cell store.
//!
//! Excel's grid is 1,048,576 × 16,384 — seventeen billion cells, so a dense array
//! is not an option. But real sheets are not randomly sparse either: they are
//! dense rectangles surrounded by nothing. A per-cell hash map would handle the
//! sparsity and throw away that locality, costing a hash lookup per cell on every
//! scroll and every recalculation sweep.
//!
//! So: a sorted map of fixed 16×16 chunks. Sparse between chunks, dense within
//! one. A visible screenful of cells touches a handful of chunks and then walks
//! contiguous memory.

use std::collections::BTreeMap;

use crate::cell::{Cell, CellRef, MAX_COLS, MAX_ROWS};
use crate::shift::Shift;

const CHUNK_BITS: u32 = 4;
const CHUNK_SIDE: u32 = 1 << CHUNK_BITS; // 16
const CHUNK_CELLS: usize = (CHUNK_SIDE * CHUNK_SIDE) as usize; // 256

/// Bits needed for a chunk column index, so chunk keys sort row-major.
const CHUNK_COL_BITS: u32 = 10; // MAX_COLS / 16 = 1024 chunk columns

/// A 16×16 block of cells, stored densely.
#[derive(Clone, PartialEq)]
struct Chunk {
    cells: Box<[Cell; CHUNK_CELLS]>,
    /// Number of non-vacant cells, so the chunk can be dropped when it empties.
    occupied: u16,
}

impl Chunk {
    fn new() -> Self {
        Chunk {
            cells: Box::new([Cell::default(); CHUNK_CELLS]),
            occupied: 0,
        }
    }
}

#[inline]
fn chunk_key(row: u32, col: u32) -> u32 {
    ((row >> CHUNK_BITS) << CHUNK_COL_BITS) | (col >> CHUNK_BITS)
}

#[inline]
fn offset_in_chunk(row: u32, col: u32) -> usize {
    (((row & (CHUNK_SIDE - 1)) << CHUNK_BITS) | (col & (CHUNK_SIDE - 1))) as usize
}

#[inline]
fn chunk_origin(key: u32) -> (u32, u32) {
    (
        (key >> CHUNK_COL_BITS) << CHUNK_BITS,
        (key & ((1 << CHUNK_COL_BITS) - 1)) << CHUNK_BITS,
    )
}

#[derive(Clone, Default, PartialEq)]
pub struct CellStore {
    chunks: BTreeMap<u32, Chunk>,
    len: usize,
}

impl CellStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of non-vacant cells.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The cell at `at`, or `None` if nothing is stored there.
    ///
    /// Out-of-range addresses read as empty rather than panicking: a formula can
    /// legally compute an offset past the edge of the grid, and that is a `#REF!`
    /// for the evaluator to raise, not a crash for the store to cause.
    pub fn get(&self, at: CellRef) -> Option<&Cell> {
        if !at.is_valid() {
            return None;
        }
        let chunk = self.chunks.get(&chunk_key(at.row, at.col))?;
        let cell = &chunk.cells[offset_in_chunk(at.row, at.col)];
        (!cell.is_vacant()).then_some(cell)
    }

    /// Writes `cell` at `at`. Writing a vacant cell erases the entry.
    ///
    /// Returns false if the address is outside the grid, in which case nothing
    /// is written.
    pub fn set(&mut self, at: CellRef, cell: Cell) -> bool {
        if !at.is_valid() {
            return false;
        }
        if cell.is_vacant() {
            self.clear(at);
            return true;
        }

        let key = chunk_key(at.row, at.col);
        let offset = offset_in_chunk(at.row, at.col);
        let chunk = self.chunks.entry(key).or_insert_with(Chunk::new);

        let was_vacant = chunk.cells[offset].is_vacant();
        chunk.cells[offset] = cell;
        if was_vacant {
            chunk.occupied += 1;
            self.len += 1;
        }
        true
    }

    /// Erases the cell at `at`, dropping the chunk if it becomes empty.
    pub fn clear(&mut self, at: CellRef) {
        if !at.is_valid() {
            return;
        }
        let key = chunk_key(at.row, at.col);
        let Some(chunk) = self.chunks.get_mut(&key) else {
            return;
        };
        let offset = offset_in_chunk(at.row, at.col);
        if chunk.cells[offset].is_vacant() {
            return;
        }

        chunk.cells[offset] = Cell::default();
        chunk.occupied -= 1;
        self.len -= 1;

        // Reclaiming empty chunks matters: deleting a large range would otherwise
        // leave the memory footprint of the data that used to be there.
        if chunk.occupied == 0 {
            self.chunks.remove(&key);
        }
    }

    /// The bounding box of all non-vacant cells, or `None` if the sheet is empty.
    pub fn used_range(&self) -> Option<(CellRef, CellRef)> {
        let mut min = CellRef::new(MAX_ROWS, MAX_COLS);
        let mut max = CellRef::new(0, 0);
        let mut any = false;

        for (&key, chunk) in &self.chunks {
            let (base_row, base_col) = chunk_origin(key);
            for (i, cell) in chunk.cells.iter().enumerate() {
                if cell.is_vacant() {
                    continue;
                }
                any = true;
                let row = base_row + (i as u32 >> CHUNK_BITS);
                let col = base_col + (i as u32 & (CHUNK_SIDE - 1));
                min.row = min.row.min(row);
                min.col = min.col.min(col);
                max.row = max.row.max(row);
                max.col = max.col.max(col);
            }
        }

        any.then_some((min, max))
    }

    /// Iterates non-vacant cells in row-major order.
    ///
    /// Row-major matters because that is the order xlsx serialization needs, and
    /// chunk order alone is not it — a chunk spans 16 rows, so walking chunks
    /// would emit rows 0..16 of one column band before row 0 of the next.
    pub fn iter(&self) -> impl Iterator<Item = (CellRef, &Cell)> {
        // Grouping by chunk-row band first keeps this from being a full sort:
        // within a band, rows are visited in order across the band's chunks.
        let mut bands: BTreeMap<u32, Vec<(&u32, &Chunk)>> = BTreeMap::new();
        for (key, chunk) in &self.chunks {
            bands
                .entry(key >> CHUNK_COL_BITS)
                .or_default()
                .push((key, chunk));
        }

        bands.into_values().flat_map(|band| {
            let mut out: Vec<(CellRef, &Cell)> = Vec::new();
            for local_row in 0..CHUNK_SIDE {
                for (key, chunk) in &band {
                    let (base_row, base_col) = chunk_origin(**key);
                    let start = (local_row << CHUNK_BITS) as usize;
                    for local_col in 0..CHUNK_SIDE {
                        let cell = &chunk.cells[start + local_col as usize];
                        if !cell.is_vacant() {
                            out.push((
                                CellRef::new(base_row + local_row, base_col + local_col),
                                cell,
                            ));
                        }
                    }
                }
            }
            out
        })
    }

    /// Applies `shift` to every cell, returning those it destroyed.
    ///
    /// The returned cells are what undo needs: an insert can push content off
    /// the far edge of the grid and a delete removes a band outright, and in
    /// both cases the data is gone from the store the moment this returns.
    ///
    /// Cells before the shift point do not move, so only the ones at or after it
    /// are touched. Every destination is also at or after it, so lifting them
    /// out and putting them back cannot collide with a cell that stayed put.
    pub fn shift(&mut self, shift: Shift) -> Vec<(CellRef, Cell)> {
        let moving: Vec<(CellRef, Cell)> = self
            .iter()
            .filter(|(at, _)| shift.touches(shift.axis.index(*at)))
            .map(|(at, cell)| (at, *cell))
            .collect();

        for (at, _) in &moving {
            self.clear(*at);
        }

        let mut destroyed = Vec::new();
        for (at, cell) in moving {
            match shift.point(shift.axis.index(at)) {
                Some(index) => {
                    self.set(shift.axis.with(at, index), cell);
                }
                None => destroyed.push((at, cell)),
            }
        }
        destroyed
    }

    /// Reorders rows within a band of columns: the row landing at `first + i`
    /// is the one `order[i]` holds now.
    ///
    /// A permutation, so nothing is created and nothing is destroyed — every
    /// source row is also a destination row. That is what lets the whole thing
    /// be a list of row numbers instead of a copy of every cell, and it is why
    /// the inverse of a sort is just the inverse permutation.
    ///
    /// Reads by *band* rather than by cell. Ten columns of a row live in one
    /// chunk, so one map lookup answers for all of them; going through
    /// [`get`](Self::get) would pay for the lookup ten times over.
    pub fn permute_rows(&mut self, first: u32, cols: (u32, u32), order: &[u32]) {
        let (c0, c1) = cols;
        if c1 < c0 || order.is_empty() {
            return;
        }
        let width = (c1 - c0 + 1) as usize;

        // Everything is lifted out before anything is put back: the source of
        // one row is the destination of another.
        let mut lifted: Vec<Cell> = Vec::with_capacity(order.len() * width);
        for &row in order {
            self.read_band(row, c0, c1, &mut lifted);
        }
        for (i, row_cells) in lifted.chunks_exact(width).enumerate() {
            self.write_band(first + i as u32, c0, row_cells);
        }
    }

    /// Appends `[c0..=c1]` of `row` to `out`, vacant cells included.
    fn read_band(&self, row: u32, c0: u32, c1: u32, out: &mut Vec<Cell>) {
        let mut col = c0;
        while col <= c1 {
            let end = (col | (CHUNK_SIDE - 1)).min(c1);
            match self.chunks.get(&chunk_key(row, col)) {
                Some(chunk) => {
                    let base = (row & (CHUNK_SIDE - 1)) << CHUNK_BITS;
                    out.extend(
                        (col..=end).map(|c| chunk.cells[(base | (c & (CHUNK_SIDE - 1))) as usize]),
                    );
                }
                None => out.resize(out.len() + (end - col + 1) as usize, Cell::default()),
            }
            col = end + 1;
        }
    }

    /// Writes `cells` across `[c0..]` of `row`, vacant entries erasing.
    fn write_band(&mut self, row: u32, c0: u32, cells: &[Cell]) {
        let c1 = c0 + cells.len() as u32 - 1;
        let mut col = c0;
        while col <= c1 {
            let end = (col | (CHUNK_SIDE - 1)).min(c1);
            let key = chunk_key(row, col);
            let span = &cells[(col - c0) as usize..=(end - c0) as usize];

            // A band of nothing written where there is no chunk stays nothing.
            // Creating one would be a chunk of 256 vacant cells kept alive by a
            // sort that moved an empty row.
            if !self.chunks.contains_key(&key) && span.iter().all(Cell::is_vacant) {
                col = end + 1;
                continue;
            }

            let chunk = self.chunks.entry(key).or_insert_with(Chunk::new);
            let base = (row & (CHUNK_SIDE - 1)) << CHUNK_BITS;
            let mut delta: i64 = 0;
            for (c, cell) in (col..=end).zip(span) {
                let slot = &mut chunk.cells[(base | (c & (CHUNK_SIDE - 1))) as usize];
                delta += i64::from(slot.is_vacant()) - i64::from(cell.is_vacant());
                *slot = *cell;
            }
            chunk.occupied = (i64::from(chunk.occupied) + delta) as u16;
            self.len = (self.len as i64 + delta) as usize;
            if chunk.occupied == 0 {
                self.chunks.remove(&key);
            }
            col = end + 1;
        }
    }

    /// Whether any cell in the rectangle carries a formula.
    ///
    /// Asked before a sort, to know whether the rows can simply be permuted or
    /// whether their formulas have to be rewritten as they travel.
    pub fn any_formula(&self, rows: (u32, u32), cols: (u32, u32)) -> bool {
        let (r0, r1) = rows;
        let (c0, c1) = cols;
        let mut row = r0;
        let mut band: Vec<Cell> = Vec::new();
        while row <= r1 {
            band.clear();
            self.read_band(row, c0, c1, &mut band);
            if band.iter().any(|c| c.formula.is_some()) {
                return true;
            }
            row += 1;
        }
        false
    }

    /// Approximate heap footprint, for diagnostics and the performance chunk.
    pub fn memory_bytes(&self) -> usize {
        self.chunks.len() * (CHUNK_CELLS * std::mem::size_of::<Cell>() + 32)
    }
}

impl std::fmt::Debug for CellStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CellStore")
            .field("cells", &self.len)
            .field("chunks", &self.chunks.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellValue, MAX_COLS, MAX_ROWS};
    use crate::style::StyleId;

    fn num(v: f64) -> Cell {
        Cell {
            value: CellValue::Number(v),
            ..Default::default()
        }
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut s = CellStore::new();
        let at = CellRef::new(5, 7);
        assert!(s.set(at, num(42.0)));
        assert_eq!(s.get(at).unwrap().value, CellValue::Number(42.0));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn overwriting_does_not_double_count() {
        let mut s = CellStore::new();
        let at = CellRef::new(1, 1);
        s.set(at, num(1.0));
        s.set(at, num(2.0));
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(at).unwrap().value, CellValue::Number(2.0));
    }

    #[test]
    fn writing_a_vacant_cell_erases() {
        let mut s = CellStore::new();
        let at = CellRef::new(3, 3);
        s.set(at, num(1.0));
        s.set(at, Cell::default());
        assert_eq!(s.get(at), None);
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn emptied_chunks_are_reclaimed() {
        // Deleting a large range must release the memory, not just blank the cells.
        let mut s = CellStore::new();
        for row in 0..64 {
            for col in 0..64 {
                s.set(CellRef::new(row, col), num(1.0));
            }
        }
        assert!(s.memory_bytes() > 0);

        for row in 0..64 {
            for col in 0..64 {
                s.clear(CellRef::new(row, col));
            }
        }
        assert_eq!(s.len(), 0);
        assert_eq!(s.memory_bytes(), 0, "all chunks should have been dropped");
    }

    #[test]
    fn a_styled_blank_cell_is_stored() {
        // An empty cell with a border is real content.
        let mut s = CellStore::new();
        let at = CellRef::new(2, 2);
        s.set(
            at,
            Cell {
                value: CellValue::Blank,
                style: StyleId(4),
                formula: None,
            },
        );
        assert!(s.get(at).is_some());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn out_of_range_addresses_are_refused_not_panics() {
        let mut s = CellStore::new();
        let bad = CellRef::new(MAX_ROWS, 0);
        assert!(!s.set(bad, num(1.0)));
        assert_eq!(s.get(bad), None);
        s.clear(bad); // must not panic
        assert_eq!(s.len(), 0);

        let bad_col = CellRef::new(0, MAX_COLS);
        assert!(!s.set(bad_col, num(1.0)));
    }

    #[test]
    fn iteration_is_row_major_across_chunk_boundaries() {
        // The failure this catches: walking chunks in key order emits 16 rows of
        // one column band before row 0 of the next band.
        let mut s = CellStore::new();
        let addresses = [
            CellRef::new(0, 0),
            CellRef::new(0, 20), // next chunk column, same row
            CellRef::new(3, 5),
            CellRef::new(3, 40),
            CellRef::new(20, 1), // next chunk row band
        ];
        for (i, at) in addresses.iter().enumerate() {
            s.set(*at, num(i as f64));
        }

        let visited: Vec<CellRef> = s.iter().map(|(at, _)| at).collect();
        let mut expected = addresses.to_vec();
        expected.sort_by_key(|c| (c.row, c.col));
        assert_eq!(visited, expected);
    }

    #[test]
    fn used_range_bounds_the_data() {
        let mut s = CellStore::new();
        assert_eq!(s.used_range(), None);

        s.set(CellRef::new(10, 3), num(1.0));
        s.set(CellRef::new(2, 40), num(1.0));
        s.set(CellRef::new(7, 7), num(1.0));

        let (min, max) = s.used_range().unwrap();
        assert_eq!(min, CellRef::new(2, 3));
        assert_eq!(max, CellRef::new(10, 40));
    }

    #[test]
    fn handles_the_far_corner_of_the_grid() {
        let mut s = CellStore::new();
        let corner = CellRef::new(MAX_ROWS - 1, MAX_COLS - 1);
        assert!(s.set(corner, num(1.0)));
        assert_eq!(s.get(corner).unwrap().value, CellValue::Number(1.0));
        assert_eq!(s.used_range(), Some((corner, corner)));
    }

    /// Randomized differential test against a naive map.
    ///
    /// The chunked store's whole risk is index arithmetic — a wrong shift shows up
    /// only at chunk boundaries, which hand-written cases tend to miss. This runs
    /// a scripted sequence of operations against both implementations and asserts
    /// they agree, with addresses biased toward boundaries.
    #[test]
    fn agrees_with_a_naive_map_under_random_operations() {
        use std::collections::BTreeMap;

        // Deterministic LCG: reproducible failures matter more than true randomness.
        let mut seed: u64 = 0x5EED_1234_ABCD_0001;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };

        let mut store = CellStore::new();
        let mut reference: BTreeMap<(u32, u32), Cell> = BTreeMap::new();

        // Boundary-heavy address pool: chunk edges are where the arithmetic breaks.
        let interesting = [0u32, 1, 14, 15, 16, 17, 31, 32, 33, 47, 48, 255, 256, 1000];

        for step in 0..20_000 {
            let row = interesting[(next() as usize) % interesting.len()] + (next() % 3);
            let col = interesting[(next() as usize) % interesting.len()] + (next() % 3);
            let at = CellRef::new(row, col);

            match next() % 4 {
                0 | 1 => {
                    let cell = num((next() % 100) as f64);
                    store.set(at, cell);
                    reference.insert((row, col), cell);
                }
                2 => {
                    store.clear(at);
                    reference.remove(&(row, col));
                }
                _ => {
                    let expected = reference.get(&(row, col));
                    let actual = store.get(at);
                    assert_eq!(actual, expected, "mismatch at {at} on step {step}");
                }
            }

            assert_eq!(
                store.len(),
                reference.len(),
                "length diverged at step {step}"
            );
        }

        // Final full comparison, including iteration order.
        let from_store: Vec<((u32, u32), Cell)> =
            store.iter().map(|(at, c)| ((at.row, at.col), *c)).collect();
        let from_reference: Vec<((u32, u32), Cell)> =
            reference.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(from_store, from_reference);
    }

    #[test]
    fn permuting_rows_reorders_a_band_and_leaves_its_neighbours_alone() {
        // Rows 20..=22 of columns 1..=2, with column 3 standing outside the
        // band to prove the permutation stops where it was told to.
        let mut store = CellStore::new();
        for row in 20..=22 {
            for col in 1..=3 {
                store.set(CellRef::new(row, col), num(f64::from(row * 10 + col)));
            }
        }
        let before = store.len();

        store.permute_rows(20, (1, 2), &[22, 20, 21]);

        let at = |row, col| store.get(CellRef::new(row, col)).map(|c| c.value);
        assert_eq!(at(20, 1), Some(CellValue::Number(221.0)));
        assert_eq!(at(21, 1), Some(CellValue::Number(201.0)));
        assert_eq!(at(22, 2), Some(CellValue::Number(212.0)));
        assert_eq!(
            at(20, 3),
            Some(CellValue::Number(203.0)),
            "column 3 is outside the band and does not travel"
        );
        assert_eq!(store.len(), before, "a permutation creates nothing");
    }

    #[test]
    fn a_row_of_nothing_carries_its_emptiness_with_it() {
        // The failure this guards against is a permutation implemented as
        // "write what is there": row 6 has nothing, so row 5 would keep what it
        // already held and the sheet would gain a duplicate.
        let mut store = CellStore::new();
        store.set(CellRef::new(5, 0), num(1.0));
        store.permute_rows(5, (0, 0), &[6, 5]);

        assert_eq!(store.get(CellRef::new(5, 0)), None);
        assert_eq!(
            store.get(CellRef::new(6, 0)).map(|c| c.value),
            Some(CellValue::Number(1.0))
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_permutation_that_empties_a_chunk_gives_the_memory_back() {
        let mut store = CellStore::new();
        store.set(CellRef::new(0, 0), num(1.0));
        let one_chunk = store.memory_bytes();

        // Rotate rows 0..=31 — two chunk bands — by one, which carries the only
        // cell out of the first band and into the second.
        let order: Vec<u32> = (1..32).chain(std::iter::once(0)).collect();
        store.permute_rows(0, (0, 0), &order);

        assert_eq!(store.get(CellRef::new(0, 0)), None);
        assert_eq!(
            store.get(CellRef::new(31, 0)).map(|c| c.value),
            Some(CellValue::Number(1.0))
        );
        assert_eq!(
            store.memory_bytes(),
            one_chunk,
            "the band it left is gone, and the empty rows it crossed made no chunks"
        );
    }

    #[test]
    fn a_band_is_asked_about_formulas_without_being_read_cell_by_cell() {
        let mut store = CellStore::new();
        store.set(CellRef::new(4, 4), num(1.0));
        assert!(!store.any_formula((0, 10), (0, 10)));
        store.set(
            CellRef::new(4, 4),
            Cell {
                formula: Some(crate::FormulaId::from_index(0)),
                ..num(1.0)
            },
        );
        assert!(store.any_formula((0, 10), (0, 10)));
        assert!(
            !store.any_formula((0, 3), (0, 10)),
            "a band that stops above it does not see it"
        );
        assert!(!store.any_formula((0, 10), (0, 3)));
    }
}
