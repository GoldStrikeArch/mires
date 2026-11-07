//! A small JavaScript AST and pretty printer.
//!
//! The module is self contained: it uses only `std`, so it can be included from the codegen
//! backend without pulling anything else in. Build an [`Expr`]/[`Stmt`] tree with the
//! convenience constructors at the bottom of the file and render it with
//! [`program_to_string`].
//!
//! The printer indents with two spaces and puts every statement on its own line, because the
//! spike output is read by humans. Parenthesization is driven by a precedence table and errs
//! on the side of extra parentheses, never missing ones.
//!
//! Notes for a MIR lowering pass:
//!
//! * Case bodies of a [`Stmt::Switch`] are printed flat, without an extra block per case, so
//!   the contract's `case N: ...; bb = M; continue L;` shape comes out verbatim. All cases of
//!   one switch therefore share a block scope: declare function locals once at the top of the
//!   function body (with [`let_uninit`]) and only assign inside the cases, otherwise two
//!   `let` declarations of the same name in different cases are a JavaScript syntax error.
//! * Arrows with an expression body print on one line. Prefer them for the accessor objects
//!   described by the contract; [`accessor`] builds one.
//! * [`Expr::Raw`] and [`Stmt::Raw`] are escape hatches. A raw expression is parenthesized
//!   whenever it appears inside another expression, since its precedence is unknown.

// The module offers a complete surface, of which any one consumer uses a part.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;

/// Precedence levels, from loosest to tightest binding.
///
/// The numbers only matter relative to each other. An expression is parenthesized when its
/// level is lower than the level its parent requires in that position.
const PREC_SEQ: u8 = 1;
const PREC_ASSIGN: u8 = 2;
const PREC_COND: u8 = 3;
const PREC_OR: u8 = 5;
const PREC_AND: u8 = 6;
const PREC_BIT_OR: u8 = 7;
const PREC_BIT_XOR: u8 = 8;
const PREC_BIT_AND: u8 = 9;
const PREC_EQ: u8 = 10;
const PREC_REL: u8 = 11;
const PREC_SHIFT: u8 = 12;
const PREC_ADD: u8 = 13;
const PREC_MUL: u8 = 14;
const PREC_UNARY: u8 = 16;
const PREC_CALL: u8 = 18;
const PREC_PRIMARY: u8 = 20;

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
    /// An identifier, emitted verbatim. Use [`sanitize_ident`] on names from Rust.
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
    CompoundAssign(BinOp, Box<Expr>, Box<Expr>),
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
    fn head_is_call(&self) -> bool {
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

/// How much whitespace the printer spends.
///
/// [`Style::Readable`] is the default and the only style anything but `-Cllvm-args=js-minify=on`
/// ever asks for; every rule below is conditional on the style, so readable output is byte for byte
/// what it was before compact printing existed.
///
/// [`Style::Compact`] drops the indentation, the newlines inside a declaration and the spaces that
/// only separate tokens, elides the semicolon before a `}`, drops the braces around a
/// single-statement `if`/`while` body that cannot be an `if` itself, and spells `true`, `false` and
/// `undefined` as `!0`, `!1` and `void 0`. Where two tokens would fuse — `a - -b`, `a + +b`, the
/// `//` of an accidental comment, the `<!` and `-->` of an HTML comment — a space goes back in;
/// [`need_space`] is the whole of that rule and is checked *after* the second token is written, on
/// the characters that actually met.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
    Readable,
    Compact,
}

/// Renders a list of top level statements as a JavaScript program.
///
/// Function declarations are spaced out from their neighbours, and a comment that introduces one
/// travels with it: the blank line goes *before* the comment, never between the comment and the
/// declaration it describes. Anything else would read as a comment about the code above it.
pub fn program_to_string(items: &[Stmt]) -> String {
    program_to_string_styled(items, Style::Readable)
}

/// Renders a program in the requested [`Style`].
pub fn program_to_string_styled(items: &[Stmt], style: Style) -> String {
    let mut printer = Printer::new(style);
    print_program(&mut printer, items);
    printer.out
}

/// Renders a program and the source map entries for it.
///
/// `names` maps an emitted JavaScript name to the Rust name it stands for; an entry is what turns
/// a declaration's mapping into a `Gen_Ori_Name` one, which is how a debugger shows `acc` where the
/// output says `acc$1`. `at` is the location in effect before the first [`Stmt::Loc`] marker — the
/// item's own span, so that its header line maps somewhere useful.
///
/// The generated positions come out resolved: the printer records byte offsets as it goes and this
/// converts them to line and UTF-16 column in one pass at the end, which is what keeps the sixty
/// odd `push_str` sites in the printer from having to count anything.
pub fn program_to_string_mapped(
    items: &[Stmt],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
) -> (String, Vec<Mapping>) {
    program_to_string_mapped_styled(items, names, at, Style::Readable)
}

/// Renders a program and its source map entries in the requested [`Style`].
///
/// A [`Style::Compact`] program still tracks positions, so the pair of options is *representable*;
/// `opts.rs` refuses it anyway, because the names in the map would be the ones `minify.rs` has
/// already renamed away.
pub fn program_to_string_mapped_styled(
    items: &[Stmt],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
    style: Style,
) -> (String, Vec<Mapping>) {
    let mut printer = Printer::new(style);
    printer.maps = Some(MapState { names, pending: at, current: None, named: false, raw: Vec::new() });
    print_program(&mut printer, items);
    let state = printer.maps.take().expect("just installed");
    let mappings = resolve_offsets(&printer.out, state.raw);
    (printer.out, mappings)
}

fn print_program(printer: &mut Printer, items: &[Stmt]) {
    for (i, item) in items.iter().enumerate() {
        // Compact output has no blank lines to give, but it does keep one newline per top level
        // declaration. Not for looks: an object file's `//# rcgjs:` footer has to start a line, and
        // a finished program is a concatenation of item texts that must not fuse at the seams.
        if printer.compact() {
            printer.stmt(item);
            if !printer.out.is_empty() && !printer.out.ends_with('\n') {
                printer.out.push('\n');
            }
            continue;
        }
        let starts_group = is_decl(item) || introduces_decl(items, i);
        let ends_group = i > 0 && is_decl(&items[i - 1]);
        // ... and never inside one: a comment and the declaration it introduces are one unit.
        let inside_group = i > 0 && introduces_decl(items, i - 1);
        if i > 0 && (starts_group || ends_group) && !inside_group {
            printer.out.push('\n');
        }
        printer.stmt(item);
    }
}

/// Whether item `i` is a comment (or a run of them) introducing a declaration.
fn introduces_decl(items: &[Stmt], i: usize) -> bool {
    if !matches!(items[i], Stmt::Comment(_)) {
        return false;
    }
    items[i + 1..]
        .iter()
        .find(|item| !matches!(item, Stmt::Comment(_)))
        .is_some_and(is_decl)
}

/// Renders a single statement, including its trailing newline.
pub fn stmt_to_string(stmt: &Stmt) -> String {
    stmt_to_string_styled(stmt, Style::Readable)
}

/// Renders a single statement in the requested [`Style`].
pub fn stmt_to_string_styled(stmt: &Stmt, style: Style) -> String {
    let mut printer = Printer::new(style);
    printer.stmt(stmt);
    printer.out
}

/// Renders a single expression.
pub fn expr_to_string(expr: &Expr) -> String {
    expr_to_string_styled(expr, Style::Readable)
}

/// Renders a single expression in the requested [`Style`].
pub fn expr_to_string_styled(expr: &Expr, style: Style) -> String {
    let mut printer = Printer::new(style);
    printer.expr(expr, 0);
    printer.out
}

/// Whether a statement is a top level declaration, which [`print_program`] spaces out.
fn is_decl(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::FunctionDecl { .. } | Stmt::Import { .. })
}

/// Calls `f` with every identifier a statement mentions, in source order.
///
/// Only [`Expr::Ident`] counts. Member names and object keys are `String`s in this AST rather than
/// expressions, so `a.length` and `{ get: ... }` are structurally excluded — an item can never be
/// mistaken for a property of the same name. That is what lets the backend collect a function's
/// outgoing references mechanically instead of recording them by hand at every call site.
///
/// Declared names — a `let`, a parameter, a function's own name — are *not* reported: nothing
/// depends on them, and reporting them would make every function look like it referenced itself.
/// [`Expr::Raw`] and [`Stmt::Raw`] are opaque text and contribute nothing; do not put a reference
/// to another item inside one.
pub fn visit_idents(stmt: &Stmt, f: &mut impl FnMut(&str)) {
    Visitor { f }.stmt(stmt);
}

struct Visitor<'a, F> {
    f: &'a mut F,
}

impl<F: FnMut(&str)> Visitor<'_, F> {
    fn stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) => self.expr(expr),
            Stmt::Let(_, init) => {
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(init) = init {
                        self.expr(init);
                    }
                }
            }
            Stmt::Const(_, init) => self.expr(init),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Stmt::If(test, then, els) => {
                self.expr(test);
                self.stmts(then);
                if let Some(els) = els {
                    self.stmts(els);
                }
            }
            Stmt::Switch(disc, cases) => {
                self.expr(disc);
                for case in cases {
                    for test in &case.tests {
                        self.expr(test);
                    }
                    self.stmts(&case.body);
                }
            }
            Stmt::Labeled(_, inner) => self.stmt(inner),
            Stmt::LoopForever(_, body) | Stmt::Block(body) => self.stmts(body),
            Stmt::While(test, body) => {
                self.expr(test);
                self.stmts(body);
            }
            Stmt::FunctionDecl { body, .. } => self.stmts(body),
            // An import *declares* its specifiers, exactly like a `let` or a parameter, so it
            // reports nothing: an import that reported its own bindings would be a reference to
            // itself, and the link step would root every import ever emitted.
            Stmt::Import { .. }
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name) => (self.f)(name),
            Expr::Member(obj, _) => self.expr(obj),
            Expr::Index(obj, index) => {
                self.expr(obj);
                self.expr(index);
            }
            Expr::Call(callee, args) | Expr::New(callee, args) => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::Unary(_, operand) => self.expr(operand),
            Expr::Binary(_, lhs, rhs)
            | Expr::Assign(lhs, rhs)
            | Expr::CompoundAssign(_, lhs, rhs) => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.expr(consequent);
                self.expr(alternate);
            }
            Expr::Object(entries) => {
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(_, body) => match body {
                ArrowBody::Expr(body) => self.expr(body),
                ArrowBody::Block(body) => self.stmts(body),
            },
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Raw(_) => {}
        }
    }
}

/// The source map half of the printer's state. Absent unless a map was asked for.
struct MapState {
    /// Emitted JavaScript name to the Rust name behind it. See [`program_to_string_mapped`].
    names: BTreeMap<String, String>,
    /// The location the next statement will be mapped to, set by a [`Stmt::Loc`] marker.
    pending: Option<Loc>,
    /// The location in effect, so that a run of statements from one line emits one mapping.
    current: Option<Loc>,
    /// Whether the last mapping carried a name. A named mapping must not bleed onto the text after
    /// the identifier — Chrome reads the name of whatever mapping covers a variable — so the next
    /// mapping is emitted even when the location did not change (js_of_ocaml does the same).
    named: bool,
    /// Mappings by *byte offset* into `out`, resolved to line and column once printing is done.
    raw: Vec<(usize, Loc, Option<String>)>,
}

/// The rendering state: an output buffer and the current indentation depth.
struct Printer {
    out: String,
    indent: usize,
    maps: Option<MapState>,
    style: Style,
}

impl Printer {
    fn new(style: Style) -> Self {
        Printer {
            out: String::new(),
            indent: 0,
            maps: None,
            style,
        }
    }

    fn compact(&self) -> bool {
        self.style == Style::Compact
    }

    /// Writes `text`, and the space after it that only readable output wants.
    ///
    /// The space is what separates a keyword from what follows it (`return x`, `case 1`); in
    /// compact output [`Printer::glue`] puts one back only where the two tokens would fuse.
    fn spaced(&mut self, text: &str) {
        self.out.push_str(text);
        if !self.compact() {
            self.out.push(' ');
        }
    }

    /// Writes `text`, surrounded by the spaces that only readable output wants.
    fn spaced_around(&mut self, text: &str) {
        if self.compact() {
            self.out.push_str(text);
        } else {
            self.out.push(' ');
            self.out.push_str(text);
            self.out.push(' ');
        }
    }

