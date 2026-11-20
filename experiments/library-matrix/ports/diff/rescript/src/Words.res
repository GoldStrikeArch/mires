// jsdiff 9.0.0 diff/word.js and the whitespace helpers of util/string.js.

open Diff

// Words are runs of word characters (Latin letters with diacritics and the spacing modifier
// letters included), whitespace runs, or single other characters. String.match with a global
// RegExp starts from lastIndex 0.
let tokenizeIncludingWhitespace = /[a-zA-Z0-9_\u{AD}\u{C0}-\u{D6}\u{D8}-\u{F6}\u{F8}-\u{2C6}\u{2C8}-\u{2D7}\u{2DE}-\u{2FF}\u{1E00}-\u{1EFF}]+|\s+|[^a-zA-Z0-9_\u{AD}\u{C0}-\u{D6}\u{D8}-\u{F6}\u{F8}-\u{2C6}\u{2C8}-\u{2D7}\u{2DE}-\u{2FF}\u{1E00}-\u{1EFF}]/gu
let whitespace = /\s/
let leadingWhitespace = /^\s*/

let isWhitespaceUnit = unit =>
  (unit >= 9 && unit <= 13) ||
  unit == 32 ||
  unit == 0xa0 ||
  unit == 0x1680 ||
  (unit >= 0x2000 && unit <= 0x200a) ||
  unit == 0x2028 ||
  unit == 0x2029 ||
  unit == 0x202f ||
  unit == 0x205f ||
  unit == 0x3000 ||
  unit == 0xfeff

/** leadingWs: the match of `^\s*`. */
let leadingWs = s =>
  switch s->String.match(leadingWhitespace) {
  | Some(found) => found->RegExp.Result.fullMatch
  | None => ""
  }

/** trailingWs: the whitespace units at the end, tested one at a time. */
let trailingWs = s => {
  let rec start = i => i > 0 && isWhitespaceUnit(s->String.charCodeAtUnsafe(i - 1)) ? start(i - 1) : i
  s->String.slice(~start=start(s->String.length))
}

let tokenize = value => {
  // Whitespace runs are joined to the neighbouring word or punctuation tokens.
  let parts = switch value->Host.matchAll(tokenizeIncludingWhitespace) {
  | Value(parts) => parts
  | Null => []
  }
  let tokens = []
  let prevPart = ref(None)
  let joinLast = part => {
    let last = tokens->Array.length - 1
    tokens->Array.setUnsafe(last, tokens->Array.getUnsafe(last) ++ part)
  }
  parts->Array.forEach(part => {
    if whitespace->RegExp.test(part) {
      switch prevPart.contents {
      | None => tokens->Array.push(part)
      | Some(_) => joinLast(part)
      }
    } else {
      switch prevPart.contents {
      | Some(prev) if whitespace->RegExp.test(prev) =>
        tokens->Array.at(-1) == Some(prev) ? joinLast(part) : tokens->Array.push(prev ++ part)
      | _ => tokens->Array.push(part)
      }
    }
    prevPart := Some(part)
  })
  tokens
}

let equals = (options, left, right) =>
  options.ignoreCase
    ? left->String.toLowerCase->String.trim == right->String.toLowerCase->String.trim
    : left->String.trim == right->String.trim

// Tokens that are joined appeared consecutively, so each but the first drops its leading whitespace.
let join = tokens =>
  tokens
  ->Array.mapWithIndex((token, i) => i == 0 ? token : token->String.slice(~start=leadingWs(token)->String.length))
  ->Array.join("")

let longestCommonPrefix = (a, b) => {
  let rec common = i =>
    i < a->String.length && i < b->String.length && a->String.charCodeAtUnsafe(i) == b->String.charCodeAtUnsafe(i)
      ? common(i + 1)
      : i
  a->String.slice(~start=0, ~end=common(0))
}

let longestCommonSuffix = (a, b) => {
  let rec common = i =>
    i < a->String.length &&
    i < b->String.length &&
    a->String.charCodeAtUnsafe(a->String.length - 1 - i) == b->String.charCodeAtUnsafe(b->String.length - 1 - i)
      ? common(i + 1)
      : i
  a->String.slice(~start=a->String.length - common(0))
}

let replacePrefix = (string, oldPrefix, newPrefix) =>
  string->String.startsWith(oldPrefix)
    ? newPrefix ++ string->String.slice(~start=oldPrefix->String.length)
    : JsError.throwWithMessage("string doesn't start with prefix; this is a bug")

let replaceSuffix = (string, oldSuffix, newSuffix) =>
  string->String.endsWith(oldSuffix)
    ? string->String.slice(~start=0, ~end=string->String.length - oldSuffix->String.length) ++ newSuffix
    : JsError.throwWithMessage("string doesn't end with suffix; this is a bug")

let removePrefix = (string, oldPrefix) => replacePrefix(string, oldPrefix, "")
let removeSuffix = (string, oldSuffix) => replaceSuffix(string, oldSuffix, "")

