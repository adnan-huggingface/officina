//! Server-sent events, read by hand: `event:` and `data:` lines, and a blank
//! line after each event. Both services stream their answers this way, and the
//! format is small enough that a parser of its own costs less than a crate.

use std::io::{self, BufRead, Read};

/// The most one event may hold. A service's pieces are a few kilobytes; a
/// stream that sends more in one event is not one to keep reading.
const MOST: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// The `event:` line's value, or empty when there was none.
    pub name: String,
    pub data: String,
}

pub(crate) struct Sse<R> {
    reader: R,
    line: String,
    most: usize,
}

impl<R: BufRead> Sse<R> {
    pub fn new(reader: R) -> Sse<R> {
        Sse::within(reader, MOST)
    }

    /// A reader that refuses any event, or any one line, longer than `most`
    /// bytes.
    pub fn within(reader: R, most: usize) -> Sse<R> {
        Sse {
            reader,
            line: String::new(),
            most,
        }
    }

    /// The next whole event, or `None` when the stream has ended. An event
    /// the stream ended in the middle of is not one: it is dropped, as the
    /// format says.
    pub fn next(&mut self) -> io::Result<Option<SseEvent>> {
        let mut name = String::new();
        let mut data = String::new();
        let mut any_data = false;
        let too_long = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "an event larger than any answer sends",
            )
        };
        loop {
            self.line.clear();
            // A line is read no further than the limit: a server that sends
            // one without end is refused before it fills the memory.
            let limit = self.most as u64 + 2;
            let read = (&mut self.reader).take(limit).read_line(&mut self.line)?;
            if read == 0 {
                return Ok(None);
            }
            if self.line.len() > self.most {
                return Err(too_long());
            }
            let line = self.line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if any_data {
                    return Ok(Some(SseEvent { name, data }));
                }
                name.clear();
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            match field {
                "event" => name = value.to_owned(),
                "data" => {
                    if any_data {
                        data.push('\n');
                    }
                    data.push_str(value);
                    any_data = true;
                }
                _ => {}
            }
            if data.len() > self.most {
                return Err(too_long());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(text: &str) -> Vec<SseEvent> {
        let mut sse = Sse::new(text.as_bytes());
        let mut out = Vec::new();
        while let Some(event) = sse.next().unwrap() {
            out.push(event);
        }
        out
    }

    #[test]
    fn events_are_read_with_their_names_their_data_joined_and_comments_skipped() {
        let got = events(
            ": a comment\r\nevent: ping\r\ndata: {}\r\n\r\n\
             data: one\ndata:two\n\n\
             event: lonely\n\n\
             event: cut\ndata: off",
        );
        assert_eq!(
            got,
            vec![
                SseEvent {
                    name: "ping".into(),
                    data: "{}".into()
                },
                SseEvent {
                    name: String::new(),
                    data: "one\ntwo".into()
                },
            ],
            "an event with no data is nothing, and one the stream ended inside is dropped"
        );
    }

    #[test]
    fn a_line_or_an_event_past_the_limit_is_refused_before_it_is_read_whole() {
        let endless = "data: ".to_owned() + &"x".repeat(10_000);
        let mut sse = Sse::within(endless.as_bytes(), 100);
        assert_eq!(sse.next().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(
            endless.len() - sse.reader.len() < 200,
            "only the limit's worth was read"
        );
        let many = "data: 0123456789\n".repeat(20) + "\n";
        assert!(Sse::within(many.as_bytes(), 100).next().is_err());
        assert!(Sse::within(many.as_bytes(), 1000).next().unwrap().is_some());
    }
}
