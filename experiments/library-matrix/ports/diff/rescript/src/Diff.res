// jsdiff 9.0.0 diff/base.js with the character, line, JSON and array diffs.

/** The synchronous options of the contract; patch creation passes none of them. */
type options = {
  ignoreCase: bool,
  oneChangePerToken: bool,
  stripTrailingCr: bool,
  ignoreWhitespace: bool,
  newlineIsToken: bool,
  ignoreNewlineAtEof: bool,
}

let defaults = {
  ignoreCase: false,
  oneChangePerToken: false,
  stripTrailingCr: false,
  ignoreWhitespace: false,
  newlineIsToken: false,
  ignoreNewlineAtEof: false,
}

/** A change object; the record is the JS object jsdiff returns. */
type change<'v> = {count: int, added: bool, removed: bool, mutable value: 'v}

/** An edit-history node; each path shares its predecessors with its neighbours. */
type rec component = {count: int, added: bool, removed: bool, previous: option<component>}

type path = {mutable oldPos: int, mutable lastComponent: option<component>}

/** How tokens compare and how a run of them becomes a change's value. `longest` is diffJson's
 `useLongestToken`: kept lines take the longer of the old and the new token. */
type policy<'t, 'v> = {
  equals: ('t, 't) => bool,
  join: array<'t> => 'v,
  longest: option<('t, 't) => 't>,
}

let addToPath = (path, ~added, ~removed, ~oldPosInc, options) => {
  let lastComponent = switch path.lastComponent {
  | Some(last) if !options.oneChangePerToken && last.added == added && last.removed == removed => {
      count: last.count + 1,
      added,
      removed,
      previous: last.previous,
    }
  | last => {count: 1, added, removed, previous: last}
  }
  {oldPos: path.oldPos + oldPosInc, lastComponent: Some(lastComponent)}
}

/** Follows the diagonal from `basePath` while the tokens agree; the new position reached. */
let extractCommon = (basePath, newTokens, oldTokens, diagonalPath, equals, options) => {
  let rec extend = (oldPos, newPos, commonCount) =>
    if (
      newPos + 1 < newTokens->Array.length &&
      oldPos + 1 < oldTokens->Array.length &&
      equals(oldTokens->Array.getUnsafe(oldPos + 1), newTokens->Array.getUnsafe(newPos + 1))
    ) {
      if options.oneChangePerToken {
        basePath.lastComponent = Some({
          count: 1,
          added: false,
          removed: false,
          previous: basePath.lastComponent,
        })
      }
      extend(oldPos + 1, newPos + 1, commonCount + 1)
    } else {
      if commonCount > 0 && !options.oneChangePerToken {
        basePath.lastComponent = Some({
          count: commonCount,
          added: false,
          removed: false,
          previous: basePath.lastComponent,
        })
      }
      basePath.oldPos = oldPos
      newPos
    }
  extend(basePath.oldPos, basePath.oldPos - diagonalPath, 0)
}

