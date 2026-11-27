//! Core JavaScript AST types, operator precedence, loop views, and structurizer payload impl.

use std::fmt;

use super::print::{expr_to_string, stmt_to_string};

/// Precedence levels, from loosest to tightest binding.
///
/// The numbers only matter relative to each other. An expression is parenthesized when its
/// level is lower than the level its parent requires in that position.
pub(super) const PREC_SEQ: u8 = 1;
pub(super) const PREC_ASSIGN: u8 = 2;
pub(super) const PREC_COND: u8 = 3;
pub(super) const PREC_OR: u8 = 5;
pub(super) const PREC_AND: u8 = 6;
pub(super) const PREC_BIT_OR: u8 = 7;
pub(super) const PREC_BIT_XOR: u8 = 8;
pub(super) const PREC_BIT_AND: u8 = 9;
pub(super) const PREC_EQ: u8 = 10;
pub(super) const PREC_REL: u8 = 11;
pub(super) const PREC_SHIFT: u8 = 12;
pub(super) const PREC_ADD: u8 = 13;
pub(super) const PREC_MUL: u8 = 14;
pub(super) const PREC_UNARY: u8 = 16;
pub(super) const PREC_CALL: u8 = 18;
pub(super) const PREC_PRIMARY: u8 = 20;

/// A prefix operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `+x`
    Pos,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `typeof x`
    TypeOf,
    /// `void x`
    Void,
}

impl UnOp {
    /// The operator token, including a trailing space for word operators.
    pub fn symbol(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Pos => "+",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
            UnOp::TypeOf => "typeof ",
            UnOp::Void => "void ",
        }
    }
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol().trim_end())
    }
}

/// An infix operator. Every operator here is left associative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

impl BinOp {
    /// The operator token.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::StrictEq => "===",
            BinOp::StrictNe => "!==",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::UShr => ">>>",
        }
    }

    /// The precedence level of the operator.
    pub fn prec(self) -> u8 {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Rem => PREC_MUL,
            BinOp::Add | BinOp::Sub => PREC_ADD,
            BinOp::Shl | BinOp::Shr | BinOp::UShr => PREC_SHIFT,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => PREC_REL,
            BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe => PREC_EQ,
            BinOp::BitAnd => PREC_BIT_AND,
            BinOp::BitXor => PREC_BIT_XOR,
            BinOp::BitOr => PREC_BIT_OR,
            BinOp::And => PREC_AND,
            BinOp::Or => PREC_OR,
        }
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// An operator usable in a compound assignment (`target op= value`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    And,
    Or,
}

impl CompoundOp {
    /// The operator token written before `=`.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::UShr => ">>>",
            Self::And => "&&",
            Self::Or => "||",
        }
    }

    /// The corresponding binary operator.
    pub const fn to_bin_op(self) -> BinOp {
        match self {
            Self::Add => BinOp::Add,
            Self::Sub => BinOp::Sub,
            Self::Mul => BinOp::Mul,
            Self::Div => BinOp::Div,
            Self::Rem => BinOp::Rem,
            Self::BitAnd => BinOp::BitAnd,
            Self::BitOr => BinOp::BitOr,
            Self::BitXor => BinOp::BitXor,
            Self::Shl => BinOp::Shl,
            Self::Shr => BinOp::Shr,
            Self::UShr => BinOp::UShr,
            Self::And => BinOp::And,
            Self::Or => BinOp::Or,
        }
    }
}

impl From<CompoundOp> for BinOp {
    fn from(op: CompoundOp) -> BinOp {
        op.to_bin_op()
    }
}

impl TryFrom<BinOp> for CompoundOp {
    type Error = ();

