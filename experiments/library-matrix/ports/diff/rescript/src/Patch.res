// jsdiff 9.0.0 patch/create.js, parse.js, apply.js, reverse.js and line-endings.js.

open Host

/** A hunk; the record is the JS object. Its ranges are JavaScript numbers: a parsed header can
 exceed int32 or be NaN. */
type hunk = {
  mutable oldStart: float,
  mutable oldLines: float,
  mutable newStart: float,
  mutable newLines: float,
  lines: array<string>,
}

/** structuredPatch's result; the record is the JS object, with jsdiff's key order. */
type structuredPatch = {
  oldFileName?: string,
  newFileName?: string,
  oldHeader?: string,
  newHeader?: string,
  hunks: array<hunk>,
}

let underline = "==================================================================="

// ---- structuredPatch and formatPatch ----

/** The lines of `text`, each with its newline where it has one. */
let splitLines = text => {
  let result = text->String.split("\n")->Array.map(line => line ++ "\n")
  if text->String.endsWith("\n") {
    result->Array.pop->ignore
  } else {
    let last = result->Array.length - 1
    result->Array.setUnsafe(last, result->Array.getUnsafe(last)->String.slice(~start=0, ~end=-1))
  }
  result
}

// context is a JavaScript number, compared and sliced as npm does it.
let structuredPatch = (~oldFileName, ~newFileName, oldStr, newStr, ~oldHeader, ~newHeader, ~context: float) => {
  // Step 1: hunks of lines that keep their newlines; an empty entry at the end closes the last one.
  let diff =
    Diff.lines(oldStr, newStr, Diff.defaults)->Array.map(change => (
      change.added,
      change.removed,
      splitLines(change.value),
    ))
  diff->Array.push((false, false, []))
  let hunks = []
  let oldRangeStart = ref(0)
  let newRangeStart = ref(0)
  let curRange = ref([])
  let oldLine = ref(1)
  let newLine = ref(1)
  diff->Array.forEachWithIndex(((added, removed, lines), i) =>
    if added || removed {
      // If we have previous context, start with that.
      if oldRangeStart.contents == 0 {
        oldRangeStart := oldLine.contents
        newRangeStart := newLine.contents
        switch diff[i - 1] {
        | Some((_, _, previous)) =>
          let count = Math.min(context, previous->Array.length->Int.toFloat)->Float.toInt
          curRange := (
              context > 0.
                ? previous->Array.slice(~start=previous->Array.length - count)->Array.map(line => " " ++ line)
                : []
            )
          oldRangeStart := oldRangeStart.contents - curRange.contents->Array.length
          newRangeStart := newRangeStart.contents - curRange.contents->Array.length
        | None => ()
        }
      }
      lines->Array.forEach(line => curRange.contents->Array.push((added ? "+" : "-") ++ line))
      if added {
        newLine := newLine.contents + lines->Array.length
      } else {
        oldLine := oldLine.contents + lines->Array.length
      }
    } else {
      if oldRangeStart.contents != 0 {
        if lines->Array.length->Int.toFloat <= context *. 2. && i < diff->Array.length - 2 {
          // Overlapping: the context joins the next change.
          lines->Array.forEach(line => curRange.contents->Array.push(" " ++ line))
        } else {
          let contextSize = Math.min(lines->Array.length->Int.toFloat, context)->Float.toInt
          lines
          ->Array.slice(~start=0, ~end=contextSize)
          ->Array.forEach(line => curRange.contents->Array.push(" " ++ line))
          hunks->Array.push({
            oldStart: oldRangeStart.contents->Int.toFloat,
            oldLines: (oldLine.contents - oldRangeStart.contents + contextSize)->Int.toFloat,
            newStart: newRangeStart.contents->Int.toFloat,
            newLines: (newLine.contents - newRangeStart.contents + contextSize)->Int.toFloat,
            lines: curRange.contents,
          })
          oldRangeStart := 0
          newRangeStart := 0
          curRange := []
        }
      }
      oldLine := oldLine.contents + lines->Array.length
      newLine := newLine.contents + lines->Array.length
    }
  )
  // Step 2: drop each line's newline, or mark that it has none.
  hunks->Array.forEach(hunk => {
    let rec tidy = i =>
      if i < hunk.lines->Array.length {
        let line = hunk.lines->Array.getUnsafe(i)
        if line->String.endsWith("\n") {
          hunk.lines->Array.setUnsafe(i, line->String.slice(~start=0, ~end=-1))
          tidy(i + 1)
        } else {
          hunk.lines->Array.splice(~start=i + 1, ~remove=0, ~insert=["\\ No newline at end of file"])
          tidy(i + 2)
        }
      }
    tidy(0)
  })
  {?oldFileName, ?newFileName, ?oldHeader, ?newHeader, hunks}
}

