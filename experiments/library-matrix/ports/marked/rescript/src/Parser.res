// Parser for marked@18.1.1 (ported faithfully from `src/Parser.ts`).

open Tokens

let rec parseInline = (tokens: array<inlineToken>): string => {
  let out = ref("")
  let len = tokens->Array.length
  for i in 0 to len - 1 {
    let html = switch tokens->Array.getUnsafe(i) {
    | Escape({text}) => Renderer.inlineText(text, false)
    | Html({text}) => text
    | Link({href, title, text, tokens, autolink}) =>
      Renderer.link(href, title, text, tokens, autolink, ~parseInline)
    | Image({href, title, tokens}) => Renderer.image(href, title, tokens)
    | Checkbox({checked}) => Renderer.checkbox(checked)
    | Strong({tokens}) => Renderer.strong(tokens, ~parseInline)
    | Em({tokens}) => Renderer.em(tokens, ~parseInline)
    | Codespan({text}) => Renderer.codespan(text)
    | Br(_) => Renderer.br()
    | Del({tokens}) => Renderer.del(tokens, ~parseInline)
    | Text({text, escaped}) => Renderer.inlineText(text, escaped)
    }
    out := out.contents ++ html
  }
  out.contents
}

let rec parse = (tokens: array<blockToken>): string => {
  let out = ref("")
  let len = tokens->Array.length
  for i in 0 to len - 1 {
    let html = switch tokens->Array.getUnsafe(i) {
    | Space(_) => ""
    | Hr(_) => Renderer.hr()
    | Heading({depth, tokens}) => Renderer.heading(depth, tokens, ~parseInline)
    | Code({lang, text}) => Renderer.code(lang, text)
    | Table({header, rows}) => Renderer.table(header, rows, ~parseInline)
    | Blockquote({tokens}) => Renderer.blockquote(tokens, ~parse)
    | List({ordered, start, items}) => Renderer.list(ordered, start, items, ~parse)
    | Checkbox({checked}) => Renderer.checkbox(checked)
    | Html({text}) => text
    | Def(_) => ""
    | Paragraph({tokens}) => Renderer.paragraph(tokens, ~parseInline)
    | Text({tokens}) => parseInline(tokens)
    }
    out := out.contents ++ html
  }
  out.contents
}