/** Myers' O(ND) search: the history of the first path to reach the end of both token lists. */
let search = (oldTokens, newTokens, equals, options) => {
  let newLen = newTokens->Array.length
  let oldLen = oldTokens->Array.length
  let maxEditLength = newLen + oldLen
  // The best path on each diagonal, which runs from -maxEditLength - 1 to maxEditLength + 1.
  let bestPath = Array.make(~length=2 * maxEditLength + 3, None)
  let at = diagonal => diagonal + maxEditLength + 1
  let origin = {oldPos: -1, lastComponent: None}
  bestPath->Array.setUnsafe(at(0), Some(origin))
  let newPos = extractCommon(origin, newTokens, oldTokens, 0, equals, options)
  if origin.oldPos + 1 >= oldLen && newPos + 1 >= newLen {
    origin.lastComponent
  } else {
    // Once a path reaches the right or bottom edge, the diagonals beyond it are pointless. No
    // diagonal lies outside [-maxEditLength, maxEditLength], so those bounds limit nothing.
    let minDiagonalToConsider = ref(-maxEditLength)
    let maxDiagonalToConsider = ref(maxEditLength)
    // One edit length: the finished history, if a path reaches the end.
    let rec diagonals = (diagonalPath, editLength) =>
      if diagonalPath > Math.Int.min(maxDiagonalToConsider.contents, editLength) {
        None
      } else {
        let removePath = bestPath->Array.getUnsafe(at(diagonalPath - 1))
        let addPath = bestPath->Array.getUnsafe(at(diagonalPath + 1))
        // No one else is going to use this value.
        if removePath->Option.isSome {
          bestPath->Array.setUnsafe(at(diagonalPath - 1), None)
        }
        // The new position after an insertion must lie inside the new tokens.
        let canAdd = switch addPath {
        | Some(add) => add.oldPos - diagonalPath >= 0 && add.oldPos - diagonalPath < newLen
        | None => false
        }
        let canRemove = switch removePath {
        | Some(remove) => remove.oldPos + 1 < oldLen
        | None => false
        }
        // Branch from the path that is furthest along the old tokens.
        let basePath = switch (addPath, removePath) {
        | (Some(add), Some(remove)) if canAdd && (!canRemove || remove.oldPos < add.oldPos) =>
          Some(addToPath(add, ~added=true, ~removed=false, ~oldPosInc=0, options))
        | (Some(add), _) if canAdd && !canRemove =>
          Some(addToPath(add, ~added=true, ~removed=false, ~oldPosInc=0, options))
        | (_, Some(remove)) if canRemove =>
          Some(addToPath(remove, ~added=false, ~removed=true, ~oldPosInc=1, options))
        | _ => None
        }
        switch basePath {
        | None =>
          bestPath->Array.setUnsafe(at(diagonalPath), None)
          diagonals(diagonalPath + 2, editLength)
        | Some(basePath) =>
          let newPos = extractCommon(basePath, newTokens, oldTokens, diagonalPath, equals, options)
          if basePath.oldPos + 1 >= oldLen && newPos + 1 >= newLen {
            Some(basePath.lastComponent)
          } else {
            bestPath->Array.setUnsafe(at(diagonalPath), Some(basePath))
            if basePath.oldPos + 1 >= oldLen {
              maxDiagonalToConsider := Math.Int.min(maxDiagonalToConsider.contents, diagonalPath - 1)
            }
            if newPos + 1 >= newLen {
              minDiagonalToConsider := Math.Int.max(minDiagonalToConsider.contents, diagonalPath + 1)
            }
            diagonals(diagonalPath + 2, editLength)
          }
        }
      }
    let rec rounds = editLength =>
      if editLength > maxEditLength {
        JsError.throwWithMessage("the edit graph has no path")
      } else {
        switch diagonals(Math.Int.max(minDiagonalToConsider.contents, -editLength), editLength) {
        | Some(history) => history
        | None => rounds(editLength + 1)
        }
      }
    rounds(1)
  }
}

let buildValues = (lastComponent, newTokens, oldTokens, policy) => {
  let components = []
  let rec collect = component =>
    switch component {
    | Some(component) =>
      components->Array.push(component)
      collect(component.previous)
    | None => ()
    }
  collect(lastComponent)
  components->Array.reverse
  let newPos = ref(0)
  let oldPos = ref(0)
  components->Array.map(component => {
    let tokens = if !component.removed {
      let kept = newTokens->Array.slice(~start=newPos.contents, ~end=newPos.contents + component.count)
      let value = switch policy.longest {
      | Some(longest) if !component.added =>
        kept->Array.mapWithIndex((token, i) => longest(oldTokens->Array.getUnsafe(oldPos.contents + i), token))
      | _ => kept
      }
      newPos := newPos.contents + component.count
      if !component.added {
        oldPos := oldPos.contents + component.count
      }
      value
    } else {
      let removed = oldTokens->Array.slice(~start=oldPos.contents, ~end=oldPos.contents + component.count)
      oldPos := oldPos.contents + component.count
      removed
    }
    {count: component.count, added: component.added, removed: component.removed, value: policy.join(tokens)}
  })
}

/** base.js `diff` over tokens that are already cast, tokenized and stripped of empty ones. */
let diff = (oldTokens, newTokens, policy, options) =>
  buildValues(search(oldTokens, newTokens, policy.equals, options), newTokens, oldTokens, policy)

/** removeEmpty for string tokens. */
let nonEmpty = tokens => tokens->Array.filter(token => token != "")

let textEquals = (options, left, right) =>
  left == right || (options.ignoreCase && left->String.toLowerCase == right->String.toLowerCase)

let concat = tokens => tokens->Array.join("")

/** character.js: tokens are code points (`Array.from`), so a lone surrogate is one token. */
let chars = (before, after, options) =>
  diff(
    Array.fromString(before)->nonEmpty,
    Array.fromString(after)->nonEmpty,
    {equals: textEquals(options, ...), join: concat, longest: None},
    options,
  )