let needsQuoting = s => {
  let rec scan = i =>
    if i >= s->String.length {
      false
    } else {
      let unit = s->String.charCodeAtUnsafe(i)
      unit < 0x20 || unit > 0x7e || unit == 0x22 || unit == 0x5c ? true : scan(i + 1)
    }
  scan(0)
}

/** C-style quoting of a name, over TextEncoder's UTF-8, as Git and GNU diffutils write it. */
let quoteFileNameIfNeeded = s =>
  if !needsQuoting(s) {
    s
  } else {
    let bytes = textEncoder()->encode(s)
    let result = ref("\"")
    for i in 0 to bytes->TypedArray.length - 1 {
      let escaped = switch bytes->TypedArray.get(i)->Option.getOr(0) {
      | 0x07 => "\\a"
      | 0x08 => "\\b"
      | 0x09 => "\\t"
      | 0x0a => "\\n"
      | 0x0b => "\\v"
      | 0x0c => "\\f"
      | 0x0d => "\\r"
      | 0x22 => "\\\""
      | 0x5c => "\\\\"
      | printable if printable >= 0x20 && printable <= 0x7e => String.fromCharCode(printable)
      | other => "\\" ++ other->Int.toString(~radix=8)->String.padStart(3, "0")
      }
      result := result.contents ++ escaped
    }
    result.contents ++ "\""
  }

/** formatPatch with every header, for a patch from `structuredPatch`. */
let formatPatch = (patch: structuredPatch) => {
  let ret = []
  switch (patch.oldFileName, patch.newFileName) {
  | (Some(oldName), Some(newName)) if oldName == newName => ret->Array.push("Index: " ++ oldName)
  | _ => ()
  }
  ret->Array.push(underline)
  let header = value =>
    switch value {
    | Some(header) if header != "" => "\t" ++ header
    | _ => ""
    }
  switch (patch.oldFileName, patch.newFileName) {
  | (Some(oldName), Some(newName)) =>
    ret->Array.push("--- " ++ quoteFileNameIfNeeded(oldName) ++ header(patch.oldHeader))
    ret->Array.push("+++ " ++ quoteFileNameIfNeeded(newName) ++ header(patch.newHeader))
  | _ => ()
  }
  // The start printed for an empty range is one lower.
  let range = (start, count) => (count == 0. ? start -. 1. : start)->Float.toString ++ "," ++ count->Float.toString
  patch.hunks->Array.forEach(hunk => {
    ret->Array.push("@@ -" ++ range(hunk.oldStart, hunk.oldLines) ++ " +" ++ range(hunk.newStart, hunk.newLines) ++ " @@")
    hunk.lines->Array.forEach(line => ret->Array.push(line))
  })
  ret->Array.join("\n") ++ "\n"
}

// ---- parsePatch ----

// parse.js's line patterns, as it writes them; none is global, so exec and test do not depend on
// lastIndex.
let gitDiffHeader = /^diff --git /
let indexHeader = /^Index:\s/
let hgHeader = /^diff(?: -r \w+)+\s/
let headerPrefix = /^(?:Index:|diff(?: -r \w+)+)\s+/
let fileHeader = /^(---|\+\+\+)\s/
let fileHeaderPrefix = /^(---|\+\+\+)\s+/
let hunkHeader = /^@@\s/
let chunkHeader = /@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/
let hunkBodyLine = /^[+ -]/
let renameFrom = /^rename from (.*)/
let renameTo = /^rename to (.*)/
let copyFrom = /^copy from (.*)/
let copyTo = /^copy to (.*)/
let newFileMode = /^new file mode (\d+)/
let deletedFileMode = /^deleted file mode (\d+)/
let oldMode = /^old mode (\d+)/
let newMode = /^new mode (\d+)/
let binaryFiles = /^Binary files /

