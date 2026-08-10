//! The BIFF record stream: a `u16` type, a `u16` length, and that many bytes.
//!
//! The one structural complication is `CONTINUE`. A record's payload cannot
//! exceed 8224 bytes, so anything longer — the shared string table, a drawing,
//! a long text object — is written as a first record followed by `CONTINUE`
//! records carrying the rest. For nearly every record the pieces simply
//! concatenate, so [`Record`] keeps them as a list and hands back a joined view
//! on request. `SST` is the exception, and it reads the pieces itself: see
//! `string::sst`.

/// Records this reader acts on. The rest are skipped, which is most of them.
pub(crate) mod kind {
    pub const FORMULA: u16 = 0x0006;
    pub const EOF: u16 = 0x000A;
    pub const SELECTION: u16 = 0x001D;
    pub const DATEMODE: u16 = 0x0022;
    pub const FILEPASS: u16 = 0x002F;
    pub const FONT: u16 = 0x0031;
    pub const CONTINUE: u16 = 0x003C;
    pub const PANE: u16 = 0x0041;
    pub const COLINFO: u16 = 0x007D;
    pub const BOUNDSHEET: u16 = 0x0085;
    pub const PALETTE: u16 = 0x0092;
    pub const SCL: u16 = 0x00A0;
    pub const MULRK: u16 = 0x00BD;
    pub const MULBLANK: u16 = 0x00BE;
    pub const RSTRING: u16 = 0x00D6;
    pub const XF: u16 = 0x00E0;
    pub const MERGEDCELLS: u16 = 0x00E5;
    pub const SST: u16 = 0x00FC;
    pub const LABELSST: u16 = 0x00FD;
    pub const BLANK: u16 = 0x0201;
    pub const NUMBER: u16 = 0x0203;
    pub const LABEL: u16 = 0x0204;
    pub const BOOLERR: u16 = 0x0205;
    pub const STRING: u16 = 0x0207;
    pub const ROW: u16 = 0x0208;
    pub const ARRAY: u16 = 0x0221;
    pub const WINDOW2: u16 = 0x023E;
    pub const RK: u16 = 0x027E;
    pub const FORMAT: u16 = 0x041E;
    pub const SHRFMLA: u16 = 0x04BC;
    pub const BOF: u16 = 0x0809;
}

/// One record, with any `CONTINUE` blocks that belong to it.
pub(crate) struct Record<'a> {
    pub kind: u16,
    /// The record's own payload first, then each continuation in order.
    pub parts: Vec<&'a [u8]>,
}

impl<'a> Record<'a> {
    /// The payload as one slice. Borrowed when there was no continuation,
    /// which is the overwhelming majority of records.
    pub fn body(&self) -> std::borrow::Cow<'a, [u8]> {
        match self.parts.as_slice() {
            [only] => std::borrow::Cow::Borrowed(only),
            parts => std::borrow::Cow::Owned(parts.concat()),
        }
    }
}

pub(crate) struct Records<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Records<'a> {
    pub fn new(data: &'a [u8]) -> Records<'a> {
        Records { data, at: 0 }
    }

    /// Start reading at a byte offset — which is how a sheet is found, since
    /// `BOUNDSHEET` gives the absolute position of its substream's `BOF`.
    pub fn at(data: &'a [u8], offset: usize) -> Records<'a> {
        Records { data, at: offset }
    }

    /// The header and payload at `at`, without consuming it.
    fn peek(&self) -> Option<(u16, &'a [u8], usize)> {
        let head = self.data.get(self.at..self.at + 4)?;
        let kind = u16::from_le_bytes([head[0], head[1]]);
        let len = u16::from_le_bytes([head[2], head[3]]) as usize;
        let start = self.at + 4;
        // A record whose length runs off the end of the stream ends the stream.
        // Real files are padded with zeros to a sector boundary, which reads as
        // a record of type 0 and length 0 rather than as damage.
        let body = self.data.get(start..start + len)?;
        Some((kind, body, start + len))
    }

    pub fn pop(&mut self) -> Option<Record<'a>> {
        let (kind, body, mut end) = self.peek()?;
        if kind == 0 && body.is_empty() {
            return None; // the zero padding after the last record
        }
        self.at = end;

        let mut parts = vec![body];
        while let Some((next, more, next_end)) = self.peek() {
            if next != kind::CONTINUE {
                break;
            }
            parts.push(more);
            end = next_end;
            self.at = end;
        }
        Some(Record { kind, parts })
    }
}

pub(crate) fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]))
}

pub(crate) fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn f64_at(data: &[u8], at: usize) -> Option<f64> {
    let b = data.get(at..at + 8)?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(b);
    Some(f64::from_le_bytes(bytes))
}

/// Decodes an RK number: a `f64` squeezed into 32 bits.
///
/// Two flag bits at the bottom say how. Bit 1 chooses between a truncated
/// double — the mantissa's low 34 bits thrown away, which is lossless for the
/// integers and simple fractions this is used for — and a 30-bit signed
/// integer. Bit 0 says the value was multiplied by 100 first, which is how a
/// currency column fits in half the space.
pub(crate) fn rk(value: u32) -> f64 {
    let number = if value & 2 != 0 {
        // Arithmetic shift: these are signed, and a logical shift turns every
        // negative number in the file into a large positive one.
        ((value as i32) >> 2) as f64
    } else {
        f64::from_bits(((value & 0xFFFF_FFFC) as u64) << 32)
    };
    if value & 1 != 0 {
        number / 100.0
    } else {
        number
    }
}