    /// Separates two tokens that met at `start` and must not fuse.
    ///
    /// Called *after* the second one is written, so the characters compared are the ones that
    /// actually ended up next to each other — the second token's first character is not knowable
    /// from its AST node (`-1`, `(a + b)`, `!0` all start differently). A no-op in readable output,
    /// where the space is already there.
    fn glue(&mut self, start: usize) {
        if start == 0 || start >= self.out.len() {
            return;
        }
        let before = self.out[..start].chars().next_back().expect("start is past a character");
        let after = self.out[start..].chars().next().expect("start is a character boundary");
        if need_space(before, after) {
            self.out.insert(start, ' ');
        }
    }

    /// A newline, unless the style has none.
    fn newline(&mut self) {
        if !self.compact() {
            self.out.push('\n');
        }
    }

    /// The `;` and the newline that end a statement.
    fn end_stmt(&mut self) {
        self.out.push(';');
        self.newline();
    }

    /// Drops the `;` a compact block's last statement left behind: `{a=1;}` is `{a=1}`.
    ///
    /// Only ever called immediately before a `}`, where the grammar inserts the semicolon back.
    fn drop_final_semi(&mut self) {
        if self.compact() && self.out.ends_with(';') {
            self.out.pop();
        }
    }

    /// The precedence an expression is printed at, which the style can change.
    ///
    /// `true` is a primary expression but `!0` is a unary one, so `x.f` over a boolean has to gain
    /// the parentheses it did not need. Nothing in this backend writes that, and the printer is
    /// still the wrong place to be lucky.
    fn prec_of(&self, expr: &Expr) -> u8 {
        if self.compact() && matches!(expr, Expr::Bool(_) | Expr::Undefined) {
            return PREC_UNARY;
        }
        expr.prec()
    }

    /// Opens a mapping for the statement about to be printed at the current offset.
    ///
    /// A marker that repeats the location already in effect emits nothing: a run of statements
    /// lowered from one Rust line is one stepping stop, not one per statement.
    fn map_stmt(&mut self) {
        let offset = self.out.len();
        let Some(state) = &mut self.maps else { return };
        let Some(loc) = state.pending.take() else { return };
        if state.current.as_ref() == Some(&loc) && !state.named {
            return;
        }
        state.raw.push((offset, loc.clone(), None));
        state.current = Some(loc);
        state.named = false;
    }

    /// Writes a declared name, mapping it to the Rust name it stands for when one is known.
    fn declared_name(&mut self, name: &str) {
        let offset = self.out.len();
        self.out.push_str(name);
        let after = self.out.len();
        self.name_mapping(name, offset, after);
    }

    /// Records a mapping naming `name` over the output range `offset..after`.
    ///
    /// A debugger reads the name off whatever mapping covers the identifier, so the range is
    /// closed with a nameless mapping — otherwise the name would go on describing the initializer,
    /// or the parameter list, that follows it.
    fn name_mapping(&mut self, name: &str, offset: usize, after: usize) {
        let Some(state) = &mut self.maps else { return };
        let (Some(loc), Some(original)) = (state.current.clone(), state.names.get(name)) else {
            return;
        };
        state.raw.push((offset, loc.clone(), Some(original.clone())));
        state.raw.push((after, loc, None));
        state.named = false;
    }

    fn write_indent(&mut self) {
        if self.compact() {
            return;
        }
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
    }

    /// Writes an expression, parenthesizing it when it binds looser than `min_prec`.
    fn expr(&mut self, expr: &Expr, min_prec: u8) {
        let parens = self.prec_of(expr) < min_prec;
        if parens {
            self.out.push('(');
        }
        self.expr_bare(expr);
        if parens {
            self.out.push(')');
        }
    }

    fn expr_bare(&mut self, expr: &Expr) {
        match expr {
            Expr::Num(n) => write_number(&mut self.out, *n),
            Expr::RawNum(s) => self.out.push_str(s),
            Expr::BigInt(digits) => {
                self.out.push_str(digits);
                self.out.push('n');
            }
            Expr::Str(s) => write_js_string(&mut self.out, s),
            // `!0`/`!1` are the shortest spellings of the booleans, and `void 0` of `undefined`,
            // which is also how this backend spells unit and every zero sized type.
            Expr::Bool(b) => self.out.push_str(match (self.compact(), b) {
                (false, true) => "true",
                (false, false) => "false",
                (true, true) => "!0",
                (true, false) => "!1",
            }),
            Expr::Undefined => {
                self.out.push_str(if self.compact() { "void 0" } else { "undefined" })
            }
            Expr::Null => self.out.push_str("null"),
            Expr::Ident(name) => self.out.push_str(name),
            Expr::Member(obj, name) => {
                self.object_of_access(obj);
                if is_plain_ident(name) {
                    self.out.push('.');
                    self.out.push_str(name);
                } else {
                    self.out.push('[');
                    write_js_string(&mut self.out, name);
                    self.out.push(']');
                }
            }
            Expr::Index(obj, index) => {
                self.object_of_access(obj);
                self.out.push('[');
                self.expr(index, 0);
                self.out.push(']');
            }
            Expr::Call(callee, args) => {
                self.object_of_access(callee);
                self.args(args);
            }
            Expr::New(callee, args) => {
                self.spaced("new");
                let start = self.out.len();
                if callee.head_is_call() {
                    self.out.push('(');
                    self.expr_bare(callee);
                    self.out.push(')');
                } else {
                    self.object_of_access(callee);
                }
                self.glue(start);
                self.args(args);
            }
            Expr::Unary(op, operand) => {
                let symbol = op.symbol();
                if self.compact() {
                    self.out.push_str(symbol.trim_end());
                } else {
                    self.out.push_str(symbol);
                }
                let start = self.out.len();
                self.expr(operand, PREC_UNARY);
                // `- -x` and `+ +x` need the space, otherwise they read as `--` and `++`; so does
                // `typeof x`, once the word operator has lost its trailing space.
                self.glue(start);
            }
            Expr::Binary(op, lhs, rhs) => {
                let prec = op.prec();
                self.expr(lhs, prec);
                self.spaced_around(op.symbol());
                let start = self.out.len();
                self.expr(rhs, prec + 1);
                self.glue(start);
            }
            Expr::Assign(target, value) => {
                self.expr(target, PREC_CALL);
                self.spaced_around("=");
                self.expr(value, PREC_ASSIGN);
            }
            Expr::CompoundAssign(op, target, value) => {
                self.expr(target, PREC_CALL);
                if !self.compact() {
                    self.out.push(' ');
                }
                self.out.push_str(op.symbol());
                self.spaced("=");
                let start = self.out.len();
                self.expr(value, PREC_ASSIGN);
                self.glue(start);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test, PREC_COND + 1);
                self.spaced_around("?");
                self.expr(consequent, PREC_ASSIGN);
                self.spaced_around(":");
                self.expr(alternate, PREC_ASSIGN);
            }
            Expr::Object(entries) => {
                if entries.is_empty() {
                    self.out.push_str("{}");
                    return;
                }
                self.out.push('{');
                if !self.compact() {
                    self.out.push(' ');
                }
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.separator();
                    }
                    if is_plain_ident(key) {
                        self.out.push_str(key);
                    } else {
                        write_js_string(&mut self.out, key);
                    }
                    self.spaced(":");
                    self.expr(value, PREC_ASSIGN);
                }
                if !self.compact() {
                    self.out.push(' ');
                }
                self.out.push('}');
            }
            Expr::Array(items) => {
                self.out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.separator();
                    }
                    self.expr(item, PREC_ASSIGN);
                }
                self.out.push(']');
            }
            Expr::Arrow(params, body) => {
                // `a => ..` rather than `(a) => ..`. Only for exactly one parameter, which is the
                // only shape the grammar allows bare — and the only shape this AST can build,
                // having no destructuring and no defaults.
                match (self.compact(), params.as_slice()) {
                    (true, [only]) => self.out.push_str(only),
                    _ => self.params(params),
                }
                match body {
                    // An object literal body would read as a block, so it needs parens.
                    ArrowBody::Expr(value) if matches!(**value, Expr::Object(_)) => {
                        self.spaced_around("=>");
                        self.out.push('(');
                        self.expr_bare(value);
                        self.out.push(')');
                    }
                    ArrowBody::Expr(value) => {
                        self.spaced_around("=>");
                        self.expr(value, PREC_ASSIGN);
                    }
                    // A block writes its own leading space, so the arrow must not write one too.
                    ArrowBody::Block(stmts) => {
                        self.out.push_str(if self.compact() { "=>" } else { " =>" });
                        self.body(stmts);
                    }
                }
            }
            Expr::Seq(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.separator();
                    }
                    self.expr(item, PREC_ASSIGN);
                }
            }
            Expr::Raw(text) => self.out.push_str(text),
        }
    }

    /// Writes the object of a member access, an index or a call. A number literal always gets
    /// parentheses so that `1 .toString()` never comes out as `1.toString()`.
    fn object_of_access(&mut self, obj: &Expr) {
        if matches!(obj, Expr::Num(_) | Expr::RawNum(_) | Expr::BigInt(_)) {
            self.out.push('(');
            self.expr_bare(obj);
            self.out.push(')');
        } else {
            self.expr(obj, PREC_CALL);
        }
    }

    /// The `, ` between list elements, without the space in compact output.
    fn separator(&mut self) {
        self.spaced(",");
    }

    fn args(&mut self, args: &[Expr]) {
        self.out.push('(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.separator();
            }
            self.expr(arg, PREC_ASSIGN);
        }
        self.out.push(')');
    }

    fn params(&mut self, params: &[String]) {
        self.out.push('(');
        for (i, param) in params.iter().enumerate() {
            if i > 0 {
                self.separator();
            }
            self.out.push_str(param);
        }
        self.out.push(')');
    }

    /// Writes ` { .. }` for a statement body, leaving the cursor after the closing brace.
    fn body(&mut self, stmts: &[Stmt]) {
        if stmts.is_empty() {
            self.out.push_str(if self.compact() { "{}" } else { " {}" });
            return;
        }
        self.out.push_str(if self.compact() { "{" } else { " {\n" });
        self.indent += 1;
        for stmt in stmts {
            self.stmt(stmt);
        }
        self.indent -= 1;
        self.drop_final_semi();
        self.write_indent();
        self.out.push('}');
    }

    /// Writes the body of an `if`, a `while` or a `for (;;)`, dropping the braces when compact
    /// output can.
    ///
    /// Only a body that is exactly one statement of a *terminal* kind loses its braces: an
    /// expression, a `return`, a `break`, a `continue` or a `throw`. None of those can contain an
    /// `if`, which is what makes the rule immune to the dangling-else problem — `if (a) if (b) x;
    /// else y;` binds the `else` to the inner `if`, so the braces of an outer `then` branch are
    /// load bearing whenever that branch could hold one. Declarations are excluded too: a `let` is
    /// not a statement the grammar allows there at all.
    fn branch_body(&mut self, stmts: &[Stmt]) {
        if self.compact() {
            if let [only] = stmts {
                if is_terminal_stmt(only) {
                    self.stmt(only);
                    return;
                }
            }
        }
        self.body(stmts);
    }

    /// Writes a statement on its own line, including the indentation and the newline.
    ///
    /// A [`Stmt::Loc`] marker writes nothing at all, indentation included: it only arms the
    /// mapping the next statement opens.
    fn stmt(&mut self, stmt: &Stmt) {
        if let Stmt::Loc(loc) = stmt {
            if let Some(state) = &mut self.maps {
                state.pending = Some(loc.clone());
            }
            return;
        }
        // A comment has no compact spelling: it would need the newline that compact output does
        // not have, and it says nothing to a reader who is not reading this anyway.
        if self.compact() && matches!(stmt, Stmt::Comment(_)) {
            return;
        }
        self.write_indent();
        self.map_stmt();
        self.stmt_bare(stmt);
    }

    /// Writes a statement assuming the indentation is already there. Always ends with a
    /// newline.
    fn stmt_bare(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) => {
                let start = self.out.len();
                self.expr(expr, 0);
                if needs_stmt_parens(&self.out[start..]) {
                    self.out.insert(start, '(');
                    self.out.push(')');
                }
                self.end_stmt();
            }
            Stmt::Let(name, init) => {
                self.declaration("let", std::slice::from_ref(&(name.clone(), init.clone())));
            }
            Stmt::Lets(decls) => self.declaration("let", decls),
            Stmt::Const(name, init) => {
                self.declaration("const", &[(name.clone(), Some(init.clone()))]);
            }
            Stmt::Return(value) => {
                self.out.push_str("return");
                if let Some(value) = value {
                    if !self.compact() {
                        self.out.push(' ');
                    }
                    let start = self.out.len();
                    self.expr(value, 0);
                    self.glue(start);
                }
                self.end_stmt();
            }
            Stmt::If(test, then, els) => {
                self.if_chain(test, then, els.as_deref());
                self.newline();
            }
            Stmt::Switch(disc, cases) => {
                self.spaced("switch");
                self.out.push('(');
                self.expr(disc, 0);
                self.out.push(')');
                self.out.push_str(if self.compact() { "{" } else { " {\n" });
                self.indent += 1;
                for case in cases {
                    if case.tests.is_empty() {
                        self.write_indent();
                        self.out.push_str("default:");
                        self.newline();
                    } else {
                        for test in &case.tests {
                            self.write_indent();
                            self.spaced("case");
                            let start = self.out.len();
                            self.expr(test, PREC_ASSIGN);
                            self.glue(start);
                            self.out.push(':');
                            self.newline();
                        }
                    }
                    self.indent += 1;
                    for stmt in &case.body {
                        self.stmt(stmt);
                    }
                    self.indent -= 1;
                }
                self.indent -= 1;
                self.drop_final_semi();
                self.write_indent();
                self.out.push('}');
                self.newline();
            }
            Stmt::Labeled(label, inner) => {
                self.out.push_str(label);
                self.spaced(":");
                self.stmt_bare(inner);
            }
            Stmt::LoopForever(label, body) => {
                if let Some(label) = label {
                    self.out.push_str(label);
                    self.spaced(":");
                }
                self.out.push_str(if self.compact() { "for(;;)" } else { "for (;;)" });
                self.branch_body(body);
                self.newline();
            }
            Stmt::While(test, body) => {
                self.spaced("while");
                self.out.push('(');
                self.expr(test, 0);
                self.out.push(')');
                self.branch_body(body);
                self.newline();
            }
            Stmt::Continue(label) => {
                self.out.push_str("continue");
                if let Some(label) = label {
                    self.out.push(' ');
                    self.out.push_str(label);
                }
                self.end_stmt();
            }
            Stmt::Break(label) => {
                self.out.push_str("break");
                if let Some(label) = label {
                    self.out.push(' ');
                    self.out.push_str(label);
                }
                self.end_stmt();
            }
            Stmt::Throw(value) => {
                self.out.push_str("throw");
                if !self.compact() {
                    self.out.push(' ');
                }
                let start = self.out.len();
                self.expr(value, 0);
                self.glue(start);
                self.end_stmt();
            }
            Stmt::FunctionDecl { name, params, body } => {
                // The space is not optional: a declaration always names the function.
                self.out.push_str("function ");
                let start = self.out.len();
                self.out.push_str(name);
                let paren = self.out.len();
                self.params(params);
                // Chrome takes a bound name from the mapping at the identifier and a *function's*
                // stack name from the one at its opening parenthesis, so the named range covers
                // both and stops before the parameters, which have names of their own.
                self.name_mapping(name, start, paren + 1);
                self.body(body);
                self.newline();
            }
            Stmt::Import { specifiers, source } => {
                // The keyword is followed by `*` or `{`, neither of which can fuse with it, so the
                // space after it is the compact printer's to drop. The one before `from` is not:
                // `as ns from` and `}from` are the tightest either form gets.
                self.out.push_str("import");
                match specifiers {
                    ImportSpecifiers::Namespace(name) => {
                        if !self.compact() {
                            self.out.push(' ');
                        }
                        self.out.push('*');
                        if !self.compact() {
                            self.out.push(' ');
                        }
                        self.out.push_str("as ");
                        self.out.push_str(name);
                        self.out.push(' ');
                    }
                    ImportSpecifiers::Named(names) => {
                        if !self.compact() {
                            self.out.push(' ');
                        }
                        self.out.push('{');
                        for (i, (exported, local)) in names.iter().enumerate() {
                            if i > 0 {
                                self.out.push(',');
                            }
                            if !self.compact() {
                                self.out.push(' ');
                            }
                            self.out.push_str(exported);
                            if exported != local {
                                self.out.push_str(" as ");
                                self.out.push_str(local);
                            }
                        }
                        self.out.push_str(if self.compact() { "}" } else { " } " });
                    }
                }
                self.out.push_str("from");
                if !self.compact() {
                    self.out.push(' ');
                }
                write_js_string(&mut self.out, source);
                self.end_stmt();
            }
            Stmt::Block(stmts) => {
                if stmts.is_empty() {
                    self.out.push_str("{}");
                    self.newline();
                    return;
                }
                self.out.push('{');
                self.newline();
                self.indent += 1;
                for stmt in stmts {
                    self.stmt(stmt);
                }
                self.indent -= 1;
                self.drop_final_semi();
                self.write_indent();
                self.out.push('}');
                self.newline();
            }
            Stmt::Comment(text) => {
                self.out.push_str("// ");
                for ch in text.chars() {
                    self.out
                        .push(if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') { ' ' } else { ch });
                }
                self.out.push('\n');
            }
            Stmt::Raw(text) => {
                for (i, line) in text.lines().enumerate() {
                    if i > 0 {
                        self.write_indent();
                    }
                    self.out.push_str(line);
                    self.out.push('\n');
                }
            }
            // Handled by `stmt`, which never reaches here with one.
            Stmt::Loc(_) => {}
        }
    }

    fn declaration(&mut self, keyword: &str, decls: &[(String, Option<Expr>)]) {
        self.out.push_str(keyword);
        self.out.push(' ');
        for (i, (name, init)) in decls.iter().enumerate() {
            if i > 0 {
                self.separator();
            }
            self.declared_name(name);
            if let Some(init) = init {
                self.spaced_around("=");
                self.expr(init, PREC_ASSIGN);
            }
        }
        self.end_stmt();
    }

    /// Writes an `if` with its `else if` chain, without the trailing newline.
    ///
    /// The `then` branch keeps its braces whenever there is an `else`: a braceless branch of the
    /// one statement kind that could hold an `if` is what the dangling-else problem is made of, and
    /// [`Printer::branch_body`] refuses those anyway — this is the second lock on the same door.
    fn if_chain(&mut self, test: &Expr, then: &[Stmt], els: Option<&[Stmt]>) {
        self.spaced("if");
        self.out.push('(');
        self.expr(test, 0);
        self.out.push(')');
        match els {
            None => self.branch_body(then),
            Some(_) => self.body(then),
        }
        match els.map(single_if) {
            None => {}
            Some(Some((markers, test, then, els))) => {
                self.spaced_around("else");
                if self.compact() {
                    self.out.push(' ');
                }
                // The markers print nothing; running them here is what puts the `if` they
                // describe on the mapping it deserves instead of breaking the chain in two.
                for marker in markers {
                    self.stmt(marker);
                }
                self.map_stmt();
                self.if_chain(test, then, els);
            }
            Some(None) => {
                if !self.compact() {
                    self.out.push(' ');
                }
                self.out.push_str("else");
                let start = self.out.len();
                self.branch_body(els.unwrap_or_default());
                self.glue(start);
            }
        }
    }
}