    fn try_from(op: BinOp) -> Result<Self, ()> {
        Ok(match op {
            BinOp::Add => Self::Add,
            BinOp::Sub => Self::Sub,
            BinOp::Mul => Self::Mul,
            BinOp::Div => Self::Div,
            BinOp::Rem => Self::Rem,
            BinOp::BitAnd => Self::BitAnd,
            BinOp::BitOr => Self::BitOr,
            BinOp::BitXor => Self::BitXor,
            BinOp::Shl => Self::Shl,
            BinOp::Shr => Self::Shr,
            BinOp::UShr => Self::UShr,
            BinOp::And => Self::And,
            BinOp::Or => Self::Or,
            _ => return Err(()),
        })
    }
}

impl fmt::Display for CompoundOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// The body of an arrow function.
#[derive(Clone, Debug, PartialEq)]
pub enum ArrowBody {
    /// A single expression, printed on one line.
    Expr(Box<Expr>),
    /// A statement block, printed over several lines.
    Block(Vec<Stmt>),
}

/// A JavaScript expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A number literal, formatted with the shortest representation that round trips.
    Num(f64),
    /// A number literal emitted verbatim, for values that need an exact spelling.
    RawNum(String),
    /// A `BigInt` literal: the decimal digits, printed with the trailing `n`. Holds text rather
    /// than an integer because 128 bit literals have to survive unrounded.
    BigInt(String),
    /// A string literal. The printer adds the quotes and the escapes.
    Str(String),
    /// `true` or `false`.
    Bool(bool),
    /// `undefined`.
    Undefined,
    /// `null`.
    Null,
    /// An identifier, emitted verbatim. Use [`super::sanitize_ident`] on names from Rust.
    Ident(String),
    /// `obj.name`, or `obj["name"]` when the name is not a plain identifier.
    Member(Box<Expr>, String),
    /// `obj[index]`.
    Index(Box<Expr>, Box<Expr>),
    /// `callee(args...)`.
    Call(Box<Expr>, Vec<Expr>),
    /// `new callee(args...)`.
    New(Box<Expr>, Vec<Expr>),
    /// A prefix operator application.
    Unary(UnOp, Box<Expr>),
    /// An infix operator application.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `target = value`.
    Assign(Box<Expr>, Box<Expr>),
    /// `target op= value`.
    CompoundAssign(CompoundOp, Box<Expr>, Box<Expr>),
    /// `test ? consequent : alternate`.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// An object literal. Keys are quoted only when they are not plain identifiers.
    Object(Vec<(String, Expr)>),
    /// An array literal.
    Array(Vec<Expr>),
    /// An arrow function.
    Arrow(Vec<String>, ArrowBody),
    /// A comma expression.
    Seq(Vec<Expr>),
    /// Verbatim JavaScript. Parenthesized whenever it is nested in another expression.
    Raw(String),
}

impl Expr {
    /// The precedence level of the expression.
    pub fn prec(&self) -> u8 {
        match self {
            Expr::Num(n) => {
                // A negative literal parses as a unary expression, so it binds like one.
                if n.is_sign_negative() && !n.is_nan() {
                    PREC_UNARY
                } else {
                    PREC_PRIMARY
                }
            }
            // A negative literal parses as a unary expression, so it binds like one.
            Expr::RawNum(s) | Expr::BigInt(s) => {
                if s.starts_with('-') || s.starts_with('+') {
                    PREC_UNARY
                } else {
                    PREC_PRIMARY
                }
            }
            Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Ident(_)
            | Expr::Object(_)
            | Expr::Array(_) => PREC_PRIMARY,
            Expr::Member(..) | Expr::Index(..) | Expr::Call(..) | Expr::New(..) => PREC_CALL,
            Expr::Unary(..) => PREC_UNARY,
            Expr::Binary(op, ..) => op.prec(),
            Expr::Assign(..) | Expr::CompoundAssign(..) | Expr::Arrow(..) => PREC_ASSIGN,
            Expr::Cond(..) => PREC_COND,
            Expr::Seq(_) => PREC_SEQ,
            Expr::Raw(_) => PREC_SEQ,
        }
    }