let isGitDiffHeader = line => gitDiffHeader->RegExp.test(line)
/** A line that starts the next file: `diff --git`, `Index:` or Mercurial's `diff -r`. */
let isDiffHeader = line => isGitDiffHeader(line) || indexHeader->RegExp.test(line) || hgHeader->RegExp.test(line)
let isFileHeader = line => fileHeader->RegExp.test(line)
let isHunkHeader = line => hunkHeader->RegExp.test(line)

/** The capture groups of `regexp` in `line`, from the first. */
let captures = (regexp, line) => regexp->RegExp.exec(line)->Option.map(RegExp.Result.matches)

/** The first capture of `regexp` in `line`. */
let capture = (regexp, line) => captures(regexp, line)->Option.flatMap(groups => groups->Array.getUnsafe(0))

let isOctal = (s, i) => {
  let unit = s->String.charCodeAt(i)->Option.getOr(0)
  unit >= 48 && unit <= 55
}

/** A C-style quoted name as Git and GNU diff write it: the unescaped name and the length of the
 quoted text, or None for an unterminated quote or an unknown escape. */
let parseQuotedFileName = s => {
  let length = s->String.length
  let octalEscape = j => s->String.charAt(j) == "\\" && isOctal(s, j + 1) && isOctal(s, j + 2) && isOctal(s, j + 3)
  // Octal escapes are raw bytes, a run of them decoded together as UTF-8 (Uint8Array keeps the low
  // byte of each). The index after the run, or None when an escape in it is cut short.
  let rec octets = (j, bytes) =>
    if !(s->String.charAt(j) == "\\" && isOctal(s, j + 1)) {
      Some(j)
    } else if octalEscape(j) {
      bytes->Array.push(s->String.slice(~start=j + 1, ~end=j + 4)->Int.fromString(~radix=8)->Option.getOr(0))
      octets(j + 4, bytes)
    } else {
      None
    }
  let rec scan = (j, result) =>
    if j >= length {
      None
    } else if s->String.charAt(j) == "\"" {
      Some((result, j + 1))
    } else if s->String.charAt(j) == "\\" && j + 1 < length {
      let simple = switch s->String.charAt(j + 1) {
      | "a" => Some("\u0007")
      | "b" => Some("\b")
      | "f" => Some("\u000c")
      | "n" => Some("\n")
      | "r" => Some("\r")
      | "t" => Some("\t")
      | "v" => Some("\u000b")
      | ("\\" | "\"") as escaped => Some(escaped)
      | _ => None
      }
      switch simple {
      | Some(unescaped) => scan(j + 2, result ++ unescaped)
      | None if octalEscape(j) =>
        let bytes = []
        switch octets(j, bytes) {
        | Some(next) => scan(next, result ++ textDecoder()->decode(Uint8Array.fromArray(bytes)))
        | None => None
        }
      | None => None
      }
    } else {
      scan(j + 1, result ++ s->String.charAt(j))
    }
  s->String.startsWith("\"") ? scan(1, "") : None
}

let unquoteIfQuoted = s =>
  s->String.startsWith("\"") ? parseQuotedFileName(s)->Option.mapOr(s, ((fileName, _)) => fileName) : s

