//! Conformance suite for the function library.
//!
//! Every expectation here is Excel's answer, not a reasonable answer. Where the
//! two differ the comment says so. The point of the suite is the awkward cases:
//! when text becomes a number, when it does not, which error wins, and what
//! happens at the boundaries where Excel's arithmetic parts ways with IEEE 754.
//!
//! Formulas are written without the leading `=`, exactly as a file stores them.

use std::collections::BTreeMap;

use ss_formula::graph::AreaRef;
use ss_formula::value::format_general;
use ss_formula::{parse, Context, Evaluator, Operand, Position, RefSet, Value};
use ss_model::{CellError, CellRange, CellRef};

// ---------------------------------------------------------------- the harness

#[derive(Default)]
struct Book {
    sheets: Vec<(String, BTreeMap<CellRef, Value>)>,
    names: BTreeMap<String, RefSet>,
}

impl Book {
    fn sheet(mut self, name: &str, cells: &[(&str, Value)]) -> Self {
        let map = cells
            .iter()
            .map(|(at, v)| (CellRef::from_a1(at).expect("test address"), v.clone()))
            .collect();
        self.sheets.push((name.to_string(), map));
        self
    }

    fn name(mut self, name: &str, sheet: usize, range: &str) -> Self {
        let (start, end) = range.split_once(':').unwrap_or((range, range));
        let range = CellRange::new(
            CellRef::from_a1(start).expect("test address"),
            CellRef::from_a1(end).expect("test address"),
        );
        self.names.insert(
            name.to_ascii_uppercase(),
            RefSet::one(AreaRef { sheet, range }),
        );
        self
    }
}

impl Context for Book {
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    fn cell(&self, sheet: usize, at: CellRef) -> Value {
        self.sheets
            .get(sheet)
            .and_then(|(_, cells)| cells.get(&at))
            .cloned()
            .unwrap_or(Value::Blank)
    }

    fn used_bounds(&self, sheet: usize) -> Option<CellRange> {
        let (_, cells) = self.sheets.get(sheet)?;
        let mut keys = cells.keys();
        let first = *keys.next()?;
        let mut range = CellRange::new(first, first);
        for &at in cells.keys() {
            range = CellRange::new(
                CellRef::new(range.start.row.min(at.row), range.start.col.min(at.col)),
                CellRef::new(range.end.row.max(at.row), range.end.col.max(at.col)),
            );
        }
        Some(range)
    }

