// jmespath's Runtime: the function table, argument validation and the 26 built-in functions.

open Host

// jmespath's TYPE_* constants.
type jmesType =
  | TypeNumber
  | TypeAny
  | TypeString
  | TypeArray
  | TypeObject
  | TypeBoolean
  | TypeExpref
  | TypeNull
  | TypeArrayNumber
  | TypeArrayString

// jmespath's `_getTypeName`; None where its switch falls through (undefined).
let typeName = v =>
  switch Type.typeof(v) {
  | #string => Some(TypeString)
  | #number => Some(TypeNumber)
  | #boolean => Some(TypeBoolean)
  | #object =>
    if isNull(v) {
      Some(TypeNull)
    } else if isArray(v) {
      Some(TypeArray)
    } else if asNode(v).jmespathType === fromString("Expref") {
      Some(TypeExpref)
    } else {
      Some(TypeObject)
    }
  | _ => None
  }

type parameter = {types: array<jmesType>, variadic: bool}
type visit = (option<node>, value) => value
type function = {signature: array<parameter>, body: (array<value>, visit) => value}

let param = types => {types, variadic: false}
let arg = (args, i) => args->Array.getUnsafe(i)

// `for (key in value)` over a value jmespath's merge did not type-check: an object's or array's own
// keys, a string's indices, and nothing for other primitives, null and undefined.
let forIn = (v, f) =>
  if isString(v) {
    let text = asString(v)
    for i in 0 to String.length(text) - 1 {
      f(Int.toString(i), fromString(text->String.getUnsafe(i)))
    }
  } else if Type.typeof(v) == #object && !isNull(v) {
    keys(v)->Array.forEach(key => f(key, property(v, key)))
  }

let createKeyFunction = (visit, exprefNode, allowedTypes) => x => {
  let current = visit(exprefNode, x)
  switch typeName(current) {
  | Some(t) if allowedTypes->Array.includes(t) => current
  | _ => throw(Errors.typeError("expected one of number,string"))
  }
}

// max_by and min_by: the first element whose key beats the running one, starting from ∓∞.
let extremeBy = (args, visit, better, initial) => {
  let keyFunction = createKeyFunction(visit, asOptionNode(arg(args, 1)), [TypeNumber, TypeString])
  let best = ref(fromFloat(initial))
  let record = ref(jsUndefined)
  asArray(arg(args, 0))->Array.forEach(element => {
    let current = keyFunction(element)
    if better(current, best.contents) {
      best := current
      record := element
    }
  })
  record.contents
}

// sort_by: decorate with the index, sort with the engine's `Array.prototype.sort`, and check each
// key's type inside the comparator, as jmespath does. With NaN keys the comparator is
// inconsistent, and the engine's algorithm decides the order.
let sortBy = (array, exprefNode, visit) => {
  let sortedArray = Array.copy(array)
  if Array.length(sortedArray) == 0 {
    fromArray(sortedArray)
  } else {
    let requiredType = typeName(visit(exprefNode, sortedArray->Array.getUnsafe(0)))
    if requiredType != Some(TypeNumber) && requiredType != Some(TypeString) {
      throw(Errors.typeError("sort_by"))
    }
    let decorated = Array.fromInitializer(~length=Array.length(sortedArray), i => i)
    decorated->Array.sort((a, b) => {
      let exprA = visit(exprefNode, sortedArray->Array.getUnsafe(a))
      let exprB = visit(exprefNode, sortedArray->Array.getUnsafe(b))
      if typeName(exprA) != requiredType {
        throw(Errors.typeError("expected the first key's type"))
      } else if typeName(exprB) != requiredType {
        throw(Errors.typeError("expected the first key's type"))
      }
      if greater(exprA, exprB) {
        1.0
      } else if less(exprA, exprB) {
        -1.0
      } else {
        Int.toFloat(a - b)
      }
    })
    fromArray(decorated->Array.map(i => sortedArray->Array.getUnsafe(i)))
  }
}

