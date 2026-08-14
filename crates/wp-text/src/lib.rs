//! Plain text and Markdown, read and written.
//!
//! The two formats a word processor has to be able to open without complaint,
//! and the two where every question is about what is *lost*. A `.docx` saved as
//! `.txt` keeps its words and nothing else; the application says so before it
//! saves, because a user who did not mean it has no way back.

#![forbid(unsafe_code)]

pub mod encoding;
pub mod markdown;

pub use encoding::{decode, encode, line_ending, lines, Encoding, LineEnding};
pub use markdown::{read, read_plain, write, write_plain};