    fn resolve_name(&self, _sheet: usize, name: &str) -> Option<Operand> {
        self.names
            .get(&name.to_ascii_uppercase())
            .cloned()
            .map(Operand::Ref)
    }
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

fn text(s: &str) -> Value {
    Value::text(s)
}

/// Evaluates a formula and renders the result in a form a test can read.
///
/// Text is quoted so `"1"` and `1` are visibly different — which, given how much
/// of this suite is about exactly that distinction, is the whole point.
fn eval_in(book: &Book, formula: &str) -> String {
    eval_at_cell(book, CellRef::new(0, 0), formula)
}

/// The same, for a formula sitting somewhere other than A1.
///
/// Where the formula lives is not decoration once implicit intersection is in
/// play: `=A1:A3` is a different number in every row.
fn eval_at_cell(book: &Book, at: CellRef, formula: &str) -> String {
    let expr = match parse(formula) {
        Ok(e) => e,
        Err(e) => return format!("<parse error: {e}>"),
    };
    let mut ev = Evaluator::new(book, Position::new(0, at));
    show(&ev.eval(&expr), &ev)
}

fn show(op: &Operand, ev: &Evaluator) -> String {
    match op {
        Operand::Value(v) => show_value(v),
        Operand::Array(a) => {
            let rows: Vec<String> = (0..a.rows())
                .map(|r| {
                    (0..a.cols())
                        .map(|c| show_value(a.get(r, c).expect("in bounds")))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect();
            format!("{{{}}}", rows.join(";"))
        }
        // A single-cell reference reads as the value in it, the same way a
        // one-by-one array collapses.
        Operand::Ref(_) => {
            let a = ev.spread(op);
            if a.rows() == 1 && a.cols() == 1 {
                show_value(&a.first())
            } else {
                show(&Operand::Array(a), ev)
            }
        }
    }
}

fn show_value(v: &Value) -> String {
    match v {
        Value::Blank => "<blank>".to_string(),
        Value::Number(n) => format_general(*n),
        Value::Text(s) => format!("\"{s}\""),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Error(e) => e.as_str().to_string(),
    }
}

/// Runs a table of `(formula, expected)` against an empty workbook.
#[track_caller]
fn check(cases: &[(&str, &str)]) {
    check_in(&Book::default().sheet("Sheet1", &[]), cases);
}

#[track_caller]
fn check_in(book: &Book, cases: &[(&str, &str)]) {
    check_at(book, "A1", cases);
}

/// Runs a table with the formula placed in a named cell.
#[track_caller]
fn check_at(book: &Book, at: &str, cases: &[(&str, &str)]) {
    let at = CellRef::from_a1(at).expect("test address");
    let mut failures = Vec::new();
    for (formula, expected) in cases {
        let got = eval_at_cell(book, at, formula);
        if got != *expected {
            failures.push(format!(
                "  ={formula}\n    expected {expected}\n    got      {got}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases disagree with Excel:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

// ------------------------------------------------------------------ coercion

#[test]
fn arithmetic_coerces_text_and_booleans() {
    check(&[
        ("\"1\"+1", "2"),
        ("\"1.5\"*2", "3"),
        ("TRUE+1", "2"),
        ("FALSE+1", "1"),
        ("\"a\"+1", "#VALUE!"),
        ("\"\"+1", "#VALUE!"),
        // A percent-suffixed string is a number to Excel's coercion.
        ("\"50%\"+0", "0.5"),
        ("-\"2\"", "-2"),
    ]);
}

#[test]
fn concatenation_coerces_everything_to_text() {
    check(&[
        ("1&1", "\"11\""),
        ("TRUE&\"\"", "\"TRUE\""),
        ("1.5&\"\"", "\"1.5\""),
        // Fifteen significant digits, which is where the binary representation
        // stops being visible.
        ("(1/3)&\"\"", "\"0.333333333333333\""),
        ("(0.1+0.2)&\"\"", "\"0.3\""),
        ("1000000&\"\"", "\"1000000\""),
    ]);
}

#[test]
fn unary_plus_passes_its_operand_through_untouched() {
    // Unlike unary minus, which coerces. `+"abc"` is text, not #VALUE!.
    check(&[("+\"abc\"", "\"abc\""), ("-\"abc\"", "#VALUE!")]);
}

#[test]
fn exponentiation_follows_excel_not_ieee() {
    check(&[
        // Unary minus binds tighter than `^`, so this is (-2)^2.
        ("-2^2", "4"),
        ("0^0", "1"),
        ("0^-1", "#DIV/0!"),
        // No complex results: a negative base with a fractional power is #NUM!,
        // where `f64::powf` would hand back NaN.
        ("(-8)^(1/3)", "#NUM!"),
        ("(-8)^2", "64"),
        ("2^-1", "0.5"),
        ("1/0", "#DIV/0!"),
    ]);
}

#[test]
fn comparison_orders_by_type_before_value() {
    check(&[
        ("1<\"a\"", "TRUE"),
        ("\"a\"<TRUE", "TRUE"),
        ("9.99999E+307<\"\"", "TRUE"),
        ("\"A\"=\"a\"", "TRUE"),
        ("EXACT(\"A\",\"a\")", "FALSE"),
        // Different types, so not equal — the classic trap when testing for an
        // empty cell with `=0`.
        ("\"\"=0", "FALSE"),
        ("\"10\"<\"9\"", "TRUE"),
        ("10<9", "FALSE"),
    ]);
}

#[test]
fn a_blank_cell_equals_both_zero_and_empty_text() {
    let book = Book::default().sheet("Sheet1", &[("B1", num(5.0))]);
    check_in(
        &book,
        &[
            ("A1=0", "TRUE"),
            ("A1=\"\"", "TRUE"),
            ("A1+1", "1"),
            ("A1&\"x\"", "\"x\""),
            ("ISBLANK(A1)", "TRUE"),
            ("ISBLANK(B1)", "FALSE"),
        ],
    );
}

// --------------------------------------------------------------- aggregation

#[test]
fn aggregation_treats_direct_arguments_differently_from_ranges() {
    // The single most surprising rule in the language, and the reason arguments
    // are not simply flattened before a function sees them.
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", text("2")),
            ("A3", Value::Bool(true)),
            ("A4", text("not a number")),
        ],
    );
    check_in(
        &book,
        &[
            ("SUM(TRUE)", "1"),
            ("SUM({TRUE})", "0"),
            ("SUM(\"1\",1)", "2"),
            ("SUM(\"a\")", "#VALUE!"),
            // Text and booleans inside a range are skipped, so this is A1 alone.
            ("SUM(A1:A4)", "1"),
            ("SUM(A1:A4,\"1\")", "2"),
            ("COUNTOFNOTHING()", "#NAME?"),
        ],
    );
}

#[test]
fn aggregation_never_skips_an_error() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", Value::Error(CellError::Div0)),
            ("A3", num(3.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUM(A1:A3)", "#DIV/0!"),
            ("SUM(1,1/0)", "#DIV/0!"),
            ("PRODUCT(A1:A3)", "#DIV/0!"),
        ],
    );
}

#[test]
fn product_of_an_empty_range_is_zero_not_one() {
    let book = Book::default().sheet("Sheet1", &[("D1", text("x"))]);
    check_in(&book, &[("PRODUCT(A1:A3)", "0"), ("PRODUCT(2,3)", "6")]);
}

#[test]
fn ranges_and_range_operators() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", num(2.0)),
            ("B1", num(10.0)),
            ("B2", num(20.0)),
            ("C3", num(100.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUM(A1:B2)", "33"),
            ("SUM(A:A)", "3"),
            ("SUM(1:1)", "11"),
            // A space is the intersection operator.
            ("SUM(A1:B2 B1:C3)", "30"),
            ("SUM(A1:A2 C3:C3)", "#NULL!"),
            // A comma inside extra parentheses is a union, not an argument list.
            ("SUM((A1,B2))", "21"),
            ("SUM(A1:A2,B1:B2)", "33"),
        ],
    );
}

#[test]
fn whole_column_references_stop_at_the_used_range() {
    // Not a correctness test so much as a promise about cost: without clipping,
    // this is a million calls into the context for one number.
    let mut visited = 0usize;
    struct Counting<'a>(&'a std::cell::Cell<usize>);
    impl Context for Counting<'_> {
        fn sheet_index(&self, _: &str) -> Option<usize> {
            Some(0)
        }
        fn cell(&self, _: usize, at: CellRef) -> Value {
            self.0.set(self.0.get() + 1);
            Value::Number(f64::from(at.row))
        }
        fn used_bounds(&self, _: usize) -> Option<CellRange> {
            Some(CellRange::new(CellRef::new(0, 0), CellRef::new(9, 3)))
        }
    }
    let counter = std::cell::Cell::new(0);
    let ctx = Counting(&counter);
    let expr = parse("SUM(A:A)").expect("parses");
    let mut ev = Evaluator::new(&ctx, Position::new(0, CellRef::new(0, 0)));
    let got = ev.eval(&expr);
    visited += counter.get();

    assert_eq!(show(&got, &ev), "45", "rows 0..9 summed");
    assert_eq!(
        visited, 10,
        "visited only the used range, not 1,048,576 rows"
    );
}

// ------------------------------------------------------------------ rounding

#[test]
fn rounding_matches_excel_at_the_halfway_point() {
    check(&[
        ("ROUND(2.5,0)", "3"),
        ("ROUND(-2.5,0)", "-3"),
        ("ROUND(2.4,0)", "2"),
        // 1.005 is really 1.00499999999999989 as an f64. Excel rounds the
        // decimal it displays, so this is 1.01 and not 1.
        ("ROUND(1.005,2)", "1.01"),
        ("ROUND(2.675,2)", "2.68"),
        ("ROUND(1234,-2)", "1200"),
        ("ROUNDUP(3.1,0)", "4"),
        ("ROUNDUP(-3.1,0)", "-4"),
        ("ROUNDDOWN(3.9,0)", "3"),
        ("ROUNDDOWN(-3.9,0)", "-3"),
    ]);
}

#[test]
fn int_floors_but_trunc_truncates() {
    check(&[
        ("INT(2.7)", "2"),
        ("INT(-2.7)", "-3"),
        ("TRUNC(2.7)", "2"),
        ("TRUNC(-2.7)", "-2"),
        ("TRUNC(-2.789,2)", "-2.78"),
    ]);
}

#[test]
fn mod_takes_the_sign_of_its_divisor() {
    // Where every language that borrows C's `%` disagrees with Excel.
    check(&[
        ("MOD(-3,2)", "1"),
        ("MOD(3,-2)", "-1"),
        ("MOD(3,2)", "1"),
        ("MOD(-3,-2)", "-1"),
        ("MOD(3,0)", "#DIV/0!"),
        ("QUOTIENT(-3,2)", "-1"),
    ]);
}

#[test]
fn ceiling_and_floor_step_toward_the_significance_sign() {
    check(&[
        ("CEILING(4.5,2)", "6"),
        ("CEILING(-4.5,2)", "-4"),
        ("CEILING(-4.5,-2)", "-6"),
        ("CEILING(4.5,-2)", "#NUM!"),
        ("CEILING(4.5,0)", "0"),
        ("FLOOR(4.5,2)", "4"),
        ("FLOOR(-4.5,2)", "-6"),
        ("FLOOR(-4.5,-2)", "-4"),
        ("FLOOR(4.5,0)", "#DIV/0!"),
        // The .MATH pair ignores the significance sign and takes a mode instead.
        ("CEILING.MATH(-4.5)", "-4"),
        ("CEILING.MATH(-4.5,1,-1)", "-5"),
        ("CEILING.MATH(4.5,2)", "6"),
        ("FLOOR.MATH(-4.5)", "-5"),
        ("FLOOR.MATH(-4.5,1,-1)", "-4"),
        ("ISO.CEILING(-4.5)", "-4"),
    ]);
}

#[test]
fn even_odd_and_mround() {
    check(&[
        ("EVEN(1.5)", "2"),
        ("EVEN(-1.5)", "-2"),
        ("EVEN(2)", "2"),
        ("ODD(2)", "3"),
        ("ODD(-2)", "-3"),
        ("ODD(3)", "3"),
        ("ODD(0)", "1"),
        ("MROUND(10,3)", "9"),
        ("MROUND(-10,-3)", "-9"),
        // Rounding toward a multiple of the opposite sign is refused.
        ("MROUND(10,-3)", "#NUM!"),
        ("MROUND(10,0)", "0"),
    ]);
}

#[test]
fn math_domain_errors_are_num_not_nan() {
    check(&[
        ("SQRT(-1)", "#NUM!"),
        ("LN(0)", "#NUM!"),
        ("LOG(-1)", "#NUM!"),
        ("LOG(8,2)", "3"),
        ("LOG(8,1)", "#NUM!"),
        ("ASIN(2)", "#NUM!"),
        ("ACOS(-1)", "3.14159265358979"),
        ("ACOSH(0.5)", "#NUM!"),
        ("FACT(-1)", "#NUM!"),
        ("FACT(171)", "#NUM!"),
        ("FACT(5)", "120"),
        ("FACT(0)", "1"),
        ("COMBIN(5,2)", "10"),
        ("COMBIN(2,5)", "#NUM!"),
        ("PERMUT(5,2)", "20"),
        ("GCD(12,18)", "6"),
        ("LCM(4,6)", "12"),
        ("GCD(-1)", "#NUM!"),
        ("SIGN(-3)", "-1"),
        ("SIGN(0)", "0"),
    ]);
}

// ------------------------------------------------------------------- logical

#[test]
fn branches_that_are_not_taken_are_not_evaluated() {
    // If this failed, `IF(A1=0,0,1/A1)` — the standard guard against division by
    // zero — would return #DIV/0! exactly when it is supposed to help.
    check(&[
        ("IF(FALSE,1/0,5)", "5"),
        ("IF(TRUE,5,1/0)", "5"),
        ("IFERROR(1/0,\"caught\")", "\"caught\""),
        ("IFERROR(5,1/0)", "5"),
        ("IFNA(1/0,\"caught\")", "#DIV/0!"),
        ("IFNA(NA(),\"caught\")", "\"caught\""),
    ]);
}

#[test]
fn an_omitted_branch_and_an_empty_branch_are_different() {
    check(&[
        ("IF(FALSE,1)", "FALSE"),
        ("IF(FALSE,1,)", "0"),
        ("IF(TRUE,,9)", "0"),
    ]);
}

#[test]
fn and_or_ignore_text_in_ranges_but_not_in_arguments() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", Value::Bool(true)),
            ("A2", text("header")),
            ("A3", Value::Bool(false)),
            ("C1", text("header")),
            ("C2", text("also text")),
        ],
    );
    check_in(
        &book,
        &[
            ("AND(TRUE,1)", "TRUE"),
            ("AND(TRUE,0)", "FALSE"),
            ("OR(FALSE,\"TRUE\")", "TRUE"),
            ("AND(\"yes\")", "#VALUE!"),
            ("AND(A1:A3)", "FALSE"),
            ("OR(A1:A3)", "TRUE"),
            // Nothing usable in the range at all, so #VALUE! rather than TRUE.
            ("AND(C1:C2)", "#VALUE!"),
            ("NOT(0)", "TRUE"),
            ("XOR(TRUE,TRUE,TRUE)", "TRUE"),
            ("XOR(TRUE,TRUE)", "FALSE"),
        ],
    );
}

