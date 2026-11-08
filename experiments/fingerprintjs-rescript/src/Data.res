open Bindings

let includes = (haystack: array<'a>, needle: 'b) => {
  let length = arrayLength(haystack)
  let rec loop = i =>
    if i >= length {
      false
    } else if strictEq(getInt(haystack, i), needle) {
      true
    } else {
      loop(i + 1)
    }
  loop(0)
}

let excludes = (haystack, needle) => !includes(haystack, needle)

let toInt = (value: 'a) => parseInt(value)

let toFloat = (value: 'a) => parseFloat(value)

let replaceNaN = (value: 'a, replacement: 'b) =>
  if isNumber(value) && isNaN(value) {
    identity(replacement)
  } else {
    identity(value)
  }

let countTruthy = (values: array<'a>) => {
  let length = arrayLength(values)
  let rec loop = (i, sum) =>
    if i >= length {
      sum
    } else {
      loop(
        i + 1,
        sum + if isTruthy(getInt(values, i)) {
          1
        } else {
          0
        },
      )
    }
  loop(0, 0)
}

let round = (value: float, ~base: float=1.) =>
  if mathAbs(base) >= 1. {
    mathRound(value /. base) *. base
  } else {
    let counterBase = 1. /. base
    mathRound(value *. counterBase) /. counterBase
  }

let parseSimpleCssSelector = (selector: string) => {
  let errorMessage = "Unexpected syntax '" ++ selector ++ "'"
  let tagRe = newRegExp("^\\s*([a-z-]*)(.*)$", "i")
  let tagMatch = execRe(tagRe, selector)
  let tagRaw = asString(getInt(tagMatch, 1))
  let tag = if tagRaw == "" {
    jsUndefined
  } else {
    identity(tagRaw)
  }
  let attributes = emptyObject()
  let partsRegex = newRegExp("([.:#][\\w-]+|\\[.+?\\])", "gi")
  let addAttribute = (name, value) => {
    let current = get(attributes, name)
    if isUndefined(current) {
      set(attributes, name, [value])
    } else {
      arrayPush(asArray(current), value)->ignore
    }
  }
  let rec loop = () => {
    let match = execRe(partsRegex, asString(getInt(tagMatch, 2)))
    if isNullish(match) || isFalsy(match) {
      ()
    } else {
      let part = asString(getInt(match, 0))
      switch charAt(part, 0) {
      | "." => addAttribute("class", sliceString(part, 1, 2147483647))
      | "#" => addAttribute("id", sliceString(part, 1, 2147483647))
      | "[" => {
          let attributeMatch = execRe(
            newRegExp("^\\[([\\w-]+)([~|^$*]?=(\"(.*?)\"|([\\w-]+)))?(\\s+[is])?\\]$", ""),
            part,
          )
          if isTruthy(attributeMatch) {
            let value = coalesce(getInt(attributeMatch, 4), coalesce(getInt(attributeMatch, 5), ""))
            addAttribute(asString(getInt(attributeMatch, 1)), asString(value))
          } else {
            throwJs(newError(errorMessage))
          }
        }
      | _ => throwJs(newError(errorMessage))
      }
      loop()
    }
  }
  loop()
  (tag, attributes)
}

let areSetsEqual = (set1: unknown, set2: unknown) => {
  if strictEq(set1, set2) {
    true
  } else if !strictEq(get(set1, "size"), get(set2, "size")) {
    false
  } else {
    let iter = call0(set1, "values")
    let rec loop = () => {
      let step = call0(iter, "next")
      if isTruthy(get(step, "done")) {
        true
      } else if isFalsy(call1(set2, "has", get(step, "value"))) {
        false
      } else {
        loop()
      }
    }
    loop()
  }
}

let maxInIterator = (iterator: unknown, getItemScore: 'a => float) => {
  let maxItem = ref(jsUndefined)
  let maxItemScore = ref(jsUndefined)
  let rec loop = () => {
    let step = call0(iterator, "next")
    if isTruthy(get(step, "done")) {
      maxItem.contents
    } else {
      let item = get(step, "value")
      let score = getItemScore(item)
      if isUndefined(maxItemScore.contents) || score > asFloat(maxItemScore.contents) {
        maxItem := item
        maxItemScore := identity(score)
      }
      loop()
    }
  }
  loop()
}

let getUTF8Bytes = (input: string) => {
  let length = String.length(input)
  let result = newUint8Array(length)
  let rec loop = i =>
    if i >= length {
      result
    } else {
      let charCode = stringCharCodeAt(input, i)
      if charCode > 127 {
        encodeText(newTextEncoder(), input)
      } else {
        jsSetIndex(result, i, charCode)
        loop(i + 1)
      }
    }
  loop(0)
}
