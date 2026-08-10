//! Read back what `fixture` lays out, and refuse what is not a compound file.

use super::*;
use crate::fixture::Builder;

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn a_stream_below_the_cutoff_comes_back_whole() {
    // The mini path. A reader that implements only the large one returns
    // nothing here, which is most of what a .xls contains.
    let data = pattern(100);
    let file = Builder::new().stream("Workbook", data.clone()).build();
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.stream("Workbook").expect("reads"), Some(data));
}

#[test]
fn a_stream_above_the_cutoff_comes_back_whole() {
    let data = pattern(40_000);
    let file = Builder::new().stream("Workbook", data.clone()).build();
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.stream("Workbook").expect("reads"), Some(data));
}

#[test]
fn a_stream_exactly_at_the_cutoff_is_a_large_one() {
    // 4096 is the first size that is *not* mini. Off by one here and the
    // stream is looked for in the wrong table entirely.
    for len in [4095usize, 4096, 4097] {
        let data = pattern(len);
        let file = Builder::new().stream("S", data.clone()).build();
        let cfb = Cfb::read(file).expect("opens");
        assert_eq!(cfb.stream("S").expect("reads"), Some(data), "at {len}");
    }
}

#[test]
fn several_streams_of_both_kinds_keep_their_own_bytes() {
    let small = pattern(37);
    let big = pattern(9_000);
    let other = pattern(2_048);
    let file = Builder::new()
        .stream("Workbook", big.clone())
        .stream("\u{5}SummaryInformation", small.clone())
        .stream("Ctls", other.clone())
        .build();
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.stream("Workbook").expect("reads"), Some(big));
    assert_eq!(
        cfb.stream("\u{5}SummaryInformation").expect("reads"),
        Some(small)
    );
    assert_eq!(cfb.stream("Ctls").expect("reads"), Some(other));
}

#[test]
fn an_empty_stream_is_empty_rather_than_missing() {
    let file = Builder::new().stream("Empty", Vec::new()).build();
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.stream("Empty").expect("reads"), Some(Vec::new()));
}

#[test]
fn a_name_is_matched_without_regard_to_case() {
    let file = Builder::new().stream("Workbook", pattern(10)).build();
    let cfb = Cfb::read(file).expect("opens");
    assert!(cfb.entry("WORKBOOK").is_some());
    assert!(cfb.stream("nothing here").expect("asks").is_none());
}

#[test]
fn the_root_entry_is_listed_and_is_not_a_stream() {
    let file = Builder::new().stream("Workbook", pattern(10)).build();
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.entries()[0].kind, Kind::Root);
    assert_eq!(cfb.entries()[0].name, "Root Entry");
    assert!(
        cfb.entry("Root Entry").is_none(),
        "the root is not a stream"
    );
}

#[test]
fn something_that_is_not_a_compound_file_says_so() {
    // An .xlsx handed to the legacy reader has to be told apart from a corrupt
    // .xls, or the message blames the file for the caller's mistake.
    let err = Cfb::read(b"PK\x03\x04 and the rest of a zip".to_vec()).expect_err("refused");
    assert!(matches!(err, Error::NotCompound), "{err}");
    assert_eq!(err.to_string(), "not a compound file");
}

#[test]
fn a_truncated_file_is_refused_rather_than_read_short() {
    let mut file = Builder::new().stream("Workbook", pattern(40_000)).build();
    file.truncate(file.len() / 2);
    let err = Cfb::read(file).expect_err("refused");
    assert!(matches!(err, Error::Malformed(_)), "{err}");
}

#[test]
fn a_chain_that_loops_is_refused_rather_than_followed() {
    // Point a FAT entry back at its own sector. Without the bound in `chain`
    // this is an infinite loop and a growing allocation, which is the shape a
    // malformed file most wants a reader to take.
    let fat = vec![1u32, 1, 2];
    let err = fat::chain(&fat, 0).expect_err("refused");
    assert!(matches!(err, Error::Malformed(_)), "{err}");
}

#[test]
fn a_file_past_the_hundred_and_ninth_fat_sector_follows_the_difat_chain() {
    // The header holds 109 FAT sectors, which at 512 bytes covers about 7 MB.
    // Past that the DIFAT becomes a chain of its own, and this is the only way
    // to find out whether that chain is walked — so the fixture is genuinely
    // that big rather than contrived.
    let data = pattern(8 * 1024 * 1024);
    let file = Builder::new().stream("Workbook", data.clone()).build();
    let header = crate::header::Header::parse(&file).expect("a header");
    assert_ne!(
        header.first_difat,
        crate::header::ENDOFCHAIN,
        "the fixture did not get large enough to need a DIFAT chain"
    );
    let cfb = Cfb::read(file).expect("opens");
    assert_eq!(cfb.stream("Workbook").expect("reads"), Some(data));
}
