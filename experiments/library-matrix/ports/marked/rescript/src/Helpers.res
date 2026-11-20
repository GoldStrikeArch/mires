// Helper functions for marked@18.1.1 (ported faithfully from `src/helpers.ts`).

@val external encodeURI: string => string = "encodeURI"
@val external parseIntRadix: (string, int) => int = "Number.parseInt"
@send external spliceFrom: (array<'a>, int) => array<'a> = "splice"

let getEscapeReplacement = (ch: string): string =>
  switch ch {
  | "&" => "&amp;"
  | "<" => "&lt;"
  | ">" => "&gt;"
  | "\"" => "&quot;"
  | "'" => "&#39;"
  | _ => ch
  }

let escapeHtmlEntities = (html: string, ~encode: bool=false): string =>
  if encode {
    if Rules.escapeTest->RegExp.test(html) {
      html->Rules.replaceBy1(Rules.escapeReplace, getEscapeReplacement)
    } else {
      html
    }
  } else if Rules.escapeTestNoEncode->RegExp.test(html) {
    html->Rules.replaceBy1(Rules.escapeReplaceNoEncode, getEscapeReplacement)
  } else {
    html
  }

let decodeNumericCharacterReferences = (text: string): string =>
  if !(Rules.numericCharacterReference->RegExp.test(text)) {
    text
  } else {
    text->Rules.replaceBy2Opt(Rules.numericCharacterReferenceOrAmpersand, (_ref, dec, hex) =>
      switch (dec, hex) {
      | (None, None) => "&amp;"
      | _ =>
        let code = switch dec {
        | Some(d) => parseIntRadix(d, 10)
        | None =>
          switch hex {
          | Some(h) => parseIntRadix(h, 16)
          | None => 0
          }
        }
        if code == 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) {
          "\uFFFD"
        } else if code == 38 {
          "&amp;"
        } else {
          String.fromCodePoint(code)
        }
      }
    )
  }

let cleanUrl = (href: string): option<string> =>
  try {
    Some(encodeURI(href)->String.replaceRegExp(Rules.percentDecode, "%"))
  } catch {
  | _ => None
  }

let splitCells = (tableRow: string, ~count: option<int>=?): array<string> => {
  let row = tableRow->Rules.replaceBy0OffsetInput(Rules.findPipe, (_match, offset, str) => {
    let escaped = ref(false)
    let curr = ref(offset - 1)
    while curr.contents >= 0 && str->String.charCodeAtUnsafe(curr.contents) == 92 {
      escaped := !escaped.contents
      curr := curr.contents - 1
    }
    escaped.contents ? "|" : " |"
  })
  let cells = row->String.split(" |")

  if cells->Array.length > 0 && cells->Array.getUnsafe(0)->String.trim == "" {
    ignore(cells->Array.shift)
  }
  let len = cells->Array.length
  if len > 0 && cells->Array.getUnsafe(len - 1)->String.trim == "" {
    ignore(cells->Array.pop)
  }

  switch count {
  | Some(c) if c > 0 =>
    if cells->Array.length > c {
      ignore(cells->spliceFrom(c))
    } else {
      while cells->Array.length < c {
        cells->Array.push("")
      }
    }
  | _ => ()
  }

  for i in 0 to cells->Array.length - 1 {
    cells->Array.setUnsafe(
      i,
      cells->Array.getUnsafe(i)->String.trim->String.replaceRegExp(Rules.slashPipe, "|"),
    )
  }
  cells
}

let rtrim = (str: string, c: string, ~invert: bool=false): string => {
  let l = str->String.length
  if l == 0 {
    ""
  } else {
    let targetCode = c->String.charCodeAtUnsafe(0)
    let suffLen = ref(0)
    let keepGoing = ref(true)
    while suffLen.contents < l && keepGoing.contents {
      let currCode = str->String.charCodeAtUnsafe(l - suffLen.contents - 1)
      if (currCode == targetCode && !invert) || (currCode != targetCode && invert) {
        suffLen := suffLen.contents + 1
      } else {
        keepGoing := false
      }
    }
    str->String.slice(~start=0, ~end=l - suffLen.contents)
  }
}