let functionTable: Map.t<string, function> = Map.fromArray([
  ("abs", {signature: [param([TypeNumber])], body: (args, _) => fromFloat(Math.abs(asFloat(arg(args, 0))))}),
  (
    "avg",
    {
      signature: [param([TypeArrayNumber])],
      body: (args, _) => {
        let inputArray = asArray(arg(args, 0))
        let sum = ref(0.0)
        inputArray->Array.forEach(x => sum := sum.contents +. asFloat(x))
        fromFloat(sum.contents /. Int.toFloat(Array.length(inputArray)))
      },
    },
  ),
  ("ceil", {signature: [param([TypeNumber])], body: (args, _) => fromFloat(Math.ceil(asFloat(arg(args, 0))))}),
  (
    "contains",
    {
      signature: [param([TypeString, TypeArray]), param([TypeAny])],
      // `indexOf`: String.prototype.indexOf converts its argument with String(); the array version
      // compares with `===`, so only the same object is found.
      body: (args, _) => {
        let subject = arg(args, 0)
        let search = arg(args, 1)
        fromBool(
          isString(subject)
            ? asString(subject)->String.indexOf(toString(search)) >= 0
            : asArray(subject)->Array.some(element => element === search),
        )
      },
    },
  ),
  (
    "ends_with",
    {
      signature: [param([TypeString]), param([TypeString])],
      body: (args, _) => {
        let searchStr = asString(arg(args, 0))
        let suffix = asString(arg(args, 1))
        fromBool(searchStr->String.indexOfFrom(suffix, String.length(searchStr) - String.length(suffix)) != -1)
      },
    },
  ),
  ("floor", {signature: [param([TypeNumber])], body: (args, _) => fromFloat(Math.floor(asFloat(arg(args, 0))))}),
  (
    "length",
    {
      signature: [param([TypeString, TypeArray, TypeObject])],
      body: (args, _) => {
        let subject = arg(args, 0)
        let length = if isString(subject) {
          String.length(asString(subject))
        } else if isArray(subject) {
          Array.length(asArray(subject))
        } else {
          Array.length(keys(subject))
        }
        fromFloat(Int.toFloat(length))
      },
    },
  ),
  (
    "map",
    {
      signature: [param([TypeExpref]), param([TypeArray])],
      body: (args, visit) =>
        fromArray(asArray(arg(args, 1))->Array.map(element => visit(asOptionNode(arg(args, 0)), element))),
    },
  ),
  (
    "max",
    {
      signature: [param([TypeArrayNumber, TypeArrayString])],
      body: (args, _) => {
        let elements = asArray(arg(args, 0))
        if Array.length(elements) == 0 {
          jsNull
        } else if typeName(elements->Array.getUnsafe(0)) == Some(TypeNumber) {
          // Math.max.apply: NaN wins, and +0 is larger than -0.
          fromFloat(elements->Array.reduce(Float.Constants.negativeInfinity, (acc, x) => Math.max(acc, asFloat(x))))
        } else {
          elements->Array.reduce(elements->Array.getUnsafe(0), (maxElement, x) =>
            String.localeCompare(asString(maxElement), asString(x)) < 0.0 ? x : maxElement
          )
        }
      },
    },
  ),
  (
    "merge",
    {
      signature: [{types: [TypeObject], variadic: true}],
      // `for (key in current)`: only the first argument is type-checked, so later ones may be
      // strings or arrays (their index keys) or primitives and null (no keys).
      body: (args, _) => {
        let merged = Dict.make()
        args->Array.forEach(current => forIn(current, (key, v) => merged->Dict.set(key, v)))
        fromDict(merged)
      },
    },
  ),
  (
    "max_by",
    {
      signature: [param([TypeArray]), param([TypeExpref])],
      body: (args, visit) => extremeBy(args, visit, greater, Float.Constants.negativeInfinity),
    },
  ),
  (
    "sum",
    {
      signature: [param([TypeArrayNumber])],
      body: (args, _) => fromFloat(asArray(arg(args, 0))->Array.reduce(0.0, (sum, x) => sum +. asFloat(x))),
    },
  ),
  (
    "starts_with",
    {
      signature: [param([TypeString]), param([TypeString])],
      // `lastIndexOf(prefix) === 0`, searching from the end: `starts_with('abab', 'ab')` is false.
      body: (args, _) => {
        let subject = asString(arg(args, 0))
        fromBool(subject->String.lastIndexOfFrom(asString(arg(args, 1)), String.length(subject)) == 0)
      },
    },
  ),
  (
    "min",
    {
      signature: [param([TypeArrayNumber, TypeArrayString])],
      body: (args, _) => {
        let elements = asArray(arg(args, 0))
        if Array.length(elements) == 0 {
          jsNull
        } else if typeName(elements->Array.getUnsafe(0)) == Some(TypeNumber) {
          fromFloat(elements->Array.reduce(Float.Constants.positiveInfinity, (acc, x) => Math.min(acc, asFloat(x))))
        } else {
          elements->Array.reduce(elements->Array.getUnsafe(0), (minElement, x) =>
            String.localeCompare(asString(x), asString(minElement)) < 0.0 ? x : minElement
          )
        }
      },
    },
  ),
  (
    "min_by",
    {
      signature: [param([TypeArray]), param([TypeExpref])],
      body: (args, visit) => extremeBy(args, visit, less, Float.Constants.positiveInfinity),
    },
  ),
  (
    "type",
    {
      signature: [param([TypeAny])],
      body: (args, _) =>
        switch typeName(arg(args, 0)) {
        | Some(TypeNumber) => fromString("number")
        | Some(TypeString) => fromString("string")
        | Some(TypeArray) => fromString("array")
        | Some(TypeObject) => fromString("object")
        | Some(TypeBoolean) => fromString("boolean")
        | Some(TypeExpref) => fromString("expref")
        | Some(TypeNull) => fromString("null")
        | _ => jsUndefined
        },
    },
  ),
  ("keys", {signature: [param([TypeObject])], body: (args, _) => fromStrings(keys(arg(args, 0)))}),
  ("values", {signature: [param([TypeObject])], body: (args, _) => fromArray(Values.objValues(arg(args, 0)))}),
  (
    "sort",
    {
      signature: [param([TypeArrayString, TypeArrayNumber])],
      // The engine's default sort: elements compared as strings, numbers too.
      body: (args, _) => {
        let sortedArray = Array.copy(asArray(arg(args, 0)))
        sortedArray->sortDefault
        fromArray(sortedArray)
      },
    },
  ),
  (
    "sort_by",
    {
      signature: [param([TypeArray]), param([TypeExpref])],
      body: (args, visit) => sortBy(asArray(arg(args, 0)), asOptionNode(arg(args, 1)), visit),
    },
  ),
  (
    "join",
    {
      signature: [param([TypeString]), param([TypeArrayString])],
      body: (args, _) => fromString(asStrings(arg(args, 1))->Array.join(asString(arg(args, 0)))),
    },
  ),
  (
    "reverse",
    {
      signature: [param([TypeString, TypeArray])],
      body: (args, _) => {
        let subject = arg(args, 0)
        if isString(subject) {
          // Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
          let text = asString(subject)
          let reversed = ref("")
          for i in String.length(text) - 1 downto 0 {
            reversed := reversed.contents ++ text->String.getUnsafe(i)
          }
          fromString(reversed.contents)
        } else {
          let reversed = Array.copy(asArray(subject))
          reversed->Array.reverse
          fromArray(reversed)
        }
      },
    },
  ),
  (
    "to_array",
    {
      signature: [param([TypeAny])],
      body: (args, _) => typeName(arg(args, 0)) == Some(TypeArray) ? arg(args, 0) : fromArray([arg(args, 0)]),
    },
  ),
  (
    "to_string",
    {
      signature: [param([TypeAny])],
      body: (args, _) => typeName(arg(args, 0)) == Some(TypeString) ? arg(args, 0) : stringify(arg(args, 0)),
    },
  ),
  (
    "to_number",
    {
      signature: [param([TypeAny])],
      body: (args, _) =>
        switch typeName(arg(args, 0)) {
        | Some(TypeNumber) => arg(args, 0)
        | Some(TypeString) =>
          let convertedValue = toNumber(arg(args, 0))
          Float.isNaN(convertedValue) ? jsNull : fromFloat(convertedValue)
        | _ => jsNull
        },
    },
  ),
  (
    "not_null",
    {
      signature: [{types: [TypeAny], variadic: true}],
      // Undefined is not null here: `not_null(max_by(`[]`, &a))` is undefined.
      body: (args, _) =>
        switch args->Array.findIndex(v => typeName(v) != Some(TypeNull)) {
        | -1 => jsNull
        | i => arg(args, i)
        },
    },
  ),
])

