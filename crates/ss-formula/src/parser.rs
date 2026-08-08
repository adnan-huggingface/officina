//! Pratt parser for Excel formulas.
//!
//! Precedence follows Excel's own table, lowest binding first:
//!
//! ```text
//!   = <> < > <= >=      comparison
//!   &                   concatenation
//!   + -                 addition
//!   * /                 multiplication
//!   ^                   exponentiation
//!   %                   postfix percent
//!   unary -             negation
//!   : (space) ,         reference operators
//! ```
//!
//! Two places differ from the textbook shape. Unary minus binds *tighter* than
//! `^`, so `-2^2` is 4 in Excel and −4 almost everywhere else. And the reference
//! operators bind tightest of all, which is why `SUM(A1:A9 B1:B9)` intersects
//! before summing.

use crate::ast::{Area, BinaryOp, Expr, Reference, SheetRef, UnaryOp};
use crate::error::{ParseError, ParseErrorKind};
use crate::lexer::{tokenize, Tok, Token};

/// Cap on expression nesting.
///
/// Generated formulas reach surprising depths, but not this. The limit exists so
/// a pathological input fails as an error rather than a stack overflow, which on
/// most platforms is not a catchable condition.
const MAX_DEPTH: u32 = 128;

/// Parses formula text, which must not include the leading `=`.
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(input)?;
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
        end: input.len(),
        depth: 0,
    };
    let expr = p.expr(0)?;
    if p.pos < p.tokens.len() {
        return Err(p.err_here(ParseErrorKind::TrailingInput));
    }
    Ok(expr)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Offset just past the input, for errors raised at end of formula.
    end: usize,
    depth: u32,
}

/// Binding powers. Left < right makes an operator left-associative.
fn infix_power(tok: &Tok) -> Option<(u8, u8)> {
    Some(match tok {
        Tok::Eq | Tok::Ne | Tok::Lt | Tok::Gt | Tok::Le | Tok::Ge => (1, 2),
        Tok::Amp => (3, 4),
        Tok::Plus | Tok::Minus => (5, 6),
        Tok::Star | Tok::Slash => (7, 8),
        // Right-associative: 2^3^2 is 2^(3^2).
        Tok::Caret => (10, 9),
        Tok::Colon => (17, 18),
        _ => return None,
    })
}

fn binary_op(tok: &Tok) -> BinaryOp {
    match tok {
        Tok::Eq => BinaryOp::Eq,
        Tok::Ne => BinaryOp::Ne,
        Tok::Lt => BinaryOp::Lt,
        Tok::Gt => BinaryOp::Gt,
        Tok::Le => BinaryOp::Le,
        Tok::Ge => BinaryOp::Ge,
        Tok::Amp => BinaryOp::Concat,
        Tok::Plus => BinaryOp::Add,
        Tok::Minus => BinaryOp::Sub,
        Tok::Star => BinaryOp::Mul,
        Tok::Slash => BinaryOp::Div,
        Tok::Caret => BinaryOp::Pow,
        Tok::Colon => BinaryOp::Range,
        other => unreachable!("{other:?} is not an infix operator"),
    }
}

