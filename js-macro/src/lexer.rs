//! Which identifiers in a `js!{}` block are free, found by lexing rather than by parsing.
//!
//! A parser would answer this exactly and cost a full ECMAScript grammar with Unicode tables. The
//! question is smaller than a grammar: an identifier in a block is either a name JavaScript itself
//! supplies, a name the block binds, a property, or a Rust binding to capture. Telling those apart
//! needs a lexer, a bracket depth counter and the handful of binding forms, which is what this is.
//!
//! # The direction it fails in
//!
//! An identifier this cannot classify is reported FREE. A free identifier becomes a real Rust
//! identifier in the expansion, so one that is not a Rust binding is a spanned "cannot find value"
//! from rustc. The failure mode is an error at the call site, never a capture that went missing.
//!
//! # The four places an identifier is not an identifier
//!
//! Strings, template literals (which nest, through `${}`), comments and regular expression
//! literals. Each is skipped whole. The one ambiguity JavaScript has no lexical answer to is
//! whether `/` opens a regular expression or divides, and [`Scan::regex_follows`] resolves it the
//! conservative way for this job: see its docs.
//!
//! # What is not free
//!
//! * a reserved word, and the four host names in [`HOST_NAMES`];
//! * a contextual keyword (`of` in `for (... of ...)`, `async` before `function` or arrow params,
//!   `get`/`set`/`async` in object-literal method/accessor positions) — anywhere else those four
//!   names are ordinary identifiers and can capture Rust locals of the same name;
//! * an identifier after `.` or `?.`, which is a property and a different namespace;
//! * an object literal key: `{ method: "POST" }` binds nothing, while the shorthand `{ body }` is a
//!   reference and so is a capture, rewritten to `{ body: <slot> }`;
//! * a name the block itself binds, in the region that binds it. The binding forms are `let`,
//!   `const`, `var`, function parameters, arrow parameters, a function's own name, and `catch`.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use mires_marker_abi::block;

/// What lexing a block found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Analysis {
    /// The free identifiers, in first occurrence order. Capture `i` fills slot `i`.
    pub free: Vec<String>,
    /// Character offset of the first free occurrence of each identifier in [`Analysis::free`].
    pub free_offsets: Vec<usize>,
    /// The block, with every free occurrence replaced by its slot name.
    pub text: String,
}

/// Error returned by [`analyze`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LexError {
    /// An opening `(`, `[`, or `{` reached EOF without its matching closer.
    UnclosedBracket(char),
    /// A closing `)`, `]`, or `}` appeared with no open bracket.
    UnopenedBracket(char),
    /// A closing bracket did not match the innermost open bracket.
    MismatchedBracket { open: char, close: char },
    /// A template literal reached EOF without a closing `` ` ``.
    UnterminatedTemplate,
    /// A `/*` block comment reached EOF without `*/`.
    UnterminatedBlockComment,
    /// A single- or double-quoted string literal reached EOF without its closing quote.
    UnterminatedString(char),
    /// A regular expression literal hit a newline or EOF before its closing `/`.
    UnterminatedRegex,
    /// The source mentioned a reserved capture slot name such as `_$js0`.
    ReservedSlotName(String),
    /// A statement-opening keyword appeared at top level.
    StatementKeyword(String),
    /// A `;` appeared at top level.
    TopLevelSemicolon,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedBracket(open) => write!(f, "`{open}` is never closed"),
            Self::UnopenedBracket(close) => {
                write!(f, "`{close}` closes a bracket that was never opened")
            }
            Self::MismatchedBracket { open, close } => {
                write!(f, "`{open}` is closed by `{close}`")
            }
            Self::UnterminatedTemplate => f.write_str("a template literal is never closed"),
            Self::UnterminatedBlockComment => f.write_str("a block comment is never closed"),
            Self::UnterminatedString(quote) => write!(f, "a {quote} string is never closed"),
            Self::UnterminatedRegex => f.write_str("a regular expression is never closed"),
            Self::ReservedSlotName(word) => write!(
                f,
                "`{word}` is what a capture is emitted as, so a block cannot spell it"
            ),
            Self::StatementKeyword(word) => write!(
                f,
                "`{word}` opens a statement and a `js!{{}}` block is one expression"
            ),
            Self::TopLevelSemicolon => {
                f.write_str("a `js!{}` block is one expression, and `;` ends a statement")
            }
        }
    }
}

impl std::error::Error for LexError {}

/// The names the analysis treats as always bound, on top of the reserved words.
///
/// The first three are values JavaScript spells as globals rather than as literals. `globalThis` is
/// the door to everything else the host has: `js!{ globalThis.fetch(url) }` reaches the host's
/// `fetch` and captures only `url`. That is deliberate rather than a shortcut around a builtins
/// list. A list would go stale, and going through `globalThis` makes every host name a block
/// depends on greppable, which is what keeps the rule "a free identifier is a Rust binding" total.
pub const HOST_NAMES: [&str; 4] = ["globalThis", "undefined", "NaN", "Infinity"];

/// The unconditionally reserved words, which are never captured as identifiers.
///
/// Contextual keywords (`async`, `get`, `of`, `set`) are intentionally excluded here and handled
/// positionally so Rust locals with those names can still be captured in ordinary expressions.
const RESERVED: [&str; 38] = [
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
];

/// The rest of the reserved words. Split only to keep each array a readable width.
const RESERVED_REST: [&str; 8] = [
    "true", "try", "typeof", "var", "void", "while", "with", "yield",
];

/// Reserved words that END a value, which is what decides a `/` written after one.
const VALUE_WORDS: [&str; 6] = ["this", "super", "true", "false", "null", "yield"];

/// Words that open a statement. A block is one expression, so one of these at the outermost depth
/// is a body this macro does not take.
const STATEMENT_WORDS: [&str; 13] = [
    "break", "const", "continue", "debugger", "do", "for", "if", "let", "return", "switch",
    "throw", "var", "while",
];

/// Whether `word` is an unconditionally reserved name a block never captures.
#[must_use]
pub fn is_reserved(word: &str) -> bool {
    RESERVED.contains(&word) || RESERVED_REST.contains(&word) || HOST_NAMES.contains(&word)
}

/// Whether `character` can start an identifier.
fn is_ident_start(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphabetic()
}

/// Whether `character` can continue one.
fn is_ident_continue(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphanumeric()
}

/// The multi-character operators, longest first so a munch is greedy. `/` and `/=` are absent
/// because a leading `/` is decided by [`Scan::regex_follows`] before it reaches here.
const OPERATORS: [&str; 25] = [
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=",
];

/// The rest of the multi-character operators. Split only to keep each array a readable width.
const OPERATORS_REST: [&str; 7] = ["%=", "&=", "|=", "^=", "**", "<<", ">>"];

/// The last significant thing the scan read, which is what several decisions turn on.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Prev {
    /// Nothing yet.
    Start,
    /// An identifier or a reserved word.
    Word(String),
    /// A number, a string or a template literal.
    Literal,
    /// A regular expression literal.
    Regex,
    /// An opening bracket.
    Open(char),
    /// A closing bracket.
    Close(char),
    /// A multi-character operator (or `${`).
    Op(&'static str),
    /// A single punctuation character.
    Punct(char),
}

