// jmespath's TreeInterpreter.

open Host
open Values

let capSliceRange = (arrayLength, actualValue, step) =>
  if actualValue < 0.0 {
    let value = actualValue +. arrayLength
    value < 0.0 ? step < 0.0 ? -1.0 : 0.0 : value
  } else if actualValue >= arrayLength {
    step < 0.0 ? arrayLength -. 1.0 : arrayLength
  } else {
    actualValue
  }

// `Slice` with jmespath's computeSliceParams, in JS numbers: the tokens come from parseInt, so a
// bound may be NaN (`[0:3:-]`) or past 2^53, and the loop reads `value[i]` as it goes.
let slice = (array, params) => {
  let param = i => {
    let p = params->Array.getUnsafe(i)
    isNull(p) ? None : Some(asFloat(p))
  }
  let length = Int.toFloat(Array.length(array))
  let step = switch param(2) {
  | Some(0.0) => throw(Errors.runtimeError("Invalid slice, step cannot be 0"))
  | Some(step) => step
  | None => 1.0
  }
  let stepValueNegative = step < 0.0
  let start = switch param(0) {
  | Some(start) => capSliceRange(length, start, step)
  | None => stepValueNegative ? length -. 1.0 : 0.0
  }
  let stop = switch param(1) {
  | Some(stop) => capSliceRange(length, stop, step)
  | None => stepValueNegative ? -1.0 : length
  }
  let result = []
  let i = ref(start)
  if step > 0.0 {
    while i.contents < stop {
      result->Array.push(elementAt(array, i.contents))
      i := i.contents +. step
    }
  } else {
    while i.contents > stop {
      result->Array.push(elementAt(array, i.contents))
      i := i.contents +. step
    }
  }
  result
}

let rec visit = (maybeNode: option<node>, value: value): value =>
  switch maybeNode {
  // `node.type` of a missing node is a TypeError in jmespath.
  | None => throw(Errors.syntaxError("Cannot read properties of undefined"))
  | Some(node) =>
    let child = i => node.children->Array.getUnsafe(i)
    switch stringValue(node.nodeType) {
    | Some("Field") =>
      if isObject(value) {
        let field = property(value, asString(node.name))
        isUndefined(field) ? jsNull : field
      } else {
        jsNull
      }
    | Some("Subexpression" | "IndexExpression" | "Pipe") => visit(child(1), visit(child(0), value))
    | Some("Index") =>
      if isArray(value) {
        let array = asArray(value)
        let index = asFloat(node.nodeValue)
        let result = elementAt(array, index < 0.0 ? index +. Int.toFloat(Array.length(array)) : index)
        isUndefined(result) ? jsNull : result
      } else {
        jsNull
      }
    | Some("Slice") => isArray(value) ? fromArray(slice(asArray(value), sliceParams(node.children))) : jsNull
    | Some("Projection") =>
      let base = visit(child(0), value)
      isArray(base) ? collect(asArray(base), child(1)) : jsNull
    | Some("ValueProjection") =>
      let base = visit(child(0), value)
      isObject(base) ? collect(objValues(base), child(1)) : jsNull
    | Some("FilterProjection") =>
      let base = visit(child(0), value)
      if isArray(base) {
        // Every condition first, then the projection: the first error is jmespath's.
        let items = asArray(base)
        let filtered = []
        for i in 0 to Array.length(items) - 1 {
          let element = items->Array.getUnsafe(i)
          if !isFalse(visit(child(2), element)) {
            filtered->Array.push(element)
          }
        }
        collect(filtered, child(1))
      } else {
        jsNull
      }
    | Some("Comparator") =>
      let first = visit(child(0), value)
      let second = visit(child(1), value)
      fromBool(
        switch stringValue(node.name) {
        | Some("EQ") => strictDeepEqual(first, second)
        | Some("NE") => !strictDeepEqual(first, second)
        | Some("GT") => greater(first, second)
        | Some("GTE") => greaterOrEqual(first, second)
        | Some("LT") => less(first, second)
        | Some("LTE") => lessOrEqual(first, second)
        | _ => throw(Errors.syntaxError("Unknown comparator"))
        },
      )
    | Some("Flatten") =>
      let original = visit(child(0), value)
      if isArray(original) {
        let items = asArray(original)
        let merged = []
        for i in 0 to Array.length(items) - 1 {
          let current = items->Array.getUnsafe(i)
          if isArray(current) {
            let inner = asArray(current)
            for j in 0 to Array.length(inner) - 1 {
              merged->Array.push(inner->Array.getUnsafe(j))
            }
          } else {
            merged->Array.push(current)
          }
        }
        fromArray(merged)
      } else {
        jsNull
      }
    | Some("Identity" | "Current") => value
    | Some("MultiSelectList") =>
      isNull(value) ? jsNull : fromArray(visitEach(node.children, value))
    | Some("MultiSelectHash") =>
      if isNull(value) {
        jsNull
      } else {
        let collected = Dict.make()
        for i in 0 to Array.length(node.children) - 1 {
          switch node.children->Array.getUnsafe(i) {
          | Some(pair) => collected->Dict.set(asString(pair.name), visit(asOptionNode(pair.nodeValue), value))
          | None => throw(Errors.syntaxError("Cannot read properties of undefined"))
          }
        }
        fromDict(collected)
      }
    | Some("OrExpression") =>
      let matched = visit(child(0), value)
      isFalse(matched) ? visit(child(1), value) : matched
    | Some("AndExpression") =>
      let first = visit(child(0), value)
      isFalse(first) ? first : visit(child(1), value)
    | Some("NotExpression") => fromBool(isFalse(visit(child(0), value)))
    | Some("Literal") => node.nodeValue
    | Some("Function") =>
      Runtime.callFunction(node.name, visitEach(node.children, value), ~visit)
    | Some("ExpressionReference") =>
      // jmespath tags the referenced node in place and returns the node itself.
      switch child(0) {
      | None => throw(Errors.syntaxError("Cannot set properties of undefined"))
      | Some(refNode) =>
        refNode.jmespathType = fromString("Expref")
        fromNode(Some(refNode))
      }
    | _ => throw(Errors.syntaxError("Unknown node type"))
    }
  }

// Projections keep every result except null; undefined (an empty `max_by`) is kept. The loops on
// the interpreter's recursive paths are `for` loops: an Array.forEach or Array.map callback adds
// frames to each level, which costs the nesting bound in Firefox.
and collect = (base, right) => {
  let collected = []
  for i in 0 to Array.length(base) - 1 {
    let current = visit(right, base->Array.getUnsafe(i))
    if !isNull(current) {
      collected->Array.push(current)
    }
  }
  fromArray(collected)
}

and visitEach = (nodes, value) => {
  let results = []
  for i in 0 to Array.length(nodes) - 1 {
    results->Array.push(visit(nodes->Array.getUnsafe(i), value))
  }
  results
}
