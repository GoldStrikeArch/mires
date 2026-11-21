//! Options, grammar mode, link definitions, and block/inline token types mirroring marked 18.1.1.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy)]
pub struct Options {
    pub gfm: bool,
    pub breaks: bool,
    pub pedantic: bool,
}

impl Options {
    pub const fn defaults() -> Self {
        Self {
            gfm: true,
            breaks: false,
            pedantic: false,
        }
    }

    #[inline]
    pub fn mode(self) -> Mode {
        if self.pedantic {
            Mode::Pedantic
        } else if self.gfm {
            if self.breaks {
                Mode::Breaks
            } else {
                Mode::Gfm
            }
        } else {
            Mode::Normal
        }
    }
}

/// Active grammar rule set selected by `Lexer` constructor (`Lexer.ts` lines 56-66).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Gfm,
    Breaks,
    Pedantic,
}

impl Mode {
    #[inline]
    pub fn is_pedantic(self) -> bool {
        matches!(self, Mode::Pedantic)
    }

    #[inline]
    pub fn is_gfm(self) -> bool {
        matches!(self, Mode::Gfm | Mode::Breaks)
    }

    #[inline]
    pub fn is_breaks(self) -> bool {
        matches!(self, Mode::Breaks)
    }
}

#[derive(Clone)]
pub struct LinkDef {
    pub href: String,
    pub title: Option<String>,
}

/// One queued inline-lexing job (`Lexer.inlineQueue` in `Lexer.ts`), owned in a flat arena so
/// block tokens (`Heading`, `Paragraph`, `Text`, `TableCell`) hold a lightweight `usize` slot id
/// with no `Rc`/`RefCell`.
pub struct InlineSlot {
    pub src: String,
    pub tokens: Vec<InlineToken>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

pub struct TableCell {
    pub header: bool,
    pub align: Align,
    pub inline_id: usize,
}

pub struct ListItem {
    pub raw: String,
    pub task: bool,
    pub checked: bool,
    pub loose: bool,
    pub text: String,
    pub tokens: Vec<BlockToken>,
}

pub enum BlockToken {
    Space {
        raw: String,
    },
    Hr {
        raw_len: usize,
    },
    Heading {
        raw_len: usize,
        depth: usize,
        inline_id: usize,
    },
    Code {
        raw_len: usize,
        lang: Option<String>,
        text: String,
    },
    Html {
        raw_len: usize,
        text: String,
    },
    Def {
        raw_len: usize,
    },
    Blockquote {
        raw: String,
        tokens: Vec<BlockToken>,
    },
    List {
        raw: String,
        ordered: bool,
        start: u64,
        loose: bool,
        items: Vec<ListItem>,
    },
    Checkbox {
        checked: bool,
    },
    Table {
        raw_len: usize,
        header: Vec<TableCell>,
        rows: Vec<Vec<TableCell>>,
    },
    Paragraph {
        raw_len: usize,
        text: String,
        inline_id: usize,
    },
    Text {
        raw_len: usize,
        text: String,
        inline_id: usize,
    },
}

impl BlockToken {
    pub fn raw_len(&self) -> usize {
        match self {
            BlockToken::Space { raw }
            | BlockToken::Blockquote { raw, .. }
            | BlockToken::List { raw, .. } => raw.len(),
            BlockToken::Hr { raw_len }
            | BlockToken::Heading { raw_len, .. }
            | BlockToken::Code { raw_len, .. }
            | BlockToken::Html { raw_len, .. }
            | BlockToken::Def { raw_len }
            | BlockToken::Table { raw_len, .. }
            | BlockToken::Paragraph { raw_len, .. }
            | BlockToken::Text { raw_len, .. } => *raw_len,
            BlockToken::Checkbox { .. } => 0,
        }
    }

    pub fn append_newline_to_raw(&mut self) {
        match self {
            BlockToken::Space { raw }
            | BlockToken::Blockquote { raw, .. }
            | BlockToken::List { raw, .. } => raw.push('\n'),
            BlockToken::Hr { raw_len }
            | BlockToken::Heading { raw_len, .. }
            | BlockToken::Code { raw_len, .. }
            | BlockToken::Html { raw_len, .. }
            | BlockToken::Def { raw_len }
            | BlockToken::Table { raw_len, .. }
            | BlockToken::Paragraph { raw_len, .. }
            | BlockToken::Text { raw_len, .. } => *raw_len += 1,
            BlockToken::Checkbox { .. } => {}
        }
    }
}

pub enum InlineToken {
    Escape {
        text: String,
    },
    Html {
        text: String,
    },
    Link {
        href: Option<String>,
        title: Option<String>,
        text: String,
        autolink: bool,
        tokens: Vec<InlineToken>,
    },
    Image {
        href: Option<String>,
        title: Option<String>,
        text: String,
        tokens: Vec<InlineToken>,
    },
    Checkbox {
        raw: String,
        checked: bool,
    },
    Strong {
        text: String,
        tokens: Vec<InlineToken>,
    },
    Em {
        text: String,
        tokens: Vec<InlineToken>,
    },
    Codespan {
        text: String,
    },
    Br,
    Del {
        text: String,
        tokens: Vec<InlineToken>,
    },
    Text {
        text: String,
        escaped: bool,
    },
}
