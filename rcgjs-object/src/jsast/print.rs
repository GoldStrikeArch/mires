//! JavaScript pretty-printer, compact emitter, and source-map offset resolver.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::ast::{
    ArrowBody, BinOp, CompoundOp, Expr, ImportSpecifiers, Loc, Mapping, Stmt, SwitchCase, UnOp,
    PREC_ASSIGN, PREC_CALL, PREC_COND, PREC_UNARY,
};
use super::build::{is_plain_ident, lower_logical_assignment};
use crate::opts::EcmaTarget;

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
/// `need_space` is the whole of that rule and is checked *after* the second token is written, on
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
    program_to_string_styled_for_target(items, style, EcmaTarget::Latest)
}

/// Renders a program in the requested style and JavaScript syntax target.
pub fn program_to_string_styled_for_target(
    items: &[Stmt],
    style: Style,
    target: EcmaTarget,
) -> String {
    let mut printer = Printer::new(style, target);
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
/// A [`Style::Compact`] program tracks positions exactly like readable output. The minifier moves
/// the map's emitted-name keys with local renames and rebases columns after link-time item renames.
pub fn program_to_string_mapped_styled(
    items: &[Stmt],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
    style: Style,
) -> (String, Vec<Mapping>) {
    program_to_string_mapped_styled_for_target(items, names, at, style, EcmaTarget::Latest)
}

/// Renders a mapped program in the requested style and JavaScript syntax target.
pub fn program_to_string_mapped_styled_for_target(
    items: &[Stmt],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
    style: Style,
    target: EcmaTarget,
) -> (String, Vec<Mapping>) {
    render_mapped(items, names, at, style, target)
}

/// Renders a mapped program from statement references without cloning the statements.
pub fn program_refs_to_string_mapped_styled_for_target(
    items: &[&Stmt],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
    style: Style,
    target: EcmaTarget,
) -> (String, Vec<Mapping>) {
    render_mapped(items, names, at, style, target)
}

fn render_mapped<S: Borrow<Stmt>>(
    items: &[S],
    names: BTreeMap<String, String>,
    at: Option<Loc>,
    style: Style,
    target: EcmaTarget,
) -> (String, Vec<Mapping>) {
    let mut printer = Printer::new(style, target);
    printer.maps = Some(MapState {
        names,
        pending: at,
        current: None,
        named: false,
        raw: Vec::new(),
    });
    print_program(&mut printer, items);
    let state = printer.maps.take().expect("just installed");
    let mappings = resolve_offsets(&printer.out, state.raw);
    (printer.out, mappings)
}

fn print_program<S: Borrow<Stmt>>(printer: &mut Printer, items: &[S]) {
    for (i, item) in items.iter().enumerate() {
        let item = item.borrow();
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
        let ends_group = i > 0 && is_decl(items[i - 1].borrow());
        // ... and never inside one: a comment and the declaration it introduces are one unit.
        let inside_group = i > 0 && introduces_decl(items, i - 1);
        if i > 0 && (starts_group || ends_group) && !inside_group {
            printer.out.push('\n');
        }
        printer.stmt(item);
    }
}

/// Whether item `i` is a comment (or a run of them) introducing a declaration.
fn introduces_decl<S: Borrow<Stmt>>(items: &[S], i: usize) -> bool {
    if !matches!(items[i].borrow(), Stmt::Comment(_)) {
        return false;
    }
    items[i + 1..]
        .iter()
        .map(Borrow::borrow)
        .find(|item| !matches!(item, Stmt::Comment(_)))
        .is_some_and(is_decl)
}

/// Renders a single statement, including its trailing newline.
pub fn stmt_to_string(stmt: &Stmt) -> String {
    stmt_to_string_styled(stmt, Style::Readable)
}

/// Renders a single statement in the requested [`Style`].
pub fn stmt_to_string_styled(stmt: &Stmt, style: Style) -> String {
    stmt_to_string_styled_for_target(stmt, style, EcmaTarget::Latest)
}

/// Renders a statement in the requested style and JavaScript syntax target.
pub fn stmt_to_string_styled_for_target(stmt: &Stmt, style: Style, target: EcmaTarget) -> String {
    let mut printer = Printer::new(style, target);
    printer.stmt(stmt);
    printer.out
}

/// Renders a single expression.
pub fn expr_to_string(expr: &Expr) -> String {
    expr_to_string_styled(expr, Style::Readable)
}

/// Renders a single expression in the requested [`Style`].
pub fn expr_to_string_styled(expr: &Expr, style: Style) -> String {
    expr_to_string_styled_for_target(expr, style, EcmaTarget::Latest)
}

/// Renders an expression in the requested style and JavaScript syntax target.
pub fn expr_to_string_styled_for_target(expr: &Expr, style: Style, target: EcmaTarget) -> String {
    let mut printer = Printer::new(style, target);
    printer.expr(expr, 0);
    printer.out
}

/// Whether a statement is a top level declaration, which [`print_program`] spaces out.
fn is_decl(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::FunctionDecl { .. } | Stmt::Import { .. })
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
    target: EcmaTarget,
}