/// Whether a statement can stand as an unbraced `if`/`while` body in compact output.
///
/// Nothing here can contain an `if`, so nothing here can catch an `else` that was meant for an
/// enclosing one, and nothing here is a declaration, which the grammar does not allow in that
/// position at all.
fn is_terminal_stmt(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::ExprStmt(_) | Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Throw(_)
    )
}

/// Whether two characters that met in the output would read as one token, or as the start of a
/// comment, and so need a space between them.
///
/// js_of_ocaml's table (`js_output.ml`): two identifier characters fuse into one identifier, `//`
/// opens a comment, `--` and `++` are operators of their own, and `<!`/`-->` are the HTML comment
/// delimiters that a `<script>` body still honours.
fn need_space(a: char, b: char) -> bool {
    let part_of_ident =
        |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$' || !c.is_ascii();
    if part_of_ident(a) && part_of_ident(b) {
        return true;
    }
    matches!((a, b), ('/', '/') | ('-', '-') | ('+', '+') | ('-', '>') | ('<', '!'))
}

/// The parts of an `else` branch that is a single `if`, markers aside.
///
/// A [`Stmt::Loc`] prints nothing, so a branch of markers and one `if` is still an `else if`
/// chain. Reading it as one is what keeps `-Cllvm-args=js-source-map=on` from changing the
/// JavaScript it is supposed to only describe.
#[allow(clippy::type_complexity)]
fn single_if(stmts: &[Stmt]) -> Option<(&[Stmt], &Expr, &[Stmt], Option<&[Stmt]>)> {
    let (last, markers) = stmts.split_last()?;
    if !markers.iter().all(|stmt| matches!(stmt, Stmt::Loc(_))) {
        return None;
    }
    match last {
        Stmt::If(test, then, els) => Some((markers, test, then, els.as_deref())),
        _ => None,
    }
}

/// Turns the byte offsets the printer recorded into generated line and column positions.
///
/// The offsets only ever increase, so one walk over the text serves all of them. Columns count
/// UTF-16 code units — a non-ASCII character in a string literal is one unit, an astral one is two
/// — because that is the unit the source map format and the browser's view of the file agree on.
fn resolve_offsets(out: &str, raw: Vec<(usize, Loc, Option<String>)>) -> Vec<Mapping> {
    let mut mappings = Vec::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut line = 0u32;
    let mut col = 0u32;
    for (offset, loc, name) in raw {
        let offset = offset.min(out.len());
        while cursor < offset {
            // The text is UTF-8 and every offset the printer records is a character boundary, so
            // stepping by character is safe and the width in UTF-16 units is the character's.
            let ch = out[cursor..].chars().next().expect("offset is a character boundary");
            cursor += ch.len_utf8();
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += ch.len_utf16() as u32;
            }
        }
        mappings.push(Mapping { gen_line: line, gen_col: col, loc, name });
    }
    mappings
}

/// Whether the leftmost token of a printed expression makes it unusable as a statement on its
/// own, which happens for object literals and for function or class expressions.
fn needs_stmt_parens(printed: &str) -> bool {
    printed.starts_with('{')
        || starts_with_word(printed, "function")
        || starts_with_word(printed, "class")
}

/// Whether `text` starts with `word` followed by something that cannot continue it.
fn starts_with_word(text: &str, word: &str) -> bool {
    match text.strip_prefix(word) {
        Some(rest) => !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$'),
        None => false,
    }
}

/// Whether `name` can be written without quotes as a member name or an object key.
///
/// Reserved words are rejected even though modern JavaScript allows them in both positions,
/// so that the output stays valid under any parser.
pub fn is_plain_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(c) => c.is_ascii_alphabetic() || c == '_' || c == '$',
        None => false,
    };
    valid && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') && !is_reserved(name)
}

/// Whether `name` is a JavaScript reserved word.
pub fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

/// Turns an arbitrary name into a usable JavaScript identifier.
///
/// Characters outside `[A-Za-z0-9_$]` become `_`, so a Rust path separator `::` turns into
/// `__`. A name that starts with a digit or that collides with a reserved word is prefixed
/// with `_`. The mapping is not injective: append a hash when uniqueness matters.
pub fn sanitize_ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 1);
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) || is_reserved(&out) {
        out.insert(0, '_');
    }
    out
}

/// Writes a number as a JavaScript literal.
fn write_number(out: &mut String, n: f64) {
    if n.is_nan() {
        out.push_str("NaN");
    } else if n.is_infinite() {
        out.push_str(if n > 0.0 { "Infinity" } else { "-Infinity" });
    } else if n == 0.0 && n.is_sign_negative() {
        out.push_str("-0");
    } else {
        // The `Display` impl for `f64` is the shortest form that round trips, and it never
        // uses exponent notation, so it is always a valid JavaScript literal.
        out.push_str(&n.to_string());
    }
}

/// Writes a string as a double quoted JavaScript literal.
fn write_js_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            // Line separators terminate a string literal in older parsers.
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str("\\x");
                out.push(hex_digit((c as u32) >> 4));
                out.push(hex_digit(c as u32 & 0xf));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex_digit(value: u32) -> char {
    char::from_digit(value, 16).unwrap_or('0')
}

/// A number literal. Accepts anything that converts to `f64`, so `num(1)` works.
pub fn num(value: impl Into<f64>) -> Expr {
    Expr::Num(value.into())
}

/// A number literal spelled exactly as given.
pub fn raw_num(text: impl Into<String>) -> Expr {
    Expr::RawNum(text.into())
}

/// A `BigInt` literal, printed with the trailing `n`. Accepts `i128` and `u128` alike.
pub fn bigint(value: impl fmt::Display) -> Expr {
    Expr::BigInt(value.to_string())
}

