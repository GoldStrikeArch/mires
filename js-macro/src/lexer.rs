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
//! * an identifier after `.` or `?.`, which is a property and a different namespace;
//! * an object literal key: `{ method: "POST" }` binds nothing, while the shorthand `{ body }` is a
//!   reference and so is a capture, rewritten to `{ body: <slot> }`;
//! * a name the block itself binds, in the region that binds it. The binding forms are `let`,
//!   `const`, `var`, function parameters, arrow parameters, a function's own name, and `catch`.

use std::collections::BTreeSet;

use crate::block;

/// What lexing a block found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Analysis {
    /// The free identifiers, in first occurrence order. Capture `i` fills slot `i`.
    pub free: Vec<String>,
    /// The block, with every free occurrence replaced by its slot name.
    pub text: String,
}

/// The names the analysis treats as always bound, on top of the reserved words.
///
/// The first three are values JavaScript spells as globals rather than as literals. `globalThis` is
/// the door to everything else the host has: `js!{ globalThis.fetch(url) }` reaches the host's
/// `fetch` and captures only `url`. That is deliberate rather than a shortcut around a builtins
/// list. A list would go stale, and going through `globalThis` makes every host name a block
/// depends on greppable, which is what keeps the rule "a free identifier is a Rust binding" total.
pub const HOST_NAMES: [&str; 4] = ["globalThis", "undefined", "NaN", "Infinity"];

/// The reserved words, which are never identifiers.
const RESERVED: [&str; 42] = [
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for",
    "function", "get", "if", "implements", "import", "in", "instanceof", "interface", "let", "new",
    "null", "of", "package", "private", "protected", "public", "return", "set", "static", "super",
    "switch", "this", "throw",
];

/// The rest of the reserved words. Split only to keep each array a readable width.
const RESERVED_REST: [&str; 8] =
    ["true", "try", "typeof", "var", "void", "while", "with", "yield"];

/// Reserved words that END a value, which is what decides a `/` written after one.
const VALUE_WORDS: [&str; 6] = ["this", "super", "true", "false", "null", "yield"];

/// Words that open a statement. A block is one expression, so one of these at the outermost depth
/// is a body this macro does not take.
const STATEMENT_WORDS: [&str; 13] = [
    "break", "const", "continue", "debugger", "do", "for", "if", "let", "return", "switch",
    "throw", "var", "while",
];

/// Whether `word` is a name a block never captures.
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

/// The multi character operators, longest first so a munch is greedy. `/` and `/=` are absent
/// because a leading `/` is decided by [`Scan::regex_follows`] before it reaches here.
const OPERATORS: [&str; 25] = [
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=",
];

/// The rest of them. Split only to keep each array a readable width.
const OPERATORS_REST: [&str; 8] = ["%=", "&=", "|=", "^=", "**", "<<", ">>", "="];

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
    /// Anything else, as the operator it munched to. `${` is spelled as one.
    Op(String),
}

impl Prev {
    /// Whether what came before ends a VALUE, which is what makes a following `/` a division.
    fn ends_a_value(&self) -> bool {
        match self {
            Prev::Word(word) => !is_reserved(word) || VALUE_WORDS.contains(&word.as_str()),
            Prev::Literal | Prev::Regex | Prev::Close(_) => true,
            Prev::Op(op) => op == "++" || op == "--",
            Prev::Start | Prev::Open(_) => false,
        }
    }
}

/// One bracketed region, and what the scan needs to remember about it.
///
/// The stack always holds a root frame for the block itself, so a `?:` and a `,` written outside
/// every bracket are tracked the same way as ones written inside.
#[derive(Debug)]
struct Frame {
    /// `(`, `[`, `{`, or `\0` for the root.
    open: char,
    /// Whether a `{` is an object literal rather than a block.
    object: bool,
    /// Whether a `{` opened a template literal's `${`, so its `}` resumes the template.
    substitution: bool,
    /// Open `?`s waiting for their `:` at this depth, so a ternary's `:` is not read as a key's.
    ternary: usize,
    /// Where this frame's occurrences begin, for the retroactive pass that turns `(a, b) =>` into
    /// a parameter list.
    occurrences: usize,
    /// Whether a bare `=` has been written at this frame's own depth since the last `,`. A
    /// parameter list's default value is a reference and must survive that pass.
    default: bool,
    /// Whether the frame is a parameter list already, which `function`, `catch` and a method
    /// shorthand say at the `(` and an arrow only says at the `)`.
    params: bool,
}