/** The names in a `diff --git` line, or None when they cannot be told apart. */
let parseGitDiffHeader = line => {
  let rest = line->String.slice(~start="diff --git "->String.length)
  if rest->String.startsWith("\"") {
    parseQuotedFileName(rest)->Option.flatMap(((oldFileName, rawLength)) => {
      // Skip the one unit after the quote, assumed to be a space.
      let afterOld = rest->String.substring(~start=rawLength + 1)
      afterOld->String.startsWith("\"")
        ? parseQuotedFileName(afterOld)->Option.map(((newFileName, _)) => (oldFileName, newFileName))
        : Some((oldFileName, afterOld))
    })
  } else {
    let quoteIdx = rest->String.indexOf("\"")
    if quoteIdx > 0 {
      parseQuotedFileName(rest->String.slice(~start=quoteIdx))->Option.map(((newFileName, _)) => (
        rest->String.slice(~start=0, ~end=quoteIdx - 1),
        newFileName,
      ))
    } else if rest->String.startsWith("a/") {
      // Split on the middle " b/", which is right whenever both names are the same.
      let splits = []
      let rec find = from =>
        switch rest->String.indexOfFrom(" b/", from) {
        | -1 => ()
        | idx =>
          splits->Array.push(idx)
          find(idx + 1)
        }
      find(1)
      splits[splits->Array.length / 2]->Option.map(mid => (
        rest->String.slice(~start=0, ~end=mid),
        rest->String.slice(~start=mid + 1),
      ))
    } else {
      None
    }
  }
}

/** Git extended headers: each pattern is tried, and at most one matches. */
let extendedHeader = (index, extLine) => {
  let set = (key, value) => index->Dict.set(key, toUnknown(value))
  capture(renameFrom, extLine)->Option.forEach(name => {
    set("oldFileName", "a/" ++ unquoteIfQuoted(name))
    set("isRename", true)
  })
  capture(renameTo, extLine)->Option.forEach(name => {
    set("newFileName", "b/" ++ unquoteIfQuoted(name))
    set("isRename", true)
  })
  capture(copyFrom, extLine)->Option.forEach(name => {
    set("oldFileName", "a/" ++ unquoteIfQuoted(name))
    set("isCopy", true)
  })
  capture(copyTo, extLine)->Option.forEach(name => {
    set("newFileName", "b/" ++ unquoteIfQuoted(name))
    set("isCopy", true)
  })
  capture(newFileMode, extLine)->Option.forEach(mode => {
    set("isCreate", true)
    set("newMode", mode)
  })
  capture(deletedFileMode, extLine)->Option.forEach(mode => {
    set("isDelete", true)
    set("oldMode", mode)
  })
  capture(oldMode, extLine)->Option.forEach(mode => set("oldMode", mode))
  capture(newMode, extLine)->Option.forEach(mode => set("newMode", mode))
  if binaryFiles->RegExp.test(extLine) {
    set("isBinary", true)
  }
}

/** parsePatch: the unified diff text as file patches, each a JS object whose keys are in the order
 jsdiff first assigns them. */