    /// Whether a `new` callee contains a call, which would swallow the argument list.
    pub(super) fn head_is_call(&self) -> bool {
        match self {
            Expr::Call(..) => true,
            Expr::Member(obj, _) => obj.head_is_call(),
            Expr::Index(obj, _) => obj.head_is_call(),
            _ => false,
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&expr_to_string(self))
    }
}

/// One `case` group of a [`Stmt::Switch`].
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchCase {
    /// The case labels. An empty list is the `default` group.
    pub tests: Vec<Expr>,
    /// The statements of the group, printed without an enclosing block.
    pub body: Vec<Stmt>,
}

/// A position in a Rust source file: where a run of generated JavaScript came from.
///
/// The file is spelled as rustc spells it in a diagnostic, so `--remap-path-prefix` applies and a
/// map does not depend on where the checkout lives. `line` is 1 based and `col` is 0 based, which
/// is how a source map's decoded form reads; the encoder makes the line 0 based.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loc {
    pub file: String,
    pub line: u32,
    pub col: u32,
}

/// One source map entry: a position in the printed JavaScript and the [`Loc`] it came from.
///
/// `gen_line` and `gen_col` are 0 based, and the column counts UTF-16 code units — the unit the
/// source map format is defined in, and the one a browser's own view of the file uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mapping {
    pub gen_line: u32,
    pub gen_col: u32,
    pub loc: Loc,
    /// The original Rust name of the identifier printed here, for the map's `names` table.
    pub name: Option<String>,
}

/// A JavaScript statement.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::enum_variant_names,
    reason = "ExprStmt mirrors JS grammar terminology"
)]
pub enum Stmt {
    /// An expression evaluated for its effect.
    ExprStmt(Expr),
    /// `let name;` or `let name = init;`.
    Let(String, Option<Expr>),
    /// `let a = 1, b, c = 3;`.
    Lets(Vec<(String, Option<Expr>)>),
    /// `const name = init;`.
    Const(String, Expr),
    /// `return;` or `return value;`.
    Return(Option<Expr>),
    /// `if (test) { .. }` with an optional `else` branch. An `else` branch holding a single
    /// [`Stmt::If`] is printed as an `else if` chain.
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    /// `switch (disc) { .. }`.
    Switch(Expr, Vec<SwitchCase>),
    /// `label: stmt`.
    Labeled(String, Box<Stmt>),
    /// `label: for (;;) { .. }`, the trampoline loop.
    LoopForever(Option<String>, Vec<Stmt>),
    /// `while (test) { .. }`.
    While(Expr, Vec<Stmt>),
    /// `continue;` or `continue label;`.
    Continue(Option<String>),
    /// `break;` or `break label;`.
    Break(Option<String>),
    /// `throw value;`.
    Throw(Expr),
    /// `try { body } catch (binding) { handler }`. Both bodies are lexical scopes.
    /// The catch binding receives the original thrown value, including primitives.
    TryCatch {
        body: Vec<Stmt>,
        binding: String,
        handler: Vec<Stmt>,
    },
    /// A top level function declaration.
    FunctionDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    /// `import * as ns from "source";` or `import { a, b as c } from "source";`.
    ///
    /// Only ES module output has one; see the "Modules" section of CONTRACT.md.
    Import {
        specifiers: ImportSpecifiers,
        source: String,
    },
    /// A bare block.
    Block(Vec<Stmt>),
    /// A `//` line comment. Newlines in the text are replaced with spaces.
    Comment(String),
    /// Verbatim JavaScript lines, re-indented to the current level.
    Raw(String),
    /// A source location marker. Prints **nothing**: it says where the statements that follow it
    /// came from, until the next marker.
    ///
    /// This is the whole of the source map's presence in the AST — deliberately a marker in the
    /// statement stream rather than a `loc` field on every variant or an `At(Loc, Box<Stmt>)`
    /// wrapper. A wrapper would have to be seen through by every `match` in `queue.rs`,
    /// `emit.rs` and this file's own printer; a marker is invisible to all of them, is `PartialEq`
    /// without special cases (so the 40-odd tests below needed no edit), and travels with its run
    /// of statements when the structurizer moves a block. The price is that a statement the
    /// expression queue moves *past* a marker keeps the marker's neighbour's location rather than
    /// its own, which costs a stepping stop, never correctness.
    ///
    /// `names.rs::Locs` is the only thing that emits one, and only under
    /// `-Cllvm-args=js-source-map=on`; with the option off the AST is exactly what it was.
    Loc(Loc),
}