/** The longest suffix of `a` that is a prefix of `b`, by Knuth-Morris-Pratt (overlapCount). */
let maximumOverlap = (a, b) => {
  let unit = (s, i) => s->String.charCodeAtUnsafe(i)
  let startA = a->String.length > b->String.length ? a->String.length - b->String.length : 0
  let endB = Math.Int.min(a->String.length, b->String.length)
  let map = Array.make(~length=endB, 0)
  let k = ref(0)
  for j in 1 to endB - 1 {
    map->Array.setUnsafe(j, unit(b, j) == unit(b, k.contents) ? map->Array.getUnsafe(k.contents) : k.contents)
    while k.contents > 0 && unit(b, j) != unit(b, k.contents) {
      k := map->Array.getUnsafe(k.contents)
    }
    if unit(b, j) == unit(b, k.contents) {
      k := k.contents + 1
    }
  }
  k := 0
  for i in startA to a->String.length - 1 {
    while k.contents > 0 && unit(a, i) != unit(b, k.contents) {
      k := map->Array.getUnsafe(k.contents)
    }
    if unit(a, i) == unit(b, k.contents) {
      k := k.contents + 1
    }
  }
  b->String.slice(~start=0, ~end=k.contents)
}

/** dedupeWhitespaceInChangeObjects: whitespace that trails one change object and leads the next is
 kept once, on the side where both texts have it. */
let dedupeWhitespace = (startKeep, deletion, insertion, endKeep) =>
  switch (deletion, insertion) {
  | (Some(deletion), Some(insertion)) =>
    let oldWsPrefix = leadingWs(deletion.value)
    let oldWsSuffix = trailingWs(deletion.value)
    let newWsPrefix = leadingWs(insertion.value)
    let newWsSuffix = trailingWs(insertion.value)
    startKeep->Option.forEach(startKeep => {
      let commonWsPrefix = longestCommonPrefix(oldWsPrefix, newWsPrefix)
      startKeep.value = replaceSuffix(startKeep.value, newWsPrefix, commonWsPrefix)
      deletion.value = removePrefix(deletion.value, commonWsPrefix)
      insertion.value = removePrefix(insertion.value, commonWsPrefix)
    })
    endKeep->Option.forEach(endKeep => {
      let commonWsSuffix = longestCommonSuffix(oldWsSuffix, newWsSuffix)
      endKeep.value = replacePrefix(endKeep.value, newWsSuffix, commonWsSuffix)
      deletion.value = removeSuffix(deletion.value, commonWsSuffix)
      insertion.value = removeSuffix(insertion.value, commonWsSuffix)
    })
  | (None, Some(insertion)) =>
    // The whitespace is all from the new text: each object keeps its trailing whitespace.
    if startKeep->Option.isSome {
      insertion.value = insertion.value->String.slice(~start=leadingWs(insertion.value)->String.length)
    }
    endKeep->Option.forEach(endKeep =>
      endKeep.value = endKeep.value->String.slice(~start=leadingWs(endKeep.value)->String.length)
    )
  | (Some(deletion), None) =>
    switch (startKeep, endKeep) {
    | (Some(startKeep), Some(endKeep)) =>
      let newWsFull = leadingWs(endKeep.value)
      let delWsStart = leadingWs(deletion.value)
      let delWsEnd = trailingWs(deletion.value)
      // Whitespace right after startKeep in both texts goes to startKeep.
      let newWsStart = longestCommonPrefix(newWsFull, delWsStart)
      deletion.value = removePrefix(deletion.value, newWsStart)
      // Whitespace right before endKeep in both texts, not given to startKeep, goes to endKeep.
      let newWsEnd = longestCommonSuffix(removePrefix(newWsFull, newWsStart), delWsEnd)
      deletion.value = removeSuffix(deletion.value, newWsEnd)
      endKeep.value = replacePrefix(endKeep.value, newWsFull, newWsEnd)
      // Any whitespace from the new text not yet assigned goes to the start.
      startKeep.value = replaceSuffix(
        startKeep.value,
        newWsFull,
        newWsFull->String.slice(~start=0, ~end=newWsFull->String.length - newWsEnd->String.length),
      )
    | (None, Some(endKeep)) =>
      // At the start of the text: drop the deletion's trailing whitespace that endKeep repeats.
      deletion.value = removeSuffix(deletion.value, maximumOverlap(trailingWs(deletion.value), leadingWs(endKeep.value)))
    | (Some(startKeep), None) =>
      // At the end of the text: drop the deletion's leading whitespace that startKeep repeats.
      deletion.value = removePrefix(deletion.value, maximumOverlap(trailingWs(startKeep.value), leadingWs(deletion.value)))
    | (None, None) => ()
    }
  | (None, None) => ()
  }

let postProcess = (changes: array<change<string>>, options) => {
  if !options.oneChangePerToken {
    let lastKeep = ref(None)
    // The insertion and deletion since the last kept change; there is at most one of each.
    let insertion = ref(None)
    let deletion = ref(None)
    changes->Array.forEach(change =>
      if change.added {
        insertion := Some(change)
      } else if change.removed {
        deletion := Some(change)
      } else {
        if insertion.contents->Option.isSome || deletion.contents->Option.isSome {
          dedupeWhitespace(lastKeep.contents, deletion.contents, insertion.contents, Some(change))
        }
        lastKeep := Some(change)
        insertion := None
        deletion := None
      }
    )
    if insertion.contents->Option.isSome || deletion.contents->Option.isSome {
      dedupeWhitespace(lastKeep.contents, deletion.contents, insertion.contents, None)
    }
  }
  changes
}

let words = (before, after, options) =>
  diff(
    tokenize(before)->nonEmpty,
    tokenize(after)->nonEmpty,
    {equals: equals(options, ...), join, longest: None},
    options,
  )->postProcess(options)