impl Prev {
    /// Whether what came before ends a VALUE, which is what makes a following `/` a division.
    fn ends_a_value(&self) -> bool {
        match self {
            Prev::Word(word) => !is_reserved(word) || VALUE_WORDS.contains(&word.as_str()),
            Prev::Literal | Prev::Regex | Prev::Close(_) => true,
            Prev::Op("++" | "--") => true,
            Prev::Start | Prev::Open(_) | Prev::Op(_) | Prev::Punct(_) => false,
        }
    }
}

/// Which kind of `{ ... }` construct a brace frame represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BraceKind {
    Block,
    Object,
    TemplateSubstitution,
}

/// Which bracket construct a [`Frame`] represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameKind {
    /// The top-level expression itself.
    Root,
    /// A `(...)` group: possibly a parameter list, a `for (...)` head still on its LHS, or
    /// immediately preceded by a free `async` token that becomes contextual if `=>` follows `)`.
    Paren {
        params: bool,
        for_head: bool,
        async_prefix: Option<usize>,
    },
    /// A `[...]` group, possibly a computed property key inside an object literal.
    Bracket { computed_key: bool },
    /// A `{...}` group.
    Brace(BraceKind),
}

impl FrameKind {
    const fn open_char(self) -> Option<char> {
        match self {
            Self::Root => None,
            Self::Paren { .. } => Some('('),
            Self::Bracket { .. } => Some('['),
            Self::Brace(_) => Some('{'),
        }
    }

    const fn close_char(self) -> Option<char> {
        match self {
            Self::Root => None,
            Self::Paren { .. } => Some(')'),
            Self::Bracket { .. } => Some(']'),
            Self::Brace(_) => Some('}'),
        }
    }
}

/// One bracketed region, and what the scan needs to remember about it.
///
/// The stack always holds a root frame for the block itself, so a `?:` and a `,` written outside
/// every bracket are tracked the same way as ones written inside.
#[derive(Debug)]
struct Frame {
    kind: FrameKind,
    /// Open `?`s waiting for their `:` at this depth, so a ternary's `:` is not read as a key's.
    ternary: usize,
    /// Where this frame's occurrences begin, for the retroactive pass that turns `(a, b) =>` into
    /// a parameter list.
    occurrences: usize,
    /// Whether a bare `=` has been written at this frame's own depth since the last `,`. A
    /// parameter list's default value is a reference and must survive that pass.
    default: bool,
}

impl Frame {
    /// The frame the block itself is written in.
    fn root() -> Self {
        Self::new(FrameKind::Root, 0)
    }

    fn new(kind: FrameKind, occurrences: usize) -> Self {
        Self {
            kind,
            ternary: 0,
            occurrences,
            default: false,
        }
    }
}

/// One identifier the scan read.
#[derive(Debug)]
struct Occurrence {
    /// Character offsets into the block.
    start: usize,
    end: usize,
    name: String,
    /// Whether this is a capture. Cleared by the retroactive parameter pass.
    free: bool,
    /// Whether it sat after a `=` in its frame, so a parameter pass leaves it alone.
    default: bool,
    /// Whether it is an object literal shorthand, which a slot cannot replace in place: `{ body }`
    /// has to become `{ body: <slot> }` or the object gains a key nobody asked for.
    shorthand: bool,
}

/// How far a [`Scope`] reaches before it is popped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeExtent {
    /// The scope has collected its parameters and is waiting to learn whether its body is a block
    /// or an expression.
    Pending,
    /// A block-bodied scope valid while `depth >= depth`.
    Block { depth: usize },
    /// An expression-bodied scope valid while `depth >= depth`, also ended by `,`, `;`, or a
    /// non-ternary `:` at `depth`.
    Expression { depth: usize },
}

/// A region of the block over which some names are bound.
#[derive(Debug)]
struct Scope {
    names: BTreeSet<String>,
    extent: ScopeExtent,
}

impl Scope {
    fn insert_name(&mut self, name: &str) {
        self.names.insert(name.to_owned());
    }
}

/// Which mode the scanner is currently in inside a template literal stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemplateMode {
    /// Raw template text between `` ` `` / `}` and `${` / `` ` ``.
    Text,
    /// JavaScript code inside a `${ ... }` substitution.
    Substitution,
}

/// Reads a block, one character at a time.
struct Scan<'a> {
    chars: &'a [char],
    at: usize,
    /// One entry per template literal nesting level.
    templates: Vec<TemplateMode>,
    frames: Vec<Frame>,
    scopes: Vec<Scope>,
    occurrences: Vec<Occurrence>,
    prev: Prev,
    /// The scope whose body extent is not known yet.
    pending: Option<usize>,
    /// The next `(` opens a parameter list, said by `function`, `catch`, or a method shorthand.
    expect_params: bool,
    /// The next identifier is a function's own name.
    expect_name: bool,
    /// The next `(` opens a `for (...)` header.
    expect_for_head: bool,
    /// A contextual object-literal entry modifier (`get`, `set`, `async`, or `*`) was just read,
    /// so the following token is still in property/method-name position.
    entry_modifier: bool,
    /// If the previous token was a potentially-free `async` followed on the same line by `(`, its
    /// index in `occurrences` so closing `)` followed by `=>` can clear its `free` flag.
    pending_async_occ: Option<usize>,
    /// The frame depth a `let`, `const`, or `var` was written at, while it is still in effect.
    declaring: Option<usize>,
    /// The depth of the `=` (or `of`/`in`) whose right hand side the scan is in, so an
    /// initializer reads as references rather than as bindings.
    initializing: Option<usize>,
}

