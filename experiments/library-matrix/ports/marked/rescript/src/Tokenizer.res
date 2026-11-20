// Tokenizer for marked@18.1.1 (ported faithfully from `src/Tokenizer.ts`).

open Tokens

let outputLink = (
  cap0: string,
  cap1: string,
  link: linkDef,
  raw: string,
  lexer: lexer,
  ~inlineTokens: (lexer, string, array<inlineToken>) => array<inlineToken>,
): option<inlineToken> => {
  let href = link.href
  let title = link.title
  let text = cap1->String.replaceRegExp(Rules.outputLinkReplace, "$1")
  let isImage = cap0->String.charCodeAtUnsafe(0) == 33

  let outerInLink = lexer.inLink
  let outerLinkEmitted = lexer.linkEmitted
  let outerInRawBlock = lexer.inRawBlock
  lexer.inLink = true
  lexer.linkEmitted = false
  let tokens = inlineTokens(lexer, text, [])
  let textHasLink = lexer.linkEmitted
  lexer.linkEmitted = outerLinkEmitted
  lexer.inLink = outerInLink

  if !isImage {
    if textHasLink {
      lexer.inRawBlock = outerInRawBlock
      None
    } else {
      lexer.linkEmitted = true
      Some(Link({raw, href, title, text, tokens, autolink: false}))
    }
  } else {
    Some(Image({raw, href, title, text, tokens}))
  }
}

let indentCodeCompensation = (raw: string, text: string): string =>
  switch Rules.indentCodeCompensation->Rules.execNull(raw) {
  | Null => text
  | Value(matchIndentToCode) =>
    let indentLen = matchIndentToCode->Rules.groupUnsafe(1)->String.length
    let lines = text->String.split("\n")
    let len = lines->Array.length
    for i in 0 to len - 1 {
      let node = lines->Array.getUnsafe(i)
      switch Rules.beginningSpace->Rules.execNull(node) {
      | Null => ()
      | Value(matchIndentInNode) =>
        let inLen = matchIndentInNode->RegExp.Result.fullMatch->String.length
        let cut = inLen < indentLen ? inLen : indentLen
        lines->Array.setUnsafe(i, node->String.slice(~start=cut))
      }
    }
    lines->Array.join("\n")
  }

let isLabelEndInsideToken = (
  src: string,
  label: string,
  labelStart: int,
  inlineRules: Rules.inlineRules,
): bool =>
  if !(label->String.includes("<")) {
    false
  } else {
    let len = label->String.length
    let i = ref(0)
    let inside = ref(false)
    while i.contents < len && !inside.contents {
      let idx = i.contents
      let ch = label->String.charCodeAtUnsafe(idx)
      if ch == 92 {
        i := idx + 2
      } else if ch == 96 {
        switch inlineRules.code->Rules.execNull(label->String.slice(~start=idx)) {
        | Value(code) => i := idx + code->RegExp.Result.fullMatch->String.length
        | Null => i := idx + 1
        }
      } else if ch != 60 {
        i := idx + 1
      } else {
        let tokenSrc = src->String.slice(~start=labelStart + idx)
        let tok = switch inlineRules.tag->Rules.execNull(tokenSrc) {
        | Value(_) as v => v
        | Null => inlineRules.autolink->Rules.execNull(tokenSrc)
        }
        switch tok {
        | Null => i := idx + 1
        | Value(m) =>
          let mLen = m->RegExp.Result.fullMatch->String.length
          if mLen > len - idx {
            inside := true
          } else {
            i := idx + mLen
          }
        }
      }
    }
    inside.contents
  }

let space = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.newline->Rules.execNull(src) {
  | Value(cap) =>
    let raw = cap->RegExp.Result.fullMatch
    raw->String.length > 0 ? Some(Space({raw: raw})) : None
  | Null => None
  }

let code = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.code->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let m0 = cap->RegExp.Result.fullMatch
    let raw = lexer.options.pedantic ? m0 : Helpers.trimTrailingBlankLines(m0)
    let text = raw->String.replaceRegExp(Rules.codeRemoveIndent, "")
    Some(Code({raw, lang: "", text}))
  }

let fences = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.fences {
  | None => None
  | Some(re) =>
    switch re->Rules.execNull(src) {
    | Null => None
    | Value(cap) =>
      let raw = cap->RegExp.Result.fullMatch
      let body = switch cap->Rules.groupOpt(3) {
      | Some(s) => s
      | None => ""
      }
      let text = indentCodeCompensation(raw, body)
      let lang = switch cap->Rules.groupOpt(2) {
      | Some(s) if s != "" =>
        s->String.trim->String.replaceRegExp(lexer.rules.inline.anyPunctuation, "$1")
      | _ => ""
      }
      Some(Code({raw, lang, text}))
    }
  }