/// A string literal.
pub fn string(value: impl Into<String>) -> Expr {
    Expr::Str(value.into())
}

/// A boolean literal.
pub fn boolean(value: bool) -> Expr {
    Expr::Bool(value)
}

/// `undefined`, which is also the representation of unit and of zero sized types.
pub fn undefined() -> Expr {
    Expr::Undefined
}

/// `null`.
pub fn null() -> Expr {
    Expr::Null
}

/// An identifier reference.
pub fn id(name: impl Into<String>) -> Expr {
    Expr::Ident(name.into())
}

/// Verbatim JavaScript in expression position.
pub fn raw_expr(text: impl Into<String>) -> Expr {
    Expr::Raw(text.into())
}

/// `obj.name`, with a property read out of an object literal folded away.
///
/// `{ buf: xs, off: 0 }.buf` is `xs`. The fold exists for the pointer model: a `&mut` to a local
/// is the slot literal `{ buf: x, off: 0 }`, dereferencing it is `p.buf[p.off]`, and putting the
/// two together without this would print `{ buf: x, off: 0 }.buf[{ buf: x, off: 0 }.off]` where
/// `x[0]` is meant. Every entry that is dropped has to be one whose evaluation could not be
/// observed ([`is_droppable`]), and a duplicated key declines the fold rather than guessing which
/// one JavaScript would have kept.
pub fn member(obj: Expr, name: impl Into<String>) -> Expr {
    let name = name.into();
    if let Expr::Object(entries) = &obj {
        if let Some(value) = fold_property(entries, &name) {
            return value;
        }
    }
    Expr::Member(Box::new(obj), name)
}

/// Whether reading `name` out of this object literal folds away to one of its entries.
///
/// The question [`member`] answers by folding, asked ahead of time: `queue.rs` moves an expression
/// to where it is read only when doing so does not duplicate it, and a slot literal substituted
/// into both halves of `p.buf[p.off]` duplicates nothing exactly when both reads fold.
pub fn folds_property(entries: &[(String, Expr)], name: &str) -> bool {
    fold_property(entries, name).is_some()
}

/// The value an object literal gives `name`, when reading it can drop the other entries.
fn fold_property(entries: &[(String, Expr)], name: &str) -> Option<Expr> {
    let mut found: Option<&Expr> = None;
    for (key, value) in entries {
        if key == name {
            // A repeated key is JavaScript's business, not this peephole's.
            if found.is_some() {
                return None;
            }
            found = Some(value);
        } else if !is_droppable(value) {
            return None;
        }
    }
    found.cloned()
}

/// Whether an expression can be dropped without changing what the program does.
///
/// Deliberately narrow: anything that calls, assigns or is verbatim text is kept.
fn is_droppable(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Arrow(..) => true,
        Expr::Member(object, _) | Expr::Unary(_, object) => is_droppable(object),
        Expr::Index(lhs, rhs) | Expr::Binary(_, lhs, rhs) => is_droppable(lhs) && is_droppable(rhs),
        Expr::Cond(test, consequent, alternate) => {
            is_droppable(test) && is_droppable(consequent) && is_droppable(alternate)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().all(is_droppable),
        Expr::Object(entries) => entries.iter().all(|(_, value)| is_droppable(value)),
        Expr::Call(..)
        | Expr::New(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::Raw(_) => false,
    }
}

/// `obj[index]`, spelled `obj.name` when the index is a fixed property name.
///
/// A slot whose `off` is a string names a field, so `p.buf[p.off]` on one folds to `x["field"]`;
/// that is the same property access as `x.field`, and the printer only puts the brackets back for
/// a name that is not an identifier.
pub fn index(obj: Expr, index: Expr) -> Expr {
    if let Expr::Str(name) = &index {
        if is_plain_ident(name) {
            return member(obj, name.clone());
        }
    }
    Expr::Index(Box::new(obj), Box::new(index))
}

/// `callee(args...)`.
pub fn call(callee: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(Box::new(callee), args)
}

/// `obj.name(args...)`.
pub fn method_call(obj: Expr, name: impl Into<String>, args: Vec<Expr>) -> Expr {
    call(member(obj, name), args)
}

/// `new callee(args...)`.
pub fn new(callee: Expr, args: Vec<Expr>) -> Expr {
    Expr::New(Box::new(callee), args)
}

/// A prefix operator application.
pub fn unary(op: UnOp, operand: Expr) -> Expr {
    Expr::Unary(op, Box::new(operand))
}

/// `-operand`.
pub fn neg(operand: Expr) -> Expr {
    unary(UnOp::Neg, operand)
}

/// `!operand`.
pub fn not(operand: Expr) -> Expr {
    unary(UnOp::Not, operand)
}

/// `~operand`.
pub fn bit_not(operand: Expr) -> Expr {
    unary(UnOp::BitNot, operand)
}

/// `typeof operand`.
pub fn type_of(operand: Expr) -> Expr {
    unary(UnOp::TypeOf, operand)
}

/// An infix operator application.
pub fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::Binary(op, Box::new(lhs), Box::new(rhs))
}

/// `target = value`.
pub fn assign(target: Expr, value: Expr) -> Expr {
    Expr::Assign(Box::new(target), Box::new(value))
}

/// `target op= value`.
pub fn compound_assign(op: BinOp, target: Expr, value: Expr) -> Expr {
    Expr::CompoundAssign(op, Box::new(target), Box::new(value))
}

/// `test ? consequent : alternate`.
pub fn cond(test: Expr, consequent: Expr, alternate: Expr) -> Expr {
    Expr::Cond(Box::new(test), Box::new(consequent), Box::new(alternate))
}

/// An object literal.
pub fn object(entries: Vec<(String, Expr)>) -> Expr {
    Expr::Object(entries)
}

