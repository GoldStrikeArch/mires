// HTML Renderer for marked@18.1.1 (ported faithfully from `src/Renderer.ts`).

open Tokens

let code = (lang: string, text: string): string => {
  let langString = switch Rules.notSpaceStart->Rules.execNull(lang) {
  | Value(m) => m->RegExp.Result.fullMatch
  | Null => ""
  }
  let body = text != "" ? text->String.replaceRegExp(Rules.endingNewline, "") ++ "\n" : ""
  if langString == "" {
    "<pre><code>" ++ Helpers.escapeHtmlEntities(body, ~encode=true) ++ "</code></pre>\n"
  } else {
    "<pre><code class=\"language-" ++
    Helpers.escapeHtmlEntities(langString) ++
    "\">" ++
    Helpers.escapeHtmlEntities(body, ~encode=true) ++
    "</code></pre>\n"
  }
}

let blockquote = (tokens: array<blockToken>, ~parse: array<blockToken> => string): string =>
  "<blockquote>\n" ++ parse(tokens) ++ "</blockquote>\n"

let heading = (depth: int, tokens: array<inlineToken>, ~parseInline: array<inlineToken> => string): string => {
  let d = Int.toString(depth)
  "<h" ++ d ++ ">" ++ parseInline(tokens) ++ "</h" ++ d ++ ">\n"
}

let hr = (): string => "<hr>\n"

let listitem = (item: listItem, ~parse: array<blockToken> => string): string =>
  "<li>" ++ parse(item.tokens) ++ "</li>\n"

let list = (
  ordered: bool,
  start: int,
  items: array<listItem>,
  ~parse: array<blockToken> => string,
): string => {
  let body = ref("")
  let len = items->Array.length
  for j in 0 to len - 1 {
    body := body.contents ++ listitem(items->Array.getUnsafe(j), ~parse)
  }
  let tag = ordered ? "ol" : "ul"
  let startAttr = ordered && start != 1 ? " start=\"" ++ Int.toString(start) ++ "\"" : ""
  "<" ++ tag ++ startAttr ++ ">\n" ++ body.contents ++ "</" ++ tag ++ ">\n"
}

let checkbox = (checked: bool): string =>
  "<input " ++ (checked ? "checked=\"\" " : "") ++ "disabled=\"\" type=\"checkbox\"> "

let paragraph = (tokens: array<inlineToken>, ~parseInline: array<inlineToken> => string): string =>
  "<p>" ++ parseInline(tokens) ++ "</p>\n"

let tablecell = (cell: tableCell, ~parseInline: array<inlineToken> => string): string => {
  let content = parseInline(cell.tokens)
  let tag = cell.header ? "th" : "td"
  let openTag = switch cell.align {
  | Some(align) => "<" ++ tag ++ " align=\"" ++ align ++ "\">"
  | None => "<" ++ tag ++ ">"
  }
  openTag ++ content ++ "</" ++ tag ++ ">\n"
}

let table = (
  header: array<tableCell>,
  rows: array<array<tableCell>>,
  ~parseInline: array<inlineToken> => string,
): string => {
  let headCells = ref("")
  let hLen = header->Array.length
  for j in 0 to hLen - 1 {
    headCells := headCells.contents ++ tablecell(header->Array.getUnsafe(j), ~parseInline)
  }
  let headerHtml = "<tr>\n" ++ headCells.contents ++ "</tr>\n"

  let body = ref("")
  let rLen = rows->Array.length
  for j in 0 to rLen - 1 {
    let row = rows->Array.getUnsafe(j)
    let rowCells = ref("")
    let cLen = row->Array.length
    for k in 0 to cLen - 1 {
      rowCells := rowCells.contents ++ tablecell(row->Array.getUnsafe(k), ~parseInline)
    }
    body := body.contents ++ "<tr>\n" ++ rowCells.contents ++ "</tr>\n"
  }
  let bodyHtml = body.contents != "" ? "<tbody>" ++ body.contents ++ "</tbody>" : ""
  "<table>\n<thead>\n" ++ headerHtml ++ "</thead>\n" ++ bodyHtml ++ "</table>\n"
}

let strong = (tokens: array<inlineToken>, ~parseInline: array<inlineToken> => string): string =>
  "<strong>" ++ parseInline(tokens) ++ "</strong>"

let em = (tokens: array<inlineToken>, ~parseInline: array<inlineToken> => string): string =>
  "<em>" ++ parseInline(tokens) ++ "</em>"

let codespan = (text: string): string =>
  "<code>" ++ Helpers.escapeHtmlEntities(text, ~encode=true) ++ "</code>"

let br = (): string => "<br>"

let del = (tokens: array<inlineToken>, ~parseInline: array<inlineToken> => string): string =>
  "<del>" ++ parseInline(tokens) ++ "</del>"

let link = (
  href: string,
  title: string,
  text: string,
  tokens: array<inlineToken>,
  autolink: bool,
  ~parseInline: array<inlineToken> => string,
): string => {
  let parsedText = autolink ? Helpers.escapeHtmlEntities(text, ~encode=true) : parseInline(tokens)
  switch Helpers.cleanUrl(href) {
  | None => parsedText
  | Some(cleanHref) =>
    let escapedHref = Helpers.escapeHtmlEntities(cleanHref, ~encode=autolink)
    let titleAttr =
      title != "" ? " title=\"" ++ Helpers.escapeHtmlEntities(title) ++ "\"" : ""
    "<a href=\"" ++ escapedHref ++ "\"" ++ titleAttr ++ ">" ++ parsedText ++ "</a>"
  }
}

let image = (href: string, title: string, tokens: array<inlineToken>): string => {
  let altText = TextRenderer.renderInline(tokens)
  switch Helpers.cleanUrl(href) {
  | None => Helpers.escapeHtmlEntities(altText)
  | Some(cleanHref) =>
    let titleAttr =
      title != "" ? " title=\"" ++ Helpers.escapeHtmlEntities(title) ++ "\"" : ""
    "<img src=\"" ++
    Helpers.escapeHtmlEntities(cleanHref) ++
    "\" alt=\"" ++
    Helpers.escapeHtmlEntities(altText) ++
    "\"" ++
    titleAttr ++ ">"
  }
}

let inlineText = (text: string, escaped: bool): string =>
  escaped ? text : Helpers.escapeHtmlEntities(text)