let parsePatch = (uniDiff): array<dict<unknown>> => {
  let diffstr = uniDiff->String.split("\n")
  let length = diffstr->Array.length
  let i = ref(0)
  let current = () => diffstr->Array.getUnsafe(i.contents)
  let advance = () => i := i.contents + 1
  let list = []

  // The --- or +++ line at the cursor, if there is one.
  let parseFileHeader = index =>
    if i.contents < length {
      captures(fileHeaderPrefix, current())->Option.forEach(groups => {
        let data = current()->String.slice(~start=3)->String.trim->String.split("\t")
        let header = data[1]->Option.getOr("")->String.trim
        let fileName = data->Array.getUnsafe(0)
        let fileName = fileName->String.startsWith("\"")
          ? unquoteIfQuoted(fileName)
          : fileName->String.replaceAll("\\\\", "\\")
        let side = groups->Array.getUnsafe(0) == Some("---") ? "old" : "new"
        index->Dict.set(side ++ "FileName", toUnknown(fileName))
        index->Dict.set(side ++ "Header", toUnknown(header))
        advance()
      })
    }

  // The hunk at the cursor. A header the range pattern does not match gives NaN starts.
  let parseHunk = () => {
    let chunkHeaderIndex = i.contents
    let range = captures(chunkHeader, current())
    advance()
    let number = group => range->Option.flatMap(groups => groups->Array.getUnsafe(group - 1))->Option.flatMap(Float.fromString)
    let hunk = {
      oldStart: number(1)->Option.getOr(Float.Constants.nan),
      oldLines: number(2)->Option.getOr(1.),
      newStart: number(3)->Option.getOr(Float.Constants.nan),
      newLines: number(4)->Option.getOr(1.),
      lines: [],
    }
    // Unified diff quirk: the start of an empty range is one lower.
    if hunk.oldLines == 0. {
      hunk.oldStart = hunk.oldStart +. 1.
    }
    if hunk.newLines == 0. {
      hunk.newStart = hunk.newStart +. 1.
    }
    let addCount = ref(0)
    let removeCount = ref(0)
    let rec body = () =>
      if (
        i.contents < length &&
          (removeCount.contents->Int.toFloat < hunk.oldLines ||
          addCount.contents->Int.toFloat < hunk.newLines ||
          current()->String.startsWith("\\"))
      ) {
        let line = current()
        // An empty line is context, except the last one (whose `[0]` is undefined).
        switch line == "" && i.contents != length - 1 ? " " : line->String.charAt(0) {
        | "+" => addCount := addCount.contents + 1
        | "-" => removeCount := removeCount.contents + 1
        | " " =>
          addCount := addCount.contents + 1
          removeCount := removeCount.contents + 1
        | "\\" => ()
        | _ =>
          JsError.throwWithMessage(
            `Hunk at line ${Int.toString(chunkHeaderIndex + 1)} contained invalid line ${line}`,
          )
        }
        hunk.lines->Array.push(line)
        advance()
        body()
      }
    body()
    // The empty block count case.
    if addCount.contents == 0 && hunk.newLines == 1. {
      hunk.newLines = 0.
    }
    if removeCount.contents == 0 && hunk.oldLines == 1. {
      hunk.oldLines = 0.
    }
    let at = Int.toString(chunkHeaderIndex + 1)
    if addCount.contents->Int.toFloat != hunk.newLines {
      JsError.throwWithMessage(`Added line count did not match for hunk at line ${at}`)
    }
    if removeCount.contents->Int.toFloat != hunk.oldLines {
      JsError.throwWithMessage(`Removed line count did not match for hunk at line ${at}`)
    }
    // A body line right after the counted ones means the counts were wrong, unless it is a file header.
    switch diffstr[i.contents] {
    | Some(next) if next != "" && hunkBodyLine->RegExp.test(next) && !isFileHeader(next) =>
      JsError.throwWithMessage(`Hunk at line ${at} has more lines than expected`)
    | _ => ()
    }
    hunk
  }

  let parseIndex = () => {
    let index = Dict.make()
    let hunks = []
    index->Dict.set("hunks", toUnknown(hunks))
    list->Array.push(index)
    // The diff metadata, up to the file or hunk headers; false when a second diff header, which
    // belongs to the next file, ends this one.
    let rec metadata = seenDiffHeader =>
      if i.contents >= length {
        true
      } else {
        let line = current()
        if isFileHeader(line) || isHunkHeader(line) {
          true
        } else if isGitDiffHeader(line) {
          if seenDiffHeader {
            false
          } else {
            index->Dict.set("isGit", toUnknown(true))
            parseGitDiffHeader(line)->Option.forEach(((oldFileName, newFileName)) => {
              index->Dict.set("oldFileName", toUnknown(oldFileName))
              index->Dict.set("newFileName", toUnknown(newFileName))
            })
            advance()
            while i.contents < length && !isFileHeader(current()) && !isHunkHeader(current()) && !isDiffHeader(current()) {
              extendedHeader(index, current())
              advance()
            }
            metadata(true)
          }
        } else if isDiffHeader(line) {
          if seenDiffHeader {
            false
          } else {
            // The index is the rest of an `Index:` or Mercurial header, trimmed.
            headerPrefix
            ->RegExp.exec(line)
            ->Option.forEach(headerMatch =>
              index->Dict.set(
                "index",
                toUnknown(line->String.slice(~start=headerMatch->RegExp.Result.fullMatch->String.length)->String.trim),
              )
            )
            advance()
            metadata(true)
          }
        } else {
          advance()
          metadata(seenDiffHeader)
        }
      }
    if metadata(false) {
      parseFileHeader(index)
      parseFileHeader(index)
      // One file header without the other is a malformed patch.
      switch (index->Dict.get("oldFileName"), index->Dict.get("newFileName")) {
      | (Some(_), None) => JsError.throwWithMessage(`Missing "+++ ..." file header`)
      | (None, Some(_)) => JsError.throwWithMessage(`Missing "--- ..." file header`)
      | _ => ()
      }
      // Blank lines and other content between or after hunks are skipped.
      while (
        i.contents < length &&
        !isDiffHeader(current()) &&
        !isFileHeader(current()) &&
        !(current()->String.startsWith(underline))
      ) {
        if isHunkHeader(current()) {
          hunks->Array.push(parseHunk())
        } else {
          advance()
        }
      }
    }
  }

  while i.contents < length {
    parseIndex()
  }
  list
}