let trimTrailingBlankLines = (str: string): string => {
  let lines = str->String.split("\n")
  let end = ref(lines->Array.length - 1)
  while (
    end.contents >= 0 && Rules.blankLine->RegExp.test(lines->Array.getUnsafe(end.contents))
  ) {
    end := end.contents - 1
  }
  if lines->Array.length - end.contents <= 2 {
    str
  } else {
    lines->Array.slice(~start=0, ~end=end.contents + 1)->Array.join("\n")
  }
}

let normalizeLabel = (label: string): string =>
  label->String.trim->String.toLowerCase->String.toUpperCase->String.toLowerCase

let findClosingBracket = (str: string, openStr: string, closeStr: string): int =>
  if str->String.indexOf(openStr) == -1 && str->String.indexOf(closeStr) == -1 {
    -1
  } else {
    let openCode = openStr->String.charCodeAtUnsafe(0)
    let closeCode = closeStr->String.charCodeAtUnsafe(0)
    let len = str->String.length
    let level = ref(0)
    let found = ref(-1)
    let i = ref(0)
    while i.contents < len && found.contents == -1 {
      let ch = str->String.charCodeAtUnsafe(i.contents)
      if ch == 92 {
        i := i.contents + 2
      } else if ch == openCode {
        level := level.contents + 1
        i := i.contents + 1
      } else if ch == closeCode {
        level := level.contents - 1
        if level.contents < 0 {
          found := i.contents
        } else {
          i := i.contents + 1
        }
      } else {
        i := i.contents + 1
      }
    }
    if found.contents != -1 {
      found.contents
    } else if level.contents > 0 {
      -2
    } else {
      -1
    }
  }

let expandTabs = (line: string, ~indent: int=0): string =>
  if !(line->String.includes("\t")) {
    line
  } else {
    let len = line->String.length
    let col = ref(indent)
    let segStart = ref(0)
    let out = ref("")
    let i = ref(0)
    while i.contents < len {
      let ch = line->String.charCodeAtUnsafe(i.contents)
      if ch == 9 {
        let added = 4 - mod(col.contents, 4)
        if i.contents > segStart.contents {
          out := out.contents ++ line->String.slice(~start=segStart.contents, ~end=i.contents)
        }
        out := out.contents ++ "    "->String.slice(~start=0, ~end=added)
        col := col.contents + added
        i := i.contents + 1
        segStart := i.contents
      } else if ch >= 0xd800 && ch <= 0xdbff && i.contents + 1 < len {
        let next = line->String.charCodeAtUnsafe(i.contents + 1)
        i := i.contents + (next >= 0xdc00 && next <= 0xdfff ? 2 : 1)
        col := col.contents + 1
      } else {
        i := i.contents + 1
        col := col.contents + 1
      }
    }
    if segStart.contents < len {
      out := out.contents ++ line->String.slice(~start=segStart.contents)
    }
    out.contents
  }

let firstLineOf = (s: string): string => {
  let idx = s->String.indexOf("\n")
  idx == -1 ? s : s->String.slice(~start=0, ~end=idx)
}

// Unicode scalar value length (equivalent to `[...s].length` without allocating an array or iterator).
let codePointLength = (s: string): int => {
  let len = s->String.length
  let count = ref(0)
  let i = ref(0)
  while i.contents < len {
    let c = s->String.charCodeAtUnsafe(i.contents)
    if c >= 0xd800 && c <= 0xdbff && i.contents + 1 < len {
      let next = s->String.charCodeAtUnsafe(i.contents + 1)
      i := i.contents + (next >= 0xdc00 && next <= 0xdfff ? 2 : 1)
    } else {
      i := i.contents + 1
    }
    count := count.contents + 1
  }
  count.contents
}

// UTF-16 code-unit length of the first Unicode scalar value (equivalent to `[...s][0].length`).
let firstCodePointUtf16Len = (s: string): int => {
  let len = s->String.length
  if len >= 2 {
    let c = s->String.charCodeAtUnsafe(0)
    if c >= 0xd800 && c <= 0xdbff {
      let next = s->String.charCodeAtUnsafe(1)
      next >= 0xdc00 && next <= 0xdfff ? 2 : 1
    } else {
      1
    }
  } else {
    len
  }
}