#[test]
fn ifs_and_switch_pick_the_first_match() {
    check(&[
        ("IFS(FALSE,1,TRUE,2)", "2"),
        ("IFS(FALSE,1)", "#N/A"),
        ("IFS(TRUE,1,TRUE,2)", "1"),
        ("SWITCH(2,1,\"a\",2,\"b\",\"z\")", "\"b\""),
        ("SWITCH(9,1,\"a\",2,\"b\",\"z\")", "\"z\""),
        ("SWITCH(9,1,\"a\",2,\"b\")", "#N/A"),
    ]);
}

#[test]
fn information_functions_do_not_propagate_errors() {
    // The one family that must look at an error instead of passing it on.
    check(&[
        ("ISERROR(1/0)", "TRUE"),
        ("ISERROR(NA())", "TRUE"),
        ("ISERR(NA())", "FALSE"),
        ("ISERR(1/0)", "TRUE"),
        ("ISNA(NA())", "TRUE"),
        ("ISNUMBER(1)", "TRUE"),
        ("ISNUMBER(\"1\")", "FALSE"),
        ("ISTEXT(\"a\")", "TRUE"),
        ("ISNONTEXT(1)", "TRUE"),
        ("ISLOGICAL(TRUE)", "TRUE"),
        ("ERROR.TYPE(1/0)", "2"),
        ("ERROR.TYPE(NA())", "7"),
        ("ERROR.TYPE(1)", "#N/A"),
        ("TYPE(1)", "1"),
        ("TYPE(\"a\")", "2"),
        ("TYPE(TRUE)", "4"),
        ("TYPE(NA())", "16"),
        ("TYPE({1,2})", "64"),
        ("N(\"a\")", "0"),
        ("N(TRUE)", "1"),
        ("ISEVEN(4)", "TRUE"),
        ("ISODD(-3)", "TRUE"),
    ]);
}

#[test]
fn isref_distinguishes_a_reference_from_its_value() {
    let book = Book::default().sheet("Sheet1", &[("A1", num(1.0))]);
    check_in(&book, &[("ISREF(A1)", "TRUE"), ("ISREF(1)", "FALSE")]);
}

// ---------------------------------------------------------------------- text

#[test]
fn text_positions_are_one_based() {
    check(&[
        ("LEFT(\"abc\")", "\"a\""),
        ("LEFT(\"abc\",2)", "\"ab\""),
        ("LEFT(\"abc\",10)", "\"abc\""),
        ("LEFT(\"abc\",0)", "\"\""),
        ("LEFT(\"abc\",-1)", "#VALUE!"),
        ("RIGHT(\"abc\",2)", "\"bc\""),
        ("MID(\"abcdef\",2,3)", "\"bcd\""),
        // Zero is not the start of the string; it is an error.
        ("MID(\"abc\",0,1)", "#VALUE!"),
        ("MID(\"abc\",2,10)", "\"bc\""),
        ("MID(\"abc\",10,1)", "\"\""),
        ("LEN(\"abc\")", "3"),
        ("LEN(\"\")", "0"),
    ]);
}

#[test]
fn find_is_exact_and_search_is_not() {
    check(&[
        ("FIND(\"a\",\"banana\")", "2"),
        ("FIND(\"A\",\"banana\")", "#VALUE!"),
        ("FIND(\"a\",\"banana\",3)", "4"),
        ("SEARCH(\"A\",\"banana\")", "2"),
        // Wildcards work in SEARCH and are literal in FIND.
        ("SEARCH(\"b*n\",\"banana\")", "1"),
        ("SEARCH(\"?n\",\"banana\")", "2"),
        ("FIND(\"b*n\",\"banana\")", "#VALUE!"),
        ("SEARCH(\"z\",\"banana\")", "#VALUE!"),
    ]);
}

#[test]
fn text_transforms() {
    check(&[
        ("UPPER(\"aB\")", "\"AB\""),
        ("LOWER(\"aB\")", "\"ab\""),
        ("PROPER(\"o'brien mcdonald\")", "\"O'Brien Mcdonald\""),
        ("TRIM(\"  a   b  \")", "\"a b\""),
        ("REPT(\"ab\",3)", "\"ababab\""),
        ("REPT(\"ab\",0)", "\"\""),
        ("REPLACE(\"abcdef\",2,3,\"X\")", "\"aXef\""),
        ("SUBSTITUTE(\"aaa\",\"a\",\"b\")", "\"bbb\""),
        ("SUBSTITUTE(\"aaa\",\"a\",\"b\",2)", "\"aba\""),
        ("SUBSTITUTE(\"aaa\",\"\",\"b\")", "\"aaa\""),
        ("CONCATENATE(\"a\",1,TRUE)", "\"a1TRUE\""),
        ("CONCAT(\"a\",1)", "\"a1\""),
        ("TEXTJOIN(\"-\",TRUE,\"a\",\"\",\"b\")", "\"a-b\""),
        ("TEXTJOIN(\"-\",FALSE,\"a\",\"\",\"b\")", "\"a--b\""),
        ("T(\"a\")", "\"a\""),
        ("T(1)", "\"\""),
        ("VALUE(\"50%\")", "0.5"),
        ("VALUE(\"abc\")", "#VALUE!"),
        ("NUMBERVALUE(\"1.234,56\",\",\",\".\")", "1234.56"),
    ]);
}

