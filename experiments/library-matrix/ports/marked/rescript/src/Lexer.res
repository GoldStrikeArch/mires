// Block & Inline Lexer for marked@18.1.1 (ported faithfully from `src/Lexer.ts`).

open Tokens

let make = (options: options): lexer => {
  options,
  rules: Rules.selectRules(
    ~gfm=options.gfm,
    ~breaks=options.breaks,
    ~pedantic=options.pedantic,
  ),
  links: Map.make(),
  inlineQueue: [],
  inLink: false,
  inRawBlock: false,
  linkEmitted: false,
  linkParenPossible: true,
  top: true,
}

let infiniteLoopError = (byte: int): 'a =>
  JsError.make("Infinite loop on byte: " ++ Int.toString(byte))->JsError.throw

let rec blockTokens = (
  lexer: lexer,
  srcInput: string,
  tokens: array<blockToken>,
  lastParagraphClippedInit: bool,
): array<blockToken> => {
  let src = ref(
    lexer.options.pedantic
      ? srcInput
        ->String.replaceRegExp(Rules.tabCharGlobal, "    ")
        ->String.replaceRegExp(Rules.spaceLine, "")
      : srcInput,
  )
  let lastParagraphClipped = ref(lastParagraphClippedInit)
  let prevLen = ref(src.contents->String.length + 1)

  while src.contents != "" {
    let cur = src.contents
    let curLen = cur->String.length
    if curLen < prevLen.contents {
      prevLen := curLen
    } else {
      infiniteLoopError(cur->String.charCodeAtUnsafe(0))
    }

    // newline
    switch Tokenizer.space(lexer, cur) {
    | Some(Space(sp) as tok) =>
      src := cur->String.substring(~start=sp.raw->String.length)
      let tLen = tokens->Array.length
      if sp.raw->String.length == 1 && tLen > 0 {
        appendBlockRawNewline(tokens->Array.getUnsafe(tLen - 1))
      } else {
        tokens->Array.push(tok)
      }
    | _ =>
      // code
      switch Tokenizer.code(lexer, cur) {
      | Some(Code(cd) as tok) =>
        src := cur->String.substring(~start=cd.raw->String.length)
        let tLen = tokens->Array.length
        let merged = if tLen > 0 {
          switch tokens->Array.getUnsafe(tLen - 1) {
          | Paragraph(last) | Text(last) =>
            last.raw =
              last.raw ++ (last.raw->String.endsWith("\n") ? "" : "\n") ++ cd.raw
            last.text = last.text ++ "\n" ++ cd.text
            let qLen = lexer.inlineQueue->Array.length
            if qLen > 0 {
              (lexer.inlineQueue->Array.getUnsafe(qLen - 1)).src = last.text
            }
            true
          | _ => false
          }
        } else {
          false
        }
        if !merged {
          tokens->Array.push(tok)
        }
      | _ =>
        // fences
        switch Tokenizer.fences(lexer, cur) {
        | Some(Code(cd) as tok) =>
          src := cur->String.substring(~start=cd.raw->String.length)
          tokens->Array.push(tok)
        | _ =>
          // heading
          switch Tokenizer.heading(lexer, cur) {
          | Some(Heading(hd) as tok) =>
            src := cur->String.substring(~start=hd.raw->String.length)
            tokens->Array.push(tok)
          | _ =>
            // hr
            switch Tokenizer.hr(lexer, cur) {
            | Some(Hr(h) as tok) =>
              src := cur->String.substring(~start=h.raw->String.length)
              tokens->Array.push(tok)
            | _ =>
              // blockquote
              switch Tokenizer.blockquote(lexer, cur, ~blockTokens) {
              | Some(Blockquote(bq) as tok) =>
                src := cur->String.substring(~start=bq.raw->String.length)
                tokens->Array.push(tok)
              | _ =>
                // list
                switch Tokenizer.list(lexer, cur, ~blockTokens) {
                | Some(List(ls) as tok) =>
                  src := cur->String.substring(~start=ls.raw->String.length)
                  tokens->Array.push(tok)
                | _ =>
                  // html
                  switch Tokenizer.html(lexer, cur) {
                  | Some(Html(ht) as tok) =>
                    src := cur->String.substring(~start=ht.raw->String.length)
                    tokens->Array.push(tok)
                  | _ =>
                    // def
                    switch Tokenizer.def(lexer, cur) {
                    | Some(Def(df) as tok) =>
                      src := cur->String.substring(~start=df.raw->String.length)
                      let tLen = tokens->Array.length
                      let merged = if tLen > 0 {
                        switch tokens->Array.getUnsafe(tLen - 1) {
                        | Paragraph(last) | Text(last) =>
                          last.raw =
                            last.raw ++ (last.raw->String.endsWith("\n") ? "" : "\n") ++ df.raw
                          last.text = last.text ++ "\n" ++ df.raw
                          let qLen = lexer.inlineQueue->Array.length
                          if qLen > 0 {
                            (lexer.inlineQueue->Array.getUnsafe(qLen - 1)).src = last.text
                          }
                          true
                        | _ => false
                        }
                      } else {
                        false
                      }
                      if !merged && !(lexer.links->Map.has(df.tag)) {
                        lexer.links->Map.set(df.tag, {href: df.href, title: df.title})
                        tokens->Array.push(tok)
                      }
                    | _ =>
                      // table (gfm)
                      switch Tokenizer.table(lexer, cur) {
                      | Some(Table(tb) as tok) =>
                        src := cur->String.substring(~start=tb.raw->String.length)
                        tokens->Array.push(tok)
                      | _ =>
                        // lheading
                        switch Tokenizer.lheading(lexer, cur) {
                        | Some(Heading(hd) as tok) =>
                          src := cur->String.substring(~start=hd.raw->String.length)
                          tokens->Array.push(tok)
                        | _ =>
                          // top-level paragraph
                          let handledPara = if lexer.top {
                            switch Tokenizer.paragraph(lexer, cur) {
                            | Some(Paragraph(pa) as tok) =>
                              let tLen = tokens->Array.length
                              let merged = if lastParagraphClipped.contents && tLen > 0 {
                                switch tokens->Array.getUnsafe(tLen - 1) {
                                | Paragraph(last) =>
                                  last.raw =
                                    last.raw ++
                                    (last.raw->String.endsWith("\n") ? "" : "\n") ++
                                    pa.raw
                                  last.text = last.text ++ "\n" ++ pa.text
                                  ignore(lexer.inlineQueue->Array.pop)
                                  let qLen = lexer.inlineQueue->Array.length
                                  if qLen > 0 {
                                    (lexer.inlineQueue->Array.getUnsafe(qLen - 1)).src = last.text
                                  }
                                  true
                                | _ => false
                                }
                              } else {
                                false
                              }
                              if !merged {
                                tokens->Array.push(tok)
                              }
                              lastParagraphClipped := false
                              src := cur->String.substring(~start=pa.raw->String.length)
                              true
                            | _ => false
                            }
                          } else {
                            false
                          }
                          if !handledPara {
                            // text
                            switch Tokenizer.text(lexer, cur) {
                            | Some(Text(tx) as tok) =>
                              src := cur->String.substring(~start=tx.raw->String.length)
                              let tLen = tokens->Array.length
                              let merged = if tLen > 0 {
                                switch tokens->Array.getUnsafe(tLen - 1) {
                                | Text(last) =>
                                  last.raw =
                                    last.raw ++
                                    (last.raw->String.endsWith("\n") ? "" : "\n") ++
                                    tx.raw
                                  last.text = last.text ++ "\n" ++ tx.text
                                  ignore(lexer.inlineQueue->Array.pop)
                                  let qLen = lexer.inlineQueue->Array.length
                                  if qLen > 0 {
                                    (lexer.inlineQueue->Array.getUnsafe(qLen - 1)).src = last.text
                                  }
                                  true
                                | _ => false
                                }
                              } else {
                                false
                              }
                              if !merged {
                                tokens->Array.push(tok)
                              }
                            | _ => infiniteLoopError(cur->String.charCodeAtUnsafe(0))
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }

  lexer.top = true
  tokens
}

let rec linkInText = (lexer: lexer, text: string): bool =>
  if !(text->String.includes("[")) {
    false
  } else {
    let linkRule = lexer.rules.inline.link
    let blockSkip = lexer.rules.inline.blockSkip
    let savedBlockSkip = blockSkip->RegExp.lastIndex
    blockSkip->RegExp.setLastIndex(0)
    let found = ref(false)
    let scanningSkip = ref(true)
    while scanningSkip.contents {
      switch blockSkip->Rules.execNull(text) {
      | Null => scanningSkip := false
      | Value(m) =>
        let m0 = m->RegExp.Result.fullMatch
        let idx = m->RegExp.Result.index
        if linkRule->RegExp.test(m0) && (idx == 0 || text->String.charCodeAtUnsafe(idx - 1) != 33) {
          found := true
          scanningSkip := false
        }
      }
    }
    blockSkip->RegExp.setLastIndex(savedBlockSkip)

    if found.contents {
      true
    } else {
      let reflinkSearch = lexer.rules.inline.reflinkSearch
      let savedRef = reflinkSearch->RegExp.lastIndex
      reflinkSearch->RegExp.setLastIndex(0)
      let scanningRef = ref(true)
      while scanningRef.contents {
        switch reflinkSearch->Rules.execNull(text) {
        | Null => scanningRef := false
        | Value(m) =>
          let match0 = m->RegExp.Result.fullMatch
          let refStart = match0->String.lastIndexOf("[")
          if (
            match0->String.charCodeAtUnsafe(0) != 33 &&
            lexer.links->Map.has(
              Helpers.normalizeLabel(match0->String.slice(~start=refStart + 1, ~end=-1)),
            )
          ) {
            let nestedLink = if refStart > 1 {
              let inner = match0->String.slice(~start=1, ~end=refStart - 1)
              // linkInText saves/restores reflinkSearch.lastIndex on recursion.
              linkInText(lexer, inner)
            } else {
              false
            }
            if !nestedLink {
              found := true
              scanningRef := false
            }
          }
        }
      }
      reflinkSearch->RegExp.setLastIndex(savedRef)
      found.contents
    }
  }

let rec maskReflink = (lexer: lexer, reflinkSearch: RegExp.t, match0: string): string => {
  let refStart = match0->String.lastIndexOf("[")
  if (
    !(
      lexer.links->Map.has(
        Helpers.normalizeLabel(match0->String.slice(~start=refStart + 1, ~end=-1)),
      )
    )
  ) {
    match0
  } else if refStart > 1 && match0->String.charCodeAtUnsafe(0) != 33 {
    let text = match0->String.slice(~start=1, ~end=refStart - 1)
    if linkInText(lexer, text) {
      "[" ++
      text->Rules.replaceBy1(reflinkSearch, m => maskReflink(lexer, reflinkSearch, m)) ++
      "][" ++
      "a"->String.repeat(match0->String.length - refStart - 2) ++ "]"
    } else {
      "[" ++ "a"->String.repeat(match0->String.length - 2) ++ "]"
    }
  } else {
    "[" ++ "a"->String.repeat(match0->String.length - 2) ++ "]"
  }
}

let maskAnyPunctuationOutsideCode = (
  match0: string,
  code: option<string>,
  punct: option<string>,
): string =>
  switch punct {
  | Some(p) if p != "" => "+"->String.repeat(match0->String.length)
  | _ =>
    switch code {
    | Some(c) if c != "" =>
      let cLen = c->String.length
      c ++
      match0
      ->String.slice(~start=cLen, ~end=-cLen)
      ->String.replaceRegExp(Rules.backtickGlobal, "+") ++ c
    | _ => match0
    }
  }

let maskBlockSkip = (match0: string, _link: option<string>, context: option<string>): string => {
  let offset = switch context {
  | Some(c) => c->String.length
  | None => 0
  }
  match0->String.slice(~start=0, ~end=offset) ++
  "[" ++
  "a"->String.repeat(match0->String.length - offset - 2) ++ "]"
}

let rec inlineTokensInner = (
  lexer: lexer,
  srcInput: string,
  tokens: array<inlineToken>,
): array<inlineToken> => {
  let masked1 = if lexer.links->Map.size > 0 && srcInput->String.includes("[") {
    let reflinkSearch = lexer.rules.inline.reflinkSearch
    srcInput->Rules.replaceBy1(reflinkSearch, m => maskReflink(lexer, reflinkSearch, m))
  } else {
    srcInput
  }

  let masked2 =
    masked1->Rules.replaceBy2Opt(
      lexer.rules.inline.anyPunctuationOutsideCode,
      maskAnyPunctuationOutsideCode,
    )

  let maskedSrc = masked2->Rules.replaceBy2Opt(lexer.rules.inline.blockSkip, maskBlockSkip)

  let src = ref(srcInput)
  let keepPrevChar = ref(false)
  let prevChar = ref("")
  let prevLen = ref(srcInput->String.length + 1)

  while src.contents != "" {
    let cur = src.contents
    let curLen = cur->String.length
    if curLen < prevLen.contents {
      prevLen := curLen
    } else {
      infiniteLoopError(cur->String.charCodeAtUnsafe(0))
    }

    if !keepPrevChar.contents {
      prevChar := ""
    }
    keepPrevChar := false

    // escape
    switch Tokenizer.escape(lexer, cur) {
    | Some(Escape(es) as tok) =>
      src := cur->String.substring(~start=es.raw->String.length)
      tokens->Array.push(tok)
    | _ =>
      // tag
      switch Tokenizer.tag(lexer, cur) {
      | Some(Html(ht) as tok) =>
        src := cur->String.substring(~start=ht.raw->String.length)
        tokens->Array.push(tok)
      | _ =>
        // link
        switch Tokenizer.link(lexer, cur, ~inlineTokens) {
        | Some((Link({raw}) | Image({raw})) as tok) =>
          src := cur->String.substring(~start=raw->String.length)
          tokens->Array.push(tok)
        | _ =>
          // reflink, nolink
          switch Tokenizer.reflink(lexer, cur, ~inlineTokens) {
          | Some(Text(tx) as tok) =>
            src := cur->String.substring(~start=tx.raw->String.length)
            let tLen = tokens->Array.length
            let merged = if tLen > 0 {
              switch tokens->Array.getUnsafe(tLen - 1) {
              | Text(last) =>
                last.raw = last.raw ++ tx.raw
                last.text = last.text ++ tx.text
                true
              | _ => false
              }
            } else {
              false
            }
            if !merged {
              tokens->Array.push(tok)
            }
          | Some((Link({raw}) | Image({raw})) as tok) =>
            src := cur->String.substring(~start=raw->String.length)
            tokens->Array.push(tok)
          | _ =>
            // em & strong
            switch Tokenizer.emStrong(lexer, cur, maskedSrc, prevChar.contents, ~inlineTokens) {
            | Some((Em({raw}) | Strong({raw})) as tok) =>
              src := cur->String.substring(~start=raw->String.length)
              tokens->Array.push(tok)
            | _ =>
              // codespan
              switch Tokenizer.codespan(lexer, cur) {
              | Some(Codespan(cs) as tok) =>
                src := cur->String.substring(~start=cs.raw->String.length)
                tokens->Array.push(tok)
              | _ =>
                // br
                switch Tokenizer.br(lexer, cur) {
                | Some(Br(b) as tok) =>
                  src := cur->String.substring(~start=b.raw->String.length)
                  tokens->Array.push(tok)
                | _ =>
                  // del (gfm)
                  switch Tokenizer.del(lexer, cur, maskedSrc, prevChar.contents, ~inlineTokens) {
                  | Some(Del(dl) as tok) =>
                    src := cur->String.substring(~start=dl.raw->String.length)
                    tokens->Array.push(tok)
                  | _ =>
                    // autolink
                    switch Tokenizer.autolink(lexer, cur) {
                    | Some(Link(lk) as tok) =>
                      src := cur->String.substring(~start=lk.raw->String.length)
                      tokens->Array.push(tok)
                    | _ =>
                      // url (gfm)
                      let handledUrl = if !lexer.inLink {
                        switch Tokenizer.url(lexer, cur) {
                        | Some(Link(lk) as tok) =>
                          src := cur->String.substring(~start=lk.raw->String.length)
                          tokens->Array.push(tok)
                          true
                        | _ => false
                        }
                      } else {
                        false
                      }
                      if !handledUrl {
                        // inlineText
                        switch Tokenizer.inlineText(lexer, cur) {
                        | Some(Text(tx) as tok) =>
                          let rawLen = tx.raw->String.length
                          src := cur->String.substring(~start=rawLen)
                          if rawLen > 0 && tx.raw->String.charCodeAtUnsafe(rawLen - 1) != 95 {
                            prevChar := tx.raw->String.slice(~start=-1)
                          }
                          keepPrevChar := true
                          let tLen = tokens->Array.length
                          let merged = if tLen > 0 {
                            switch tokens->Array.getUnsafe(tLen - 1) {
                            | Text(last) =>
                              last.raw = last.raw ++ tx.raw
                              last.text = last.text ++ tx.text
                              true
                            | _ => false
                            }
                          } else {
                            false
                          }
                          if !merged {
                            tokens->Array.push(tok)
                          }
                        | _ => infiniteLoopError(cur->String.charCodeAtUnsafe(0))
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }

  tokens
}

and inlineTokens = (
  lexer: lexer,
  src: string,
  tokens: array<inlineToken>,
): array<inlineToken> => {
  let prevLinkParenPossible = lexer.linkParenPossible
  lexer.linkParenPossible = prevLinkParenPossible && src->String.includes(")")
  try {
    let res = inlineTokensInner(lexer, src, tokens)
    lexer.linkParenPossible = prevLinkParenPossible
    res
  } catch {
  | err =>
    lexer.linkParenPossible = prevLinkParenPossible
    throw(err)
  }
}

let lex = (src: string, options: options): array<blockToken> => {
  let lexer = make(options)
  let normalized = src->String.replaceRegExp(Rules.carriageReturn, "\n")
  let tokens = blockTokens(lexer, normalized, [], false)
  let i = ref(0)
  while i.contents < lexer.inlineQueue->Array.length {
    let next = lexer.inlineQueue->Array.getUnsafe(i.contents)
    ignore(inlineTokens(lexer, next.src, next.tokens))
    i := i.contents + 1
  }
  tokens
}

let lexInline = (src: string, options: options): array<inlineToken> => {
  let lexer = make(options)
  inlineTokens(lexer, src, [])
}
