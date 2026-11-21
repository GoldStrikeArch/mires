// The JavaScript values jmespath works on, and the host functions it calls.
//
// jmespath 0.16.0 evaluates over plain JS values (the JSON document, objects it builds, and its own
// AST nodes, which `&expr` hands to functions and to the caller) and leans on JS semantics for them:
// `Object.keys` order, `===` identity in `contains`, relational operators with type coercion,
// `JSON.parse`/`JSON.stringify`, `parseInt`, `Number(string)`, `localeCompare` and the default
// `Array.prototype.sort`. The port keeps those values as they are, as the abstract type `value`,
// and binds those functions, so every one of those semantics is the engine's own.

type value

// An AST node as jmespath builds it: a plain object whose properties depend on its type. The parser
// builds each shape as an object literal; this record is the view the interpreter reads it through.
// `None` children are jmespath's `undefined` nodes (`a.`, `()`).
type rec node = {
  @as("type") nodeType: value,
  name: value,
  @as("value") nodeValue: value,
  children: array<option<node>>,
  mutable jmespathType: value,
}

external fromString: string => value = "%identity"
external fromFloat: float => value = "%identity"
external fromBool: bool => value = "%identity"
external fromArray: array<value> => value = "%identity"
external fromDict: dict<value> => value = "%identity"
external fromNode: option<node> => value = "%identity"
external fromStrings: array<string> => value = "%identity"
external nodeOf: {..} => node = "%identity"

external asString: value => string = "%identity"
external asFloat: value => float = "%identity"
external asArray: value => array<value> = "%identity"
external asDict: value => dict<value> = "%identity"
external asNode: value => node = "%identity"
external asOptionNode: value => option<node> = "%identity"
external asStrings: value => array<string> = "%identity"

// A Slice node's children are its numbers and nulls.
external sliceParams: array<option<node>> => array<value> = "%identity"

@val external jsUndefined: value = "undefined"
external fromNull: Null.t<value> => value = "%identity"
let jsNull = fromNull(Null.null)

let isUndefined = (v: value) => v === jsUndefined
let isNull = (v: value) => v === jsNull
let isString = (v: value) => Type.typeof(v) == #string
let isNumber = (v: value) => Type.typeof(v) == #number
@val external isArray: value => bool = "Array.isArray"

// jmespath's `isObject`: `Object.prototype.toString` is `[object Object]`, which for these values
// means a non-null, non-array object (JSON objects, built objects and AST nodes alike).
let isObject = (v: value) => Type.typeof(v) == #object && !isNull(v) && !isArray(v)

let stringValue = (v: value) => isString(v) ? Some(asString(v)) : None

@val external keys: value => array<string> = "Object.keys"

// `object[key]`, through the prototype chain as in JS.
let property = (object: value, key: string) => asDict(object)->Dict.getUnsafe(key)

// `array[index]` for a JS number index: an element, or undefined off the end, at NaN and at ±∞.
let elementAt = (array: array<value>, index: float) =>
  index >= 0.0 && index < Int.toFloat(Array.length(array)) && index == Math.floor(index)
    ? array->Array.getUnsafe(Float.toInt(index))
    : jsUndefined

@val external parseJson: string => value = "JSON.parse"
@val external stringify: value => value = "JSON.stringify"

// Stack exhaustion, which the reference adapter lets propagate rather than file as "syntax": the
// engine's RangeError "Maximum call stack size exceeded" (V8, JavaScriptCore) or InternalError
// "too much recursion" (SpiderMonkey).
let stackExhausted = (error: JsExn.t) =>
  switch (JsExn.name(error), JsExn.message(error)) {
  | (Some("RangeError"), Some(message)) => message->String.includes("call stack")
  | (Some("InternalError"), Some(message)) => message->String.includes("too much recursion")
  | _ => false
  }
@val external parseInt: string => float = "parseInt"

// `Number(value)`: the unary `+` that `to_number` applies to strings.
@val external toNumber: value => float = "Number"

// `String(value)`: the conversion `String.prototype.indexOf` applies to its argument.
@val external toString: value => string = "String"

// The default `Array.prototype.sort`, in place: elements compared as strings, numbers too.
@send external sortDefault: array<value> => unit = "sort"

// jmespath's relational operators (`first > second`) coerce their operands themselves: arrays and
// objects through their string forms, then strings by code unit or everything else as numbers.
// ReScript compiles float comparisons to those very operators, so seen as floats the values compare
// exactly as in jmespath.
external operand: value => float = "%identity"
let greater = (a, b) => operand(a) > operand(b)
let greaterOrEqual = (a, b) => operand(a) >= operand(b)
let less = (a, b) => operand(a) < operand(b)
let lessOrEqual = (a, b) => operand(a) <= operand(b)