#[test]
fn char_and_code_speak_windows_1252() {
    check(&[
        ("CHAR(65)", "\"A\""),
        ("CODE(\"A\")", "65"),
        ("CHAR(0)", "#VALUE!"),
        ("CHAR(256)", "#VALUE!"),
        // 128-159 is where the Windows code page diverges from Latin-1.
        ("CHAR(133)", "\"\u{2026}\""),
        ("CODE(\"\u{2026}\")", "133"),
        ("UNICHAR(8364)", "\"\u{20AC}\""),
        ("UNICODE(\"\u{20AC}\")", "8364"),
    ]);
}

#[test]
fn text_length_counts_characters() {
    // Documented divergence: Excel counts UTF-16 code units, so LEN of an astral
    // character is 2 there and 1 here. Matching it would let MID cut a surrogate
    // pair in half, which a Rust `String` cannot hold.
    check(&[
        ("LEN(\"h\u{e9}llo\")", "5"),
        ("MID(\"h\u{e9}llo\",2,1)", "\"\u{e9}\""),
    ]);
}

// -------------------------------------------------------------------- arrays

#[test]
fn operators_broadcast_over_arrays() {
    check(&[
        ("{1,2}+{10,20}", "{11,22}"),
        ("{1,2}+10", "{11,12}"),
        ("{1;2}*{10,20}", "{10,20;20,40}"),
        ("ABS({-1,-2})", "{1,2}"),
        // Shapes that genuinely disagree give #N/A in the cells that do not
        // line up, not an error over the whole result.
        ("{1,2,3}+{10,20}", "{11,22,#N/A}"),
        ("SUM({1,2;3,4})", "10"),
        ("SUMPRODUCT({1,2},{3,4})", "11"),
        ("SUMPRODUCT({1,2},{3,4,5})", "#VALUE!"),
    ]);
}

#[test]
fn sumproduct_treats_non_numbers_as_zero() {
    // What makes the `(a=b)*(c=d)` conditional-sum idiom work at all.
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", text("x")),
            ("A3", num(3.0)),
            ("B1", num(10.0)),
            ("B2", num(20.0)),
            ("B3", num(30.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUMPRODUCT(A1:A3,B1:B3)", "100"),
            // A reminder that `>` is not the criteria language: text sorts
            // above every number, so A2 satisfies `>2` even though it is "x".
            // `SUMIF` would not count it; a comparison operator does.
            ("SUMPRODUCT((A1:A3>2)*1,B1:B3)", "50"),
        ],
    );
}

// ---------------------------------------------------------------- conditional

#[test]
fn sumif_matches_by_criteria() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", num(3.0)),
            ("A3", num(5.0)),
            ("A4", text("apple")),
            ("B1", num(10.0)),
            ("B2", num(20.0)),
            ("B3", num(30.0)),
            ("B4", num(40.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUMIF(A1:A4,\">2\")", "8"),
            ("SUMIF(A1:A4,\">2\",B1:B4)", "50"),
            ("SUMIF(A1:A4,\"apple\",B1:B4)", "40"),
            ("SUMIF(A1:A4,\"a*\",B1:B4)", "40"),
            ("SUMIF(A1:A4,3,B1:B4)", "20"),
            ("SUMIF(A1:A4,\"<>3\",B1:B4)", "80"),
            ("SUMIFS(B1:B4,A1:A4,\">1\",A1:A4,\"<5\")", "20"),
            ("SUMIFS(B1:B4,A1:A4,\">0\")", "60"),
        ],
    );
}

#[test]
fn whole_column_criteria_ranges_stay_aligned_with_their_sum_range() {
    // Clipping a whole-column reference to the used range moves where iteration
    // starts. If the offsets into the sum range were measured from there rather
    // than from the range as written, every row would pair with the wrong one —
    // and because both columns are still the same *length*, the answer would
    // simply be quietly wrong rather than an error.
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A4", num(1.0)),
            ("A5", num(9.0)),
            ("A6", num(9.0)),
            ("B4", num(100.0)),
            ("B5", num(200.0)),
            ("B6", num(400.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUMIF(A:A,9,B:B)", "600"),
            ("SUMIF(A:A,\">0\",B:B)", "700"),
            ("SUMIFS(B:B,A:A,9)", "600"),
            ("SUMIFS(B:B,A:A,\">0\",B:B,\">150\")", "600"),
        ],
    );
}

#[test]
fn sumif_does_not_treat_a_blank_cell_as_zero() {
    // If it did, `SUMIF(range,"=0")` over a sparse column would pick up every
    // empty row in it.
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(0.0)),
            ("A3", num(0.0)),
            ("B1", num(1.0)),
            ("B2", num(2.0)),
            ("B3", num(4.0)),
        ],
    );
    check_in(
        &book,
        &[
            ("SUMIF(A1:A3,0,B1:B3)", "5"),
            ("SUMIF(A1:A3,\"=\",B1:B3)", "2"),
            ("SUMIF(A1:A3,\"<>\",B1:B3)", "5"),
        ],
    );
}

// --------------------------------------------------------------- names, sheets

#[test]
fn references_reach_other_sheets_and_defined_names() {
    let book = Book::default()
        .sheet("Sheet1", &[("A1", num(1.0))])
        .sheet("Data Sheet", &[("A1", num(10.0)), ("A2", num(20.0))])
        .name("Totals", 1, "A1:A2");
    check_in(
        &book,
        &[
            ("'Data Sheet'!A1", "10"),
            ("SUM('Data Sheet'!A1:A2)", "30"),
            ("SUM(Totals)", "30"),
            ("SUM(Sheet1:'Data Sheet'!A1)", "11"),
            ("Nowhere!A1", "#REF!"),
            ("SUM(Unknown_Name)", "#NAME?"),
        ],
    );
}

#[test]
fn an_unknown_function_is_a_name_error_not_a_crash() {
    // Files contain functions we have not implemented yet. Every one of them has
    // to come back as a value the user can see, never as a failure to open.
    check(&[
        ("NOSUCHFUNCTION()", "#NAME?"),
        ("NOSUCHFUNCTION(1,2,3)", "#NAME?"),
        // The `_xlfn.` prefix is how Excel stores post-2007 functions.
        ("_xlfn.CONCAT(\"a\",\"b\")", "\"ab\""),
        ("_xlfn.NOSUCHTHING()", "#NAME?"),
    ]);
}

#[test]
fn deep_nesting_is_bounded() {
    // A generated or hostile formula must not blow the stack.
    let deep = format!("{}1{}", "ABS(".repeat(200), ")".repeat(200));
    let book = Book::default().sheet("Sheet1", &[]);
    let got = eval_in(&book, &deep);
    assert!(
        got == "#NUM!" || got.starts_with("<parse error"),
        "deep nesting should be refused, got {got}"
    );
}