/// Borrowed view of a loop statement (`LoopForever`, `While`, or a `Labeled` loop).
#[derive(Debug)]
pub struct LoopRef<'a> {
    pub label: Option<&'a str>,
    pub cond: Option<&'a Expr>,
    pub body: &'a [Stmt],
}

/// Mutably borrowed view of a loop statement (`LoopForever`, `While`, or a `Labeled` loop).
#[derive(Debug)]
pub struct LoopMut<'a> {
    pub label: Option<&'a str>,
    pub cond: Option<&'a mut Expr>,
    pub body: &'a mut Vec<Stmt>,
}

impl Stmt {
    /// Views `LoopForever`, `While`, and `Labeled(_, LoopForever | While)` uniformly.
    pub fn as_loop(&self) -> Option<LoopRef<'_>> {
        match self {
            Stmt::LoopForever(label, body) => Some(LoopRef {
                label: label.as_deref(),
                cond: None,
                body,
            }),
            Stmt::While(cond, body) => Some(LoopRef {
                label: None,
                cond: Some(cond),
                body,
            }),
            Stmt::Labeled(label, inner) => match inner.as_ref() {
                Stmt::LoopForever(inner_label, body) => Some(LoopRef {
                    label: inner_label.as_deref().or(Some(label.as_str())),
                    cond: None,
                    body,
                }),
                Stmt::While(cond, body) => Some(LoopRef {
                    label: Some(label.as_str()),
                    cond: Some(cond),
                    body,
                }),
                _ => None,
            },
            _ => None,
        }
    }

    /// Mutably views `LoopForever`, `While`, and `Labeled(_, LoopForever | While)` uniformly.
    pub fn as_loop_mut(&mut self) -> Option<LoopMut<'_>> {
        match self {
            Stmt::LoopForever(label, body) => Some(LoopMut {
                label: label.as_deref(),
                cond: None,
                body,
            }),
            Stmt::While(cond, body) => Some(LoopMut {
                label: None,
                cond: Some(cond),
                body,
            }),
            Stmt::Labeled(label, inner) => match inner.as_mut() {
                Stmt::LoopForever(inner_label, body) => Some(LoopMut {
                    label: inner_label.as_deref().or(Some(label.as_str())),
                    cond: None,
                    body,
                }),
                Stmt::While(cond, body) => Some(LoopMut {
                    label: Some(label.as_str()),
                    cond: Some(cond),
                    body,
                }),
                _ => None,
            },
            _ => None,
        }
    }
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&stmt_to_string(self))
    }
}

/// What an [`Stmt::Import`] binds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportSpecifiers {
    /// `* as name`: the module's namespace object under one name.
    Namespace(String),
    /// `{ exported as local, .. }`: one binding per exported name. The `as` clause is left out
    /// where the two names are equal, so `("insert", "insert")` prints as `insert`.
    Named(Vec<(String, String)>),
}

// One unit per statement: a block's cost is how many statements duplicating it would duplicate.
//
// Block payloads are straight line at this point — the only nested statement a lowering produces
// here is the `if` guarding an `Assert`'s panic, which is two or three statements' worth — so
// counting the top level is close enough for the one decision this feeds, whether a loop
// continuation is small enough to be duplicated into the loop.
impl structurizer::Payload for Stmt {
    fn cost(&self) -> i32 {
        1
    }
}
