//! The parsed shape of a formula.

use ss_model::CellError;

use crate::lexer::A1;

/// Which sheet a reference names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetRef {
    /// Unqualified: the sheet the formula lives on.
    Current,
    /// `Sheet1!A1`.
    Named(String),
    /// `Sheet1:Sheet3!A1` — a 3-D reference spanning consecutive sheets.
    Span(String, String),
}

/// The area a reference covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Area {
    Cell(A1),
    Range {
        start: A1,
        end: A1,
    },
    /// `A:C`. Rows are every row, so only the column bounds are stored.
    Cols {
        start: u32,
        start_abs: bool,
        end: u32,
        end_abs: bool,
    },
    /// `2:5`.
    Rows {
        start: u32,
        start_abs: bool,
        end: u32,
        end_abs: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub sheet: SheetRef,
    pub area: Area,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Plus,
    /// Postfix `%`, which divides by 100.
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `:` — the smallest rectangle containing both operands.
    Range,
    /// A space — the overlap of both operands.
    Intersect,
    /// `,` between references — both, as one multi-area reference.
    Union,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
    Ref(Reference),
    /// A defined name, optionally sheet-scoped.
    Name {
        sheet: SheetRef,
        name: String,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `{1,2;3,4}` — rows of constants.
    Array(Vec<Vec<Expr>>),
    /// An argument left empty, as in `IF(A1,,0)`.
    ///
    /// Distinct from a missing argument: Excel treats an omitted middle argument
    /// as an explicit blank, and collapsing the two changes what functions return.
    Missing,
}

impl Expr {
    /// Visits every sub-expression, outermost first.
    pub fn walk(&self, f: &mut impl FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Call { args, .. } => {
                for a in args {
                    a.walk(f);
                }
            }
            Expr::Unary { operand, .. } => operand.walk(f),
            Expr::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            Expr::Array(rows) => {
                for row in rows {
                    for e in row {
                        e.walk(f);
                    }
                }
            }
            _ => {}
        }
    }
}
