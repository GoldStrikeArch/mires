// jmespath's value helpers: strictDeepEqual, isFalse and objValues.

open Host

// The part of `Object.prototype.toString` that strictDeepEqual distinguishes.
let typeTag = v =>
  if isNull(v) {
    "null"
  } else if isArray(v) {
    "array"
  } else {
    switch Type.typeof(v) {
    | #string => "string"
    | #number => "number"
    | #boolean => "boolean"
    | #undefined => "undefined"
    | _ => "object"
    }
  }

// jmespath's `strictDeepEqual`: `===`, then the same `Object.prototype.toString` tag, then by
// content. Objects compare every own key of `first` against `second[key]` (so an undefined value
// equals a missing key), then require `second`'s own keys to be among them. The loops are `while`
// loops: an Array.every callback adds frames to each level of nesting, which costs the nesting
// bound in Firefox.
let rec strictDeepEqual = (first, second) =>
  if first === second {
    true
  } else {
    let firstType = typeTag(first)
    if firstType != typeTag(second) {
      false
    } else if firstType == "array" {
      let a = asArray(first)
      let b = asArray(second)
      let equal = ref(Array.length(a) == Array.length(b))
      let i = ref(0)
      while equal.contents && i.contents < Array.length(a) {
        equal := strictDeepEqual(a->Array.getUnsafe(i.contents), b->Array.getUnsafe(i.contents))
        i := i.contents + 1
      }
      equal.contents
    } else if firstType == "object" {
      let keysSeen = Set.make()
      let firstKeys = keys(first)
      let equal = ref(true)
      let i = ref(0)
      while equal.contents && i.contents < Array.length(firstKeys) {
        let key = firstKeys->Array.getUnsafe(i.contents)
        equal := strictDeepEqual(property(first, key), property(second, key))
        keysSeen->Set.add(key)
        i := i.contents + 1
      }
      equal.contents && keys(second)->Array.every(key => keysSeen->Set.has(key))
    } else {
      false
    }
  }

// jmespath's `isFalse`: "", false, null, an empty array and an object with no own keys.
let isFalse = v =>
  if v === fromString("") || v === fromBool(false) || isNull(v) {
    true
  } else if isArray(v) {
    Array.length(asArray(v)) == 0
  } else if isObject(v) {
    Array.length(keys(v)) == 0
  } else {
    false
  }

let objValues = obj => keys(obj)->Array.map(key => property(obj, key))