#[test]
fn rand_stays_inside_its_range_and_is_reproducible() {
    let book = Book::default().sheet("Sheet1", &[]);
    let expr = parse("RAND()").expect("parses");
    let sample = |seed: u64| {
        let mut ev = Evaluator::new(&book, Position::new(0, CellRef::new(0, 0))).with_seed(seed);
        let mut out = Vec::new();
        for _ in 0..64 {
            match ev.eval(&expr) {
                Operand::Value(Value::Number(n)) => out.push(n),
                other => panic!("RAND returned {other:?}"),
            }
        }
        out
    };
    let a = sample(1);
    assert!(a.iter().all(|&n| (0.0..1.0).contains(&n)), "out of range");
    assert_eq!(a, sample(1), "same seed, same sequence");
    assert_ne!(a, sample(2), "different seeds should differ");

    let between = parse("RANDBETWEEN(1,6)").expect("parses");
    let mut ev = Evaluator::new(&book, Position::new(0, CellRef::new(0, 0)));
    for _ in 0..256 {
        match ev.eval(&between) {
            Operand::Value(Value::Number(n)) => {
                assert!((1.0..=6.0).contains(&n) && n.fract() == 0.0, "got {n}");
            }
            other => panic!("RANDBETWEEN returned {other:?}"),
        }
    }
}

// --------------------------------------------------------------- dates (C7)

#[test]
fn dates_are_serials_with_a_leap_day_that_never_happened() {
    check(&[
        // The three serials that pin Excel's calendar to Lotus 1-2-3's bug.
        ("DATE(1900,2,28)", "59"),
        ("DATE(1900,2,29)", "60"),
        ("DATE(1900,3,1)", "61"),
        ("DAY(60)", "29"),
        ("MONTH(60)", "2"),
        ("YEAR(60)", "1900"),
        // Anything after it is an ordinary Gregorian date offset by one day.
        ("DATE(2024,1,1)", "45292"),
        ("DATE(2024,3,1)", "45352"),
        ("DATE(9999,12,31)", "2958465"),
        // There is no date before 1900 to count from, and written text saying
        // otherwise is refused rather than shifted into range.
        ("DATEVALUE(\"1899-12-31\")", "#VALUE!"),
    ]);
}

#[test]
fn date_treats_its_arguments_as_offsets_not_as_a_calendar() {
    check(&[
        // A year below 1900 is an offset from 1900, not a year in antiquity —
        // which is also why `DATE(1899,12,31)` is in the year 3799.
        ("YEAR(DATE(24,1,1))", "1924"),
        ("YEAR(DATE(1899,12,31))", "3799"),
        // Months and days roll over rather than erroring.
        ("DATE(2024,13,1)-DATE(2025,1,1)", "0"),
        ("DAY(DATE(2024,3,0))", "29"),
        ("MONTH(DATE(2024,3,0))", "2"),
        ("DAY(DATE(2024,1,32))", "1"),
        ("MONTH(DATE(2024,1,32))", "2"),
        // Time wraps within the day: there is nowhere to put a 27th hour.
        ("TIME(27,0,0)", "0.125"),
        ("HOUR(0.75)", "18"),
        ("MINUTE(TIME(1,30,0))", "30"),
        ("TIME(-1,0,0)", "#NUM!"),
    ]);
}

#[test]
fn weekday_and_week_numbers_follow_their_scheme_argument() {
    check(&[
        // 1 January 2024 was a Monday.
        ("WEEKDAY(DATE(2024,1,1))", "2"),
        ("WEEKDAY(DATE(2024,1,1),2)", "1"),
        ("WEEKDAY(DATE(2024,1,1),3)", "0"),
        ("WEEKDAY(DATE(2024,1,1),12)", "7"),
        ("WEEKDAY(DATE(2024,1,1),8)", "#NUM!"),
        ("WEEKNUM(DATE(2024,1,1))", "1"),
        ("ISOWEEKNUM(DATE(2024,1,1))", "1"),
        // The ISO Thursday rule: 1 January 2021 was a Friday, so it belongs to
        // the last week of 2020 rather than the first of 2021.
        ("ISOWEEKNUM(DATE(2021,1,1))", "53"),
    ]);
}

#[test]
fn month_arithmetic_clamps_to_the_length_of_the_target_month() {
    check(&[
        // 31 January plus a month is 29 February, not 2 March.
        ("EDATE(DATE(2024,1,31),1)-DATE(2024,2,29)", "0"),
        ("EOMONTH(DATE(2024,1,15),0)-DATE(2024,1,31)", "0"),
        ("EOMONTH(DATE(2024,1,15),-1)-DATE(2023,12,31)", "0"),
        ("DAYS(DATE(2024,3,1),DATE(2024,2,1))", "29"),
        // A year of twelve thirty-day months, for bond coupons.
        ("DAYS360(DATE(2024,1,31),DATE(2024,3,31))", "60"),
        ("DAYS360(DATE(2024,1,30),DATE(2024,3,31),TRUE)", "60"),
    ]);
}

#[test]
fn datedif_measures_each_component_independently() {
    check(&[
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"Y\")", "4"),
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"M\")", "49"),
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"D\")", "1516"),
        // Days ignoring months and years, borrowing from February.
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"MD\")", "24"),
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"YM\")", "1"),
        ("DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"YD\")", "55"),
        // Excel refuses a reversed interval rather than reporting a negative one.
        ("DATEDIF(DATE(2024,1,1),DATE(2023,1,1),\"D\")", "#NUM!"),
        ("DATEDIF(DATE(2024,1,1),DATE(2024,1,1),\"Q\")", "#NUM!"),
    ]);
}

#[test]
fn yearfrac_has_five_day_count_bases() {
    check(&[
        // 30/360: six months is exactly half a year whatever the calendar says.
        ("YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),0)", "0.5"),
        ("YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),4)", "0.5"),
        // Actual/actual over a leap year.
        ("YEARFRAC(DATE(2024,1,1),DATE(2025,1,1),1)", "1"),
        (
            "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),2)",
            "0.505555555555556",
        ),
        (
            "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),3)",
            "0.498630136986301",
        ),
        ("YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),5)", "#NUM!"),
        // The arguments in the other order give the same magnitude.
        ("YEARFRAC(DATE(2024,7,1),DATE(2024,1,1),0)", "0.5"),
    ]);
}

#[test]
fn working_days_skip_weekends_and_the_holidays_given() {
    check(&[
        // January 2024 starts on a Monday and has 23 weekdays.
        ("NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31))", "23"),
        (
            "NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31),DATE(2024,1,15))",
            "22",
        ),
        // A holiday that falls on a weekend was not going to be counted anyway.
        (
            "NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31),DATE(2024,1,6))",
            "23",
        ),
        // Weekend code 11 is Sunday only.
        ("NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),11)", "6"),
        (
            "NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),\"0000001\")",
            "6",
        ),
        ("NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),8)", "#NUM!"),
        // Friday plus one working day is Monday.
        ("DAY(WORKDAY(DATE(2024,1,5),1))", "8"),
        ("DAY(WORKDAY(DATE(2024,1,8),-1))", "5"),
    ]);
}

#[test]
fn written_dates_are_read_where_a_serial_is_wanted() {
    check(&[
        ("DATEVALUE(\"2024-03-01\")", "45352"),
        ("DATEVALUE(\"3/1/2024\")", "45352"),
        ("DATEVALUE(\"1-Mar-2024\")", "45352"),
        ("YEAR(\"2024-03-01\")", "2024"),
        ("TIMEVALUE(\"12:00\")", "0.5"),
        ("TIMEVALUE(\"12:00 AM\")", "0"),
        ("DATEVALUE(\"hello\")", "#VALUE!"),
        // A boolean is a number everywhere else and not a date here.
        ("YEAR(TRUE)", "#VALUE!"),
    ]);
}

// -------------------------------------------------- implicit intersection (C7)