let heading = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.heading->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let text = ref(cap->Rules.groupUnsafe(2)->String.trim)
    if Rules.endingHash->RegExp.test(text.contents) {
      let trimmed = Helpers.rtrim(text.contents, "#")
      if lexer.options.pedantic {
        text := trimmed->String.trim
      } else if trimmed == "" || Rules.endingSpaceTabChar->RegExp.test(trimmed) {
        text := trimmed->String.trim
      }
    }
    let finalText = text.contents
    Some(
      Heading({
        raw: Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n"),
        depth: cap->Rules.groupUnsafe(1)->String.length,
        text: finalText,
        tokens: queueInline(lexer, finalText),
      }),
    )
  }

let hr = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.hr->Rules.execNull(src) {
  | Null => None
  | Value(cap) => Some(Hr({raw: Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n")}))
  }

let rec blockquote = (
  lexer: lexer,
  src: string,
  ~blockTokens: (lexer, string, array<blockToken>, bool) => array<blockToken>,
): option<blockToken> =>
  switch lexer.rules.block.blockquote->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let lines = ref(Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n")->String.split("\n"))
    let raw = ref("")
    let text = ref("")
    let tokens: array<blockToken> = []
    let done = ref(false)

    while lines.contents->Array.length > 0 && !done.contents {
      let curLinesArr = lines.contents
      let curLen = curLinesArr->Array.length
      let inBlockquote = ref(false)
      let currentLines: array<string> = []
      let i = ref(0)
      let scanning = ref(true)
      while i.contents < curLen && scanning.contents {
        let line = curLinesArr->Array.getUnsafe(i.contents)
        if Rules.blockquoteStart->RegExp.test(line) {
          currentLines->Array.push(line)
          inBlockquote := true
          i := i.contents + 1
        } else if !inBlockquote.contents {
          currentLines->Array.push(line)
          i := i.contents + 1
        } else {
          scanning := false
        }
      }
      lines := curLinesArr->Array.slice(~start=i.contents)

      let currentRaw = currentLines->Array.join("\n")
      let currentText =
        currentRaw
        ->String.replaceRegExp(Rules.blockquoteSetextReplace, "\n    $1")
        ->String.replaceRegExp(Rules.blockquoteSetextReplace2, "")
      raw := (raw.contents != "" ? raw.contents ++ "\n" ++ currentRaw : currentRaw)
      text := (text.contents != "" ? text.contents ++ "\n" ++ currentText : currentText)

      let prevTop = lexer.top
      lexer.top = true
      ignore(blockTokens(lexer, currentText, tokens, true))
      lexer.top = prevTop

      if lines.contents->Array.length == 0 {
        done := true
      } else {
        let tokLen = tokens->Array.length
        if tokLen > 0 {
          switch tokens->Array.getUnsafe(tokLen - 1) {
          | Code(_) | Heading(_) | Hr(_) | Html(_) => done := true
          | Blockquote(oldBq) =>
            let continuation = lines.contents->Array.join("\n")
            let newText =
              oldBq.raw ++
              "\n" ++
              continuation->String.replaceRegExp(Rules.blockquoteSetextReplace2, "")
            switch blockquote(lexer, newText, ~blockTokens) {
            | Some(Blockquote(newBq) as newTok) =>
              tokens->Array.setUnsafe(tokLen - 1, newTok)
              let rawLeftover = newText->String.substring(~start=newBq.raw->String.length)
              let leftover =
                rawLeftover->String.length > 0 && rawLeftover->String.charCodeAtUnsafe(0) == 10
                  ? rawLeftover->String.slice(~start=1)
                  : rawLeftover
              let leftoverCount = leftover != "" ? leftover->String.split("\n")->Array.length : 0
              let consumed =
                leftoverCount > 0
                  ? lines.contents->Array.slice(~start=0, ~end=-leftoverCount)
                  : lines.contents
              if consumed->Array.length > 0 {
                raw := raw.contents ++ "\n" ++ consumed->Array.join("\n")
              }
              text :=
                text.contents->String.substring(
                  ~start=0,
                  ~end=text.contents->String.length - oldBq.text->String.length,
                ) ++ newBq.text
            | _ => ()
            }
            done := true
          | List(oldList) =>
            let newText = oldList.raw ++ "\n" ++ lines.contents->Array.join("\n")
            switch list(lexer, newText, ~blockTokens) {
            | Some(List(newList) as newTok) =>
              tokens->Array.setUnsafe(tokLen - 1, newTok)
              let oldRawLen = oldList.raw->String.length
              raw :=
                raw.contents->String.substring(~start=0, ~end=raw.contents->String.length - oldRawLen) ++
                newList.raw
              text :=
                text.contents->String.substring(
                  ~start=0,
                  ~end=text.contents->String.length - oldRawLen,
                ) ++ newList.raw
              lines := newText->String.substring(~start=newList.raw->String.length)->String.split("\n")
            | _ => done := true
            }
          | _ => ()
          }
        }
      }
    }

    Some(Blockquote({raw: raw.contents, tokens, text: text.contents}))
  }

and list = (
  lexer: lexer,
  src: string,
  ~blockTokens: (lexer, string, array<blockToken>, bool) => array<blockToken>,
): option<blockToken> =>
  switch lexer.rules.block.list->Rules.execNull(src) {
  | Null => None
  | Value(initialCap) =>
    let listSrc = src
    let listSrcLen = listSrc->String.length
    let srcRef = ref(src)
    let bull0 = initialCap->Rules.groupUnsafe(1)->String.trim
    let isOrdered = bull0->String.length > 1
    let start = isOrdered
      ? Helpers.parseIntRadix(bull0->String.slice(~start=0, ~end=-1), 10)
      : 0

    let bullPattern = if isOrdered {
      "\\d{1,9}\\" ++ bull0->String.slice(~start=-1)
    } else if lexer.options.pedantic {
      "[*+-]"
    } else {
      "\\" ++ bull0
    }

    let itemRegex = Rules.listItemRegex(bullPattern)
    let items: array<listItem> = []
    let loose = ref(false)
    let endsWithBlankLine = ref(false)
    let stopItems = ref(false)

    while srcRef.contents != "" && !stopItems.contents {
      let curSrc = srcRef.contents
      switch itemRegex->Rules.execNull(curSrc) {
      | Null => stopItems := true
      | Value(cap) =>
        if lexer.rules.block.hr->RegExp.test(curSrc) {
          stopItems := true
        } else {
          let itemStart = listSrcLen - curSrc->String.length
          let cap0 = cap->RegExp.Result.fullMatch
          let cap1 = cap->Rules.groupUnsafe(1)
          let cap2 = cap->Rules.groupUnsafe(2)
          srcRef := curSrc->String.substring(~start=cap0->String.length)

          let firstLine = Helpers.firstLineOf(cap2)
          let bulletIndent = cap1->String.length
          let line = ref(
            lexer.options.pedantic
              ? Helpers.expandTabs(firstLine, ~indent=bulletIndent)
              : firstLine->Rules.replaceBy1(Rules.leadingSpaceTab, ws =>
                  Helpers.expandTabs(ws, ~indent=bulletIndent)
                ),
          )
          let nextLineFirst = Helpers.firstLineOf(srcRef.contents)
          let blankLine = ref(line.contents->String.trim == "")
          let itemContents = ref("")
          let indent = if lexer.options.pedantic {
            itemContents := line.contents->String.trimStart
            2
          } else if blankLine.contents {
            bulletIndent + 1
          } else {
            let rawIndent = line.contents->String.search(Rules.nonSpaceChar)
            let i0 = rawIndent > 4 ? 1 : rawIndent
            itemContents := line.contents->String.slice(~start=i0)
            i0 + bulletIndent
          }

          let endEarly = if blankLine.contents && Rules.blankLine->RegExp.test(nextLineFirst) {
            srcRef := srcRef.contents->String.substring(~start=nextLineFirst->String.length + 1)
            true
          } else {
            false
          }

          if !endEarly {
            let nextBulletRegex = Rules.nextBulletRegex(indent)
            let hrRegex = Rules.hrRegex(indent)
            let fencesBeginRegex = Rules.fencesBeginRegex(indent)
            let headingBeginRegex = Rules.headingBeginRegex(indent)
            let htmlBeginRegex = Rules.htmlBeginRegex(indent)
            let blockquoteBeginRegex = Rules.blockquoteBeginRegex(indent)

            let stopLines = ref(false)
            while srcRef.contents != "" && !stopLines.contents {
              let rawLine = Helpers.firstLineOf(srcRef.contents)
              let (curNextLine, nextLineWithoutTabs) = if lexer.options.pedantic {
                let aligned = rawLine->String.replaceRegExp(Rules.listReplaceNesting, "  ")
                (aligned, aligned)
              } else {
                (
                  rawLine,
                  rawLine->Rules.replaceBy1(Rules.leadingSpaceTab, ws => Helpers.expandTabs(ws)),
                )
              }

              if (
                fencesBeginRegex->RegExp.test(curNextLine) ||
                headingBeginRegex->RegExp.test(curNextLine) ||
                htmlBeginRegex->RegExp.test(curNextLine) ||
                blockquoteBeginRegex->RegExp.test(curNextLine) ||
                nextBulletRegex->RegExp.test(curNextLine) ||
                hrRegex->RegExp.test(curNextLine)
              ) {
                stopLines := true
              } else {
                let curBlank = curNextLine->String.trim == ""
                if nextLineWithoutTabs->String.search(Rules.nonSpaceChar) >= indent || curBlank {
                  itemContents :=
                    itemContents.contents ++ "\n" ++ nextLineWithoutTabs->String.slice(~start=indent)
                  blankLine := curBlank
                  srcRef := srcRef.contents->String.substring(~start=rawLine->String.length + 1)
                  line := nextLineWithoutTabs->String.slice(~start=indent)
                } else if (
                  blankLine.contents ||
                  line.contents
                  ->String.replaceRegExp(Rules.tabCharGlobal, "    ")
                  ->String.search(Rules.nonSpaceChar) >= 4 ||
                  fencesBeginRegex->RegExp.test(line.contents) ||
                  headingBeginRegex->RegExp.test(line.contents) ||
                  hrRegex->RegExp.test(line.contents)
                ) {
                  stopLines := true
                } else {
                  itemContents := itemContents.contents ++ "\n" ++ curNextLine
                  blankLine := curBlank
                  srcRef := srcRef.contents->String.substring(~start=rawLine->String.length + 1)
                  line := nextLineWithoutTabs->String.slice(~start=indent)
                }
              }
            }
          }

          let raw = listSrc->String.slice(~start=itemStart, ~end=listSrcLen - srcRef.contents->String.length)

          if !loose.contents {
            if endsWithBlankLine.contents {
              loose := true
            } else if Rules.doubleBlankLine->RegExp.test(raw) {
              endsWithBlankLine := true
            }
          }

          items->Array.push({
            raw,
            task: lexer.options.gfm && Rules.listIsTask->RegExp.test(itemContents.contents),
            checked: false,
            loose: false,
            text: itemContents.contents,
            tokens: [],
          })
        }
      }
    }

    let itemCount = items->Array.length
    if itemCount == 0 {
      None
    } else {
      let lastItem = items->Array.getUnsafe(itemCount - 1)
      lastItem.raw = lastItem.raw->String.trimEnd
      lastItem.text = lastItem.text->String.trimEnd
      let listRaw =
        listSrc
        ->String.slice(~start=0, ~end=listSrcLen - srcRef.contents->String.length)
        ->String.trimEnd

      // First pass: tokenize items and finalize list.loose from spacers
      for i in 0 to itemCount - 1 {
        let item = items->Array.getUnsafe(i)
        lexer.top = false
        let toks = blockTokens(lexer, item.text, [], false)
        item.tokens = toks
        if !loose.contents {
          let tLen = toks->Array.length
          let j = ref(0)
          while j.contents < tLen && !loose.contents {
            switch toks->Array.getUnsafe(j.contents) {
            | Space(sp) =>
              if Rules.anyLine->RegExp.test(sp.raw) {
                loose := true
              }
            | _ => ()
            }
            j := j.contents + 1
          }
        }
      }

      // Second pass: place task checkboxes using final list.loose
      for i in 0 to itemCount - 1 {
        let item = items->Array.getUnsafe(i)
        if item.task {
          let handled = if item.tokens->Array.length > 0 {
            switch item.tokens->Array.getUnsafe(0) {
            | Text(tb) | Paragraph(tb) =>
              item.text = item.text->String.replaceRegExp(Rules.listReplaceTask, "")
              tb.raw = tb.raw->String.replaceRegExp(Rules.listReplaceTask, "")
              tb.text = tb.text->String.replaceRegExp(Rules.listReplaceTask, "")
              let qi = ref(lexer.inlineQueue->Array.length - 1)
              let foundQueue = ref(false)
              while qi.contents >= 0 && !foundQueue.contents {
                let entry = lexer.inlineQueue->Array.getUnsafe(qi.contents)
                if Rules.listIsTask->RegExp.test(entry.src) {
                  entry.src = entry.src->String.replaceRegExp(Rules.listReplaceTask, "")
                  foundQueue := true
                }
                qi := qi.contents - 1
              }
              switch Rules.listTaskCheckbox->Rules.execNull(item.raw) {
              | Value(taskRaw) =>
                let box = taskRaw->RegExp.Result.fullMatch
                let cbRaw = box ++ " "
                let isChecked = box != "[ ]"
                item.checked = isChecked
                if loose.contents {
                  tb.raw = cbRaw ++ tb.raw
                  tb.text = cbRaw ++ tb.text
                  tb.tokens->Array.unshift(Checkbox({raw: cbRaw, checked: isChecked}))
                } else {
                  item.tokens->Array.unshift(Checkbox({raw: cbRaw, checked: isChecked}))
                }
              | Null => ()
              }
              true
            | _ => false
            }
          } else {
            false
          }
          if !handled {
            item.task = false
          }
        }
      }

      // Set all items to loose if list is loose
      if loose.contents {
        for i in 0 to itemCount - 1 {
          let item = items->Array.getUnsafe(i)
          item.loose = true
          let toks = item.tokens
          let tLen = toks->Array.length
          for j in 0 to tLen - 1 {
            switch toks->Array.getUnsafe(j) {
            | Text(tb) => toks->Array.setUnsafe(j, Paragraph(tb))
            | _ => ()
            }
          }
        }
      }

      Some(List({raw: listRaw, ordered: isOrdered, start, loose: loose.contents, items}))
    }
  }

let html = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.html->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let raw = Helpers.trimTrailingBlankLines(cap->RegExp.Result.fullMatch)
    Some(Html({raw, text: raw}))
  }

let def = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.def->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let cap2 = cap->Rules.groupUnsafe(2)
    if !(Rules.startAngleBracket->RegExp.test(cap2)) && Helpers.findClosingBracket(cap2, "(", ")") != -1 {
      None
    } else {
      let tag =
        Helpers.normalizeLabel(cap->Rules.groupUnsafe(1))->String.replaceRegExp(
          Rules.multipleSpaceGlobal,
          " ",
        )
      let href =
        cap2 != ""
          ? cap2
            ->String.replaceRegExp(Rules.hrefBrackets, "$1")
            ->String.replaceRegExp(lexer.rules.inline.anyPunctuation, "$1")
          : ""
      let title = switch cap->Rules.groupOpt(3) {
      | Some(t) if t != "" =>
        t
        ->String.substring(~start=1, ~end=t->String.length - 1)
        ->String.replaceRegExp(lexer.rules.inline.anyPunctuation, "$1")
      | _ => ""
      }
      Some(Def({raw: Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n"), tag, href, title}))
    }
  }

let table = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.table {
  | None => None
  | Some(re) =>
    switch re->Rules.execNull(src) {
    | Null => None
    | Value(cap) =>
      let cap2 = cap->Rules.groupUnsafe(2)
      if !(Rules.tableDelimiter->RegExp.test(cap2)) {
        None
      } else {
        let headers = Helpers.splitCells(cap->Rules.groupUnsafe(1))
        let alignStrs = cap2->String.replaceRegExp(Rules.tableAlignChars, "")->String.split("|")
        let hLen = headers->Array.length
        if hLen != alignStrs->Array.length {
          None
        } else {
          let aligns: array<option<string>> = []
          for i in 0 to hLen - 1 {
            let a = alignStrs->Array.getUnsafe(i)
            let aligned = if Rules.tableAlignRight->RegExp.test(a) {
              Some("right")
            } else if Rules.tableAlignCenter->RegExp.test(a) {
              Some("center")
            } else if Rules.tableAlignLeft->RegExp.test(a) {
              Some("left")
            } else {
              None
            }
            aligns->Array.push(aligned)
          }

          let header: array<tableCell> = []
          for i in 0 to hLen - 1 {
            let text = headers->Array.getUnsafe(i)
            header->Array.push({
              text,
              tokens: queueInline(lexer, text),
              header: true,
              align: aligns->Array.getUnsafe(i),
            })
          }

          let rawRows = switch cap->Rules.groupOpt(3) {
          | Some(r) if r->String.trim != "" =>
            r->String.replaceRegExp(Rules.tableRowBlankLine, "")->String.split("\n")
          | _ => []
          }
          let rows: array<array<tableCell>> = []
          let rLen = rawRows->Array.length
          for r in 0 to rLen - 1 {
            let cells = Helpers.splitCells(rawRows->Array.getUnsafe(r), ~count=hLen)
            let rowCells: array<tableCell> = []
            for c in 0 to cells->Array.length - 1 {
              let cellText = cells->Array.getUnsafe(c)
              rowCells->Array.push({
                text: cellText,
                tokens: queueInline(lexer, cellText),
                header: false,
                align: aligns->Array.getUnsafe(c),
              })
            }
            rows->Array.push(rowCells)
          }

          Some(Table({raw: Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n"), header, rows}))
        }
      }
    }
  }

let lheading = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.lheading->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let text = cap->Rules.groupUnsafe(1)->String.trim
    let depth = cap->Rules.groupUnsafe(2)->String.charCodeAtUnsafe(0) == 61 ? 1 : 2
    Some(
      Heading({
        raw: Helpers.rtrim(cap->RegExp.Result.fullMatch, "\n"),
        depth,
        text,
        tokens: queueInline(lexer, text),
      }),
    )
  }

let paragraph = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.paragraph->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let cap1 = cap->Rules.groupUnsafe(1)
    let c1Len = cap1->String.length
    let text =
      c1Len > 0 && cap1->String.charCodeAtUnsafe(c1Len - 1) == 10
        ? cap1->String.slice(~start=0, ~end=-1)
        : cap1
    Some(Paragraph({raw: cap->RegExp.Result.fullMatch, text, tokens: queueInline(lexer, text)}))
  }

let text = (lexer: lexer, src: string): option<blockToken> =>
  switch lexer.rules.block.text->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let m0 = cap->RegExp.Result.fullMatch
    Some(Text({raw: m0, text: m0, tokens: queueInline(lexer, m0)}))
  }

let escape = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.escape->Rules.execNull(src) {
  | Null => None
  | Value(cap) => Some(Escape({raw: cap->RegExp.Result.fullMatch, text: cap->Rules.groupUnsafe(1)}))
  }

let tag = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.tag->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let m0 = cap->RegExp.Result.fullMatch
    if !lexer.inLink && Rules.startATag->RegExp.test(m0) {
      lexer.inLink = true
    } else if lexer.inLink && Rules.endATag->RegExp.test(m0) {
      lexer.inLink = false
    }
    if !lexer.inRawBlock && Rules.startPreScriptTag->RegExp.test(m0) {
      lexer.inRawBlock = true
    } else if lexer.inRawBlock && Rules.endPreScriptTag->RegExp.test(m0) {
      lexer.inRawBlock = false
    }
    Some(Html({raw: m0, inLink: lexer.inLink, inRawBlock: lexer.inRawBlock, text: m0}))
  }

let link = (
  lexer: lexer,
  src: string,
  ~inlineTokens: (lexer, string, array<inlineToken>) => array<inlineToken>,
): option<inlineToken> =>
  if !lexer.linkParenPossible {
    None
  } else {
    switch lexer.rules.inline.link->Rules.execNull(src) {
    | Null => None
    | Value(cap) =>
      let cap0 = ref(cap->RegExp.Result.fullMatch)
      let cap1 = cap->Rules.groupUnsafe(1)
      let cap2 = ref(cap->Rules.groupUnsafe(2))
      let cap3 = ref(
        switch cap->Rules.groupOpt(3) {
        | Some(s) => s
        | None => ""
        },
      )
      let labelStart = cap0.contents->String.charCodeAtUnsafe(0) == 33 ? 2 : 1
      if !lexer.options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, lexer.rules.inline) {
        None
      } else {
        let trimmedUrl = cap2.contents->String.trim
        let valid = if !lexer.options.pedantic && Rules.startAngleBracket->RegExp.test(trimmedUrl) {
          if !(Rules.endAngleBracket->RegExp.test(trimmedUrl)) {
            false
          } else {
            let rtrimSlash = Helpers.rtrim(trimmedUrl->String.slice(~start=0, ~end=-1), "\\")
            mod(trimmedUrl->String.length - rtrimSlash->String.length, 2) != 0
          }
        } else {
          let lastParenIndex = Helpers.findClosingBracket(cap2.contents, "(", ")")
          if lastParenIndex == -2 {
            false
          } else {
            if lastParenIndex > -1 {
              let start = cap0.contents->String.charCodeAtUnsafe(0) == 33 ? 5 : 4
              let linkLen = start + cap1->String.length + lastParenIndex
              cap2 := cap2.contents->String.substring(~start=0, ~end=lastParenIndex)
              cap0 := cap0.contents->String.substring(~start=0, ~end=linkLen)->String.trim
              cap3 := ""
            }
            true
          }
        }

        if !valid {
          None
        } else {
          let (rawHref, rawTitle) = if lexer.options.pedantic {
            switch Rules.pedanticHrefTitle->Rules.execNull(cap2.contents) {
            | Value(m) => (m->Rules.groupUnsafe(1), m->Rules.groupUnsafe(3))
            | Null => (cap2.contents, "")
            }
          } else {
            let t = cap3.contents != "" ? cap3.contents->String.slice(~start=1, ~end=-1) : ""
            (cap2.contents, t)
          }

          let hrefTrimmed = rawHref->String.trim
          let hrefUnbracketed = if Rules.startAngleBracket->RegExp.test(hrefTrimmed) {
            if lexer.options.pedantic && !(Rules.endAngleBracket->RegExp.test(trimmedUrl)) {
              hrefTrimmed->String.slice(~start=1)
            } else {
              hrefTrimmed->String.slice(~start=1, ~end=-1)
            }
          } else {
            hrefTrimmed
          }

          let anyPunct = lexer.rules.inline.anyPunctuation
          let finalHref =
            hrefUnbracketed != "" ? hrefUnbracketed->String.replaceRegExp(anyPunct, "$1") : ""
          let finalTitle =
            rawTitle != "" ? rawTitle->String.replaceRegExp(anyPunct, "$1") : ""

          outputLink(
            cap0.contents,
            cap1,
            {href: finalHref, title: finalTitle},
            cap0.contents,
            lexer,
            ~inlineTokens,
          )
        }
      }
    }
  }

let reflink = (
  lexer: lexer,
  src: string,
  ~inlineTokens: (lexer, string, array<inlineToken>) => array<inlineToken>,
): option<inlineToken> => {
  let cap = switch lexer.rules.inline.reflink->Rules.execNull(src) {
  | Value(_) as v => v
  | Null => lexer.rules.inline.nolink->Rules.execNull(src)
  }
  switch cap {
  | Null => None
  | Value(m) =>
    let cap0 = m->RegExp.Result.fullMatch
    let cap1 = m->Rules.groupUnsafe(1)
    let labelStart = cap0->String.charCodeAtUnsafe(0) == 33 ? 2 : 1
    if !lexer.options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, lexer.rules.inline) {
      None
    } else {
      let refRaw = switch m->Rules.groupOpt(2) {
      | Some(s) if s != "" => s
      | _ => cap1
      }
      let linkString = refRaw->String.replaceRegExp(Rules.multipleSpaceGlobal, " ")
      switch lexer.links->Map.get(Helpers.normalizeLabel(linkString)) {
      | None =>
        let ch = cap0->String.charAt(0)
        Some(Text({raw: ch, text: ch, escaped: false}))
      | Some(def) => outputLink(cap0, cap1, def, cap0, lexer, ~inlineTokens)
      }
    }
  }
}

let emStrong = (
  lexer: lexer,
  src: string,
  maskedSrc: string,
  prevChar: string,
  ~inlineTokens: (lexer, string, array<inlineToken>) => array<inlineToken>,
): option<inlineToken> =>
  switch lexer.rules.inline.emStrongLDelim->Rules.execNull(src) {
  | Null => None
  | Value(m) =>
    let g1 = m->Rules.groupOpt(1)
    let g2 = m->Rules.groupOpt(2)
    let g3 = m->Rules.groupOpt(3)
    let g4 = m->Rules.groupOpt(4)
    if g1 === None && g2 === None && g3 === None && g4 === None {
      None
    } else if g4 !== None && Rules.unicodeAlphaNumeric->RegExp.test(prevChar) {
      None
    } else {
      let nextChar = switch g1 {
      | Some(s) if s != "" => s
      | _ =>
        switch g3 {
        | Some(s) if s != "" => s
        | _ => ""
        }
      }
      if nextChar == "" || prevChar == "" || lexer.rules.inline.punctuation->RegExp.test(prevChar) {
        let m0 = m->RegExp.Result.fullMatch
        let lLength = Helpers.codePointLength(m0) - 1
        let delimCode = m0->String.charCodeAtUnsafe(0)
        let delimChar = delimCode == 42 ? "*" : "_"
        let midRun = prevChar == delimChar
        let endReg =
          delimCode == 42
            ? lexer.rules.inline.emStrongRDelimAst
            : lexer.rules.inline.emStrongRDelimUnd
        endReg->RegExp.setLastIndex(0)

        let clippedMasked = maskedSrc->String.slice(~start=-(src->String.length) + lLength)
        let delimTotal = ref(lLength)
        let midDelimTotal = ref(0)
        let result: ref<option<inlineToken>> = ref(None)
        let running = ref(true)

        while running.contents {
          switch endReg->Rules.execNull(clippedMasked) {
          | Null => running := false
          | Value(rm) =>
            let r1 = rm->Rules.groupOpt(1)
            let r2 = rm->Rules.groupOpt(2)
            let r3 = rm->Rules.groupOpt(3)
            let r4 = rm->Rules.groupOpt(4)
            let r5 = rm->Rules.groupOpt(5)
            let r6 = rm->Rules.groupOpt(6)
            let rDelim = switch r1 {
            | Some(_) => r1
            | None =>
              switch r2 {
              | Some(_) => r2
              | None =>
                switch r3 {
                | Some(_) => r3
                | None =>
                  switch r4 {
                  | Some(_) => r4
                  | None =>
                    switch r5 {
                    | Some(_) => r5
                    | None => r6
                    }
                  }
                }
              }
            }
            switch rDelim {
            | None => ()
            | Some(rd) =>
              // Capture groups 1..6 match only ASCII `*` or `_` runs.
              let rLen0 = rd->String.length
              if r3 !== None || r4 !== None {
                delimTotal := delimTotal.contents + rLen0
              } else {
                let skip = if r5 !== None || r6 !== None {
                  if mod(lLength, 3) != 0 && mod(lLength + rLen0, 3) == 0 {
                    midDelimTotal := midDelimTotal.contents + rLen0
                    true
                  } else if midRun {
                    running := false
                    true
                  } else {
                    false
                  }
                } else {
                  false
                }

                if !skip {
                  delimTotal := delimTotal.contents - rLen0
                  if delimTotal.contents <= 0 {
                    let sumLen = rLen0 + delimTotal.contents + midDelimTotal.contents
                    let rLength = rLen0 < sumLen ? rLen0 : sumLen
                    let lastCharLength = Helpers.firstCodePointUtf16Len(rm->RegExp.Result.fullMatch)
                    let raw =
                      src->String.slice(
                        ~start=0,
                        ~end=lLength + rm->RegExp.Result.index + lastCharLength + rLength,
                      )
                    endReg->RegExp.setLastIndex(0)
                    let minLen = lLength < rLength ? lLength : rLength
                    if mod(minLen, 2) != 0 {
                      let text = raw->String.slice(~start=1, ~end=-1)
                      result :=
                        Some(Em({raw, text, tokens: inlineTokens(lexer, text, [])}))
                    } else {
                      let text = raw->String.slice(~start=2, ~end=-2)
                      result :=
                        Some(Strong({raw, text, tokens: inlineTokens(lexer, text, [])}))
                    }
                    running := false
                  }
                }
              }
            }
          }
        }
        endReg->RegExp.setLastIndex(0)
        result.contents
      } else {
        None
      }
    }
  }

let codespan = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.code->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let rawText = cap->Rules.groupUnsafe(2)->String.replaceRegExp(Rules.newLineCharGlobal, " ")
    let hasNonSpace = Rules.nonSpaceChar->RegExp.test(rawText)
    let hasBothSpaces =
      Rules.startingSpaceChar->RegExp.test(rawText) && Rules.endingSpaceChar->RegExp.test(rawText)
    let text =
      hasNonSpace && hasBothSpaces
        ? rawText->String.substring(~start=1, ~end=rawText->String.length - 1)
        : rawText
    Some(Codespan({raw: cap->RegExp.Result.fullMatch, text}))
  }