impl Frame {
    /// The frame the block itself is written in.
    fn root() -> Self {
        Self {
            open: '\0',
            object: false,
            substitution: false,
            ternary: 0,
            occurrences: 0,
            default: false,
            params: false,
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

/// A region of the block over which some names are bound.
#[derive(Debug)]
struct Scope {
    names: BTreeSet<String>,
    /// The bracket depth this scope needs the scan to stay at or below. It ends when the depth
    /// drops under it. [`PENDING`] while the scope is waiting to learn where its body is.
    depth: usize,
    /// Whether the body is an expression rather than a block, so a `,`, a `;` or a non ternary `:`
    /// at [`Scope::depth`] ends it too.
    expression: bool,
}

/// A scope that has its parameters and not yet its body.
const PENDING: usize = usize::MAX;

/// Reads a block, one character at a time.
struct Scan<'a> {
    chars: &'a [char],
    at: usize,
    /// One entry per template literal nesting level. The top says which of the two modes the scan
    /// is in: `true` inside the literal's text, `false` inside a `${}` substitution.
    templates: Vec<bool>,
    frames: Vec<Frame>,
    scopes: Vec<Scope>,
    occurrences: Vec<Occurrence>,
    prev: Prev,
    /// The scope whose body extent is not known yet.
    pending: Option<usize>,
    /// The next `(` opens a parameter list, said by `function`, `catch` or a method shorthand.
    expect_params: bool,
    /// The next identifier is a function's own name.
    expect_name: bool,
    /// The frame depth a `let`, `const` or `var` was written at, while it is still in effect.
    declaring: Option<usize>,
    /// The depth of the `=` whose right hand side the scan is in, so an initializer reads as
    /// references rather than as bindings.
    initializing: Option<usize>,
}

impl<'a> Scan<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            at: 0,
            templates: Vec::new(),
            frames: vec![Frame::root()],
            scopes: vec![Scope { names: BTreeSet::new(), depth: 0, expression: false }],
            occurrences: Vec::new(),
            prev: Prev::Start,
            pending: None,
            expect_params: false,
            expect_name: false,
            declaring: None,
            initializing: None,
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    /// The frame the scan is in, which is never absent: the root is one.
    fn frame(&self) -> &Frame {
        self.frames.last().expect("the root frame is never popped")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the root frame is never popped")
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
        self.scopes.last_mut().expect("the root scope is never dropped").insert_name(name);
    }

    /// Drops every scope whose region the scan has left.
    fn close_scopes(&mut self) {
        let depth = self.depth();
        while self.scopes.len() > 1 {
            let scope = self.scopes.last().expect("the root scope is never dropped");
            if scope.depth == PENDING || depth >= scope.depth {
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
            if scope.depth != depth || !scope.expression {
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
        self.scopes.push(Scope { names, depth: PENDING, expression: false });
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
        self.scopes[index].depth = depth;
        self.scopes[index].expression = true;
        self.pending = None;
    }
}

impl Scope {
    fn insert_name(&mut self, name: &str) {
        self.names.insert(name.to_owned());
    }
}

/// Reads `source` and answers which of its identifiers are captures.
///
/// # Errors
///
/// Returns why the block is not one this macro takes: an unterminated string, template, comment or
/// regular expression, unbalanced brackets, a name that collides with an emitted slot, or a body
/// that is a sequence of statements rather than an expression.
pub fn analyze(source: &str) -> Result<Analysis, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut scan = Scan::new(&chars);

    while scan.at < chars.len() {
        match scan.templates.last() {
            Some(true) => template(&mut scan)?,
            _ => code(&mut scan)?,
        }
    }

    if scan.frames.len() > 1 {
        return Err(format!("`{}` is never closed", scan.frame().open));
    }
    if scan.templates.last() == Some(&true) {
        return Err("a template literal is never closed".to_owned());
    }

    Ok(splice(&chars, &scan.occurrences))
}

/// The block with every free occurrence replaced by the slot that fills it.
fn splice(chars: &[char], occurrences: &[Occurrence]) -> Analysis {
    let mut free: Vec<String> = Vec::new();
    let mut text = String::with_capacity(chars.len());
    let mut cursor = 0;
    for occurrence in occurrences.iter().filter(|occurrence| occurrence.free) {
        let slot = match free.iter().position(|name| *name == occurrence.name) {
            Some(index) => index,
            None => {
                free.push(occurrence.name.clone());
                free.len() - 1
            }
        };
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
    Analysis { free, text }
}

/// One step of the scan, outside a template literal.
fn code(scan: &mut Scan<'_>) -> Result<(), String> {
    let character = scan.chars[scan.at];

    if character.is_whitespace() {
        scan.at += 1;
        return Ok(());
    }
    if character == '/' && scan.peek(1) == Some('/') {
        while scan.peek(0).is_some_and(|c| c != '\n') {
            scan.at += 1;
        }
        return Ok(());
    }
    if character == '/' && scan.peek(1) == Some('*') {
        scan.at += 2;
        loop {
            match scan.peek(0) {
                None => return Err("a block comment is never closed".to_owned()),
                Some('*') if scan.peek(1) == Some('/') => {
                    scan.at += 2;
                    return Ok(());
                }
                Some(_) => scan.at += 1,
            }
        }
    }
    if character == '/' && scan.regex_follows() {
        return regex(scan);
    }
    if character == '"' || character == '\'' {
        return string(scan, character);
    }
    if character == '`' {
        scan.at += 1;
        scan.templates.push(true);
        return Ok(());
    }
    if character.is_ascii_digit()
        || (character == '.' && scan.peek(1).is_some_and(|c| c.is_ascii_digit()))
    {
        return number(scan);
    }
    if is_ident_start(character) {
        return identifier(scan);
    }
    punctuation(scan)
}

/// One step of the scan, inside a template literal.
fn template(scan: &mut Scan<'_>) -> Result<(), String> {
    match scan.chars[scan.at] {
        '\\' => scan.at += 2,
        '`' => {
            scan.at += 1;
            scan.templates.pop();
            scan.prev = Prev::Literal;
        }
        '$' if scan.peek(1) == Some('{') => {
            scan.at += 2;
            // The substitution is code again, and its `}` puts the scan back in the template.
            scan.templates.push(false);
            let occurrences = scan.occurrences.len();
            scan.frames.push(Frame {
                open: '{',
                substitution: true,
                occurrences,
                ..Frame::root()
            });
            scan.prev = Prev::Op("${".to_owned());
        }
        _ => scan.at += 1,
    }
    Ok(())
}

/// A string literal, skipped whole.
fn string(scan: &mut Scan<'_>, quote: char) -> Result<(), String> {
    scan.at += 1;
    loop {
        match scan.peek(0) {
            None => return Err(format!("a {quote} string is never closed")),
            Some('\\') => scan.at += 2,
            Some(character) if character == quote => {
                scan.at += 1;
                scan.prev = Prev::Literal;
                return Ok(());
            }
            Some(_) => scan.at += 1,
        }
    }
}

/// A regular expression literal, skipped whole. A `[..]` class may hold an unescaped `/`.
fn regex(scan: &mut Scan<'_>) -> Result<(), String> {
    scan.at += 1;
    let mut in_class = false;
    loop {
        match scan.peek(0) {
            None | Some('\n') => return Err("a regular expression is never closed".to_owned()),
            Some('\\') => scan.at += 2,
            Some('[') => {
                in_class = true;
                scan.at += 1;
            }
            Some(']') => {
                in_class = false;
                scan.at += 1;
            }
            Some('/') if !in_class => {
                scan.at += 1;
                while scan.peek(0).is_some_and(is_ident_continue) {
                    scan.at += 1;
                }
                scan.prev = Prev::Regex;
                return Ok(());
            }
            Some(_) => scan.at += 1,
        }
    }
}

/// A number literal, skipped whole so that `0x1f` is not read as an identifier `x1f`.
fn number(scan: &mut Scan<'_>) -> Result<(), String> {
    let mut seen_point = false;
    while let Some(character) = scan.peek(0) {
        if character == '.' && !seen_point {
            seen_point = true;
            scan.at += 1;
            continue;
        }
        if matches!(character, 'e' | 'E' | 'p' | 'P') && matches!(scan.peek(1), Some('+' | '-')) {
            scan.at += 2;
            continue;
        }
        if character.is_alphanumeric() || character == '_' {
            scan.at += 1;
            continue;
        }
        break;
    }
    scan.prev = Prev::Literal;
    Ok(())
}

/// An identifier, classified where it stands.
fn identifier(scan: &mut Scan<'_>) -> Result<(), String> {
    let start = scan.at;
    while scan.peek(0).is_some_and(is_ident_continue) {
        scan.at += 1;
    }
    let end = scan.at;
    let word: String = scan.chars[start..end].iter().collect();

    if block::is_slot_name(&word) {
        return Err(format!("`{word}` is what a capture is emitted as, so a block cannot spell it"));
    }

    let after_dot = matches!(&scan.prev, Prev::Op(op) if op == "." || op == "?.");
    let next = scan.significant(end).map(|(_, character)| character);

    if STATEMENT_WORDS.contains(&word.as_str()) && scan.depth() == 0 && !after_dot {
        return Err(format!("`{word}` opens a statement and a `js!{{}}` block is one expression"));
    }

    let mut free = false;
    let mut shorthand = false;
    if after_dot {
        // A property, which is a namespace of its own.
    } else if is_reserved(&word) {
        keyword(scan, &word);
    } else if entry_position(scan) && matches!(next, Some(':' | '(')) {
        // An object literal key binds nothing and is not a reference. A method shorthand's
        // parameters are a parameter list, which its `(` has to be told about.
        if next == Some('(') {
            scan.expect_params = true;
        }
    } else if scan.expect_name {
        scan.expect_name = false;
        scan.bind(&word);
    } else if scan.binding() {
        scan.bind(&word);
    } else if !scan.bound(&word) {
        free = true;
        shorthand = entry_position(scan) && matches!(next, Some(',' | '}'));
    }

    let default = scan.frame().default;
    scan.occurrences.push(Occurrence { start, end, name: word.clone(), free, default, shorthand });
    scan.prev = Prev::Word(word);
    Ok(())
}

/// Whether an identifier here opens an entry of an object literal, which is what makes it either a
/// key (`{ method: .. }`) or a shorthand (`{ body }`) rather than an ordinary reference.
///
/// Three things have to hold at once, and each rules out a case that would otherwise be a silent
/// miss. The enclosing bracket must be a `{` that is an object literal, so a `case x:` inside a
/// block is not a key. What came before must be that `{` or a `,`, so `a ? b : c` is not one
/// either. And the frame must have no open `?`, for the same reason one step further out.
fn entry_position(scan: &Scan<'_>) -> bool {
    let frame = scan.frame();
    if frame.open != '{' || !frame.object || frame.ternary > 0 {
        return false;
    }
    matches!(&scan.prev, Prev::Open('{')) || matches!(&scan.prev, Prev::Op(op) if op == ",")
}

/// What a reserved word does to the scan's state.
fn keyword(scan: &mut Scan<'_>, word: &str) {
    match word {
        "let" | "const" | "var" => {
            scan.declaring = Some(scan.depth());
            scan.initializing = None;
        }
        // A `for (const x of xs)` head binds `x` and references `xs`.
        "of" | "in" => scan.initializing = Some(scan.depth()),
        "function" => {
            scan.expect_params = true;
            scan.expect_name = true;
        }
        "catch" => scan.expect_params = true,
        _ => {}
    }
}

/// Everything that is not a word, a literal or a comment.
fn punctuation(scan: &mut Scan<'_>) -> Result<(), String> {
    match scan.chars[scan.at] {
        character @ ('(' | '[' | '{') => open(scan, character),
        character @ (')' | ']' | '}') => close(scan, character),
        ';' => semicolon(scan),
        ',' => {
            if scan.initializing == Some(scan.depth()) {
                scan.initializing = None;
            }
            scan.frame_mut().default = false;
            scan.at += 1;
            scan.prev = Prev::Op(",".to_owned());
            scan.close_expression_scopes();
            Ok(())
        }
        ':' => {
            let ternary = scan.frame().ternary > 0;
            if ternary {
                scan.frame_mut().ternary -= 1;
            }
            scan.at += 1;
            scan.prev = Prev::Op(":".to_owned());
            if !ternary {
                scan.close_expression_scopes();
            }
            Ok(())
        }
        '?' if !matches!(scan.peek(1), Some('.' | '?')) => {
            scan.frame_mut().ternary += 1;
            scan.at += 1;
            scan.prev = Prev::Op("?".to_owned());
            Ok(())
        }
        _ => operator(scan),
    }
}

/// An opening bracket.
fn open(scan: &mut Scan<'_>, character: char) -> Result<(), String> {
    let object = character == '{' && opens_an_object(scan);
    let params = character == '(' && scan.expect_params;
    scan.expect_params = false;
    scan.expect_name = false;
    let occurrences = scan.occurrences.len();
    scan.frames.push(Frame { open: character, object, occurrences, params, ..Frame::root() });
    // A scope waiting on its body gets it here: a `{` after a parameter list is that body.
    if character == '{' && !object {
        if let Some(index) = scan.pending.take() {
            scan.scopes[index].depth = scan.depth();
            scan.scopes[index].expression = false;
        }
    }
    scan.at += 1;
    scan.prev = Prev::Open(character);
    Ok(())
}

/// A closing bracket, and everything that closing one settles.
fn close(scan: &mut Scan<'_>, character: char) -> Result<(), String> {
    if scan.frames.len() == 1 {
        return Err(format!("`{character}` closes a bracket that was never opened"));
    }
    let frame = scan.frames.pop().expect("the length was checked");
    let expected = match frame.open {
        '(' => ')',
        '[' => ']',
        _ => '}',
    };
    if character != expected {
        return Err(format!("`{}` is closed by `{character}`", frame.open));
    }
    scan.at += 1;

    if frame.substitution {
        // Back into the template literal the `${` interrupted.
        scan.templates.pop();
        scan.prev = Prev::Literal;
        return Ok(());
    }
    scan.prev = Prev::Close(character);

    if scan.declaring.is_some_and(|root| scan.depth() < root) {
        scan.declaring = None;
        scan.initializing = None;
    }
    if scan.initializing.is_some_and(|depth| scan.depth() < depth) {
        scan.initializing = None;
    }
    scan.close_scopes();

    if character != ')' {
        return Ok(());
    }
    match (scan.arrow_ahead(), frame.params) {
        (Some(body), _) => {
            scan.take_parameters(frame.occurrences);
            scan.at = body;
            scan.prev = Prev::Op("=>".to_owned());
            scan.settle_pending();
        }
        (None, true) => {
            scan.take_parameters(frame.occurrences);
            scan.settle_pending();
        }
        (None, false) => {}
    }
    Ok(())
}

/// A `;`, which at the outermost depth is what says a body is statements.
fn semicolon(scan: &mut Scan<'_>) -> Result<(), String> {
    if scan.depth() == 0 {
        return Err("a `js!{}` block is one expression, and `;` ends a statement".to_owned());
    }
    if scan.declaring == Some(scan.depth()) {
        scan.declaring = None;
        scan.initializing = None;
    }
    scan.at += 1;
    scan.prev = Prev::Op(";".to_owned());
    scan.close_expression_scopes();
    Ok(())
}

/// Everything else, as the longest operator that starts here.
fn operator(scan: &mut Scan<'_>) -> Result<(), String> {
    let operator = munch(scan);
    scan.at += operator.chars().count();

    if operator == "=" {
        if scan.declaring.is_some() {
            scan.initializing = Some(scan.depth());
        }
        scan.frame_mut().default = true;
    }
    if operator == "=>" {
        // `a => ..`: the bare single parameter, whose `(..)` counterpart is settled at its `)`.
        let from = match (&scan.prev, scan.occurrences.last()) {
            (Prev::Word(word), Some(last)) if *word == last.name => scan.occurrences.len() - 1,
            _ => scan.occurrences.len(),
        };
        scan.take_parameters(from);
        scan.prev = Prev::Op(operator);
        scan.settle_pending();
        return Ok(());
    }

    scan.prev = Prev::Op(operator);
    Ok(())
}

/// Whether a `{` here opens an object literal rather than a block.
///
/// Everything is an object literal except after the tokens that can only be followed by a block. A
/// `js!{}` block is an expression, so the literal is the common direction; the exceptions are an
/// arrow's block body, a `)` that closed a head or a parameter list, and the keywords that take a
/// block on their own.
fn opens_an_object(scan: &Scan<'_>) -> bool {
    match &scan.prev {
        Prev::Close(')' | '}') | Prev::Open('{') => false,
        Prev::Op(op) => op != "=>" && op != ";",
        Prev::Word(word) => !matches!(word.as_str(), "else" | "do" | "try" | "finally"),
        _ => true,
    }
}

/// The longest operator that starts at the cursor, or the single character written there.
fn munch(scan: &Scan<'_>) -> String {
    let rest: String = scan.chars[scan.at..].iter().take(4).collect();
    for operator in OPERATORS.iter().chain(OPERATORS_REST.iter()) {
        if rest.starts_with(operator) {
            return (*operator).to_owned();
        }
    }
    scan.chars[scan.at].to_string()
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
        analyze(source).expect_err("it is refused")
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
        assert_eq!(free("(() => { switch (a) { case b: return c; } })"), ["a", "b", "c"]);
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
        assert_eq!(free("(() => { const { a, b: c } = d; return a + c; })"), ["d"]);
        assert_eq!(free("(() => { const [a, b] = c; return a + b; })"), ["c"]);
        // A destructuring default is a reference.
        assert_eq!(free("(() => { const { a = z } = d; return a; })"), ["z", "d"]);
    }

    #[test]
    fn a_for_head_and_a_catch_bind_what_they_declare() {
        assert_eq!(free("(() => { for (const x of xs) { f(x); } })"), ["xs", "f"]);
        assert_eq!(free("(() => { for (const k in o) { f(k); } })"), ["o", "f"]);
        assert_eq!(free("(() => { try { f(); } catch (e) { g(e); } })"), ["f", "g"]);
        // The declaration ends with its head, so what follows reads as references again.
        assert_eq!(free("(() => { for (let i = 0; i < n; i++) { f(i); } })"), ["n", "f"]);
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
        let message = error("_$js0 + a");
        assert!(message.contains("_$js0"), "{message}");
        assert!(message.contains("capture"), "{message}");
        assert!(error("(_$js1) => 1").contains("_$js1"));
    }

    #[test]
    fn a_body_that_is_statements_rather_than_an_expression_is_refused() {
        for source in ["const a = 1", "return a", "if (a) b", "for (;;) {}", "let a", "a; b"] {
            let message = error(source);
            assert!(message.contains("one expression"), "`{source}` reported `{message}`");
        }
        // Inside a function body they are ordinary statements.
        assert_eq!(free("(() => { const a = 1; return a; })"), none());
    }

    #[test]
    fn an_unterminated_anything_is_refused() {
        for (source, why) in [
            ("\"a", "never closed"),
            ("'a", "never closed"),
            ("`a", "never closed"),
            ("/* a", "never closed"),
            ("f(a", "never closed"),
            ("f(a]", "closed by"),
            ("a)", "never opened"),
            ("x ? /a", "never closed"),
        ] {
            let message = error(source);
            assert!(message.contains(why), "`{source}` reported `{message}`");
        }
    }

    #[test]
    fn a_block_with_no_captures_is_left_exactly_as_written() {
        for source in ["r => r.text()", "(a, b) => a + b", "1 + 1", "\"x\"", "`a${1}b`"] {
            assert_eq!(text(source), source);
            assert_eq!(free(source), none());
        }
    }
}