#[test]
fn a_range_in_scalar_position_intersects_with_the_formulas_own_row() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(10.0)),
            ("A2", num(20.0)),
            ("A3", num(30.0)),
            ("C1", num(7.0)),
        ],
    );
    // `N` wants one value, so the column A1:A3 offers the one in row 2.
    check_at(&book, "B2", &[("N(A1:A3)", "20")]);
    // In B3 it offers row 3 — the same formula, a different answer, which is
    // the entire point of the rule.
    check_at(&book, "B3", &[("N(A1:A3)", "30")]);
    // In B5 the range has no row 5 to offer.
    check_at(&book, "B5", &[("N(A1:A3)", "#VALUE!")]);
    // A single row intersects by column instead.
    check_at(&book, "C9", &[("N(A1:C1)", "7")]);
    // A block has nothing to offer in either direction.
    check_at(&book, "B2", &[("N(A1:C3)", "#VALUE!")]);
    // Aggregation is not scalar position, so nothing intersects there.
    check_at(&book, "B5", &[("SUM(A1:A3)", "60")]);
    // Operators broadcast rather than intersecting. That is modern Excel's
    // rule — a formula like this spills — and it is a deliberate divergence
    // from the pre-2019 behaviour, which would give 40 here.
    check_at(&book, "B2", &[("A1:A3*2", "{20;40;60}")]);
}

// --------------------------------------------------------------- lookup (C7)

fn lookup_book() -> Book {
    Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", num(3.0)),
            ("A3", num(5.0)),
            ("A4", num(7.0)),
            ("A5", num(9.0)),
            ("B1", text("a")),
            ("B2", text("b")),
            ("B3", text("c")),
            ("B4", text("d")),
            ("B5", text("e")),
        ],
    )
}

#[test]
fn vlookup_defaults_to_approximate_which_is_the_wrong_default() {
    let book = lookup_book();
    check_in(
        &book,
        &[
            ("VLOOKUP(5,A1:B5,2,FALSE)", "\"c\""),
            // No fourth argument means approximate: the largest value not
            // exceeding 6 is 5, and nothing warns that 6 was not found.
            ("VLOOKUP(6,A1:B5,2)", "\"c\""),
            ("VLOOKUP(6,A1:B5,2,FALSE)", "#N/A"),
            // Below everything in the column there is no candidate at all.
            ("VLOOKUP(0,A1:B5,2)", "#N/A"),
            ("VLOOKUP(5,A1:B5,3)", "#REF!"),
            ("VLOOKUP(5,A1:B5,0)", "#VALUE!"),
            ("HLOOKUP(\"a\",A1:B5,3,FALSE)", "\"c\""),
        ],
    );
}

#[test]
fn match_and_index_address_by_position() {
    let book = lookup_book();
    check_in(
        &book,
        &[
            ("MATCH(5,A1:A5,0)", "3"),
            ("MATCH(6,A1:A5,1)", "3"),
            ("MATCH(6,A1:A5,0)", "#N/A"),
            ("MATCH(\"c\",B1:B5,0)", "3"),
            // An exact match still honours wildcards.
            ("MATCH(\"?\",B1:B5,0)", "1"),
            ("INDEX(A1:B5,2,2)", "\"b\""),
            ("INDEX(A1:A5,3)", "5"),
            ("INDEX(A1:A5,9)", "#REF!"),
            ("INDEX(A1:B5,MATCH(7,A1:A5,0),2)", "\"d\""),
            // A zero index means the whole line, which is why this sums.
            ("SUM(INDEX(A1:B5,0,1))", "25"),
        ],
    );
}

#[test]
fn index_and_offset_return_references_not_values() {
    let book = lookup_book();
    check_in(
        &book,
        &[
            // Only a reference can be one end of a range.
            ("SUM(A1:INDEX(A1:A5,3))", "9"),
            ("SUM(OFFSET(A1,1,0,2,1))", "8"),
            ("ROWS(OFFSET(A1,0,0,3,1))", "3"),
            // Off the edge of the sheet is `#REF!`, not a clamp.
            ("OFFSET(A1,-1,0)", "#REF!"),
            ("OFFSET(A1,0,0,0,1)", "#REF!"),
            ("SUM(INDIRECT(\"A1:A3\"))", "9"),
            ("INDIRECT(\"A2\")", "3"),
            ("INDIRECT(\"nonsense\")", "#REF!"),
            // The R1C1 form has no parser behind it, and a wrong cell would be
            // worse than an error.
            ("INDIRECT(\"R1C1\",FALSE)", "#REF!"),
        ],
    );
}

#[test]
fn lookup_positions_count_from_the_range_the_user_wrote() {
    // The trap that clipping a whole-column reference sets: the used range
    // starts at row 3, so a position counted from there would be 2 rather than
    // 4 — plausible, wrong, and silent.
    let book = Book::default().sheet("Sheet1", &[("A3", num(5.0)), ("A4", num(7.0))]);
    check_in(
        &book,
        &[
            ("MATCH(7,A:A,0)", "4"),
            ("MATCH(5,A:A,0)", "3"),
            ("INDEX(A:A,4)", "7"),
        ],
    );
}

#[test]
fn choose_is_lazy_and_the_reference_functions_report_positions() {
    let book = lookup_book();
    check_in(
        &book,
        &[
            ("CHOOSE(2,\"x\",\"y\",\"z\")", "\"y\""),
            // The branch not taken is never evaluated.
            ("CHOOSE(1,1,1/0)", "1"),
            ("CHOOSE(4,\"x\",\"y\")", "#VALUE!"),
            ("ROW(A5)", "5"),
            ("COLUMN(C1)", "3"),
            ("SUM(ROW(A1:A5))", "15"),
            ("ROWS(A1:B5)", "5"),
            ("COLUMNS(A1:B5)", "2"),
            ("AREAS((A1:A5,B1:B5))", "2"),
            ("TRANSPOSE({1,2;3,4})", "{1,3;2,4}"),
            ("ADDRESS(1,1)", "\"$A$1\""),
            ("ADDRESS(2,3,4)", "\"C2\""),
            ("ADDRESS(1,1,1,TRUE,\"Sheet 1\")", "\"'Sheet 1'!$A$1\""),
        ],
    );
}

#[test]
fn xlookup_carries_its_own_not_found_answer() {
    let book = lookup_book();
    check_in(
        &book,
        &[
            ("XLOOKUP(5,A1:A5,B1:B5)", "\"c\""),
            // The fourth argument is why wrapping this in `IFNA` is unnecessary.
            ("XLOOKUP(6,A1:A5,B1:B5,\"none\")", "\"none\""),
            ("XLOOKUP(6,A1:A5,B1:B5)", "#N/A"),
            // Match mode -1 falls back to the next smaller value, 1 to the next
            // larger.
            ("XLOOKUP(6,A1:A5,B1:B5,\"none\",-1)", "\"c\""),
            ("XLOOKUP(6,A1:A5,B1:B5,\"none\",1)", "\"d\""),
            ("XMATCH(7,A1:A5)", "4"),
            ("LOOKUP(6,A1:A5,B1:B5)", "\"c\""),
            // The array form searches the first column and returns the last.
            ("LOOKUP(6,A1:B5)", "\"c\""),
        ],
    );
}

// ----------------------------------------------------------- statistics (C7)

fn mixed_book() -> Book {
    Book::default().sheet(
        "Sheet1",
        &[
            ("A1", num(1.0)),
            ("A2", num(2.0)),
            ("A3", text("x")),
            ("A4", num(4.0)),
            ("A5", Value::Bool(true)),
        ],
    )
}