let br = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.br->Rules.execNull(src) {
  | Null => None
  | Value(cap) => Some(Br({raw: cap->RegExp.Result.fullMatch}))
  }

let del = (
  lexer: lexer,
  src: string,
  maskedSrc: string,
  prevChar: string,
  ~inlineTokens: (lexer, string, array<inlineToken>) => array<inlineToken>,
): option<inlineToken> =>
  switch (lexer.rules.inline.delLDelim, lexer.rules.inline.delRDelim) {
  | (Some(delL), Some(endReg)) =>
    switch delL->Rules.execNull(src) {
    | Null => None
    | Value(m) =>
      let nextChar = switch m->Rules.groupOpt(1) {
      | Some(s) => s
      | None => ""
      }
      if nextChar == "" || prevChar == "" || lexer.rules.inline.punctuation->RegExp.test(prevChar) {
        let lLength = Helpers.codePointLength(m->RegExp.Result.fullMatch) - 1
        endReg->RegExp.setLastIndex(0)
        let clippedMasked = maskedSrc->String.slice(~start=-(src->String.length) + lLength)
        let delimTotal = ref(lLength)
        let result: ref<option<inlineToken>> = ref(None)
        let running = ref(true)

        while running.contents {
          switch endReg->Rules.execNull(clippedMasked) {
          | Null => running := false
          | Value(rm) =>
            let r1 = rm->Rules.groupOpt(1)
            let r2 = rm->Rules.groupOpt(2)
            let r3 = rm->Rules.groupOpt(3)
            let r4 = rm->Rules.groupOpt(4)
            let r5 = rm->Rules.groupOpt(5)
            let r6 = rm->Rules.groupOpt(6)
            let rDelim = switch r1 {
            | Some(_) => r1
            | None =>
              switch r2 {
              | Some(_) => r2
              | None =>
                switch r3 {
                | Some(_) => r3
                | None =>
                  switch r4 {
                  | Some(_) => r4
                  | None =>
                    switch r5 {
                    | Some(_) => r5
                    | None => r6
                    }
                  }
                }
              }
            }
            switch rDelim {
            | None => ()
            | Some(rd) =>
              // Capture groups 1..6 match only ASCII `~` or `~~`.
              let rLen0 = rd->String.length
              if rLen0 == lLength {
                if r3 !== None || r4 !== None {
                  delimTotal := delimTotal.contents + rLen0
                } else {
                  delimTotal := delimTotal.contents - rLen0
                  if delimTotal.contents <= 0 {
                    let sumLen = rLen0 + delimTotal.contents
                    let rLength = rLen0 < sumLen ? rLen0 : sumLen
                    let lastCharLength = Helpers.firstCodePointUtf16Len(rm->RegExp.Result.fullMatch)
                    let raw =
                      src->String.slice(
                        ~start=0,
                        ~end=lLength + rm->RegExp.Result.index + lastCharLength + rLength,
                      )
                    endReg->RegExp.setLastIndex(0)
                    let text = raw->String.slice(~start=lLength, ~end=-lLength)
                    result := Some(Del({raw, text, tokens: inlineTokens(lexer, text, [])}))
                    running := false
                  }
                }
              }
            }
          }
        }
        endReg->RegExp.setLastIndex(0)
        result.contents
      } else {
        None
      }
    }
  | _ => None
  }