/** line.js `tokenize`: lines with their terminators, or the terminators as separate tokens. */
let lineTokens = (value, options) => {
  let text = options.stripTrailingCr ? value->String.replaceAll("\r\n", "\n") : value
  // text.split(/(\n|\r\n)/): each \n is a separator, together with a \r before it.
  let linesAndNewlines = []
  let start = ref(0)
  for i in 0 to text->String.length - 1 {
    if text->String.charCodeAtUnsafe(i) == 10 {
      let separator = i > start.contents && text->String.charCodeAtUnsafe(i - 1) == 13 ? i - 1 : i
      linesAndNewlines->Array.push(text->String.slice(~start=start.contents, ~end=separator))
      linesAndNewlines->Array.push(text->String.slice(~start=separator, ~end=i + 1))
      start := i + 1
    }
  }
  linesAndNewlines->Array.push(text->String.slice(~start=start.contents))
  // Ignore the final empty token that occurs if the string ends with a new line.
  if linesAndNewlines->Array.at(-1) == Some("") {
    linesAndNewlines->Array.pop->ignore
  }
  let retLines = []
  linesAndNewlines->Array.forEachWithIndex((line, i) =>
    if mod(i, 2) == 1 && !options.newlineIsToken {
      let last = retLines->Array.length - 1
      retLines->Array.setUnsafe(last, retLines->Array.getUnsafe(last) ++ line)
    } else {
      retLines->Array.push(line)
    }
  )
  retLines
}

let lineEquals = (options, left, right) => {
  // With newlineIsToken, a newline token is not trimmed down to an empty line.
  let normalised = token =>
    if options.ignoreWhitespace {
      !options.newlineIsToken || !(token->String.includes("\n")) ? token->String.trim : token
    } else if options.ignoreNewlineAtEof && !options.newlineIsToken && token->String.endsWith("\n") {
      token->String.slice(~start=0, ~end=-1)
    } else {
      token
    }
  textEquals(options, normalised(left), normalised(right))
}

let lines = (before, after, options) =>
  diff(
    lineTokens(before, options)->nonEmpty,
    lineTokens(after, options)->nonEmpty,
    {equals: lineEquals(options, ...), join: concat, longest: None},
    options,
  )

/** `canonicalize` for JSON documents: arrays in order, object keys sorted and assigned to a plain
 object, so a "__proto__" key sets the prototype and is not serialized. An object with a truthy
 `toJSON` member has it called, which throws, since JSON cannot make it callable. JSON input has no
 cycles, and the default replacer keeps every value. As in npm, each level recurses from a `for`
 loop, so a nesting level costs one stack frame. */
let rec canonicalize = (value: JSON.t): JSON.t =>
  switch value {
  | Array(items) =>
    let canonical = []
    for i in 0 to items->Array.length - 1 {
      canonical->Array.push(canonicalize(items->Array.getUnsafe(i)))
    }
    Array(canonical)
  | Object(object) =>
    if object->Dict.get("toJSON")->Option.mapOr(false, Host.truthy) {
      JsError.TypeError.throwWithMessage("obj.toJSON is not a function")
    }
    let canonical = Dict.make()
    let keys = object->Dict.keysToArray->Array.toSorted(String.compare)
    for i in 0 to keys->Array.length - 1 {
      let key = keys->Array.getUnsafe(i)
      canonical->Dict.set(key, canonicalize(object->Dict.getUnsafe(key)))
    }
    Object(canonical)
  | other => other
  }

/** json.js `castInput`: a string as it is, any other document pretty-printed canonically. */
let jsonText = (value: JSON.t) =>
  switch value {
  | String(text) => text
  | document => document->canonicalize->JSON.stringify(~space=2)
  }

// replace starts a global RegExp from lastIndex 0.
let danglingComma = /,([\r\n])/g

let json = (before, after, options) => {
  let withoutDanglingComma = token => token->String.replaceRegExp(danglingComma, "$1")
  diff(
    lineTokens(jsonText(before), options)->nonEmpty,
    lineTokens(jsonText(after), options)->nonEmpty,
    {
      equals: (left, right) => textEquals(options, withoutDanglingComma(left), withoutDanglingComma(right)),
      join: concat,
      longest: Some((oldToken, newToken) => oldToken->String.length > newToken->String.length ? oldToken : newToken),
    },
    options,
  )
}

/** array.js: tokens are the JSON values themselves, compared with `===`, and with ignoreCase also by
 `toLowerCase()`, which the contract applies to strings only. */
let arrays = (before: array<JSON.t>, after: array<JSON.t>, options) => {
  let lower = (value: JSON.t) => (Host.fromJson(value): string)->String.toLowerCase
  diff(
    before,
    after,
    {
      equals: (left, right) => left === right || (options.ignoreCase && lower(left) == lower(right)),
      join: tokens => tokens,
      longest: None,
    },
    options,
  )
}
