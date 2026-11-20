// Token and lexer-state types for marked@18.1.1.

type options = {
  gfm: bool,
  breaks: bool,
  pedantic: bool,
}

type linkDef = {
  href: string,
  title: string,
}

type rec inlineToken =
  | Escape({raw: string, text: string})
  | Html({raw: string, inLink: bool, inRawBlock: bool, text: string})
  | Link({
      raw: string,
      href: string,
      title: string,
      text: string,
      tokens: array<inlineToken>,
      autolink: bool,
    })
  | Image({raw: string, href: string, title: string, text: string, tokens: array<inlineToken>})
  | Checkbox({raw: string, checked: bool})
  | Strong({raw: string, text: string, tokens: array<inlineToken>})
  | Em({raw: string, text: string, tokens: array<inlineToken>})
  | Codespan({raw: string, text: string})
  | Br({raw: string})
  | Del({raw: string, text: string, tokens: array<inlineToken>})
  | Text({mutable raw: string, mutable text: string, escaped: bool})

type tableCell = {
  text: string,
  tokens: array<inlineToken>,
  header: bool,
  align: option<string>,
}

type textBlock = {
  mutable raw: string,
  mutable text: string,
  tokens: array<inlineToken>,
}

type rec listItem = {
  mutable raw: string,
  mutable task: bool,
  mutable checked: bool,
  mutable loose: bool,
  mutable text: string,
  mutable tokens: array<blockToken>,
}
and blockToken =
  | Space({mutable raw: string})
  | Code({mutable raw: string, lang: string, text: string})
  | Heading({mutable raw: string, depth: int, text: string, tokens: array<inlineToken>})
  | Hr({mutable raw: string})
  | Blockquote({mutable raw: string, text: string, tokens: array<blockToken>})
  | List({
      mutable raw: string,
      ordered: bool,
      start: int,
      mutable loose: bool,
      items: array<listItem>,
    })
  | Html({mutable raw: string, text: string})
  | Def({mutable raw: string, tag: string, href: string, title: string})
  | Table({mutable raw: string, header: array<tableCell>, rows: array<array<tableCell>>})
  | Paragraph(textBlock)
  | Text(textBlock)
  | Checkbox({mutable raw: string, checked: bool})

type inlineQueueEntry = {
  mutable src: string,
  tokens: array<inlineToken>,
}

type lexer = {
  options: options,
  rules: Rules.rules,
  links: Map.t<string, linkDef>,
  inlineQueue: array<inlineQueueEntry>,
  mutable inLink: bool,
  mutable inRawBlock: bool,
  mutable linkEmitted: bool,
  mutable linkParenPossible: bool,
  mutable top: bool,
}

let queueInline = (lexer: lexer, src: string): array<inlineToken> => {
  let tokens: array<inlineToken> = []
  lexer.inlineQueue->Array.push({src, tokens})
  tokens
}

let appendBlockRawNewline = (token: blockToken): unit =>
  switch token {
  | Space(t) => t.raw = t.raw ++ "\n"
  | Code(t) => t.raw = t.raw ++ "\n"
  | Heading(t) => t.raw = t.raw ++ "\n"
  | Hr(t) => t.raw = t.raw ++ "\n"
  | Blockquote(t) => t.raw = t.raw ++ "\n"
  | List(t) => t.raw = t.raw ++ "\n"
  | Html(t) => t.raw = t.raw ++ "\n"
  | Def(t) => t.raw = t.raw ++ "\n"
  | Table(t) => t.raw = t.raw ++ "\n"
  | Paragraph(t) | Text(t) => t.raw = t.raw ++ "\n"
  | Checkbox(t) => t.raw = t.raw ++ "\n"
  }