let autolink = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.autolink->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let text = cap->Rules.groupUnsafe(1)
    let href = switch cap->Rules.groupOpt(2) {
    | Some("@") => "mailto:" ++ text
    | _ => text
    }
    Some(
      Link({
        raw: cap->RegExp.Result.fullMatch,
        text,
        href,
        title: "",
        autolink: true,
        tokens: [Text({raw: text, text, escaped: false})],
      }),
    )
  }

let url = (lexer: lexer, src: string): option<inlineToken> =>
  switch (lexer.rules.inline.url, lexer.rules.inline._backpedal) {
  | (Some(urlRe), Some(backpedalRe)) =>
    switch urlRe->Rules.execNull(src) {
    | Null => None
    | Value(cap) =>
      let cap0 = cap->RegExp.Result.fullMatch
      let (raw, text, href) = switch cap->Rules.groupOpt(2) {
      | Some("@") => (cap0, cap0, "mailto:" ++ cap0)
      | _ =>
        let cur = ref(cap0)
        let looping = ref(true)
        while looping.contents {
          let prev = cur.contents
          let next = switch backpedalRe->Rules.execNull(prev) {
          | Value(m) => m->RegExp.Result.fullMatch
          | Null => ""
          }
          cur := next
          if prev == next {
            looping := false
          }
        }
        let cleaned = cur.contents
        let h = switch cap->Rules.groupOpt(1) {
        | Some("www.") => "http://" ++ cleaned
        | _ => cleaned
        }
        (cleaned, cleaned, h)
      }
      Some(
        Link({
          raw,
          text,
          href,
          title: "",
          autolink: true,
          tokens: [Text({raw: text, text, escaped: false})],
        }),
      )
    }
  | _ => None
  }

let inlineText = (lexer: lexer, src: string): option<inlineToken> =>
  switch lexer.rules.inline.text->Rules.execNull(src) {
  | Null => None
  | Value(cap) =>
    let m0 = cap->RegExp.Result.fullMatch
    let escaped = lexer.inRawBlock
    let text = escaped ? m0 : Helpers.decodeNumericCharacterReferences(m0)
    Some(Text({raw: m0, text, escaped}))
  }