/// An object literal from string slices, to keep call sites short.
pub fn object_of(entries: Vec<(&str, Expr)>) -> Expr {
    Expr::Object(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// An array literal, which is also how tuples and arrays are represented.
pub fn array(items: Vec<Expr>) -> Expr {
    Expr::Array(items)
}

/// An arrow function with an expression body.
pub fn arrow(params: Vec<String>, body: Expr) -> Expr {
    Expr::Arrow(params, ArrowBody::Expr(Box::new(body)))
}

/// An arrow function with a block body.
pub fn arrow_block(params: Vec<String>, body: Vec<Stmt>) -> Expr {
    Expr::Arrow(params, ArrowBody::Block(body))
}

/// A comma expression.
pub fn seq(items: Vec<Expr>) -> Expr {
    Expr::Seq(items)
}

/// `expr | 0`, the i32 wrap.
pub fn i32_wrap(expr: Expr) -> Expr {
    binary(BinOp::BitOr, expr, num(0))
}

/// `expr >>> 0`, the u32 wrap.
pub fn u32_wrap(expr: Expr) -> Expr {
    binary(BinOp::UShr, expr, num(0))
}

/// `expr << shift >> shift`, the sign extension for a value of `bits` bits.
pub fn sign_extend(bits: u32, expr: Expr) -> Expr {
    let shift = num(32u32.saturating_sub(bits));
    binary(
        BinOp::Shr,
        binary(BinOp::Shl, expr, shift.clone()),
        shift,
    )
}

/// `expr & mask`, the unsigned truncation to `bits` bits.
pub fn zero_extend(bits: u32, expr: Expr) -> Expr {
    let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
    binary(BinOp::BitAnd, expr, num(mask))
}

/// `Math.imul(lhs, rhs)`, the 32 bit multiply.
pub fn imul(lhs: Expr, rhs: Expr) -> Expr {
    call(member(id("Math"), "imul"), vec![lhs, rhs])
}

/// `Math.fround(expr)`, the rounding to `f32` precision.
pub fn fround(expr: Expr) -> Expr {
    call(member(id("Math"), "fround"), vec![expr])
}

/// `BigInt.asIntN(bits, expr)` or `BigInt.asUintN(bits, expr)`, the truncation of a `BigInt`.
pub fn bigint_mask(signed: bool, bits: u32, expr: Expr) -> Expr {
    let name = if signed { "asIntN" } else { "asUintN" };
    call(member(id("BigInt"), name), vec![num(bits), expr])
}

/// `BigInt(expr)`, the widening of a JS number to a `BigInt`. The argument has to be an integer
/// valued number; every masked integer this backend produces is one.
pub fn to_bigint(expr: Expr) -> Expr {
    call(id("BigInt"), vec![expr])
}

/// `Number(expr)`, the narrowing of a `BigInt` to a JS number. Truncate to the destination width
/// with [`bigint_mask`] first: past 2^53 the conversion rounds, and rounding before truncating
/// gives the wrong low bits.
pub fn to_number(expr: Expr) -> Expr {
    call(id("Number"), vec![expr])
}

/// `{ v: value }`, the cell that stands in for an address taken local.
pub fn cell(value: Expr) -> Expr {
    object_of(vec![("v", value)])
}

/// `place.v`, the read through a cell reference.
pub fn cell_get(place: Expr) -> Expr {
    member(place, "v")
}

/// `{ get: () => place, set: (v) => (place = v) }`, the reference to a place projection.
///
/// `param` names the setter parameter; pick something that cannot shadow a captured local.
pub fn accessor(place: Expr, param: &str) -> Expr {
    object_of(vec![
        ("get", arrow(vec![], place.clone())),
        (
            "set",
            arrow(vec![param.to_string()], assign(place, id(param))),
        ),
    ])
}

/// `__rt.name(args...)`, a call into the runtime shim.
pub fn rt_call(name: impl Into<String>, args: Vec<Expr>) -> Expr {
    call(member(id("__rt"), name), args)
}

/// An expression evaluated for its effect.
pub fn expr_stmt(expr: Expr) -> Stmt {
    Stmt::ExprStmt(expr)
}

/// `target = value;`.
pub fn assign_stmt(target: Expr, value: Expr) -> Stmt {
    Stmt::ExprStmt(assign(target, value))
}

/// `let name = init;`.
pub fn let_(name: impl Into<String>, init: Expr) -> Stmt {
    Stmt::Let(name.into(), Some(init))
}

/// `let name;`.
pub fn let_uninit(name: impl Into<String>) -> Stmt {
    Stmt::Let(name.into(), None)
}

/// `let a = 1, b, c = 3;`.
pub fn lets(decls: Vec<(String, Option<Expr>)>) -> Stmt {
    Stmt::Lets(decls)
}

/// `const name = init;`.
pub fn const_(name: impl Into<String>, init: Expr) -> Stmt {
    Stmt::Const(name.into(), init)
}

/// `return value;`.
pub fn ret(value: Expr) -> Stmt {
    Stmt::Return(Some(value))
}

/// `return;`.
pub fn ret_void() -> Stmt {
    Stmt::Return(None)
}

/// `if (test) { .. }`.
pub fn if_(test: Expr, then: Vec<Stmt>) -> Stmt {
    Stmt::If(test, then, None)
}

/// `if (test) { .. } else { .. }`.
pub fn if_else(test: Expr, then: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
    Stmt::If(test, then, Some(els))
}

/// `switch (disc) { .. }`.
pub fn switch(disc: Expr, cases: Vec<SwitchCase>) -> Stmt {
    Stmt::Switch(disc, cases)
}

/// One `case` group. Several tests share one body, which is how fallthrough labels are built.
pub fn case(tests: Vec<Expr>, body: Vec<Stmt>) -> SwitchCase {
    SwitchCase { tests, body }
}

/// The `default` group.
pub fn default_case(body: Vec<Stmt>) -> SwitchCase {
    SwitchCase {
        tests: Vec::new(),
        body,
    }
}

/// `label: for (;;) { .. }`.
pub fn loop_forever(label: impl Into<String>, body: Vec<Stmt>) -> Stmt {
    Stmt::LoopForever(Some(label.into()), body)
}

/// `while (test) { .. }`.
pub fn while_(test: Expr, body: Vec<Stmt>) -> Stmt {
    Stmt::While(test, body)
}

/// `label: stmt`.
pub fn labeled(label: impl Into<String>, stmt: Stmt) -> Stmt {
    Stmt::Labeled(label.into(), Box::new(stmt))
}

/// `continue label;`.
pub fn continue_(label: impl Into<String>) -> Stmt {
    Stmt::Continue(Some(label.into()))
}

/// `break label;`.
pub fn break_(label: impl Into<String>) -> Stmt {
    Stmt::Break(Some(label.into()))
}

/// `throw value;`.
pub fn throw(value: Expr) -> Stmt {
    Stmt::Throw(value)
}

/// `throw new Error("message");`.
pub fn throw_error(message: impl Into<String>) -> Stmt {
    throw(new(id("Error"), vec![string(message)]))
}

/// A top level function declaration.
pub fn function(name: impl Into<String>, params: Vec<String>, body: Vec<Stmt>) -> Stmt {
    Stmt::FunctionDecl {
        name: name.into(),
        params,
        body,
    }
}

/// `import * as name from "source";`.
pub fn import_namespace(name: impl Into<String>, source: impl Into<String>) -> Stmt {
    Stmt::Import {
        specifiers: ImportSpecifiers::Namespace(name.into()),
        source: source.into(),
    }
}

/// `import { exported as local, .. } from "source";`.
pub fn import_named(names: Vec<(String, String)>, source: impl Into<String>) -> Stmt {
    Stmt::Import {
        specifiers: ImportSpecifiers::Named(names),
        source: source.into(),
    }
}

/// A `//` line comment.
pub fn comment(text: impl Into<String>) -> Stmt {
    Stmt::Comment(text.into())
}

/// Verbatim JavaScript in statement position.
pub fn raw_stmt(text: impl Into<String>) -> Stmt {
    Stmt::Raw(text.into())
}

/// A source location marker, which prints nothing. See [`Stmt::Loc`].
pub fn loc(file: impl Into<String>, line: u32, col: u32) -> Stmt {
    Stmt::Loc(Loc { file: file.into(), line, col })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(expr: Expr) -> String {
        expr_to_string(&expr)
    }

    fn params(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn numbers_use_the_shortest_round_tripping_form() {
        assert_eq!(p(num(0)), "0");
        assert_eq!(p(num(3.0)), "3");
        assert_eq!(p(num(1.5)), "1.5");
        assert_eq!(p(num(-7)), "-7");
        assert_eq!(p(num(0.1 + 0.2)), "0.30000000000000004");
        assert_eq!(p(num(f64::NAN)), "NaN");
        assert_eq!(p(num(f64::INFINITY)), "Infinity");
        assert_eq!(p(num(f64::NEG_INFINITY)), "-Infinity");
        assert_eq!(p(num(-0.0)), "-0");
        assert_eq!(p(raw_num("0x7fffffff")), "0x7fffffff");
    }

    #[test]
    fn bigint_literals_keep_their_exact_digits() {
        assert_eq!(p(bigint(0u128)), "0n");
        assert_eq!(p(bigint(u64::MAX)), "18446744073709551615n");
        assert_eq!(p(bigint(i64::MIN)), "-9223372036854775808n");
        assert_eq!(p(bigint(u128::MAX)), "340282366920938463463374607431768211455n");
        // A negative literal is a unary expression, so it is parenthesized where one is.
        assert_eq!(p(binary(BinOp::Sub, id("a"), bigint(-1i128))), "a - -1n");
        assert_eq!(p(method_call(bigint(1u32), "toString", vec![])), "(1n).toString()");
    }

    #[test]
    fn bigint_conversions_and_masks() {
        assert_eq!(
            p(bigint_mask(true, 64, id("a"))),
            "BigInt.asIntN(64, a)"
        );
        assert_eq!(
            p(bigint_mask(false, 128, to_bigint(id("a")))),
            "BigInt.asUintN(128, BigInt(a))"
        );
        assert_eq!(
            p(to_number(bigint_mask(true, 32, id("a")))),
            "Number(BigInt.asIntN(32, a))"
        );
        assert_eq!(p(fround(binary(BinOp::Add, id("a"), id("b")))), "Math.fround(a + b)");
    }

    #[test]
    fn strings_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(p(string("hi")), "\"hi\"");
        assert_eq!(p(string("a\"b")), "\"a\\\"b\"");
        assert_eq!(p(string("a\\b")), "\"a\\\\b\"");
        assert_eq!(p(string("a\nb\tc\rd")), "\"a\\nb\\tc\\rd\"");
        assert_eq!(p(string("\u{0}")), "\"\\x00\"");
        assert_eq!(p(string("\u{1b}")), "\"\\x1b\"");
        assert_eq!(p(string("\u{7f}")), "\"\\x7f\"");
        assert_eq!(p(string("\u{8}\u{b}\u{c}")), "\"\\b\\v\\f\"");
        assert_eq!(p(string("a\u{2028}b\u{2029}c")), "\"a\\u2028b\\u2029c\"");
        // Printable non-ASCII stays as is, the output is UTF-8.
        assert_eq!(p(string("caf\u{e9} \u{1f600}")), "\"caf\u{e9} \u{1f600}\"");
    }

    #[test]
    fn literals_and_identifiers() {
        assert_eq!(p(boolean(true)), "true");
        assert_eq!(p(boolean(false)), "false");
        assert_eq!(p(undefined()), "undefined");
        assert_eq!(p(null()), "null");
        assert_eq!(p(id("x")), "x");
    }

    #[test]
    fn member_access_falls_back_to_index_for_awkward_names() {
        assert_eq!(p(member(id("a"), "b")), "a.b");
        assert_eq!(p(member(id("a"), "$t")), "a.$t");
        assert_eq!(p(member(id("a"), "_0")), "a._0");
        assert_eq!(p(member(id("a"), "my field")), "a[\"my field\"]");
        assert_eq!(p(member(id("a"), "0")), "a[\"0\"]");
        assert_eq!(p(member(id("a"), "")), "a[\"\"]");
        // Reserved words are quoted so the output parses everywhere.
        assert_eq!(p(member(id("a"), "default")), "a[\"default\"]");
        assert_eq!(p(member(id("a"), "new")), "a[\"new\"]");
    }

    #[test]
    fn member_of_a_number_literal_is_parenthesized() {
        assert_eq!(p(method_call(num(1), "toString", vec![])), "(1).toString()");
        assert_eq!(p(member(num(1.5), "x")), "(1.5).x");
        assert_eq!(p(member(raw_num("1e3"), "x")), "(1e3).x");
    }

    #[test]
    fn member_of_call_chains_stay_flat() {
        let expr = method_call(
            method_call(id("a"), "b", vec![num(1)]),
            "c",
            vec![id("d")],
        );
        assert_eq!(p(expr), "a.b(1).c(d)");
        assert_eq!(
            p(index(call(id("f"), vec![]), num(0))),
            "f()[0]"
        );
        // A call on an arrow needs the parentheses back.
        assert_eq!(p(call(arrow(vec![], num(1)), vec![])), "(() => 1)()");
    }

    #[test]
    fn arithmetic_precedence_stays_clean() {
        let a = || id("a");
        let b = || id("b");
        let c = || id("c");
        assert_eq!(
            p(binary(BinOp::Add, a(), binary(BinOp::Mul, b(), c()))),
            "a + b * c"
        );
        assert_eq!(
            p(binary(BinOp::Mul, binary(BinOp::Add, a(), b()), c())),
            "(a + b) * c"
        );
        // Left associative, so the left child never needs parentheses.
        assert_eq!(
            p(binary(BinOp::Sub, binary(BinOp::Sub, a(), b()), c())),
            "a - b - c"
        );
        // The right child of the same precedence does.
        assert_eq!(
            p(binary(BinOp::Sub, a(), binary(BinOp::Sub, b(), c()))),
            "a - (b - c)"
        );
        assert_eq!(
            p(binary(BinOp::Div, a(), binary(BinOp::Mul, b(), c()))),
            "a / (b * c)"
        );
    }

    #[test]
    fn mixed_logical_and_bitwise_precedence() {
        let a = || id("a");
        let b = || id("b");
        let c = || id("c");
        // `&&` binds tighter than `||`.
        assert_eq!(
            p(binary(BinOp::Or, binary(BinOp::And, a(), b()), c())),
            "a && b || c"
        );
        assert_eq!(
            p(binary(BinOp::And, binary(BinOp::Or, a(), b()), c())),
            "(a || b) && c"
        );
        // Bitwise binds tighter than logical.
        assert_eq!(
            p(binary(BinOp::BitOr, a(), binary(BinOp::And, b(), c()))),
            "a | (b && c)"
        );
        assert_eq!(
            p(binary(BinOp::And, a(), binary(BinOp::BitOr, b(), c()))),
            "a && b | c"
        );
        // Equality binds tighter than bitwise and.
        assert_eq!(
            p(binary(BinOp::BitAnd, binary(BinOp::Eq, a(), b()), c())),
            "a == b & c"
        );
        assert_eq!(
            p(binary(BinOp::Eq, binary(BinOp::BitAnd, a(), b()), c())),
            "(a & b) == c"
        );
        // Shifts bind looser than addition, which is the classic trap.
        assert_eq!(
            p(binary(BinOp::Shl, binary(BinOp::Add, a(), b()), c())),
            "a + b << c"
        );
        assert_eq!(
            p(binary(BinOp::Add, binary(BinOp::Shl, a(), b()), c())),
            "(a << b) + c"
        );
    }

    #[test]
    fn wrapping_helpers_parenthesize_their_operand() {
        assert_eq!(
            p(i32_wrap(binary(BinOp::Add, id("a"), id("b")))),
            "a + b | 0"
        );
        assert_eq!(
            p(binary(BinOp::Add, i32_wrap(id("a")), id("b"))),
            "(a | 0) + b"
        );
        assert_eq!(p(u32_wrap(id("a"))), "a >>> 0");
        assert_eq!(p(sign_extend(8, id("a"))), "a << 24 >> 24");
        assert_eq!(p(sign_extend(16, id("a"))), "a << 16 >> 16");
        assert_eq!(p(zero_extend(8, id("a"))), "a & 255");
        assert_eq!(p(imul(id("a"), id("b"))), "Math.imul(a, b)");
        assert_eq!(
            p(i32_wrap(imul(id("a"), id("b")))),
            "Math.imul(a, b) | 0"
        );
    }

    #[test]
    fn unary_minus_never_forms_a_decrement() {
        assert_eq!(p(neg(id("a"))), "-a");
        assert_eq!(p(neg(neg(id("a")))), "- -a");
        assert_eq!(p(neg(num(-1))), "- -1");
        assert_eq!(p(unary(UnOp::Pos, unary(UnOp::Pos, id("a")))), "+ +a");
        assert_eq!(
            p(binary(BinOp::Sub, id("a"), neg(id("b")))),
            "a - -b"
        );
        assert_eq!(p(binary(BinOp::Sub, id("a"), num(-1))), "a - -1");
        assert_eq!(
            p(binary(BinOp::Add, id("a"), unary(UnOp::Pos, id("b")))),
            "a + +b"
        );
        // A negative literal is a unary expression, so it is parenthesized where one is.
        assert_eq!(p(member(id("a"), "b")), "a.b");
        assert_eq!(p(binary(BinOp::Mul, num(-1), id("b"))), "-1 * b");
        assert_eq!(p(neg(binary(BinOp::Add, id("a"), id("b")))), "-(a + b)");
        assert_eq!(p(not(binary(BinOp::Eq, id("a"), id("b")))), "!(a == b)");
        assert_eq!(p(bit_not(id("a"))), "~a");
        assert_eq!(p(type_of(id("a"))), "typeof a");
        assert_eq!(
            p(binary(BinOp::StrictEq, type_of(id("a")), string("number"))),
            "typeof a === \"number\""
        );
    }

    #[test]
    fn nested_ternaries() {
        let a = || id("a");
        let b = || id("b");
        let c = || id("c");
        let d = || id("d");
        let e = || id("e");
        // Right associative: a nested alternate needs no parentheses.
        assert_eq!(
            p(cond(a(), b(), cond(c(), d(), e()))),
            "a ? b : c ? d : e"
        );
        // A nested consequent is unambiguous too.
        assert_eq!(
            p(cond(a(), cond(b(), c(), d()), e())),
            "a ? b ? c : d : e"
        );
        // A conditional test must be parenthesized.
        assert_eq!(
            p(cond(cond(a(), b(), c()), d(), e())),
            "(a ? b : c) ? d : e"
        );
        // A conditional inside anything tighter gets parentheses.
        assert_eq!(
            p(binary(BinOp::Add, cond(a(), b(), c()), d())),
            "(a ? b : c) + d"
        );
        assert_eq!(
            p(cond(binary(BinOp::Lt, a(), b()), num(1), num(2))),
            "a < b ? 1 : 2"
        );
    }

    #[test]
    fn assignments_are_right_associative_and_nest_under_conditionals() {
        assert_eq!(p(assign(id("a"), id("b"))), "a = b");
        assert_eq!(
            p(assign(id("a"), assign(id("b"), num(1)))),
            "a = b = 1"
        );
        assert_eq!(
            p(assign(member(id("a"), "b"), num(1))),
            "a.b = 1"
        );
        assert_eq!(
            p(assign(index(id("a"), num(0)), num(1))),
            "a[0] = 1"
        );
        assert_eq!(
            p(binary(BinOp::Add, assign(id("a"), num(1)), num(2))),
            "(a = 1) + 2"
        );
        assert_eq!(
            p(cond(id("c"), assign(id("a"), num(1)), num(2))),
            "c ? a = 1 : 2"
        );
        assert_eq!(
            p(compound_assign(BinOp::Add, id("a"), num(1))),
            "a += 1"
        );
        assert_eq!(
            p(compound_assign(BinOp::UShr, id("a"), num(0))),
            "a >>>= 0"
        );
        assert_eq!(
            p(compound_assign(BinOp::Sub, id("a"), num(-1))),
            "a -= -1"
        );
    }

    #[test]
    fn object_keys_are_quoted_only_when_needed() {
        assert_eq!(p(object_of(vec![])), "{}");
        assert_eq!(
            p(object_of(vec![("x", num(1)), ("y", num(2))])),
            "{ x: 1, y: 2 }"
        );
        assert_eq!(p(object_of(vec![("$t", num(0))])), "{ $t: 0 }");
        assert_eq!(
            p(object_of(vec![("$t", num(1)), ("_0", id("a"))])),
            "{ $t: 1, _0: a }"
        );
        assert_eq!(p(object_of(vec![("a b", num(1))])), "{ \"a b\": 1 }");
        assert_eq!(p(object_of(vec![("0", num(1))])), "{ \"0\": 1 }");
        assert_eq!(p(object_of(vec![("class", num(1))])), "{ \"class\": 1 }");
        assert_eq!(
            p(object_of(vec![("a", cond(id("c"), num(1), num(2)))])),
            "{ a: c ? 1 : 2 }"
        );
        // A comma expression in a value position needs parentheses.
        assert_eq!(
            p(object_of(vec![("a", seq(vec![num(1), num(2)]))])),
            "{ a: (1, 2) }"
        );
    }

    #[test]
    fn arrays_and_cells() {
        assert_eq!(p(array(vec![])), "[]");
        assert_eq!(p(array(vec![num(1), string("a"), undefined()])), "[1, \"a\", undefined]");
        assert_eq!(p(cell(num(0))), "{ v: 0 }");
        assert_eq!(p(cell_get(id("c"))), "c.v");
        assert_eq!(
            p(assign(cell_get(id("c")), num(1))),
            "c.v = 1"
        );
    }

    #[test]
    fn arrows_and_accessors() {
        assert_eq!(p(arrow(vec![], num(1))), "() => 1");
        assert_eq!(p(arrow(params(&["x"]), id("x"))), "(x) => x");
        assert_eq!(
            p(arrow(params(&["x", "y"]), binary(BinOp::Add, id("x"), id("y")))),
            "(x, y) => x + y"
        );
        // An object body has to be parenthesized.
        assert_eq!(
            p(arrow(vec![], object_of(vec![("a", num(1))]))),
            "() => ({ a: 1 })"
        );
        assert_eq!(
            p(accessor(member(id("s"), "field"), "v")),
            "{ get: () => s.field, set: (v) => s.field = v }"
        );
        assert_eq!(
            p(accessor(index(id("t"), num(0)), "v")),
            "{ get: () => t[0], set: (v) => t[0] = v }"
        );
        assert_eq!(
            p(method_call(id("r"), "get", vec![])),
            "r.get()"
        );
    }

    #[test]
    fn new_expressions() {
        assert_eq!(p(new(id("Error"), vec![string("boom")])), "new Error(\"boom\")");
        assert_eq!(p(new(member(id("a"), "B"), vec![])), "new a.B()");
        // A call in the callee would swallow the argument list.
        assert_eq!(p(new(call(id("f"), vec![]), vec![])), "new (f())()");
        assert_eq!(p(member(new(id("A"), vec![]), "x")), "new A().x");
    }

    #[test]
    fn sequences_and_raw() {
        assert_eq!(p(seq(vec![num(1), num(2), num(3)])), "1, 2, 3");
        assert_eq!(
            p(binary(BinOp::Add, seq(vec![num(1), num(2)]), num(3))),
            "(1, 2) + 3"
        );
        assert_eq!(p(raw_expr("a ?? b")), "a ?? b");
        // Raw is always parenthesized when nested, since its precedence is unknown.
        assert_eq!(p(binary(BinOp::Add, raw_expr("a ?? b"), num(1))), "(a ?? b) + 1");
    }

    #[test]
    fn rt_calls_use_the_shim_namespace() {
        assert_eq!(
            p(rt_call("js_log_i32", vec![id("x")])),
            "__rt.js_log_i32(x)"
        );
        assert_eq!(
            p(rt_call("js_abort", vec![string("nope")])),
            "__rt.js_abort(\"nope\")"
        );
    }

    #[test]
    fn statement_position_hazards_get_parentheses() {
        // An expression statement that starts with a brace reads as a block.
        assert_eq!(
            stmt_to_string(&expr_stmt(object_of(vec![("a", num(1))]))),
            "({ a: 1 });\n"
        );
        assert_eq!(
            stmt_to_string(&expr_stmt(raw_expr("function () {}"))),
            "(function () {});\n"
        );
        // An arrow starts with a paren, so it is already fine.
        assert_eq!(
            stmt_to_string(&expr_stmt(call(arrow(vec![], num(1)), vec![]))),
            "(() => 1)();\n"
        );
        // A name that merely starts with `function` is not a hazard.
        assert_eq!(stmt_to_string(&expr_stmt(id("functional"))), "functional;\n");
    }

    #[test]
    fn declarations_and_simple_statements() {
        assert_eq!(stmt_to_string(&let_("x", num(1))), "let x = 1;\n");
        assert_eq!(stmt_to_string(&let_uninit("x")), "let x;\n");
        assert_eq!(
            stmt_to_string(&lets(vec![
                ("a".into(), Some(num(1))),
                ("b".into(), None),
                ("c".into(), Some(id("d"))),
            ])),
            "let a = 1, b, c = d;\n"
        );
        assert_eq!(stmt_to_string(&const_("k", num(2))), "const k = 2;\n");
        assert_eq!(stmt_to_string(&ret(id("x"))), "return x;\n");
        assert_eq!(stmt_to_string(&ret_void()), "return;\n");
        assert_eq!(stmt_to_string(&Stmt::Return(Some(undefined()))), "return undefined;\n");
        assert_eq!(stmt_to_string(&continue_("L")), "continue L;\n");
        assert_eq!(stmt_to_string(&Stmt::Continue(None)), "continue;\n");
        assert_eq!(stmt_to_string(&break_("L")), "break L;\n");
        assert_eq!(stmt_to_string(&Stmt::Break(None)), "break;\n");
        assert_eq!(
            stmt_to_string(&throw_error("unreachable")),
            "throw new Error(\"unreachable\");\n"
        );
        assert_eq!(
            stmt_to_string(&assign_stmt(member(id("a"), "b"), num(1))),
            "a.b = 1;\n"
        );
    }

    #[test]
    fn comments_are_single_line() {
        assert_eq!(stmt_to_string(&comment("bb0")), "// bb0\n");
        assert_eq!(
            stmt_to_string(&comment("first\nsecond\r\nthird")),
            "// first second  third\n"
        );
        assert_eq!(
            stmt_to_string(&comment("a\u{2028}b")),
            "// a b\n"
        );
    }

    #[test]
    fn if_else_chains_are_readable() {
        let stmt = if_else(
            binary(BinOp::Lt, id("x"), num(0)),
            vec![ret(num(-1))],
            vec![if_else(
                binary(BinOp::Gt, id("x"), num(0)),
                vec![ret(num(1))],
                vec![ret(num(0))],
            )],
        );
        assert_eq!(
            stmt_to_string(&stmt),
            "\
if (x < 0) {
  return -1;
} else if (x > 0) {
  return 1;
} else {
  return 0;
}
"
        );
    }

    #[test]
    fn empty_bodies_collapse() {
        assert_eq!(stmt_to_string(&if_(id("c"), vec![])), "if (c) {}\n");
        assert_eq!(
            stmt_to_string(&function("f", vec![], vec![])),
            "function f() {}\n"
        );
        assert_eq!(stmt_to_string(&Stmt::Block(vec![])), "{}\n");
    }

    #[test]
    fn while_loops_nest_and_indent() {
        let stmt = while_(
            binary(BinOp::Lt, id("i"), num(10)),
            vec![
                expr_stmt(compound_assign(BinOp::Add, id("i"), num(1))),
                if_(
                    binary(BinOp::Eq, id("i"), num(5)),
                    vec![Stmt::Break(None)],
                ),
            ],
        );
        assert_eq!(
            stmt_to_string(&stmt),
            "\
while (i < 10) {
  i += 1;
  if (i == 5) {
    break;
  }
}
"
        );
    }

    #[test]
    fn switch_cases_are_flat_and_support_fallthrough_labels() {
        let stmt = switch(
            id("bb"),
            vec![
                case(vec![num(0)], vec![assign_stmt(id("x"), num(1))]),
                case(
                    vec![num(1), num(2)],
                    vec![assign_stmt(id("x"), num(2)), continue_("L")],
                ),
                default_case(vec![throw_error("unreachable")]),
            ],
        );
        assert_eq!(
            stmt_to_string(&stmt),
            "\
switch (bb) {
  case 0:
    x = 1;
  case 1:
  case 2:
    x = 2;
    continue L;
  default:
    throw new Error(\"unreachable\");
}
"
        );
    }

    #[test]
    fn labeled_statements_share_the_line() {
        assert_eq!(
            stmt_to_string(&labeled("L", while_(boolean(true), vec![Stmt::Break(None)]))),
            "L: while (true) {\n  break;\n}\n"
        );
        assert_eq!(
            stmt_to_string(&loop_forever("L", vec![Stmt::Break(None)])),
            "L: for (;;) {\n  break;\n}\n"
        );
        assert_eq!(
            stmt_to_string(&Stmt::LoopForever(None, vec![Stmt::Break(None)])),
            "for (;;) {\n  break;\n}\n"
        );
    }

    #[test]
    fn raw_statements_are_reindented() {
        let stmt = function("f", vec![], vec![raw_stmt("a();\nb();")]);
        assert_eq!(
            stmt_to_string(&stmt),
            "function f() {\n  a();\n  b();\n}\n"
        );
    }

    #[test]
    fn identifier_checks_and_sanitizing() {
        assert!(is_plain_ident("a"));
        assert!(is_plain_ident("_0"));
        assert!(is_plain_ident("$t"));
        assert!(is_plain_ident("a$b_1"));
        assert!(!is_plain_ident(""));
        assert!(!is_plain_ident("0a"));
        assert!(!is_plain_ident("a-b"));
        assert!(!is_plain_ident("a b"));
        assert!(!is_plain_ident("class"));

        assert_eq!(sanitize_ident("foo"), "foo");
        assert_eq!(sanitize_ident("my_crate::foo::bar"), "my_crate__foo__bar");
        assert_eq!(sanitize_ident("Foo<i32>::bar"), "Foo_i32___bar");
        assert_eq!(sanitize_ident("0start"), "_0start");
        assert_eq!(sanitize_ident(""), "_");
        assert_eq!(sanitize_ident("class"), "_class");
        assert_eq!(sanitize_ident("foo$1a2b"), "foo$1a2b");
    }

    #[test]
    fn programs_space_out_function_declarations() {
        let program = vec![
            comment("generated"),
            function("a", vec![], vec![ret(num(1))]),
            function("b", vec![], vec![ret(num(2))]),
        ];
        assert_eq!(
            program_to_string(&program),
            "\
// generated
function a() {
  return 1;
}

function b() {
  return 2;
}
"
        );
    }

    /// A comment introducing a declaration belongs to it: the blank line goes above the comment,
    /// never between the comment and the code it describes. This is how every item is printed —
    /// `item.rs` renders a `// <def path>` header and the declaration as a two statement program —
    /// so getting it wrong labels every emitted function with the name of the one before it.
    #[test]
    fn a_comment_travels_with_the_declaration_it_introduces() {
        let program = vec![
            function("a", vec![], vec![]),
            comment("about b"),
            comment("still about b"),
            function("b", vec![], vec![]),
            comment("about nothing in particular"),
            let_("c", num(1)),
        ];
        assert_eq!(
            program_to_string(&program),
            "\
function a() {}

// about b
// still about b
function b() {}

// about nothing in particular
let c = 1;
"
        );
    }

    /// The shape the MIR walk is expected to emit: locals up front, then a labeled forever
    /// loop around a switch on the block index.
    fn sample_program() -> Vec<Stmt> {
        let bb = || id("bb");
        let sum = || id("sum");
        let i = || id("i");

        let body = vec![
            comment("locals"),
            let_("bb", num(0)),
            let_uninit("sum"),
            let_uninit("i"),
            let_("cell", cell(num(0))),
            let_(
                "acc",
                accessor(member(id("self"), "count"), "v"),
            ),
            loop_forever(
                "L",
                vec![switch(
                    bb(),
                    vec![
                        case(
                            vec![num(0)],
                            vec![
                                comment("bb0: entry"),
                                assign_stmt(sum(), num(0)),
                                assign_stmt(i(), num(0)),
                                assign_stmt(bb(), num(1)),
                                continue_("L"),
                            ],
                        ),
                        case(
                            vec![num(1)],
                            vec![
                                assign_stmt(
                                    bb(),
                                    cond(binary(BinOp::Lt, i(), id("n")), num(2), num(3)),
                                ),
                                continue_("L"),
                            ],
                        ),
                        case(
                            vec![num(2)],
                            vec![
                                assign_stmt(
                                    sum(),
                                    i32_wrap(binary(BinOp::Add, sum(), imul(i(), num(2)))),
                                ),
                                assign_stmt(i(), i32_wrap(binary(BinOp::Add, i(), num(1)))),
                                assign_stmt(bb(), num(1)),
                                continue_("L"),
                            ],
                        ),
                        case(
                            vec![num(3)],
                            vec![
                                expr_stmt(rt_call("js_log_i32", vec![sum()])),
                                expr_stmt(rt_call(
                                    "js_log_str",
                                    vec![string("done \"ok\"\n")],
                                )),
                                expr_stmt(method_call(id("acc"), "set", vec![sum()])),
                                assign_stmt(cell_get(id("cell")), neg(num(-1))),
                                ret(sum()),
                            ],
                        ),
                        default_case(vec![throw_error("unreachable basic block")]),
                    ],
                )],
            ),
        ];

        vec![
            comment("generated by rustc_codegen_mires"),
            function("rust_entry", params(&["n", "self"]), body),
        ]
    }

    #[test]
    fn trampoline_renders_the_expected_shape() {
        let text = program_to_string(&sample_program());
        assert!(text.contains("L: for (;;) {"), "{text}");
        assert!(text.contains("    case 0:\n"), "{text}");
        assert!(text.contains("      bb = 1;\n"), "{text}");
        assert!(text.contains("      continue L;\n"), "{text}");
        assert!(text.contains("bb = i < n ? 2 : 3;"), "{text}");
        assert!(text.contains("sum = sum + Math.imul(i, 2) | 0;"), "{text}");
        assert!(
            text.contains("__rt.js_log_str(\"done \\\"ok\\\"\\n\");"),
            "{text}"
        );
        assert!(text.contains("cell.v = - -1;"), "{text}");
        assert!(
            text.contains("let acc = { get: () => self.count, set: (v) => self.count = v };"),
            "{text}"
        );
    }

    /// Proves the printer emits syntactically valid JavaScript. Skipped when node is absent.
    #[test]
    fn sample_program_passes_node_check() {
        let text = program_to_string(&sample_program());
        let path = std::env::temp_dir().join("jsast_sample_program.js");
        if std::fs::write(&path, &text).is_err() {
            return;
        }
        let output = match std::process::Command::new("node")
            .arg("--check")
            .arg(&path)
            .output()
        {
            Ok(output) => output,
            // No node on this machine, nothing to prove.
            Err(_) => return,
        };
        assert!(
            output.status.success(),
            "node --check failed:\n{}\n--- source ---\n{text}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A grab bag of the printer's corners, checked by node rather than by eye.
    #[test]
    fn tricky_expressions_pass_node_check() {
        let exprs = vec![
            neg(neg(id("a"))),
            binary(BinOp::Sub, id("a"), num(-1)),
            method_call(num(1), "toString", vec![]),
            cond(cond(id("a"), id("b"), id("c")), id("d"), id("e")),
            binary(BinOp::BitOr, id("a"), binary(BinOp::And, id("b"), id("c"))),
            object_of(vec![("class", num(1)), ("a b", num(2)), ("$t", num(3))]),
            member(id("a"), "default"),
            arrow(vec![], object_of(vec![("a", num(1))])),
            accessor(member(id("s"), "field"), "v"),
            new(call(id("f"), vec![]), vec![]),
            seq(vec![num(1), num(2)]),
            string("quotes \" backslash \\ newline \n tab \t nul \u{0} unicode \u{2028}"),
            call(arrow(vec![], num(1)), vec![]),
            binary(BinOp::Add, assign(id("a"), num(1)), num(2)),
            i32_wrap(binary(BinOp::Add, id("a"), id("b"))),
            sign_extend(8, binary(BinOp::Add, id("a"), id("b"))),
        ];
        let mut program = vec![let_uninit("a"), let_uninit("b"), let_uninit("c")];
        program.push(let_uninit("d"));
        program.push(let_uninit("e"));
        program.push(let_uninit("s"));
        program.push(let_uninit("f"));
        for expr in exprs {
            program.push(expr_stmt(expr));
        }
        let text = program_to_string(&program);
        let path = std::env::temp_dir().join("jsast_tricky_exprs.js");
        if std::fs::write(&path, &text).is_err() {
            return;
        }
        let output = match std::process::Command::new("node")
            .arg("--check")
            .arg(&path)
            .output()
        {
            Ok(output) => output,
            Err(_) => return,
        };
        assert!(
            output.status.success(),
            "node --check failed:\n{}\n--- source ---\n{text}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Runs a program through node and compares stdout, so the precedence table is checked
    /// against real evaluation and not just against expected text.
    #[test]
    fn precedence_matches_node_evaluation() {
        let cases: Vec<(Expr, &str)> = vec![
            (binary(BinOp::Add, num(1), binary(BinOp::Mul, num(2), num(3))), "7"),
            (binary(BinOp::Mul, binary(BinOp::Add, num(1), num(2)), num(3)), "9"),
            (binary(BinOp::Sub, num(1), binary(BinOp::Sub, num(2), num(3))), "2"),
            (binary(BinOp::Sub, num(1), num(-2)), "3"),
            (neg(neg(num(3))), "3"),
            (
                binary(BinOp::Add, binary(BinOp::Shl, num(1), num(2)), num(1)),
                "5",
            ),
            (
                binary(BinOp::Shl, binary(BinOp::Add, num(1), num(2)), num(1)),
                "6",
            ),
            (
                binary(BinOp::BitOr, num(1), binary(BinOp::And, num(0), num(2))),
                "1",
            ),
            (cond(boolean(false), num(1), cond(boolean(true), num(2), num(3))), "2"),
            (
                i32_wrap(binary(BinOp::Add, raw_num("2147483647"), num(1))),
                "-2147483648",
            ),
            (u32_wrap(neg(num(1))), "4294967295"),
            (sign_extend(8, num(255)), "-1"),
            (imul(num(65536), num(65536)), "0"),
            (member(object_of(vec![("class", num(7))]), "class"), "7"),
            (method_call(num(255), "toString", vec![num(16)]), "ff"),
        ];

        let mut program = Vec::new();
        for (expr, _) in &cases {
            program.push(expr_stmt(method_call(
                id("console"),
                "log",
                vec![expr.clone()],
            )));
        }
        let text = program_to_string(&program);
        let path = std::env::temp_dir().join("jsast_precedence_eval.js");
        if std::fs::write(&path, &text).is_err() {
            return;
        }
        let output = match std::process::Command::new("node").arg(&path).output() {
            Ok(output) => output,
            Err(_) => return,
        };
        assert!(
            output.status.success(),
            "node failed:\n{}\n--- source ---\n{text}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), cases.len(), "{stdout}");
        for (line, (expr, expected)) in lines.iter().zip(&cases) {
            assert_eq!(line, expected, "for `{}`", expr_to_string(expr));
        }
    }

    /// The marker is invisible in the text, which is what lets `js-source-map=on` describe the
    /// output without changing it. Everything below prints exactly as it does without the markers.
    #[test]
    fn location_markers_print_nothing() {
        let program = vec![function(
            "f",
            vec![],
            vec![
                loc("a.rs", 1, 0),
                let_("x", num(1)),
                loc("a.rs", 2, 4),
                loc("a.rs", 2, 4),
                ret(id("x")),
            ],
        )];
        assert_eq!(
            program_to_string(&program),
            "function f() {\n  let x = 1;\n  return x;\n}\n"
        );
        // Even an `else if` chain, which is a pattern match over the branch it prints.
        let chain = if_else(
            id("a"),
            vec![ret(num(1))],
            vec![loc("a.rs", 9, 2), if_(id("b"), vec![ret(num(2))])],
        );
        assert_eq!(
            stmt_to_string(&chain),
            "if (a) {\n  return 1;\n} else if (b) {\n  return 2;\n}\n"
        );
    }

    #[test]
    fn mappings_point_at_the_statements_the_markers_introduced() {
        let program = vec![function(
            "f$h00",
            vec![],
            vec![loc("a.rs", 7, 4), let_("acc$1", num(1)), loc("a.rs", 8, 4), ret(id("acc$1"))],
        )];
        let names = BTreeMap::from([
            ("f$h00".to_string(), "the_fn".to_string()),
            ("acc$1".to_string(), "acc".to_string()),
        ]);
        let (text, mappings) =
            program_to_string_mapped(&program, names, Some(Loc { file: "a.rs".into(), line: 6, col: 0 }));
        assert_eq!(text, "function f$h00() {\n  let acc$1 = 1;\n  return acc$1;\n}\n");

        let at = |mapping: &Mapping| {
            (mapping.gen_line, mapping.gen_col, mapping.loc.line, mapping.loc.col, mapping.name.clone())
        };
        assert_eq!(
            mappings.iter().map(at).collect::<Vec<_>>(),
            vec![
                // The declaration itself, from the location the item carried in.
                (0, 0, 6, 0, None),
                // `f$h00` at column 9, named through the opening parenthesis at column 14, and a
                // fresh mapping right after it so that the name does not bleed onto the
                // parameter list.
                (0, 9, 6, 0, Some("the_fn".to_string())),
                (0, 15, 6, 0, None),
                // The `let`, then its declared name.
                (1, 2, 7, 4, None),
                (1, 6, 7, 4, Some("acc".to_string())),
                (1, 11, 7, 4, None),
                (2, 2, 8, 4, None),
            ]
        );
    }

    /// The column a mapping carries counts UTF-16 code units, not bytes: that is the unit the
    /// format is defined in and the one a browser measures the line with.
    #[test]
    fn generated_columns_are_counted_in_utf16_units() {
        let program = vec![
            Stmt::Loc(Loc { file: "a.rs".into(), line: 3, col: 1 }),
            lets(vec![
                ("a".to_string(), Some(string("caf\u{e9}\u{1f600}"))),
                ("b".to_string(), None),
            ]),
        ];
        let names = BTreeMap::from([("b".to_string(), "beta".to_string())]);
        let (text, mappings) = program_to_string_mapped(&program, names, None);
        assert_eq!(text, "let a = \"caf\u{e9}\u{1f600}\", b;\n");

        // `b` is the 19th UTF-16 unit of the line: the astral emoji before it counts as two, and
        // the accented letter as one, where the byte offset would say 23.
        let named = mappings.iter().find(|m| m.name.is_some()).expect("a named mapping");
        assert_eq!((named.gen_line, named.gen_col), (0, 18));
    }

    #[test]
    fn visit_idents_sees_references_but_not_names() {
        // A function that calls another item, reads a property of the same name as a third item,
        // and builds an object whose key looks like an item too.
        let decl = function(
            "caller",
            vec!["_1".to_string()],
            vec![
                let_("_0", call(id("callee"), vec![id("_1")])),
                expr_stmt(assign(member(id("_0"), "callee"), num(1))),
                expr_stmt(assign(id("_0"), object(vec![("callee".to_string(), id("other"))]))),
                ret(id("_0")),
            ],
        );

        let mut seen = Vec::new();
        visit_idents(&decl, &mut |name| seen.push(name.to_owned()));

        // `caller` (its own name), `_0` (a declaration) and the two `callee` *keys* are absent;
        // the two genuine references are present.
        assert_eq!(seen, vec!["callee", "_1", "_0", "_0", "other", "_0"]);
        assert!(!seen.contains(&"caller".to_string()));
    }

    #[test]
    fn visit_idents_reaches_into_every_nested_position() {
        let decl = function(
            "f",
            vec![],
            vec![
                if_else(
                    id("a"),
                    vec![switch(
                        id("b"),
                        vec![case(vec![id("c")], vec![expr_stmt(id("d"))])],
                    )],
                    vec![while_(id("e"), vec![throw(id("g"))])],
                ),
                expr_stmt(arrow_block(vec![], vec![expr_stmt(id("h"))])),
                expr_stmt(arrow(vec![], cond(id("i"), id("j"), id("k")))),
                expr_stmt(array(vec![index(id("l"), id("m"))])),
            ],
        );

        let mut seen = Vec::new();
        visit_idents(&decl, &mut |name| seen.push(name.to_owned()));
        assert_eq!(seen, ["a", "b", "c", "d", "e", "g", "h", "i", "j", "k", "l", "m"]);
    }

    // ---------------------------------------------------------------------------------------
    // Imports (`-Cllvm-args=js-modules=esm`)
    // ---------------------------------------------------------------------------------------

    #[test]
    fn an_import_prints_both_specifier_styles() {
        let namespace = import_namespace("__rt", "./shim.js");
        assert_eq!(stmt_to_string(&namespace), "import * as __rt from \"./shim.js\";\n");

        let named = import_named(
            vec![
                ("insert".to_string(), "_$insert".to_string()),
                ("template".to_string(), "template".to_string()),
            ],
            "mires-dom",
        );
        assert_eq!(
            stmt_to_string(&named),
            "import { insert as _$insert, template } from \"mires-dom\";\n"
        );
    }

    #[test]
    fn an_import_elides_the_alias_of_a_binding_that_keeps_its_name() {
        let named = import_named(vec![("effect".to_string(), "effect".to_string())], "./dom.js");
        assert_eq!(stmt_to_string(&named), "import { effect } from \"./dom.js\";\n");
    }

    #[test]
    fn a_compact_import_keeps_only_the_separators_it_needs() {
        let namespace = import_namespace("__rt", "./shim.js");
        assert_eq!(compact(&namespace), "import*as __rt from\"./shim.js\";");

        let named = import_named(
            vec![
                ("a".to_string(), "a".to_string()),
                ("b".to_string(), "c".to_string()),
            ],
            "m",
        );
        assert_eq!(compact(&named), "import{a,b as c}from\"m\";");
    }

    /// A compact program still ends every top level declaration with a newline, an import
    /// included: an object file's `//# rcgjs:` footer has to start a line of its own.
    #[test]
    fn a_compact_program_keeps_a_newline_after_an_import() {
        let program = [
            import_namespace("__rt", "./shim.js"),
            function("f", vec![], vec![ret(num(1))]),
        ];
        assert_eq!(
            program_to_string_styled(&program, Style::Compact),
            "import*as __rt from\"./shim.js\";\nfunction f(){return 1}\n"
        );
    }

    /// An import is a declaration, so a readable program spaces it out from what follows.
    #[test]
    fn a_readable_program_spaces_an_import_out_like_any_declaration() {
        let program = [
            import_namespace("__rt", "./shim.js"),
            import_named(vec![("x".to_string(), "x".to_string())], "./x.js"),
            function("f", vec![], vec![]),
        ];
        assert_eq!(
            program_to_string(&program),
            "import * as __rt from \"./shim.js\";\n\nimport { x } from \"./x.js\";\n\n\
             function f() {}\n"
        );
    }

    /// The specifiers are declarations, not references. An import that reported them would name
    /// itself, and `link.rs` would root every import ever emitted instead of dropping the unused
    /// ones.
    #[test]
    fn an_import_mentions_no_identifier() {
        let mut seen: Vec<String> = Vec::new();
        for import in [
            import_namespace("__rt", "./shim.js"),
            import_named(vec![("a".to_string(), "b".to_string())], "m"),
        ] {
            visit_idents(&import, &mut |name| seen.push(name.to_owned()));
        }
        assert!(seen.is_empty(), "an import reported {seen:?}");
    }

    #[test]
    fn an_import_source_is_escaped_like_any_string() {
        let import = import_namespace("m", "./a\"b.js");
        assert_eq!(stmt_to_string(&import), "import * as m from \"./a\\\"b.js\";\n");
    }

    // ---------------------------------------------------------------------------------------
    // Compact printing (`-Cllvm-args=js-minify`)
    // ---------------------------------------------------------------------------------------

    fn compact(stmt: &Stmt) -> String {
        stmt_to_string_styled(stmt, Style::Compact)
    }

    #[test]
    fn compact_output_drops_the_whitespace_that_only_separates_tokens() {
        let decl = function(
            "f",
            params(&["a", "b"]),
            vec![
                let_("c", binary(BinOp::Add, id("a"), id("b"))),
                assign_stmt(id("c"), cond(id("a"), num(1), num(2))),
                ret(id("c")),
            ],
        );
        assert_eq!(compact(&decl), "function f(a,b){let c=a+b;c=a?1:2;return c}");
    }

    #[test]
    fn compact_output_puts_a_space_back_where_tokens_would_fuse() {
        // `a - -1` must not become `a--1`, and `return a` must not become `returna`.
        assert_eq!(
            compact(&expr_stmt(binary(BinOp::Sub, id("a"), num(-1)))),
            "a- -1;"
        );
        assert_eq!(compact(&expr_stmt(neg(neg(id("a"))))), "- -a;");
        assert_eq!(compact(&ret(id("a"))), "return a;");
        // ... but only where they would: a `return` of something that starts with punctuation
        // needs nothing.
        assert_eq!(compact(&ret(num(-1))), "return-1;");
        assert_eq!(compact(&throw(new(id("Error"), vec![string("x")]))), "throw new Error(\"x\");");
        assert_eq!(compact(&expr_stmt(type_of(id("a")))), "typeof a;");
    }

    #[test]
    fn compact_output_elides_the_semicolon_before_a_brace() {
        let decl = function("f", vec![], vec![assign_stmt(id("a"), num(1))]);
        assert_eq!(compact(&decl), "function f(){a=1}");
    }

    #[test]
    fn compact_output_drops_braces_only_around_a_statement_that_cannot_hold_an_if() {
        assert_eq!(
            compact(&if_(id("a"), vec![assign_stmt(id("b"), num(1))])),
            "if(a)b=1;"
        );
        assert_eq!(
            compact(&if_else(id("a"), vec![ret(num(1))], vec![ret(num(2))])),
            "if(a){return 1}else return 2;"
        );
        // A `let` is not a statement the grammar allows there, and an `if` would catch the `else`.
        assert_eq!(
            compact(&if_(id("a"), vec![let_("b", num(1))])),
            "if(a){let b=1}"
        );
        assert_eq!(
            compact(&if_else(
                id("a"),
                vec![if_(id("b"), vec![assign_stmt(id("c"), num(1))])],
                vec![assign_stmt(id("d"), num(2))],
            )),
            "if(a){if(b)c=1}else d=2;"
        );
        // An `else if` chain stays a chain.
        assert_eq!(
            compact(&if_else(
                id("a"),
                vec![ret(num(1))],
                vec![if_else(id("b"), vec![ret(num(2))], vec![ret(num(3))])],
            )),
            "if(a){return 1}else if(b){return 2}else return 3;"
        );
    }

    #[test]
    fn compact_output_spells_the_short_literals() {
        assert_eq!(compact(&expr_stmt(boolean(true))), "!0;");
        assert_eq!(compact(&expr_stmt(boolean(false))), "!1;");
        assert_eq!(compact(&expr_stmt(undefined())), "void 0;");
        // `!0` binds like a unary expression, so an access over one gains parentheses.
        assert_eq!(compact(&expr_stmt(member(boolean(true), "x"))), "(!0).x;");
        assert_eq!(compact(&expr_stmt(member(undefined(), "x"))), "(void 0).x;");
    }

    #[test]
    fn compact_output_unwraps_a_single_arrow_parameter() {
        assert_eq!(compact(&expr_stmt(arrow(params(&["v"]), id("v")))), "v=>v;");
        assert_eq!(compact(&expr_stmt(arrow(params(&["a", "b"]), id("a")))), "(a,b)=>a;");
        assert_eq!(compact(&expr_stmt(arrow(vec![], num(1)))), "()=>1;");
        assert_eq!(
            compact(&expr_stmt(accessor(member(id("s"), "f"), "v"))),
            "({get:()=>s.f,set:v=>s.f=v});"
        );
    }

    #[test]
    fn compact_output_keeps_a_newline_per_top_level_item() {
        let program = [
            function("f", vec![], vec![ret(num(1))]),
            let_("g", num(2)),
            function("h", vec![], vec![]),
        ];
        assert_eq!(
            program_to_string_styled(&program, Style::Compact),
            "function f(){return 1}\nlet g=2;\nfunction h(){}\n"
        );
    }

    #[test]
    fn compact_output_omits_comments_entirely() {
        let program = [comment("a header"), function("f", vec![], vec![])];
        assert_eq!(program_to_string_styled(&program, Style::Compact), "function f(){}\n");
    }

    /// The real check: the same program, printed both ways, has to *do* the same thing.
    ///
    /// Every rule compact printing adds is a rule about what may be left out, and the only way to
    /// be sure none of them changed a meaning is to run both. The cases are chosen for the hazards:
    /// dangling `else`, semicolon elision at the end of every kind of block, the `- -` and `+ +`
    /// fusions, `!0`/`void 0` in positions where precedence matters, labelled loops.
    #[test]
    fn compact_and_readable_programs_evaluate_the_same() {
        let log = |expr: Expr| expr_stmt(method_call(id("console"), "log", vec![expr]));
        let program = vec![
            function(
                "f",
                params(&["a", "b"]),
                vec![
                    let_("c", binary(BinOp::Sub, id("a"), num(-1))),
                    Stmt::If(
                        binary(BinOp::Gt, id("c"), num(0)),
                        vec![Stmt::If(id("b"), vec![assign_stmt(id("c"), num(10))], None)],
                        Some(vec![assign_stmt(id("c"), num(20))]),
                    ),
                    labeled(
                        "outer",
                        while_(
                            binary(BinOp::Lt, id("c"), num(30)),
                            vec![
                                Stmt::If(
                                    binary(BinOp::Eq, id("c"), num(25)),
                                    vec![Stmt::Break(Some("outer".to_string()))],
                                    None,
                                ),
                                expr_stmt(compound_assign(BinOp::Add, id("c"), num(1))),
                            ],
                        ),
                    ),
                    switch(
                        id("c"),
                        vec![
                            case(vec![num(25)], vec![assign_stmt(id("c"), num(100))]),
                            default_case(vec![assign_stmt(id("c"), num(200))]),
                        ],
                    ),
                    ret(id("c")),
                ],
            ),
            log(call(id("f"), vec![num(1), boolean(true)])),
            log(call(id("f"), vec![num(1), boolean(false)])),
            log(call(id("f"), vec![num(-5), boolean(true)])),
            log(unary(UnOp::Neg, neg(num(3)))),
            log(binary(BinOp::Add, num(1), unary(UnOp::Pos, num(2)))),
            log(cond(boolean(false), undefined(), boolean(true))),
            log(type_of(undefined())),
            log(object_of(vec![("a", boolean(true)), ("b", undefined())])),
            log(array(vec![boolean(false), num(-1), string("x\"y")])),
            log(call(arrow(params(&["v"]), binary(BinOp::Mul, id("v"), num(2))), vec![num(21)])),
        ];

        let readable = program_to_string(&program);
        let compact = program_to_string_styled(&program, Style::Compact);
        assert!(!compact.contains("\n  "), "compact output is indented:\n{compact}");

        let run = |text: &str, name: &str| -> Option<String> {
            let path = std::env::temp_dir().join(name);
            std::fs::write(&path, text).ok()?;
            let output = std::process::Command::new("node").arg(&path).output().ok()?;
            assert!(
                output.status.success(),
                "node failed:\n{}\n--- source ---\n{text}",
                String::from_utf8_lossy(&output.stderr)
            );
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        };

        let (Some(a), Some(b)) = (
            run(&readable, "jsast_style_readable.js"),
            run(&compact, "jsast_style_compact.js"),
        ) else {
            return;
        };
        assert_eq!(a, b, "--- readable ---\n{readable}\n--- compact ---\n{compact}");
    }
}