// ---- applyPatch ----

let hasOnlyWinLineEndings = s => {
  let rec bareNewline = i =>
    if i >= s->String.length {
      false
    } else if s->String.charCodeAtUnsafe(i) == 10 && s->String.charCodeAtUnsafe(i - 1) != 13 {
      true
    } else {
      bareNewline(i + 1)
    }
  s->String.includes("\r\n") && !(s->String.startsWith("\n")) && !bareNewline(1)
}

let hasOnlyUnixLineEndings = s => !(s->String.includes("\r\n")) && s->String.includes("\n")

let beforeMarker = (lines, i) => lines[i + 1]->Option.mapOr(false, next => next->String.startsWith("\\"))

let isUnix = hunks =>
  !(hunks->Array.some(hunk => hunk.lines->Array.some(line => !(line->String.startsWith("\\")) && line->String.endsWith("\r"))))

let isWin = hunks =>
  hunks->Array.some(hunk => hunk.lines->Array.some(line => line->String.endsWith("\r"))) &&
    hunks->Array.every(hunk =>
      hunk.lines->Array.everyWithIndex((line, i) =>
        line->String.startsWith("\\") || line->String.endsWith("\r") || beforeMarker(hunk.lines, i)
      )
    )

let mapLines = (hunks, transform) =>
  hunks->Array.map(hunk => {...hunk, lines: hunk.lines->Array.mapWithIndex((_, i) => transform(hunk.lines, i))})

let unixToWin = hunks =>
  hunks->mapLines((lines, i) => {
    let line = lines->Array.getUnsafe(i)
    line->String.startsWith("\\") || line->String.endsWith("\r") || beforeMarker(lines, i) ? line : line ++ "\r"
  })

let winToUnix = hunks =>
  hunks->mapLines((lines, i) => {
    let line = lines->Array.getUnsafe(i)
    line->String.endsWith("\r") ? line->String.slice(~start=0, ~end=-1) : line
  })

/** distance-iterator.js: positions after `start`, alternating forward and backward, within the
 limits. Positions are JavaScript numbers, so a NaN start yields nothing, as in npm. */
let distanceIterator = (start: float, minLine: float, maxLine: float) => {
  let wantForward = ref(true)
  let backwardExhausted = ref(false)
  let forwardExhausted = ref(false)
  let localOffset = ref(1.)
  let rec next = () => {
    let forward = if wantForward.contents && !forwardExhausted.contents {
      if backwardExhausted.contents {
        localOffset := localOffset.contents +. 1.
      } else {
        wantForward := false
      }
      // Fit after the offset location, unless that is beyond the text.
      if start +. localOffset.contents <= maxLine {
        Some(start +. localOffset.contents)
      } else {
        forwardExhausted := true
        None
      }
    } else {
      None
    }
    switch forward {
    | Some(_) => forward
    | None if !backwardExhausted.contents =>
      if !forwardExhausted.contents {
        wantForward := true
      }
      // Fit before the offset location, unless that is before the text.
      if minLine <= start -. localOffset.contents {
        let position = start -. localOffset.contents
        localOffset := localOffset.contents +. 1.
        Some(position)
      } else {
        backwardExhausted := true
        next()
      }
    | None => None
    }
  }
  next
}