/// Binding power of the implicit intersection operator (a space).
const INTERSECT_POWER: (u8, u8) = (15, 16);
/// Unary minus, which in Excel binds tighter than `^`.
const UNARY_POWER: u8 = 11;
/// Postfix `%`.
const PERCENT_POWER: u8 = 13;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_token(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, want: &Tok) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn offset_here(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.end, |t| t.at)
    }

    fn err_here(&self, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            at: self.offset_here(),
        }
    }

    fn expr(&mut self, min_power: u8) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err_here(ParseErrorKind::TooDeep));
        }
        let result = self.expr_inner(min_power);
        self.depth -= 1;
        result
    }

    fn expr_inner(&mut self, min_power: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.prefix()?;

        while let Some(tok) = self.peek() {
            // Postfix percent, before the infix table: it takes no right operand.
            if matches!(tok, Tok::Percent) {
                if PERCENT_POWER < min_power {
                    break;
                }
                self.pos += 1;
                lhs = Expr::Unary {
                    op: UnaryOp::Percent,
                    operand: Box::new(lhs),
                };
                continue;
            }

            if let Some((left, right)) = infix_power(tok) {
                if left < min_power {
                    break;
                }
                let op = binary_op(tok);
                self.pos += 1;
                let rhs = self.expr(right)?;
                lhs = Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                continue;
            }

            // Implicit intersection: two operands separated only by a space.
            if self.starts_operand(tok) && self.peek_token().is_some_and(|t| t.space_before) {
                let (left, right) = INTERSECT_POWER;
                if left < min_power {
                    break;
                }
                let rhs = self.expr(right)?;
                lhs = Expr::Binary {
                    op: BinaryOp::Intersect,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                continue;
            }

            break;
        }

        Ok(lhs)
    }

    /// Whether a token could begin an operand, used to spot implicit intersection.
    fn starts_operand(&self, tok: &Tok) -> bool {
        matches!(
            tok,
            Tok::Cell(_)
                | Tok::ColSpan { .. }
                | Tok::RowSpan { .. }
                | Tok::Sheet(_)
                | Tok::Name(_)
                | Tok::LParen
        )
    }

    fn prefix(&mut self) -> Result<Expr, ParseError> {
        let Some(token) = self.peek_token() else {
            return Err(self.err_here(ParseErrorKind::UnexpectedEnd));
        };

        match &token.kind {
            Tok::Number(n) => {
                let n = *n;
                self.pos += 1;
                Ok(Expr::Number(n))
            }
            Tok::Text(s) => {
                let s = s.clone();
                self.pos += 1;
                Ok(Expr::Text(s))
            }
            Tok::Bool(b) => {
                let b = *b;
                self.pos += 1;
                Ok(Expr::Bool(b))
            }
            Tok::Error(e) => {
                let e = *e;
                self.pos += 1;
                Ok(Expr::Error(e))
            }
            Tok::Minus => {
                self.pos += 1;
                let operand = self.expr(UNARY_POWER)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            Tok::Plus => {
                self.pos += 1;
                let operand = self.expr(UNARY_POWER)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Plus,
                    operand: Box::new(operand),
                })
            }
            Tok::LParen => {
                self.pos += 1;
                // A parenthesised list of references is a union: `(A1:A9,C1:C9)`.
                let mut expr = self.expr(0)?;
                while self.eat(&Tok::Comma) {
                    let rhs = self.expr(0)?;
                    expr = Expr::Binary {
                        op: BinaryOp::Union,
                        lhs: Box::new(expr),
                        rhs: Box::new(rhs),
                    };
                }
                if !self.eat(&Tok::RParen) {
                    return Err(self.err_here(ParseErrorKind::ExpectedCloseParen));
                }
                Ok(expr)
            }
            Tok::LBrace => self.array_literal(),
            Tok::Cell(_) | Tok::Name(_) => {
                // `Sheet1:Sheet3!A1` starts with what looks like a name or a
                // cell; only the token two ahead reveals it is a sheet span.
                match self.try_sheet_span() {
                    Some(sheet) => self.reference_or_name(sheet),
                    None => self.reference_or_name(SheetRef::Current),
                }
            }
            Tok::ColSpan { .. } | Tok::RowSpan { .. } => self.reference_or_name(SheetRef::Current),
            Tok::Sheet(_) => self.sheet_qualified(),
            _ => Err(self.err_here(ParseErrorKind::ExpectedExpression)),
        }
    }

    /// `Sheet1!A1`, `'Q1'!Name`, or the quoted 3-D form `'S1:S3'!A1`.
    fn sheet_qualified(&mut self) -> Result<Expr, ParseError> {
        let Some(Token {
            kind: Tok::Sheet(name),
            ..
        }) = self.bump()
        else {
            unreachable!("caller checked for a sheet token");
        };

        // A quoted 3-D reference arrives as one token: `'Sheet1:Sheet3'!A1`.
        // Splitting is unambiguous because Excel forbids `:` in a sheet name.
        let sheet = match name.split_once(':') {
            Some((a, b)) if !a.is_empty() && !b.is_empty() => {
                SheetRef::Span(a.to_owned(), b.to_owned())
            }
            _ => SheetRef::Named(name.clone()),
        };

        self.reference_or_name(sheet)
    }

    /// The unquoted 3-D form, `Sheet1:Sheet3!A1`.
    ///
    /// Only the *second* name carries the `!`, so the lexer cannot see the first
    /// as a sheet qualifier — it arrives as a name, or as a cell reference when
    /// the sheet is called something like `S1`. Both are recovered here.
    fn try_sheet_span(&mut self) -> Option<SheetRef> {
        if self.tokens.get(self.pos + 1).map(|t| &t.kind) != Some(&Tok::Colon) {
            return None;
        }
        let Some(Tok::Sheet(last)) = self.tokens.get(self.pos + 2).map(|t| &t.kind) else {
            return None;
        };
        let first = match &self.tokens.get(self.pos)?.kind {
            Tok::Name(n) => n.clone(),
            // A sheet named like a cell address. The spelling is recoverable
            // because that is exactly what the lexer matched.
            Tok::Cell(a) => format!("{}{}", ss_model::column_name(a.col), a.row + 1),
            _ => return None,
        };
        let span = SheetRef::Span(first, last.clone());
        self.pos += 3;
        Some(span)
    }

    /// A reference or a defined name, both of which may carry a sheet qualifier.
    fn reference_or_name(&mut self, sheet: SheetRef) -> Result<Expr, ParseError> {
        let Some(token) = self.bump() else {
            return Err(self.err_here(ParseErrorKind::UnexpectedEnd));
        };

        match &token.kind {
            Tok::Cell(a) => {
                // `A1:B2` is one reference, not two joined by an operator — the
                // dependency graph wants the rectangle, not a pair of cells.
                if self.peek() == Some(&Tok::Colon) {
                    if let Some(Tok::Cell(b)) = self.tokens.get(self.pos + 1).map(|t| &t.kind) {
                        let area = Area::Range { start: *a, end: *b };
                        self.pos += 2;
                        return Ok(Expr::Ref(Reference { sheet, area }));
                    }
                }
                Ok(Expr::Ref(Reference {
                    sheet,
                    area: Area::Cell(*a),
                }))
            }
            Tok::ColSpan {
                start,
                start_abs,
                end,
                end_abs,
            } => Ok(Expr::Ref(Reference {
                sheet,
                area: Area::Cols {
                    start: *start,
                    start_abs: *start_abs,
                    end: *end,
                    end_abs: *end_abs,
                },
            })),
            Tok::RowSpan {
                start,
                start_abs,
                end,
                end_abs,
            } => Ok(Expr::Ref(Reference {
                sheet,
                area: Area::Rows {
                    start: *start,
                    start_abs: *start_abs,
                    end: *end,
                    end_abs: *end_abs,
                },
            })),
            Tok::Name(name) => {
                let name = name.clone();
                if self.eat(&Tok::LParen) {
                    let args = self.call_args()?;
                    return Ok(Expr::Call { name, args });
                }
                Ok(Expr::Name { sheet, name })
            }
            _ => Err(ParseError {
                kind: ParseErrorKind::ExpectedExpression,
                at: token.at,
            }),
        }
    }

    /// Arguments up to and including the closing `)`.
    fn call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            // `IF(A1,,0)` — an argument deliberately left blank.
            if matches!(self.peek(), Some(Tok::Comma) | Some(Tok::RParen)) {
                args.push(Expr::Missing);
            } else {
                args.push(self.expr(0)?);
            }
            if self.eat(&Tok::Comma) {
                continue;
            }
            if self.eat(&Tok::RParen) {
                return Ok(args);
            }
            return Err(self.err_here(ParseErrorKind::ExpectedCloseParen));
        }
    }

    /// `{1,2;3,4}` — comma separates columns, semicolon separates rows.
    fn array_literal(&mut self) -> Result<Expr, ParseError> {
        self.pos += 1; // `{`
        let mut rows: Vec<Vec<Expr>> = Vec::new();
        let mut row: Vec<Expr> = Vec::new();

        if self.eat(&Tok::RBrace) {
            return Ok(Expr::Array(Vec::new()));
        }

        loop {
            row.push(self.expr(0)?);
            if self.eat(&Tok::Comma) {
                continue;
            }
            if self.eat(&Tok::Semicolon) {
                rows.push(std::mem::take(&mut row));
                continue;
            }
            if self.eat(&Tok::RBrace) {
                rows.push(row);
                return Ok(Expr::Array(rows));
            }
            return Err(self.err_here(ParseErrorKind::ExpectedCloseBrace));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::CellError;

    fn p(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
    }

    /// Renders the tree in prefix form, so precedence is visible in a test.
    fn shape(e: &Expr) -> String {
        match e {
            Expr::Number(n) => format!("{n}"),
            Expr::Text(s) => format!("{s:?}"),
            Expr::Bool(b) => b.to_string(),
            Expr::Error(err) => err.as_str().to_owned(),
            Expr::Missing => "_".into(),
            Expr::Ref(r) => match &r.area {
                Area::Cell(a) => format!("{}{}", ss_model::column_name(a.col), a.row + 1),
                Area::Range { start, end } => format!(
                    "{}{}:{}{}",
                    ss_model::column_name(start.col),
                    start.row + 1,
                    ss_model::column_name(end.col),
                    end.row + 1
                ),
                Area::Cols { start, end, .. } => format!(
                    "{}:{}",
                    ss_model::column_name(*start),
                    ss_model::column_name(*end)
                ),
                Area::Rows { start, end, .. } => format!("{}:{}", start + 1, end + 1),
            },
            Expr::Name { name, .. } => name.clone(),
            Expr::Call { name, args } => {
                let inner: Vec<String> = args.iter().map(shape).collect();
                format!("{name}({})", inner.join(" "))
            }
            Expr::Unary { op, operand } => {
                let sym = match op {
                    UnaryOp::Neg => "neg",
                    UnaryOp::Plus => "pos",
                    UnaryOp::Percent => "pct",
                };
                format!("({sym} {})", shape(operand))
            }
            Expr::Binary { op, lhs, rhs } => {
                let sym = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Pow => "^",
                    BinaryOp::Concat => "&",
                    BinaryOp::Eq => "=",
                    BinaryOp::Ne => "<>",
                    BinaryOp::Lt => "<",
                    BinaryOp::Gt => ">",
                    BinaryOp::Le => "<=",
                    BinaryOp::Ge => ">=",
                    BinaryOp::Range => ":",
                    BinaryOp::Intersect => "isect",
                    BinaryOp::Union => "union",
                };
                format!("({sym} {} {})", shape(lhs), shape(rhs))
            }
            Expr::Array(rows) => {
                let r: Vec<String> = rows
                    .iter()
                    .map(|row| {
                        let c: Vec<String> = row.iter().map(shape).collect();
                        c.join(",")
                    })
                    .collect();
                format!("{{{}}}", r.join(";"))
            }
        }
    }

    #[test]
    fn arithmetic_precedence() {
        assert_eq!(shape(&p("1+2*3")), "(+ 1 (* 2 3))");
        assert_eq!(shape(&p("(1+2)*3")), "(* (+ 1 2) 3)");
        assert_eq!(shape(&p("1+2-3")), "(- (+ 1 2) 3)", "left associative");
        assert_eq!(shape(&p("2^3^2")), "(^ 2 (^ 3 2))", "right associative");
    }

    #[test]
    fn unary_minus_binds_tighter_than_the_caret() {
        // Excel evaluates -2^2 as 4. Nearly every other language says -4, and
        // getting this backwards silently changes results rather than erroring.
        assert_eq!(shape(&p("-2^2")), "(^ (neg 2) 2)");
        assert_eq!(shape(&p("2^-1")), "(^ 2 (neg 1))");
    }

    #[test]
    fn comparison_binds_loosest() {
        assert_eq!(shape(&p("1+2=3")), "(= (+ 1 2) 3)");
        assert_eq!(shape(&p("A1&B1=\"x\"")), r#"(= (& A1 B1) "x")"#);
    }

    #[test]
    fn concatenation_sits_between_comparison_and_addition() {
        assert_eq!(shape(&p("1&2+3")), "(& 1 (+ 2 3))");
    }

    #[test]
    fn postfix_percent() {
        assert_eq!(shape(&p("50%")), "(pct 50)");
        assert_eq!(shape(&p("1+50%")), "(+ 1 (pct 50))");
        assert_eq!(shape(&p("-50%")), "(neg (pct 50))");
    }

    #[test]
    fn ranges_parse_as_one_reference_not_two() {
        // The dependency graph needs the rectangle. A pair of cells joined by an
        // operator would make every range look like two single-cell dependencies.
        assert_eq!(shape(&p("A1:B2")), "A1:B2");
        assert_eq!(shape(&p("SUM(A1:A9)")), "SUM(A1:A9)");
    }

    #[test]
    fn whole_column_and_row_references() {
        assert_eq!(shape(&p("SUM(A:C)")), "SUM(A:C)");
        assert_eq!(shape(&p("SUM(2:5)")), "SUM(2:5)");
    }

    #[test]
    fn function_calls_with_mixed_arguments() {
        assert_eq!(
            shape(&p(r#"IF(A1>0,"yes","no")"#)),
            r#"IF((> A1 0) "yes" "no")"#
        );
        assert_eq!(shape(&p("SUM()")), "SUM()");
        assert_eq!(shape(&p("NOW()")), "NOW()");
    }

    #[test]
    fn an_omitted_argument_is_kept_as_missing() {
        // `IF(A1,,0)` returns 0, not FALSE — the blank is meaningful.
        assert_eq!(shape(&p("IF(A1,,0)")), "IF(A1 _ 0)");
        assert_eq!(shape(&p("SUM(1,)")), "SUM(1 _)");
    }

    #[test]
    fn nested_calls() {
        assert_eq!(
            shape(&p("SUM(MAX(A1:A9),MIN(B1:B9))")),
            "SUM(MAX(A1:A9) MIN(B1:B9))"
        );
    }

    #[test]
    fn sheet_qualified_references() {
        let e = p("Sheet1!A1");
        match &e {
            Expr::Ref(r) => assert_eq!(r.sheet, SheetRef::Named("Sheet1".into())),
            other => panic!("expected a reference, got {other:?}"),
        }

        let e = p("'Q1 Results'!A1:B2");
        match &e {
            Expr::Ref(r) => {
                assert_eq!(r.sheet, SheetRef::Named("Q1 Results".into()));
                assert!(matches!(r.area, Area::Range { .. }));
            }
            other => panic!("expected a reference, got {other:?}"),
        }
    }

    #[test]
    fn three_dimensional_sheet_spans() {
        let e = p("Sheet1:Sheet3!A1");
        match &e {
            Expr::Ref(r) => assert_eq!(
                r.sheet,
                SheetRef::Span("Sheet1".into(), "Sheet3".into()),
                "a 3-D reference covers every sheet between the two"
            ),
            other => panic!("expected a reference, got {other:?}"),
        }
    }

    #[test]
    fn defined_names_and_functions_are_told_apart_by_the_paren() {
        assert!(matches!(p("TaxRate"), Expr::Name { .. }));
        assert!(matches!(p("TaxRate()"), Expr::Call { .. }));
    }

    #[test]
    fn implicit_intersection_by_space() {
        assert_eq!(shape(&p("A1:B5 B1:C9")), "(isect A1:B5 B1:C9)");
        // A space inside a call's arguments still intersects.
        assert_eq!(shape(&p("SUM(A1:A9 A5:B9)")), "SUM((isect A1:A9 A5:B9))");
    }

    #[test]
    fn a_space_around_an_operator_is_not_an_intersection() {
        assert_eq!(shape(&p("1 + 2")), "(+ 1 2)");
        assert_eq!(shape(&p("A1 * B1")), "(* A1 B1)");
    }

    #[test]
    fn parenthesised_reference_lists_are_unions() {
        assert_eq!(shape(&p("SUM((A1:A9,C1:C9))")), "SUM((union A1:A9 C1:C9))");
    }

    #[test]
    fn array_literals() {
        assert_eq!(shape(&p("{1,2;3,4}")), "{1,2;3,4}");
        assert_eq!(shape(&p("{1}")), "{1}");
        assert_eq!(shape(&p(r#"{"a","b"}"#)), r#"{"a","b"}"#);
    }

    #[test]
    fn error_literals_are_expressions() {
        assert_eq!(shape(&p("#REF!")), "#REF!");
        assert_eq!(shape(&p("IFERROR(A1,#N/A)")), "IFERROR(A1 #N/A)");
        assert!(matches!(p("#DIV/0!"), Expr::Error(CellError::Div0)));
    }

    #[test]
    fn absoluteness_survives_parsing() {
        let e = p("$A$1");
        match &e {
            Expr::Ref(Reference {
                area: Area::Cell(a),
                ..
            }) => {
                assert!(a.col_abs && a.row_abs);
            }
            other => panic!("expected an absolute cell, got {other:?}"),
        }
    }

    #[test]
    fn malformed_formulas_are_errors_not_panics() {
        for bad in [
            "1+",
            "(1",
            "SUM(",
            ")",
            "*",
            "",
            "{1,",
            "1 2 3 @",
            "'unclosed!A1",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn deep_nesting_is_rejected_rather_than_overflowing_the_stack() {
        let deep = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        match parse(&deep) {
            Err(e) => assert_eq!(e.kind, ParseErrorKind::TooDeep),
            Ok(_) => panic!("500 levels should exceed the depth cap"),
        }
    }

    #[test]
    fn realistic_formulas_round_the_course() {
        for f in [
            "SUM(A1:A100)/COUNT(A1:A100)",
            r#"IF(ISBLANK(A1),"",VLOOKUP(A1,Data!$A$1:$C$99,3,FALSE))"#,
            "SUMPRODUCT((A1:A9=\"x\")*(B1:B9))",
            "-B2*(1+Rate)^Years",
            "TEXT(TODAY(),\"yyyy-mm-dd\")",
            "IFERROR(A1/B1,0)",
            "COUNTIF(Data!A:A,\">25\")",
            "'Q1 Results'!A1*2",
        ] {
            parse(f).unwrap_or_else(|e| panic!("{f:?} should parse: {e}"));
        }
    }
}