let rec typeMatches = (actual, expected, argValue) =>
  switch expected {
  | TypeAny => true
  | TypeArray => actual == Some(TypeArray)
  | TypeArrayNumber | TypeArrayString =>
    let subtype = expected == TypeArrayNumber ? TypeNumber : TypeString
    actual == Some(TypeArray) &&
      asArray(argValue)->Array.every(element => typeMatches(typeName(element), subtype, element))
  | _ => actual == Some(expected)
  }

let validateArgs = (name, args, signature) => {
  let arity = Array.length(signature)
  let variadic = (signature->Array.getUnsafe(arity - 1)).variadic
  let received = Array.length(args)
  if variadic ? received < arity : received != arity {
    throw(Errors.argumentError(name ++ "() takes " ++ Int.toString(arity) ++ " arguments"))
  }
  signature->Array.forEachWithIndex((parameter, i) => {
    let argValue = arg(args, i)
    let actualType = typeName(argValue)
    if !(parameter.types->Array.some(expected => typeMatches(actualType, expected, argValue))) {
      throw(Errors.typeError(name ++ "() expected argument " ++ Int.toString(i + 1)))
    }
  })
}

let callFunction = (name, resolvedArgs, ~visit) =>
  switch stringValue(name)->Option.flatMap(name => functionTable->Map.get(name)->Option.map(entry => (name, entry))) {
  | None => throw(Errors.unknownFunction(toString(name)))
  | Some((name, functionEntry)) =>
    validateArgs(name, resolvedArgs, functionEntry.signature)
    functionEntry.body(resolvedArgs, visit)
  }