#[test]
fn the_counting_functions_disagree_with_each_other_on_purpose() {
    let book = mixed_book();
    check_in(
        &book,
        &[
            ("COUNT(A1:A6)", "3"),
            ("COUNTA(A1:A6)", "5"),
            ("COUNTBLANK(A1:A6)", "1"),
            // A boolean written directly counts; the same boolean in a range
            // does not, exactly as in `SUM`.
            ("COUNT(TRUE)", "1"),
            ("COUNT(\"1\")", "1"),
            ("COUNT(\"x\")", "0"),
        ],
    );
}

#[test]
fn average_skips_text_but_averagea_scores_it_zero() {
    let book = mixed_book();
    check_in(
        &book,
        &[
            ("AVERAGE(A1:A5)", "2.33333333333333"),
            ("AVERAGEA(A1:A5)", "1.6"),
            ("MAX(A1:A5)", "4"),
            ("MIN(A1:A5)", "1"),
            ("MINA(A1:A5)", "0"),
            // An average of nothing is a division by zero; a maximum of nothing
            // is 0, which makes an empty range and a range of zeros
            // indistinguishable.
            ("AVERAGE(A3:A3)", "#DIV/0!"),
            ("MAX(A3:A3)", "0"),
        ],
    );
}

#[test]
fn order_statistics_interpolate_between_neighbours() {
    check(&[
        ("MEDIAN({1,2,3,4})", "2.5"),
        ("MEDIAN({1,2,3})", "2"),
        ("LARGE({1,5,3},1)", "5"),
        ("SMALL({1,5,3},2)", "3"),
        ("SMALL({1,5,3},4)", "#NUM!"),
        ("MODE({1,2,2,3})", "2"),
        // Nothing repeats, so there is no mode to report.
        ("MODE({1,2,3})", "#N/A"),
        ("PERCENTILE({1,2,3,4},0.5)", "2.5"),
        ("QUARTILE({1,2,3,4},1)", "1.75"),
        // The exclusive form has no 0th or 100th percentile to give.
        ("PERCENTILE.EXC({1,2,3,4},0.5)", "2.5"),
        ("PERCENTILE.EXC({1,2,3,4},0)", "#NUM!"),
        ("RANK(3,{1,3,3,5})", "2"),
        ("RANK.AVG(3,{1,3,3,5})", "2.5"),
        ("RANK(4,{1,3,3,5})", "#N/A"),
    ]);
}

#[test]
fn spread_and_regression_statistics() {
    check(&[
        ("STDEV({2,4,4,4,5,5,7,9})", "2.1380899352994"),
        ("STDEVP({2,4,4,4,5,5,7,9})", "2"),
        ("VAR.P({2,4,4,4,5,5,7,9})", "4"),
        // A sample variance needs two observations.
        ("VAR({1})", "#DIV/0!"),
        ("AVEDEV({1,2,3,4})", "1"),
        ("DEVSQ({1,2,3,4})", "5"),
        ("GEOMEAN({1,4})", "2"),
        ("HARMEAN({1,4})", "1.6"),
        ("GEOMEAN({1,-4})", "#NUM!"),
        ("CORREL({1,2,3},{2,4,6})", "1"),
        ("SLOPE({2,4,6},{1,2,3})", "2"),
        ("INTERCEPT({2,4,6},{1,2,3})", "0"),
        ("FORECAST(4,{2,4,6},{1,2,3})", "8"),
        ("COVARIANCE.P({1,2,3},{2,4,6})", "1.33333333333333"),
    ]);
}

#[test]
fn the_conditional_aggregates_share_one_criteria_language() {
    let book = mixed_book();
    check_in(
        &book,
        &[
            ("COUNTIF(A1:A5,\">1\")", "2"),
            ("COUNTIF(A1:A5,\"x\")", "1"),
            ("AVERAGEIF(A1:A5,\">1\")", "3"),
            ("COUNTIFS(A1:A5,\">1\",A1:A5,\"<4\")", "1"),
            ("MAXIFS(A1:A5,A1:A5,\">1\")", "4"),
            ("MINIFS(A1:A5,A1:A5,\">1\")", "2"),
            // A numeric criterion never matches the text or the boolean, which
            // is what stops a column heading from being counted.
            ("COUNTIF(A1:A5,\">0\")", "3"),
            ("AVERAGEIF(A1:A5,\">100\")", "#DIV/0!"),
        ],
    );
}