impl Printer {
    fn new(style: Style, target: EcmaTarget) -> Self {
        Printer {
            out: String::new(),
            indent: 0,
            maps: None,
            style,
            target,
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
        let before = self.out[..start]
            .chars()
            .next_back()
            .expect("start is past a character");
        let after = self.out[start..]
            .chars()
            .next()
            .expect("start is a character boundary");
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
        let Some(loc) = state.pending.take() else {
            return;
        };
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
        state
            .raw
            .push((offset, loc.clone(), Some(original.clone())));
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
            Expr::BigInt(digits) => self.bigint_expr(digits),
            Expr::Str(s) => write_js_string(&mut self.out, s),
            // `!0`/`!1` are the shortest spellings of the booleans, and `void 0` of `undefined`,
            // which is also how this backend spells unit and every zero sized type.
            Expr::Bool(b) => self.out.push_str(match (self.compact(), b) {
                (false, true) => "true",
                (false, false) => "false",
                (true, true) => "!0",
                (true, false) => "!1",
            }),
            Expr::Undefined => self.out.push_str(if self.compact() {
                "void 0"
            } else {
                "undefined"
            }),
            Expr::Null => self.out.push_str("null"),
            Expr::Ident(name) => self.out.push_str(name),
            Expr::Member(obj, name) => self.member_expr(obj, name),
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
            Expr::New(callee, args) => self.new_expr(callee, args),
            Expr::Unary(op, operand) => self.unary_expr(*op, operand),
            Expr::Binary(op, lhs, rhs) => self.binary_expr(*op, lhs, rhs),
            Expr::Assign(target, value) => {
                self.expr(target, PREC_CALL);
                self.spaced_around("=");
                self.expr(value, PREC_ASSIGN);
            }
            Expr::CompoundAssign(op, target, value) => {
                self.compound_assign_expr(*op, target, value);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test, PREC_COND + 1);
                self.spaced_around("?");
                self.expr(consequent, PREC_ASSIGN);
                self.spaced_around(":");
                self.expr(alternate, PREC_ASSIGN);
            }
            Expr::Object(entries) => self.object_expr(entries),
            Expr::Array(items) => self.array_expr(items),
            Expr::Arrow(params, body) => self.arrow_expr(params, body),
            Expr::Seq(items) => self.seq_expr(items),
            Expr::Raw(text) => self.out.push_str(text),
        }
    }

    fn bigint_expr(&mut self, digits: &str) {
        if self.target == EcmaTarget::Es2018 {
            self.out.push_str("BigInt(");
            write_js_string(&mut self.out, digits);
            self.out.push(')');
        } else {
            self.out.push_str(digits);
            self.out.push('n');
        }
    }

    fn member_expr(&mut self, obj: &Expr, name: &str) {
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

    fn new_expr(&mut self, callee: &Expr, args: &[Expr]) {
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

    fn unary_expr(&mut self, op: UnOp, operand: &Expr) {
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

    fn binary_expr(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) {
        let prec = op.prec();
        self.expr(lhs, prec);
        self.spaced_around(op.symbol());
        let start = self.out.len();
        self.expr(rhs, prec + 1);
        self.glue(start);
    }

    fn compound_assign_expr(&mut self, op: CompoundOp, target: &Expr, value: &Expr) {
        if self.target == EcmaTarget::Es2018 && matches!(op, CompoundOp::And | CompoundOp::Or) {
            let lowered = lower_logical_assignment(op.to_bin_op(), target, value);
            self.expr_bare(&lowered);
            return;
        }
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

    fn object_expr(&mut self, entries: &[(String, Expr)]) {
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

    fn array_expr(&mut self, items: &[Expr]) {
        self.out.push('[');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.separator();
            }
            self.expr(item, PREC_ASSIGN);
        }
        self.out.push(']');
    }

    fn arrow_expr(&mut self, params: &[String], body: &ArrowBody) {
        // `a => ..` rather than `(a) => ..`. Only for exactly one parameter, which is the only
        // shape the grammar allows bare — and the only shape this AST can build, having no
        // destructuring and no defaults.
        match (self.compact(), params) {
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

    fn seq_expr(&mut self, items: &[Expr]) {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.separator();
            }
            self.expr(item, PREC_ASSIGN);
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
            Stmt::ExprStmt(expr) => self.expr_stmt_bare(expr),
            Stmt::Let(name, init) => {
                self.declaration("let", std::iter::once((name.as_str(), init.as_ref())));
            }
            Stmt::Lets(decls) => self.declaration(
                "let",
                decls
                    .iter()
                    .map(|(name, init)| (name.as_str(), init.as_ref())),
            ),
            Stmt::Const(name, init) => {
                self.declaration("const", std::iter::once((name.as_str(), Some(init))));
            }
            Stmt::Return(value) => self.return_stmt(value.as_ref()),
            Stmt::If(test, then, els) => {
                self.if_chain(test, then, els.as_deref());
                self.newline();
            }
            Stmt::Switch(disc, cases) => self.switch_stmt(disc, cases),
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
                self.out.push_str(if self.compact() {
                    "for(;;)"
                } else {
                    "for (;;)"
                });
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
            Stmt::Continue(label) => self.jump_stmt("continue", label.as_deref()),
            Stmt::Break(label) => self.jump_stmt("break", label.as_deref()),
            Stmt::Throw(value) => self.throw_stmt(value),
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => self.try_catch_stmt(body, binding, handler),
            Stmt::FunctionDecl { name, params, body } => {
                self.function_decl_stmt(name, params, body)
            }
            Stmt::Import { specifiers, source } => self.import_stmt(specifiers, source),
            Stmt::Block(stmts) => self.block_stmt(stmts),
            Stmt::Comment(text) => self.comment_stmt(text),
            Stmt::Raw(text) => self.raw_stmt(text),
            // Handled by `stmt`, which never reaches here with one.
            Stmt::Loc(_) => {}
        }
    }

    fn expr_stmt_bare(&mut self, expr: &Expr) {
        let start = self.out.len();
        self.expr(expr, 0);
        if needs_stmt_parens(&self.out[start..]) {
            self.out.insert(start, '(');
            self.out.push(')');
        }
        self.end_stmt();
    }

    fn return_stmt(&mut self, value: Option<&Expr>) {
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

    fn switch_stmt(&mut self, disc: &Expr, cases: &[SwitchCase]) {
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

    fn jump_stmt(&mut self, keyword: &str, label: Option<&str>) {
        self.out.push_str(keyword);
        if let Some(label) = label {
            self.out.push(' ');
            self.out.push_str(label);
        }
        self.end_stmt();
    }

    fn throw_stmt(&mut self, value: &Expr) {
        self.out.push_str("throw");
        if !self.compact() {
            self.out.push(' ');
        }
        let start = self.out.len();
        self.expr(value, 0);
        self.glue(start);
        self.end_stmt();
    }

    fn try_catch_stmt(&mut self, body: &[Stmt], binding: &str, handler: &[Stmt]) {
        self.out.push_str("try");
        self.body(body);
        self.out
            .push_str(if self.compact() { "catch(" } else { " catch (" });
        let start = self.out.len();
        self.out.push_str(binding);
        self.name_mapping(binding, start, self.out.len());
        self.out.push(')');
        self.body(handler);
        self.newline();
    }

    fn function_decl_stmt(&mut self, name: &str, params: &[String], body: &[Stmt]) {
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

    fn import_stmt(&mut self, specifiers: &ImportSpecifiers, source: &str) {
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

    fn block_stmt(&mut self, stmts: &[Stmt]) {
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

    fn comment_stmt(&mut self, text: &str) {
        self.out.push_str("// ");
        for ch in text.chars() {
            self.out
                .push(if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                    ' '
                } else {
                    ch
                });
        }
        self.out.push('\n');
    }

    fn raw_stmt(&mut self, text: &str) {
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                self.write_indent();
            }
            self.out.push_str(line);
            self.out.push('\n');
        }
    }

    fn declaration<'a>(
        &mut self,
        keyword: &str,
        decls: impl IntoIterator<Item = (&'a str, Option<&'a Expr>)>,
    ) {
        self.out.push_str(keyword);
        self.out.push(' ');
        for (i, (name, init)) in decls.into_iter().enumerate() {
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
            Some(Some(SingleIf {
                markers,
                test,
                then,
                els,
            })) => {
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
    matches!(
        (a, b),
        ('/', '/') | ('-', '-') | ('+', '+') | ('-', '>') | ('<', '!')
    )
}

/// The parts of an `else` branch that is a single `if`, markers aside.
struct SingleIf<'a> {
    markers: &'a [Stmt],
    test: &'a Expr,
    then: &'a [Stmt],
    els: Option<&'a [Stmt]>,
}

/// A [`Stmt::Loc`] prints nothing, so a branch of markers and one `if` is still an `else if`
/// chain. Reading it as one is what keeps `-Cllvm-args=js-source-map=on` from changing the
/// JavaScript it is supposed to only describe.
fn single_if(stmts: &[Stmt]) -> Option<SingleIf<'_>> {
    let (last, markers) = stmts.split_last()?;
    if !markers.iter().all(|stmt| matches!(stmt, Stmt::Loc(_))) {
        return None;
    }
    match last {
        Stmt::If(test, then, els) => Some(SingleIf {
            markers,
            test,
            then,
            els: els.as_deref(),
        }),
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
            let ch = out[cursor..]
                .chars()
                .next()
                .expect("offset is a character boundary");
            cursor += ch.len_utf8();
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += ch.len_utf16() as u32;
            }
        }
        mappings.push(Mapping {
            gen_line: line,
            gen_col: col,
            loc,
            name,
        });
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
        let _ = write!(out, "{n}");
    }
}

/// Writes a string as a double quoted JavaScript literal.
fn write_js_string(out: &mut String, s: &str) {
    // Under `js-str=bytes` a Rust `str` literal is a byte string, every unit below U+0100; its
    // non-ASCII units are written `\xNN`, which is the same JavaScript value as the raw
    // character, so the escape is harmless for every other string the program spells.
    let bytes = crate::opts::get().str_repr == crate::opts::StrRepr::Bytes;
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
            c if (c as u32) < 0x20
                || c as u32 == 0x7f
                || (bytes && (0x80..0x100).contains(&(c as u32))) =>
            {
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