let applyHunks = (lines, hunks) => {
  // lines[i], where a position outside the source, or NaN, reads as undefined.
  let lineAt = i => i >= 0. && i < lines->Array.length->Int.toFloat ? Some(lines->Array.getUnsafe(i->Float.toInt)) : None
  // applyHunk with no errors allowed: every removed and context line must equal the source line at
  // its position. Trailing context is given back, so the next hunk may start inside it.
  let applyHunk = (hunkLines, position) => {
    let patchedLines = []
    let rec step = (k, toPos, nConsecutiveOldContextLines) =>
      if k == hunkLines->Array.length {
        let kept = patchedLines->Array.length - nConsecutiveOldContextLines
        Some((patchedLines->Array.slice(~start=0, ~end=kept), toPos -. nConsecutiveOldContextLines->Int.toFloat -. 1.))
      } else {
        let hunkLine = hunkLines->Array.getUnsafe(k)
        let content = hunkLine->String.slice(~start=1)
        switch hunkLine == "" ? " " : hunkLine->String.charAt(0) {
        | "-" => lineAt(toPos) == Some(content) ? step(k + 1, toPos +. 1., 0) : None
        | "+" =>
          patchedLines->Array.push(content)
          step(k + 1, toPos, 0)
        | " " =>
          if lineAt(toPos) == Some(content) {
            patchedLines->Array.push(content)
            step(k + 1, toPos +. 1., nConsecutiveOldContextLines + 1)
          } else {
            None
          }
        | _ => step(k + 1, toPos, nConsecutiveOldContextLines)
        }
      }
    step(0, position, 0)
  }
  let resultLines = []
  // Positions are JavaScript numbers: a NaN start fits only a hunk with no removed or context
  // line, and then every later position is NaN too. A position outside the source reads as
  // undefined, which joins as an empty line.
  let rec copy = (from, until) =>
    if from < until {
      resultLines->Array.push(lineAt(from)->Option.getOr(""))
      copy(from +. 1., until)
    }
  let rec fit = (k, minLine, prevHunkOffset) =>
    if k == hunks->Array.length {
      copy(minLine, lines->Array.length->Int.toFloat)
      Some(resultLines->Array.join("\n"))
    } else {
      let hunk = hunks->Array.getUnsafe(k)
      let maxLine = lines->Array.length->Int.toFloat -. hunk.oldLines
      // The declared position first, then the positions around it.
      let toPos = hunk.oldStart +. prevHunkOffset -. 1.
      let candidates = distanceIterator(toPos, minLine, maxLine)
      let rec search = toPos =>
        switch applyHunk(hunk.lines, toPos) {
        | Some(fitted) => Some((toPos, fitted))
        | None =>
          switch candidates() {
          | Some(position) => search(position)
          | None => None
          }
        }
      switch search(toPos) {
      | None => None
      | Some((toPos, (patchedLines, oldLineLastI))) =>
        copy(minLine, toPos)
        patchedLines->Array.forEach(line => resultLines->Array.push(line))
        // Later hunks may not fit over text this one patched.
        fit(k + 1, oldLineLastI +. 1., toPos +. 1. -. hunk.oldStart)
      }
    }
  fit(0, 0., 0.)
}

let applyStructuredPatch = (source, patchHunks) => {
  let hunks = if hasOnlyWinLineEndings(source) && isUnix(patchHunks) {
    unixToWin(patchHunks)
  } else if hasOnlyUnixLineEndings(source) && isWin(patchHunks) {
    winToUnix(patchHunks)
  } else {
    patchHunks
  }
  let lines = source->String.split("\n")
  switch hunks->Array.at(-1) {
  | None => Some(source)
  | Some(last) =>
    // A change to the newline at the end of the file is in the last hunk.
    let (removeEOFNL, addEOFNL, _) = last.lines->Array.reduce((false, false, ""), (
      (removeEOFNL, addEOFNL, prevLine),
      line,
    ) =>
      switch (line->String.startsWith("\\"), prevLine->String.charAt(0)) {
      | (true, "+") => (true, addEOFNL, line)
      | (true, "-") => (removeEOFNL, true, line)
      | _ => (removeEOFNL, addEOFNL, line)
      }
    )
    let endsWithNewline = lines->Array.at(-1) == Some("")
    let lines = switch (removeEOFNL, addEOFNL) {
    // The last line changes but has no newline on either side: the source must not have one.
    | (true, true) => endsWithNewline ? None : Some(lines)
    | (true, false) => endsWithNewline ? Some(lines->Array.slice(~start=0, ~end=-1)) : None
    | (false, true) => endsWithNewline ? None : Some(lines->Array.concat([""]))
    | (false, false) => Some(lines)
    }
    lines->Option.flatMap(lines => applyHunks(lines, hunks))
  }
}