#[test]
fn text_formats_through_the_same_engine_a_cell_does() {
    // `TEXT` is the number-format language as a function. Anything it gets
    // wrong here, a cell displaying the same code gets wrong too — which is
    // exactly why there is one implementation and not two.
    check(&[
        (r##"TEXT(1234.567,"#,##0.00")"##, "\"1,234.57\""),
        (r#"TEXT(0.42,"0.0%")"#, "\"42.0%\""),
        (r##"TEXT(-5,"#,##0;(#,##0)")"##, "\"(5)\""),
        (r#"TEXT(45306,"yyyy-mm-dd")"#, "\"2024-01-15\""),
        (r#"TEXT(45306.5,"h:mm AM/PM")"#, "\"12:00 PM\""),
        // Text that reads as a number is formatted as one.
        (r##"TEXT("1234","#,##0")"##, "\"1,234\""),
        // A blank is the empty string, not "0".
        (r#"TEXT(A9,"0.00")"#, "\"\""),
        (r#"TEXT(1,"0.00","extra")"#, "#VALUE!"),
    ]);
}

#[test]
fn fixed_and_dollar_round_before_they_group() {
    check(&[
        ("FIXED(1234.567)", "\"1,234.57\""),
        ("FIXED(1234.567,1)", "\"1,234.6\""),
        ("FIXED(1234.567,1,TRUE)", "\"1234.6\""),
        // A negative count rounds left of the point, which the format language
        // cannot express on its own.
        ("FIXED(1234.567,-2)", "\"1,200\""),
        ("DOLLAR(1234.567)", "\"$1,234.57\""),
        ("DOLLAR(-1234.567)", "\"($1,234.57)\""),
        ("DOLLAR(1234.567,0)", "\"$1,235\""),
    ]);
}

#[test]
fn the_annuity_family_agrees_on_which_way_the_money_goes() {
    // Money paid out is negative. A version without the sign convention hands
    // the borrower a loan that pays them, and every downstream sum is wrong by
    // twice the payment.
    check(&[
        ("ROUND(PMT(0.05/12,360,200000),2)", "-1073.64"),
        ("ROUND(PV(0.05/12,360,PMT(0.05/12,360,200000)),2)", "200000"),
        (
            "ROUND(FV(0.05/12,360,PMT(0.05/12,360,200000),200000),2)",
            "0",
        ),
        (
            "ROUND(NPER(0.05/12,PMT(0.05/12,360,200000),200000),2)",
            "360",
        ),
        (
            "ROUND(RATE(360,PMT(0.05/12,360,200000),200000)*12,4)",
            "0.05",
        ),
        // Paying at the start of the period instead of the end.
        ("ROUND(PMT(0.05/12,360,200000,0,1),2)", "-1069.19"),
        ("ROUND(PMT(0,10,1000),2)", "-100"),
    ]);
}

#[test]
fn interest_and_principal_add_back_up_to_the_payment() {
    check(&[
        ("ROUND(IPMT(0.05/12,1,360,200000),2)", "-833.33"),
        ("ROUND(PPMT(0.05/12,1,360,200000),2)", "-240.31"),
        (
            "ROUND(IPMT(0.05/12,1,360,200000)+PPMT(0.05/12,1,360,200000),2)",
            "-1073.64",
        ),
        ("ROUND(CUMIPMT(0.05/12,360,200000,1,12,0),2)", "-9932.99"),
    ]);
}

#[test]
fn npv_discounts_the_first_flow_the_way_excel_does_and_not_the_textbook_way() {
    // Excel's NPV discounts the *first* cash flow by one period, so an initial
    // outlay has to be added outside the call. Matching the textbook here would
    // give every existing spreadsheet a different answer.
    check(&[
        ("ROUND(NPV(0.1,100,100,100),4)", "248.6852"),
        ("ROUND(-500+NPV(0.1,100,200,300,400),2)", "254.8"),
        ("ROUND(IRR({-500,100,200,300,400}),4)", "0.2727"),
        ("ROUND(MIRR({-500,100,200,300,400},0.1,0.12),4)", "0.2254"),
    ]);
}

#[test]
fn depreciation_rounds_where_excel_rounds() {
    check(&[
        ("SLN(10000,1000,5)", "1800"),
        ("SYD(10000,1000,5,1)", "3000"),
        ("ROUND(DDB(10000,1000,5,1),2)", "4000"),
        // DDB never takes the value below the salvage: the last period gets
        // whatever is left of the depreciable base, not the full 40%.
        ("ROUND(DDB(10000,1000,5,5),2)", "296"),
        ("ROUND(DB(10000,1000,5,1),2)", "3690"),
    ]);
}

#[test]
fn base_conversion_writes_negatives_in_twos_complement() {
    // `DEC2BIN(-1)` is ten ones, not "-1", and reading it back gives -1. A
    // reader that treats the tenth digit as a value rather than a sign gets 511
    // for a number that is minus one.
    check(&[
        (r#"DEC2BIN(9)"#, "\"1001\""),
        (r#"DEC2BIN(9,8)"#, "\"00001001\""),
        (r#"DEC2BIN(-1)"#, "\"1111111111\""),
        (r#"BIN2DEC("1111111111")"#, "-1"),
        (r#"BIN2DEC("1001")"#, "9"),
        (r#"DEC2HEX(255)"#, "\"FF\""),
        (r#"HEX2DEC("FFFFFFFFFF")"#, "-1"),
        (r#"DEC2OCT(-8)"#, "\"7777777770\""),
        (r#"HEX2BIN("F")"#, "\"1111\""),
        // Out of the ten-digit range.
        ("DEC2BIN(512)", "#NUM!"),
    ]);
}

#[test]
fn the_bitwise_family_works_over_forty_eight_bit_integers() {
    check(&[
        ("BITAND(12,10)", "8"),
        ("BITOR(12,10)", "14"),
        ("BITXOR(12,10)", "6"),
        ("BITLSHIFT(3,2)", "12"),
        ("BITRSHIFT(12,2)", "3"),
        // A negative shift goes the other way, which is the documented rule.
        ("BITLSHIFT(12,-2)", "3"),
        ("BITAND(-1,1)", "#NUM!"),
    ]);
}

#[test]
fn unit_conversion_refuses_to_cross_dimensions() {
    check(&[
        ("ROUND(CONVERT(1,\"lbm\",\"kg\"),4)", "0.4536"),
        ("ROUND(CONVERT(100,\"C\",\"F\"),2)", "212"),
        ("ROUND(CONVERT(1,\"mi\",\"km\"),6)", "1.609344"),
        // Mass into distance has no answer, and #N/A says so.
        ("CONVERT(1,\"kg\",\"m\")", "#N/A"),
        ("CONVERT(1,\"kg\",\"stones\")", "#N/A"),
        ("DELTA(5,5)", "1"),
        ("GESTEP(5,4)", "1"),
        ("ROUND(ERF(1),6)", "0.842701"),
    ]);
}

#[test]
fn a_criteria_range_ands_across_and_ors_down() {
    // The whole language of the database family. Read as one flat list of
    // conditions, this returns 900 — right by coincidence on simpler data and
    // wrong here.
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", text("Region")),
            ("B1", text("Sales")),
            ("A2", text("North")),
            ("B2", Value::Number(150.0)),
            ("A3", text("North")),
            ("B3", Value::Number(50.0)),
            ("A4", text("South")),
            ("B4", Value::Number(200.0)),
            ("A5", text("South")),
            ("B5", Value::Number(30.0)),
            // North over 100, or South at all.
            ("D1", text("Region")),
            ("E1", text("Sales")),
            ("D2", text("North")),
            ("E2", text(">100")),
            ("D3", text("South")),
        ],
    );
    check_at(
        &book,
        "H1",
        &[
            ("DSUM(A1:B5,\"Sales\",D1:E3)", "380"),
            ("DCOUNT(A1:B5,\"Sales\",D1:E3)", "3"),
            ("DMAX(A1:B5,2,D1:E3)", "200"),
            ("DMIN(A1:B5,2,D1:E3)", "30"),
            ("ROUND(DAVERAGE(A1:B5,2,D1:E3),4)", "126.6667"),
            // DGET insists on exactly one match, and says so two ways.
            ("DGET(A1:B5,\"Sales\",D1:E2)", "150"),
            ("DGET(A1:B5,\"Sales\",D1:E3)", "#NUM!"),
        ],
    );
}

#[test]
fn the_dynamic_array_functions_return_rectangles() {
    let book = Book::default().sheet(
        "Sheet1",
        &[
            ("A1", Value::Number(3.0)),
            ("A2", Value::Number(1.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(2.0)),
            ("B1", text("c")),
            ("B2", text("a")),
            ("B3", text("c")),
            ("B4", text("b")),
        ],
    );
    check_at(
        &book,
        "D1",
        &[
            // The first cell of the result, which is what a scalar context sees.
            ("INDEX(UNIQUE(A1:A4),1)", "3"),
            ("INDEX(UNIQUE(A1:A4),2)", "1"),
            ("COUNT(UNIQUE(A1:A4))", "3"),
            ("INDEX(SORT(A1:A4),1)", "1"),
            ("INDEX(SORT(A1:A4,1,-1),1)", "3"),
            ("COUNT(FILTER(A1:A4,A1:A4>1))", "3"),
            ("SUM(FILTER(A1:A4,A1:A4>1))", "8"),
            ("SUM(SEQUENCE(3))", "6"),
            ("SUM(SEQUENCE(2,3,10,10))", "210"),
            ("XLOOKUP(2,A1:A4,B1:B4)", "\"b\""),
            ("XLOOKUP(9,A1:A4,B1:B4,\"none\")", "\"none\""),
            // Nearest smaller rather than "the last one passed": the array is
            // not sorted, so a scan that assumed it would answer wrongly.
            ("XLOOKUP(2.5,A1:A4,B1:B4,,-1)", "\"b\""),
            ("XMATCH(1,A1:A4)", "2"),
            ("SUM(TAKE(A1:A4,2))", "4"),
            ("SUM(DROP(A1:A4,2))", "5"),
            ("SUM(TAKE(A1:A4,-1))", "2"),
            ("SUM(VSTACK(A1:A4,A1:A4))", "18"),
        ],
    );
}

#[test]
fn splitting_and_slicing_text() {
    check(&[
        (r#"INDEX(TEXTSPLIT("a,b,c",","),1,2)"#, "\"b\""),
        (r#"TEXTBEFORE("report.final.xlsx",".")"#, "\"report\""),
        (r#"TEXTAFTER("report.final.xlsx",".")"#, "\"final.xlsx\""),
        // A negative occurrence counts from the end, which is how the
        // extension comes off a name with more than one dot in it.
        (r#"TEXTAFTER("report.final.xlsx",".",-1)"#, "\"xlsx\""),
        (
            r#"TEXTBEFORE("report.final.xlsx",".",-1)"#,
            "\"report.final\"",
        ),
        (r#"TEXTAFTER("nothing","!")"#, "#N/A"),
    ]);
}