impl<'a> Scan<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            at: 0,
            templates: Vec::new(),
            frames: vec![Frame::root()],
            scopes: vec![Scope {
                names: BTreeSet::new(),
                extent: ScopeExtent::Block { depth: 0 },
            }],
            occurrences: Vec::new(),
            prev: Prev::Start,
            pending: None,
            expect_params: false,
            expect_name: false,
            expect_for_head: false,
            entry_modifier: false,
            pending_async_occ: None,
            declaring: None,
            initializing: None,
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    /// Advances past a `\` and the escaped character without overshooting `self.chars.len()`.
    fn advance_escape(&mut self) {
        self.at = (self.at + 2).min(self.chars.len());
    }

    /// Whether the remaining characters at `self.at` start with `needle`.
    fn starts_with(&self, needle: &str) -> bool {
        (self.at..)
            .zip(needle.chars())
            .all(|(index, expected)| self.chars.get(index) == Some(&expected))
    }

    /// The frame the scan is in, which is never absent: the root is one.
    fn frame(&self) -> &Frame {
        self.frames.last().expect("the root frame is never popped")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("the root frame is never popped")
    }

    /// How many brackets are open, which is what a scope's extent is measured in.
    fn depth(&self) -> usize {
        self.frames.len() - 1
    }

    /// The next character that is neither whitespace nor a comment, and where it is.
    fn significant(&self, mut at: usize) -> Option<(usize, char)> {
        loop {
            let character = *self.chars.get(at)?;
            if character.is_whitespace() {
                at += 1;
                continue;
            }
            if character == '/' && self.chars.get(at + 1) == Some(&'/') {
                at += 2;
                while self.chars.get(at).is_some_and(|c| *c != '\n') {
                    at += 1;
                }
                continue;
            }
            if character == '/' && self.chars.get(at + 1) == Some(&'*') {
                at += 2;
                while at < self.chars.len() {
                    if self.chars[at] == '*' && self.chars.get(at + 1) == Some(&'/') {
                        at += 2;
                        break;
                    }
                    at += 1;
                }
                continue;
            }
            return Some((at, character));
        }
    }

    /// Like [`Scan::significant`], but stops and returns `None` if a line terminator intervenes
    /// (JavaScript's `[no LineTerminator here]` restriction for contextual `async`).
    fn same_line_significant(&self, mut at: usize) -> Option<(usize, char)> {
        loop {
            let character = *self.chars.get(at)?;
            if character == '\n' || character == '\r' {
                return None;
            }
            if character.is_whitespace() {
                at += 1;
                continue;
            }
            if character == '/' && self.chars.get(at + 1) == Some(&'/') {
                return None;
            }
            if character == '/' && self.chars.get(at + 1) == Some(&'*') {
                at += 2;
                while at < self.chars.len() {
                    if matches!(self.chars[at], '\n' | '\r') {
                        return None;
                    }
                    if self.chars[at] == '*' && self.chars.get(at + 1) == Some(&'/') {
                        at += 2;
                        break;
                    }
                    at += 1;
                }
                continue;
            }
            return Some((at, character));
        }
    }

    /// Where the body of a `=>` written at the cursor starts, if one is written there.
    fn arrow_ahead(&self) -> Option<usize> {
        match self.significant(self.at) {
            Some((at, '=')) if self.chars.get(at + 1) == Some(&'>') => Some(at + 2),
            _ => None,
        }
    }

    /// Whether the `/` at the cursor opens a regular expression rather than dividing.
    ///
    /// JavaScript answers this from the parser's state and a lexer has only what came before, so
    /// this is a heuristic, and which way it leans is the part worth stating.
    ///
    /// A `/` after something that ENDS A VALUE divides; after anything else it opens a regular
    /// expression. The one genuinely ambiguous token is `}`, which ends a value when it closed an
    /// object literal and does not when it closed a block, and this counts it as ending one.
    ///
    /// Both mistakes are possible and they are not equally bad. Reading a regular expression as a
    /// division lexes the pattern as code, and identifier shaped pieces of it come out FREE, which
    /// is a spanned "cannot find value" at the call site. Reading a division as a regular
    /// expression swallows the code up to the next `/`, and an identifier inside it disappears
    /// without a word. So the lean is toward division, which is the loud mistake.
    fn regex_follows(&self) -> bool {
        !self.prev.ends_a_value()
    }

    /// Whether an identifier here is being BOUND rather than referenced.
    ///
    /// Only a `let`, `const` or `var` answers yes. A parameter list is settled the other way round,
    /// at its `)`, because an arrow's `(a, b)` is indistinguishable from a call's until the `=>`.
    fn binding(&self) -> bool {
        self.initializing.is_none() && self.declaring.is_some_and(|root| self.depth() >= root)
    }

    /// Whether some live scope binds `name`.
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.names.contains(name))
    }

    /// Binds `name` in the innermost scope.
    fn bind(&mut self, name: &str) {
        self.scopes
            .last_mut()
            .expect("the root scope is never dropped")
            .insert_name(name);
    }

    /// Drops every scope whose region the scan has left.
    fn close_scopes(&mut self) {
        let depth = self.depth();
        while self.scopes.len() > 1 {
            let scope = self.scopes.last().expect("the root scope is never dropped");
            let keep = match scope.extent {
                ScopeExtent::Pending => true,
                ScopeExtent::Block { depth: d } | ScopeExtent::Expression { depth: d } => {
                    depth >= d
                }
            };
            if keep {
                break;
            }
            self.scopes.pop();
        }
    }

    /// Ends every expression bodied scope that a separator at its own depth has run past.
    fn close_expression_scopes(&mut self) {
        let depth = self.depth();
        while self.scopes.len() > 1 {
            let scope = self.scopes.last().expect("the root scope is never dropped");
            if scope.extent != (ScopeExtent::Expression { depth }) {
                break;
            }
            self.scopes.pop();
        }
    }

    /// Turns the occurrences recorded from `from` on into the parameters of a new scope.
    fn take_parameters(&mut self, from: usize) {
        let mut names = BTreeSet::new();
        for occurrence in &mut self.occurrences[from..] {
            if occurrence.free && !occurrence.default {
                occurrence.free = false;
                names.insert(occurrence.name.clone());
            }
        }
        self.scopes.push(Scope {
            names,
            extent: ScopeExtent::Pending,
        });
        self.pending = Some(self.scopes.len() - 1);
    }

    /// Settles how far the scope waiting on a body reaches, now that the body is in sight.
    ///
    /// A `{` next is a block body, and the frame it pushes is what settles the extent. Anything
    /// else is an expression body, which reaches to the end of the current bracket or to the next
    /// separator in it.
    fn settle_pending(&mut self) {
        let Some(index) = self.pending else { return };
        if matches!(self.significant(self.at), Some((_, '{'))) {
            return;
        }
        let depth = self.depth();
        self.scopes[index].extent = ScopeExtent::Expression { depth };
        self.pending = None;
    }

    /// Marks the current `for (...)` header frame as past its binding LHS (after `=`, `;`, `in`,
    /// or `of`).
    fn leave_for_head_lhs(&mut self) {
        if let FrameKind::Paren { for_head, .. } = &mut self.frame_mut().kind {
            *for_head = false;
        }
    }

    /// One step of the scan, outside a template literal.
    fn code(&mut self) -> Result<(), LexError> {
        let character = self.chars[self.at];

        if character.is_whitespace() {
            self.at += 1;
            return Ok(());
        }
        if character == '/' && self.peek(1) == Some('/') {
            while self.peek(0).is_some_and(|c| c != '\n') {
                self.at += 1;
            }
            return Ok(());
        }
        if character == '/' && self.peek(1) == Some('*') {
            self.at += 2;
            loop {
                match self.peek(0) {
                    None => return Err(LexError::UnterminatedBlockComment),
                    Some('*') if self.peek(1) == Some('/') => {
                        self.at += 2;
                        return Ok(());
                    }
                    Some(_) => self.at += 1,
                }
            }
        }
        if character == '/' && self.regex_follows() {
            return self.regex();
        }
        if character == '"' || character == '\'' {
            return self.string(character);
        }
        if character == '`' {
            self.entry_modifier = false;
            self.pending_async_occ = None;
            self.at += 1;
            self.templates.push(TemplateMode::Text);
            return Ok(());
        }
        if character.is_ascii_digit()
            || (character == '.' && self.peek(1).is_some_and(|c| c.is_ascii_digit()))
        {
            self.number();
            return Ok(());
        }
        if is_ident_start(character) {
            return self.identifier();
        }
        self.punctuation()
    }

    /// One step of the scan, inside a template literal's text segment.
    fn template(&mut self) {
        match self.chars[self.at] {
            '\\' => self.advance_escape(),
            '`' => {
                self.at += 1;
                self.templates.pop();
                self.prev = Prev::Literal;
            }
            '$' if self.peek(1) == Some('{') => {
                self.at += 2;
                // The substitution is code again, and its `}` puts the scan back in the template.
                self.templates.push(TemplateMode::Substitution);
                let occurrences = self.occurrences.len();
                self.frames.push(Frame::new(
                    FrameKind::Brace(BraceKind::TemplateSubstitution),
                    occurrences,
                ));
                self.prev = Prev::Op("${");
            }
            _ => self.at += 1,
        }
    }

    /// A string literal, skipped whole.
    fn string(&mut self, quote: char) -> Result<(), LexError> {
        let was_entry = self.entry_position();
        self.entry_modifier = false;
        self.pending_async_occ = None;
        self.at += 1;
        loop {
            match self.peek(0) {
                None => return Err(LexError::UnterminatedString(quote)),
                Some('\\') => self.advance_escape(),
                Some(character) if character == quote => {
                    self.at += 1;
                    self.prev = Prev::Literal;
                    if was_entry && matches!(self.significant(self.at), Some((_, '('))) {
                        self.expect_params = true;
                    }
                    return Ok(());
                }
                Some(_) => self.at += 1,
            }
        }
    }

    /// A regular expression literal, skipped whole. A `[..]` class may hold an unescaped `/`.
    fn regex(&mut self) -> Result<(), LexError> {
        self.entry_modifier = false;
        self.pending_async_occ = None;
        self.at += 1;
        let mut in_class = false;
        loop {
            match self.peek(0) {
                None | Some('\n') => return Err(LexError::UnterminatedRegex),
                Some('\\') => {
                    if self.peek(1) == Some('\n') {
                        return Err(LexError::UnterminatedRegex);
                    }
                    self.advance_escape();
                }
                Some('[') => {
                    in_class = true;
                    self.at += 1;
                }
                Some(']') => {
                    in_class = false;
                    self.at += 1;
                }
                Some('/') if !in_class => {
                    self.at += 1;
                    while self.peek(0).is_some_and(is_ident_continue) {
                        self.at += 1;
                    }
                    self.prev = Prev::Regex;
                    return Ok(());
                }
                Some(_) => self.at += 1,
            }
        }
    }

    /// A number literal, skipped whole so that `0x1f` is not read as an identifier `x1f`.
    fn number(&mut self) {
        let was_entry = self.entry_position();
        self.entry_modifier = false;
        self.pending_async_occ = None;
        let mut seen_point = false;
        while let Some(character) = self.peek(0) {
            if character == '.' && !seen_point {
                seen_point = true;
                self.at += 1;
                continue;
            }
            if matches!(character, 'e' | 'E' | 'p' | 'P') && matches!(self.peek(1), Some('+' | '-'))
            {
                self.at += 2;
                continue;
            }
            if character.is_alphanumeric() || character == '_' {
                self.at += 1;
                continue;
            }
            break;
        }
        self.prev = Prev::Literal;
        if was_entry && matches!(self.significant(self.at), Some((_, '('))) {
            self.expect_params = true;
        }
    }

    /// An identifier, classified where it stands.
    fn identifier(&mut self) -> Result<(), LexError> {
        let start = self.at;
        while self.peek(0).is_some_and(is_ident_continue) {
            self.at += 1;
        }
        let end = self.at;
        let word: String = self.chars[start..end].iter().collect();

        if block::is_slot_name(&word) {
            return Err(LexError::ReservedSlotName(word));
        }

        let after_dot = matches!(self.prev, Prev::Punct('.') | Prev::Op("?."));
        let next = self.significant(end).map(|(_, character)| character);

        if STATEMENT_WORDS.contains(&word.as_str()) && self.depth() == 0 && !after_dot {
            return Err(LexError::StatementKeyword(word));
        }

        let was_entry = self.entry_position();
        self.entry_modifier = false;
        self.pending_async_occ = None;

        let mut free = false;
        let mut shorthand = false;
        if after_dot {
            // A property, which is a namespace of its own.
        } else if is_reserved(&word) {
            self.keyword(&word);
        } else if word == "of" && self.is_contextual_of() {
            self.leave_for_head_lhs();
            if self.declaring.is_some() {
                self.initializing = Some(self.depth());
            }
        } else if was_entry && matches!(next, Some(':' | '(')) {
            // An object literal key binds nothing and is not a reference. A method shorthand's
            // parameters are a parameter list, which its `(` has to be told about.
            if next == Some('(') {
                self.expect_params = true;
            }
        } else if was_entry && self.is_object_modifier(&word, end, next) {
            self.entry_modifier = true;
        } else if self.expect_name {
            self.expect_name = false;
            self.bind(&word);
        } else if self.binding() {
            self.bind(&word);
        } else if word == "async" && self.is_contextual_async_keyword(end) {
            // `async function ...` or `async ident => ...`
        } else if !self.bound(&word) {
            free = true;
            shorthand = was_entry && matches!(next, Some(',' | '}'));
            if word == "async" && matches!(self.same_line_significant(end), Some((_, '('))) {
                self.pending_async_occ = Some(self.occurrences.len());
            }
        }

        let default = self.frame().default;
        self.occurrences.push(Occurrence {
            start,
            end,
            name: word.clone(),
            free,
            default,
            shorthand,
        });
        self.prev = Prev::Word(word);
        Ok(())
    }

    /// Whether an identifier here opens an entry of an object literal, which is what makes it
    /// either a key (`{ method: .. }`), an accessor/method modifier (`{ get x() {} }`), or a
    /// shorthand (`{ body }`) rather than an ordinary reference.
    ///
    /// Three things have to hold at once, and each rules out a case that would otherwise be a
    /// silent miss. The enclosing bracket must be a `{` that is an object literal, so a `case x:`
    /// inside a block is not a key. What came before must be that `{`, a `,`, or an entry modifier
    /// (`get`/`set`/`async`/`*`), so `a ? b : c` is not one either. And the frame must have no
    /// open `?`, for the same reason one step further out.
    fn entry_position(&self) -> bool {
        let frame = self.frame();
        if frame.kind != FrameKind::Brace(BraceKind::Object) || frame.ternary > 0 {
            return false;
        }
        self.entry_modifier || matches!(self.prev, Prev::Open('{') | Prev::Punct(','))
    }

    /// Whether `of` at the current cursor is the `for (... of ...)` contextual keyword rather than
    /// a variable name.
    fn is_contextual_of(&self) -> bool {
        if !matches!(self.frame().kind, FrameKind::Paren { for_head: true, .. }) {
            return false;
        }
        match &self.prev {
            Prev::Word(word) => !matches!(word.as_str(), "let" | "const" | "var"),
            Prev::Close(']' | '}' | ')') => true,
            _ => false,
        }
    }

    /// Whether `word` (`get`, `set`, or `async`) at an object literal entry starts a getter,
    /// setter, or async method rather than naming a property or shorthand capture.
    fn is_object_modifier(&self, word: &str, end: usize, next: Option<char>) -> bool {
        let is_prop_start =
            |ch: char| is_ident_start(ch) || ch.is_ascii_digit() || matches!(ch, '[' | '"' | '\'');
        match word {
            "get" | "set" => next.is_some_and(is_prop_start),
            "async" => self
                .same_line_significant(end)
                .is_some_and(|(_, ch)| is_prop_start(ch) || ch == '*'),
            _ => false,
        }
    }

    /// Whether `async` followed by what starts at `end` introduces an async function expression
    /// (`async function ...`) or a single-parameter async arrow (`async x => ...`).
    ///
    /// Parenthesized async arrows (`async (...) => ...`) are resolved retroactively at `)` via
    /// `pending_async_occ`, so calling a Rust local named `async(x)` remains a capture.
    fn is_contextual_async_keyword(&self, end: usize) -> bool {
        let Some((start, first)) = self.same_line_significant(end) else {
            return false;
        };
        if !is_ident_start(first) {
            return false;
        }
        let mut after = start;
        while self
            .chars
            .get(after)
            .copied()
            .is_some_and(is_ident_continue)
        {
            after += 1;
        }
        if self.chars[start..after] == ['f', 'u', 'n', 'c', 't', 'i', 'o', 'n'] {
            return true;
        }
        matches!(
            self.significant(after),
            Some((at, '=')) if self.chars.get(at + 1) == Some(&'>')
        )
    }

    /// What an unconditionally reserved word does to the scan's state.
    fn keyword(&mut self, word: &str) {
        match word {
            "let" | "const" | "var" => {
                self.declaring = Some(self.depth());
                self.initializing = None;
            }
            // A `for (const k in o)` head binds `k` and references `o`.
            "in" => {
                self.leave_for_head_lhs();
                self.initializing = Some(self.depth());
            }
            "for" => self.expect_for_head = true,
            "function" => {
                self.expect_params = true;
                self.expect_name = true;
            }
            "catch" => self.expect_params = true,
            _ => {}
        }
    }

    /// Everything that is not a word, a literal or a comment.
    fn punctuation(&mut self) -> Result<(), LexError> {
        match self.chars[self.at] {
            character @ ('(' | '[' | '{') => {
                self.open(character);
                Ok(())
            }
            character @ (')' | ']' | '}') => self.close(character),
            ';' => self.semicolon(),
            ',' => {
                self.entry_modifier = false;
                self.pending_async_occ = None;
                if self.initializing == Some(self.depth()) {
                    self.initializing = None;
                }
                self.frame_mut().default = false;
                self.at += 1;
                self.prev = Prev::Punct(',');
                self.close_expression_scopes();
                Ok(())
            }
            ':' => {
                self.entry_modifier = false;
                self.pending_async_occ = None;
                let ternary = self.frame().ternary > 0;
                if ternary {
                    self.frame_mut().ternary -= 1;
                }
                self.at += 1;
                self.prev = Prev::Punct(':');
                if !ternary {
                    self.close_expression_scopes();
                }
                Ok(())
            }
            '?' if !matches!(self.peek(1), Some('.' | '?')) => {
                self.entry_modifier = false;
                self.pending_async_occ = None;
                self.frame_mut().ternary += 1;
                self.at += 1;
                self.prev = Prev::Punct('?');
                Ok(())
            }
            _ => {
                self.operator();
                Ok(())
            }
        }
    }

    /// An opening bracket.
    fn open(&mut self, character: char) {
        let was_entry = self.entry_position();
        self.entry_modifier = false;
        let async_prefix = if character == '(' {
            self.pending_async_occ.take()
        } else {
            self.pending_async_occ = None;
            None
        };

        let kind = match character {
            '(' => FrameKind::Paren {
                params: self.expect_params,
                for_head: self.expect_for_head,
                async_prefix,
            },
            '[' => FrameKind::Bracket {
                computed_key: was_entry,
            },
            '{' => FrameKind::Brace(if self.opens_an_object() {
                BraceKind::Object
            } else {
                BraceKind::Block
            }),
            _ => unreachable!("open is only called on opening brackets"),
        };

        self.expect_params = false;
        self.expect_name = false;
        self.expect_for_head = false;
        let occurrences = self.occurrences.len();
        self.frames.push(Frame::new(kind, occurrences));

        // A scope waiting on its body gets it here: a `{` block after a parameter list is that
        // body.
        if kind == FrameKind::Brace(BraceKind::Block) {
            if let Some(index) = self.pending.take() {
                self.scopes[index].extent = ScopeExtent::Block {
                    depth: self.depth(),
                };
            }
        }
        self.at += 1;
        self.prev = Prev::Open(character);
    }

    /// A closing bracket, and everything that closing one settles.
    fn close(&mut self, character: char) -> Result<(), LexError> {
        self.entry_modifier = false;
        self.pending_async_occ = None;
        if self.frames.len() == 1 {
            return Err(LexError::UnopenedBracket(character));
        }
        let frame = self.frames.pop().expect("the length was checked");
        let expected = frame
            .kind
            .close_char()
            .expect("non-root frame has a closing bracket");
        if character != expected {
            let open = frame
                .kind
                .open_char()
                .expect("non-root frame has an opening bracket");
            return Err(LexError::MismatchedBracket {
                open,
                close: character,
            });
        }
        self.at += 1;

        if frame.kind == FrameKind::Brace(BraceKind::TemplateSubstitution) {
            // Back into the template literal the `${` interrupted.
            self.templates.pop();
            self.prev = Prev::Literal;
            return Ok(());
        }
        self.prev = Prev::Close(character);

        if self.declaring.is_some_and(|root| self.depth() < root) {
            self.declaring = None;
            self.initializing = None;
        }
        if self.initializing.is_some_and(|depth| self.depth() < depth) {
            self.initializing = None;
        }
        self.close_scopes();

        match frame.kind {
            FrameKind::Bracket { computed_key: true } => {
                if matches!(self.significant(self.at), Some((_, '('))) {
                    self.expect_params = true;
                }
            }
            FrameKind::Paren {
                params,
                async_prefix,
                ..
            } => match (self.arrow_ahead(), params) {
                (Some(body), _) => {
                    if let Some(async_idx) = async_prefix {
                        self.occurrences[async_idx].free = false;
                    }
                    self.take_parameters(frame.occurrences);
                    self.at = body;
                    self.prev = Prev::Op("=>");
                    self.settle_pending();
                }
                (None, true) => {
                    self.take_parameters(frame.occurrences);
                    self.settle_pending();
                }
                (None, false) => {}
            },
            _ => {}
        }
        Ok(())
    }

    /// A `;`, which at the outermost depth is what says a body is statements.
    fn semicolon(&mut self) -> Result<(), LexError> {
        self.entry_modifier = false;
        self.pending_async_occ = None;
        if self.depth() == 0 {
            return Err(LexError::TopLevelSemicolon);
        }
        self.leave_for_head_lhs();
        if self.declaring == Some(self.depth()) {
            self.declaring = None;
            self.initializing = None;
        }
        self.at += 1;
        self.prev = Prev::Punct(';');
        self.close_expression_scopes();
        Ok(())
    }

    /// Everything else, as the longest operator that starts here.
    fn operator(&mut self) {
        let was_entry = self.entry_position();
        self.entry_modifier = false;
        self.pending_async_occ = None;

        let (prev, len) = self.munch();
        self.at += len;

        if prev == Prev::Punct('*') && was_entry {
            self.entry_modifier = true;
        }
        if prev == Prev::Punct('=') {
            self.leave_for_head_lhs();
            if self.declaring.is_some() {
                self.initializing = Some(self.depth());
            }
            self.frame_mut().default = true;
        }
        if prev == Prev::Op("=>") {
            // `a => ..`: the bare single parameter, whose `(..)` counterpart is settled at its `)`.
            let from = match (&self.prev, self.occurrences.last()) {
                (Prev::Word(word), Some(last)) if *word == last.name => self.occurrences.len() - 1,
                _ => self.occurrences.len(),
            };
            self.take_parameters(from);
            self.prev = prev;
            self.settle_pending();
            return;
        }

        self.prev = prev;
    }

    /// Whether a `{` here opens an object literal rather than a block.
    ///
    /// Everything is an object literal except after the tokens that can only be followed by a
    /// block. A `js!{}` block is an expression, so the literal is the common direction; the
    /// exceptions are an arrow's block body, a `)` that closed a head or a parameter list, and the
    /// keywords that take a block on their own.
    fn opens_an_object(&self) -> bool {
        match &self.prev {
            Prev::Close(')' | '}') | Prev::Open('{') => false,
            Prev::Op("=>") | Prev::Punct(';') => false,
            Prev::Word(word) => !matches!(word.as_str(), "else" | "do" | "try" | "finally"),
            _ => true,
        }
    }

    /// The longest multi-character operator that starts at the cursor, or the single punctuation
    /// character written there, together with its length in characters.
    fn munch(&self) -> (Prev, usize) {
        for &operator in OPERATORS.iter().chain(OPERATORS_REST.iter()) {
            if self.starts_with(operator) {
                return (Prev::Op(operator), operator.len());
            }
        }
        (Prev::Punct(self.chars[self.at]), 1)
    }
}