/** applyPatch over file patches (parsed, or structured JSON of the same shape). */
let applyPatch = (source, patches: array<dict<unknown>>) =>
  switch patches {
  | [patch] => applyStructuredPatch(source, patch->Dict.getUnsafe("hunks")->fromUnknown)
  | [] => JsError.throwWithMessage("applyPatch needs a patch")
  | _ => JsError.throwWithMessage("applyPatch only works with a single input.")
  }

// ---- reversePatch, over the structured patch's own JSON values ----

let swapPrefix = (fileName: unknown): unknown =>
  switch Type.typeof(fileName) {
  | #string =>
    let name: string = fromUnknown(fileName)
    if name->String.startsWith("a/") {
      toUnknown("b/" ++ name->String.slice(~start=2))
    } else if name->String.startsWith("b/") {
      toUnknown("a/" ++ name->String.slice(~start=2))
    } else {
      fileName
    }
  | _ => fileName
  }

let reverseLine = line =>
  if line->String.startsWith("-") {
    "+" ++ line->String.slice(~start=1)
  } else if line->String.startsWith("+") {
    "-" ++ line->String.slice(~start=1)
  } else {
    line
  }

/** reversePatch: a copy of each structured patch with its sides swapped, the list reversed. */
let rec reversePatch = (structuredPatch: JSON.t): unknown =>
  switch structuredPatch {
  | Array(patches) => patches->Array.map(reversePatch)->Array.toReversed->toUnknown
  | _ =>
    let patch: dict<unknown> = fromJson(structuredPatch)
    let field = key => patch->Dict.getUnsafe(key)
    let isGit = truthy(field("isGit"))
    let name = key => isGit ? swapPrefix(field(key)) : field(key)
    // Object.assign({}, patch, {...}): the patch's own keys first, then the new ones.
    let reversed = patch->Dict.copy
    reversed->Dict.set("oldFileName", name("newFileName"))
    reversed->Dict.set("oldHeader", field("newHeader"))
    reversed->Dict.set("newFileName", name("oldFileName"))
    reversed->Dict.set("newHeader", field("oldHeader"))
    reversed->Dict.set("oldMode", field("newMode"))
    reversed->Dict.set("newMode", field("oldMode"))
    reversed->Dict.set("isCreate", field("isDelete"))
    reversed->Dict.set("isDelete", field("isCreate"))
    let hunks: array<dict<unknown>> = fromUnknown(field("hunks"))
    reversed->Dict.set(
      "hunks",
      hunks
      ->Array.map(hunk => {
        let lines: array<string> = fromUnknown(hunk->Dict.getUnsafe("lines"))
        Dict.fromArray([
          ("oldLines", hunk->Dict.getUnsafe("newLines")),
          ("oldStart", hunk->Dict.getUnsafe("newStart")),
          ("newLines", hunk->Dict.getUnsafe("oldLines")),
          ("newStart", hunk->Dict.getUnsafe("oldStart")),
          ("lines", lines->Array.map(reverseLine)->toUnknown),
        ])
      })
      ->toUnknown,
    )
    if truthy(field("isCopy")) {
      // Reversing a copy deletes the copy destination; the hunks cannot be reversed into a deletion.
      reversed->Dict.set("newFileName", toUnknown("/dev/null"))
      reversed->Dict.set("newHeader", toUnknown(Nullable.undefined))
      reversed->Dict.set("isDelete", toUnknown(true))
      ["isCreate", "isCopy", "isRename"]->Array.forEach(key => reversed->Dict.delete(key))
      reversed->Dict.set("hunks", toUnknown([]))
    }
    toUnknown(reversed)
  }