/// Reads `source` and answers which of its identifiers are captures.
///
/// # Errors
///
/// Returns why the block is not one this macro takes: an unterminated string, template, comment or
/// regular expression, unbalanced brackets, a name that collides with an emitted slot, or a body
/// that is a sequence of statements rather than an expression.
pub fn analyze(source: &str) -> Result<Analysis, LexError> {
    let chars: Vec<char> = source.chars().collect();
    let mut scan = Scan::new(&chars);

    while scan.at < chars.len() {
        match scan.templates.last() {
            Some(TemplateMode::Text) => scan.template(),
            Some(TemplateMode::Substitution) | None => scan.code()?,
        }
    }

    if let Some(open) = scan.frame().kind.open_char() {
        return Err(LexError::UnclosedBracket(open));
    }
    if scan.templates.last() == Some(&TemplateMode::Text) {
        return Err(LexError::UnterminatedTemplate);
    }

    Ok(splice(&chars, &scan.occurrences))
}

/// The block with every free occurrence replaced by the slot that fills it.
fn splice(chars: &[char], occurrences: &[Occurrence]) -> Analysis {
    let mut free: Vec<String> = Vec::new();
    let mut free_offsets: Vec<usize> = Vec::new();
    let mut indices: HashMap<&str, usize> = HashMap::new();
    let mut text = String::with_capacity(chars.len());
    let mut cursor = 0;
    for occurrence in occurrences.iter().filter(|occurrence| occurrence.free) {
        let slot = *indices.entry(occurrence.name.as_str()).or_insert_with(|| {
            let index = free.len();
            free.push(occurrence.name.clone());
            free_offsets.push(occurrence.start);
            index
        });
        text.extend(&chars[cursor..occurrence.start]);
        // A shorthand entry names its key with the same token it reads, so replacing the token
        // would rename the key. Writing the entry out in full keeps the object the one the block
        // asked for.
        if occurrence.shorthand {
            text.push_str(&occurrence.name);
            text.push_str(": ");
        }
        text.push_str(&block::slot_name(slot));
        cursor = occurrence.end;
    }
    text.extend(&chars[cursor..]);
    Analysis {
        free,
        free_offsets,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The captures a block takes, which is what most of these are about.
    fn free(source: &str) -> Vec<String> {
        analyze(source).expect("it analyzes").free
    }

    /// The text a block lowers to.
    fn text(source: &str) -> String {
        analyze(source).expect("it analyzes").text
    }

    fn error(source: &str) -> String {
        analyze(source).expect_err("it is refused").to_string()
    }

    /// `Vec::new()` needs the annotation at every use, and there are many.
    fn none() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn a_plain_reference_is_a_capture_and_a_property_is_not() {
        assert_eq!(free("a"), ["a"]);
        assert_eq!(free("a.b.c"), ["a"]);
        assert_eq!(free("a?.b"), ["a"]);
        assert_eq!(free("a[b]"), ["a", "b"]);
        assert_eq!(free("f(a, b)"), ["f", "a", "b"]);
        // First occurrence order, and one slot per name however often it is written.
        assert_eq!(free("b + a + b"), ["b", "a"]);
        assert_eq!(text("b + a + b"), "_$js0 + _$js1 + _$js0");
    }

    #[test]
    fn a_reserved_word_and_a_host_name_are_never_captures() {
        assert_eq!(free("this.x"), none());
        assert_eq!(free("typeof x"), ["x"]);
        assert_eq!(free("new X(y)"), ["X", "y"]);
        assert_eq!(free("null ?? undefined ?? NaN ?? Infinity"), none());
        // The one door to the host: `fetch` is reached through it and only `url` is captured.
        assert_eq!(free("globalThis.fetch(url)"), ["url"]);
    }

    #[test]
    fn contextual_keywords_capture_as_rust_locals_in_expression_positions() {
        assert_eq!(
            free("f(get, set, of, async)"),
            ["f", "get", "set", "of", "async"]
        );
        assert_eq!(
            free("get + set + of + async"),
            ["get", "set", "of", "async"]
        );
        assert_eq!(free("async(a, b)"), ["async", "a", "b"]);
        assert_eq!(free("get / 2 + set / 3"), ["get", "set"]);
        // Shorthand object properties with contextual names still capture and keep their key.
        assert_eq!(
            free("({ get, set, of, async })"),
            ["get", "set", "of", "async"]
        );
        assert_eq!(
            text("({ get, set, of, async })"),
            "({ get: _$js0, set: _$js1, of: _$js2, async: _$js3 })"
        );
        // Inside `for` loops, `of` is contextual only as the loop connector.
        assert_eq!(
            free("(() => { for (const x of arr) { f(x, of); } })"),
            ["arr", "f", "of"]
        );
        assert_eq!(
            free("(() => { for (const x of of) { f(x); } })"),
            ["of", "f"]
        );
        assert_eq!(
            free("(() => { for (let i = 0; i < of; i++) { f(i); } })"),
            ["of", "f"]
        );
    }

    #[test]
    fn contextual_keywords_still_work_in_their_javascript_syntax_positions() {
        // `for (... of ...)` and `for await (... of ...)`
        assert_eq!(
            free("(() => { for (const x of arr) { f(x); } })"),
            ["arr", "f"]
        );
        assert_eq!(
            free("(() => { for (const { a, b } of arr) { f(a, b); } })"),
            ["arr", "f"]
        );
        assert_eq!(
            free("(() => { for (const { a = d } of arr) { f(a); } })"),
            ["d", "arr", "f"]
        );
        assert_eq!(
            free("(() => { for (const of of arr) { f(of); } })"),
            ["arr", "f"]
        );
        assert_eq!(
            free("(() => { for (x of arr) { f(x); } })"),
            ["x", "arr", "f"]
        );
        assert_eq!(
            free("(async () => { for await (const x of arr) { f(x); } })"),
            ["arr", "f"]
        );
        // Object literal getters, setters, and async / generator methods.
        assert_eq!(
            free("({ get x() { return 1; }, set x(v) { f(v); } })"),
            ["f"]
        );
        assert_eq!(
            free("({ get x() { return get; }, set x(v) { set(v); } })"),
            ["get", "set"]
        );
        assert_eq!(
            free("({ get [k]() { return 1; }, set [k](v) { f(v); } })"),
            ["k", "f"]
        );
        assert_eq!(free("({ get: a, set: b, async: c })"), ["a", "b", "c"]);
        assert_eq!(
            free("({ get(x) { return x; }, set(x) { return x; }, async(x) { return x; } })"),
            none()
        );
        assert_eq!(free("({ async m(a) { return a; } })"), none());
        assert_eq!(free("({ async *m(a) { yield a; } })"), none());
        assert_eq!(free("({ async m(a) { return a + async; } })"), ["async"]);
        // Async arrows and async functions.
        assert_eq!(free("async () => {}"), none());
        assert_eq!(free("async (a, b) => a + b"), none());
        assert_eq!(free("async (a = async) => a"), ["async"]);
        assert_eq!(free("async r => r.json()"), none());
        assert_eq!(free("async async => async"), none());
        assert_eq!(free("async r => async(r)"), ["async"]);
        assert_eq!(free("(async function f(a) { return f(a); })"), none());
        assert_eq!(free("(async function (a) { return a + b; })"), ["b"]);
        assert_eq!(free("(async function* gen(a) { yield a; })"), none());
        // Local bindings with contextual names.
        assert_eq!(
            free("(() => { const get = 1, set = 2, of = 3, async = 4; return get + set + of + async; })"),
            none()
        );
    }

    #[test]
    fn a_string_holds_no_identifiers() {
        assert_eq!(free(r#""a" + b"#), ["b"]);
        assert_eq!(free(r"'a' + b"), ["b"]);
        assert_eq!(free(r#""a\"b" + c"#), ["c"]);
        assert_eq!(free(r"'it\'s' + c"), ["c"]);
        // A quote of the other kind inside is just text.
        assert_eq!(free(r#""don't" + c"#), ["c"]);
        assert_eq!(text(r#"f("a", b)"#), r#"_$js0("a", _$js1)"#);
    }

    #[test]
    fn a_template_literal_holds_identifiers_only_in_its_substitutions() {
        assert_eq!(free("`a b c`"), none());
        assert_eq!(free("`a${b}c`"), ["b"]);
        assert_eq!(free("`${a}${b}`"), ["a", "b"]);
        // Nesting, which is the case a naive scan gets wrong.
        assert_eq!(free("`a${ `b${c}d` }e`"), ["c"]);
        assert_eq!(free("`a${ f(`${g}`) }`"), ["f", "g"]);
        // An escaped backtick inside one does not end it.
        assert_eq!(free("`a\\`${b}`"), ["b"]);
        // Braces inside a substitution are ordinary braces.
        assert_eq!(free("`${ {k: v} }`"), ["v"]);
        assert_eq!(text("`a${b}c`"), "`a${_$js0}c`");
    }

    #[test]
    fn a_comment_holds_no_identifiers() {
        assert_eq!(free("a // b\n+ c"), ["a", "c"]);
        assert_eq!(free("a /* b */ + c"), ["a", "c"]);
        assert_eq!(free("a /* b\nc */ + d"), ["a", "d"]);
        // A comment between a key and its colon does not turn the key into a reference.
        assert_eq!(free("({ k /* x */: v })"), ["v"]);
    }

    #[test]
    fn a_regular_expression_holds_no_identifiers_where_one_is_recognized() {
        assert_eq!(free("/ab/.test(s)"), ["s"]);
        assert_eq!(free("s.replace(/a[/]b/g, t)"), ["s", "t"]);
        assert_eq!(free("x ? /a/ : /b/"), ["x"]);
        assert_eq!(free("f(/a/, b)"), ["f", "b"]);
        assert_eq!(free("/a\\/b/.test(s)"), ["s"]);
    }

    #[test]
    fn division_is_the_conservative_reading_and_its_mistake_is_the_loud_one() {
        assert_eq!(free("a / b"), ["a", "b"]);
        assert_eq!(free("a[0] / b"), ["a", "b"]);
        assert_eq!(free("f() / b"), ["f", "b"]);
        // A `}` counts as ending a value, so what follows divides. A regular expression written
        // after a block's `}` is therefore lexed as code, and its pieces come out FREE, which is
        // the documented and LOUD mistake rather than a swallowed identifier.
        assert_eq!(free("({} / a)"), ["a"]);
        assert_eq!(free("({}) / a"), ["a"]);
    }

    #[test]
    fn an_object_key_binds_nothing_and_a_shorthand_is_a_reference() {
        assert_eq!(free("({ method: \"POST\" })"), none());
        assert_eq!(free("({ method: verb })"), ["verb"]);
        assert_eq!(free("({ body })"), ["body"]);
        assert_eq!(free("({ a, b: c, d })"), ["a", "c", "d"]);
        // A computed key is an expression.
        assert_eq!(free("({ [k]: v })"), ["k", "v"]);
        // A key Rust cannot spell, which is what the macro exists for.
        assert_eq!(free("({ \"content-type\": t })"), ["t"]);
        // A spread is a reference and not an entry.
        assert_eq!(free("({ ...rest })"), ["rest"]);
    }

    #[test]
    fn a_shorthand_capture_keeps_the_key_it_named() {
        // Replacing the token in place would have renamed the key to the slot.
        assert_eq!(text("({ body })"), "({ body: _$js0 })");
        assert_eq!(text("({ a, b: c })"), "({ a: _$js0, b: _$js1 })");
        assert_eq!(text("({ ...rest })"), "({ ..._$js0 })");
    }

    #[test]
    fn a_ternary_colon_is_not_a_key_and_a_case_label_is_not_either() {
        assert_eq!(free("({ k: a ? b : c })"), ["a", "b", "c"]);
        assert_eq!(free("a ? b : c"), ["a", "b", "c"]);
        assert_eq!(free("f(a ? b : c)"), ["f", "a", "b", "c"]);
        // Inside a block, `case x:` is a statement and `x` is a reference.
        assert_eq!(
            free("(() => { switch (a) { case b: return c; } })"),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn arrow_parameters_bind_and_their_body_sees_them() {
        assert_eq!(free("r => r.text()"), none());
        assert_eq!(free("(a, b) => a + b"), none());
        assert_eq!(free("(a, b) => a + c"), ["c"]);
        assert_eq!(free("() => x"), ["x"]);
        assert_eq!(free("(a) => { return a; }"), none());
        assert_eq!(free("async r => r.json()"), none());
        // A default value is a reference and survives the parameter pass.
        assert_eq!(free("(a = d) => a"), ["d"]);
        // Destructured parameters bind, and their keys stay keys.
        assert_eq!(free("({ a, b: c }) => a + c"), none());
        assert_eq!(free("([a, b]) => a + b"), none());
        // Nested arrows, each seeing the one outside it.
        assert_eq!(free("(a) => (b) => a + b + c"), ["c"]);
    }

    #[test]
    fn an_arrow_scope_ends_where_its_body_does() {
        // The parameter is not in scope beside the arrow, so the outer `a` is still a capture.
        assert_eq!(free("[(a) => a, a]"), ["a"]);
        assert_eq!(free("f(x => x, x)"), ["f", "x"]);
        assert_eq!(free("(x => { return x; })(x)"), ["x"]);
        // A ternary inside an expression body does not end the scope at its `:`.
        assert_eq!(free("x => c ? x : x"), ["c"]);
    }

    #[test]
    fn a_function_binds_its_name_and_its_parameters() {
        assert_eq!(free("(function f(a) { return f(a); })"), none());
        assert_eq!(free("(function (a) { return a + b; })"), ["b"]);
        assert_eq!(free("({ m(a) { return a; } })"), none());
        assert_eq!(free("({ m(a) { return a + z; } })"), ["z"]);
        // A parameter default is still a reference here.
        assert_eq!(free("(function (a = z) { return a; })"), ["z"]);
    }

    #[test]
    fn a_declaration_binds_its_names_and_references_its_initializer() {
        assert_eq!(free("(() => { const a = b; return a; })"), ["b"]);
        assert_eq!(free("(() => { let a = 1, c = d; return a + c; })"), ["d"]);
        assert_eq!(
            free("(() => { const { a, b: c } = d; return a + c; })"),
            ["d"]
        );
        assert_eq!(free("(() => { const [a, b] = c; return a + b; })"), ["c"]);
        // A destructuring default is a reference.
        assert_eq!(
            free("(() => { const { a = z } = d; return a; })"),
            ["z", "d"]
        );
    }

    #[test]
    fn a_for_head_and_a_catch_bind_what_they_declare() {
        assert_eq!(
            free("(() => { for (const x of xs) { f(x); } })"),
            ["xs", "f"]
        );
        assert_eq!(free("(() => { for (const k in o) { f(k); } })"), ["o", "f"]);
        assert_eq!(
            free("(() => { try { f(); } catch (e) { g(e); } })"),
            ["f", "g"]
        );
        // The declaration ends with its head, so what follows reads as references again.
        assert_eq!(
            free("(() => { for (let i = 0; i < n; i++) { f(i); } })"),
            ["n", "f"]
        );
    }

    #[test]
    fn nesting_of_every_bracket_is_tracked() {
        assert_eq!(free("f({ a: [b, { c: d }] })"), ["f", "b", "d"]);
        assert_eq!(free("((((a))))"), ["a"]);
        assert_eq!(free("a[b[c[d]]]"), ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_number_is_not_an_identifier() {
        assert_eq!(free("0x1f + a"), ["a"]);
        assert_eq!(free("1e-3 + a"), ["a"]);
        assert_eq!(free("1_000n + a"), ["a"]);
        assert_eq!(free("1.5 + a"), ["a"]);
        assert_eq!(free(".5 + a"), ["a"]);
    }

    #[test]
    fn the_motivating_case_captures_exactly_what_rust_supplies() {
        let source = "globalThis.fetch(url, { method: \"POST\", \
                      headers: { \"content-type\": kind }, body }).then(r => r.text())";
        assert_eq!(free(source), ["url", "kind", "body"]);
        assert_eq!(
            text(source),
            "globalThis.fetch(_$js0, { method: \"POST\", \
             headers: { \"content-type\": _$js1 }, body: _$js2 }).then(r => r.text())",
        );
    }

    #[test]
    fn a_slot_name_inside_a_block_is_refused() {
        let err = analyze("_$js0 + a").unwrap_err();
        assert_eq!(err, LexError::ReservedSlotName("_$js0".to_owned()));
        let message = err.to_string();
        assert!(message.contains("_$js0"), "{message}");
        assert!(message.contains("capture"), "{message}");
        assert!(error("(_$js1) => 1").contains("_$js1"));
    }

    #[test]
    fn a_body_that_is_statements_rather_than_an_expression_is_refused() {
        for source in [
            "const a = 1",
            "return a",
            "if (a) b",
            "for (;;) {}",
            "let a",
            "a; b",
        ] {
            let message = error(source);
            assert!(
                message.contains("one expression"),
                "`{source}` reported `{message}`"
            );
        }
        // Inside a function body they are ordinary statements.
        assert_eq!(free("(() => { const a = 1; return a; })"), none());
    }

    #[test]
    fn an_unterminated_anything_is_refused() {
        for (source, why, expected) in [
            ("\"a", "never closed", LexError::UnterminatedString('"')),
            ("'a", "never closed", LexError::UnterminatedString('\'')),
            ("\"a\\", "never closed", LexError::UnterminatedString('"')),
            ("'a\\", "never closed", LexError::UnterminatedString('\'')),
            ("`a", "never closed", LexError::UnterminatedTemplate),
            ("`a\\", "never closed", LexError::UnterminatedTemplate),
            ("/* a", "never closed", LexError::UnterminatedBlockComment),
            ("f(a", "never closed", LexError::UnclosedBracket('(')),
            (
                "f(a]",
                "closed by",
                LexError::MismatchedBracket {
                    open: '(',
                    close: ']',
                },
            ),
            ("a)", "never opened", LexError::UnopenedBracket(')')),
            ("x ? /a", "never closed", LexError::UnterminatedRegex),
            ("x ? /a\\", "never closed", LexError::UnterminatedRegex),
            ("x ? /a\\\nb/", "never closed", LexError::UnterminatedRegex),
        ] {
            let err = analyze(source).expect_err("it is refused");
            assert_eq!(err, expected, "`{source}`");
            let message = err.to_string();
            assert!(message.contains(why), "`{source}` reported `{message}`");
        }
    }

    #[test]
    fn free_offsets_point_at_the_first_free_site_not_a_prior_property() {
        let analysis = analyze("a.foo + foo").expect("it analyzes");
        assert_eq!(analysis.free, ["a", "foo"]);
        // `a` is at 0, `.foo` is at 2 (not free), free `foo` is at 8.
        assert_eq!(analysis.free_offsets, [0, 8]);
    }

    #[test]
    fn a_block_with_no_captures_is_left_exactly_as_written() {
        for source in [
            "r => r.text()",
            "(a, b) => a + b",
            "1 + 1",
            "\"x\"",
            "`a${1}b`",
        ] {
            assert_eq!(text(source), source);
            assert_eq!(free(source), none());
        }
    }
}
