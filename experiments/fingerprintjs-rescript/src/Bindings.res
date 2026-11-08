/* Host interop. Browser globals are read only inside functions so imports stay SSR-safe. */

type any

external asAny: 'a => any = "%identity"
external asString: 'a => string = "%identity"
external asInt: 'a => int = "%identity"
external asFloat: 'a => float = "%identity"
external asBool: 'a => bool = "%identity"
external asArray: 'a => array<'b> = "%identity"
external asDict: 'a => Dict.t<'a> = "%identity"
external fromAny: any => 'a = "%identity"
external identity: 'a => 'b = "%identity"

@val external jsUndefined: any = "undefined"
@val external jsNull: any = "null"
@val external jsNaN: float = "NaN"
@val external globalThis: any = "globalThis"

@val external dateNow: unit => float = "Date.now"
@val external parseInt: 'a => int = "parseInt"
@val external parseFloat: 'a => float = "parseFloat"
@val external isNaN: 'a => bool = "isNaN"
@val external isFinite: 'a => bool = "isFinite"
@val external jsTypeof: 'a => string = "js_typeof"

@val @scope("JSON") external jsonStringify: 'a => any = "stringify"
@val @scope("JSON")
external jsonStringifyWith: ('a, (string, any) => any, int) => string = "stringify"
@val @scope("Object") external objectKeys: 'a => array<string> = "keys"
@val @scope("Object") external defineProperty: ('obj, string, 'desc) => 'obj = "defineProperty"
@val @scope("Object") external createObject: 'a => any = "create"
@val @scope("Math") external mathAbs: float => float = "abs"
@val @scope("Math") external mathRound: float => float = "round"
@val @scope("Math") external mathFloor: float => float = "floor"
@val @scope("Math") external mathMax: (float, float) => float = "max"
@val @scope("Math") external mathMin: (float, float) => float = "min"
@val @scope("Math") external mathPow: (float, float) => float = "pow"
@val @scope("Math") external mathSqrt: float => float = "sqrt"
@val @scope("Math") external mathLog: float => float = "log"
@val @scope("Math") external mathExp: float => float = "exp"
@val @scope("Math") external mathPI: float = "PI"
@val @scope("Math") external mathAcos: float => float = "acos"
@val @scope("Math") external mathAcosh: float => float = "acosh"
@val @scope("Math") external mathAsin: float => float = "asin"
@val @scope("Math") external mathAsinh: float => float = "asinh"
@val @scope("Math") external mathAtan: float => float = "atan"
@val @scope("Math") external mathAtanh: float => float = "atanh"
@val @scope("Math") external mathSin: float => float = "sin"
@val @scope("Math") external mathSinh: float => float = "sinh"
@val @scope("Math") external mathCos: float => float = "cos"
@val @scope("Math") external mathCosh: float => float = "cosh"
@val @scope("Math") external mathTan: float => float = "tan"
@val @scope("Math") external mathTanh: float => float = "tanh"
@val @scope("Math") external mathExpm1: float => float = "expm1"
@val @scope("Math") external mathLog1p: float => float = "log1p"

@get_index external get: ('a, string) => any = ""
@set_index external set: ('a, string, 'b) => unit = ""
@get_index external getInt: ('a, int) => 'b = ""
@set_index external setInt: ('a, int, 'b) => unit = ""

@send external arrayPush: (array<'a>, 'a) => int = "push"
@send external arrayConcat: (array<'a>, array<'a>) => array<'a> = "concat"
@send external arraySlice: (array<'a>, int) => array<'a> = "slice"
@send external arraySort: array<'a> => array<'a> = "sort"
@send external arraySortReverse: array<'a> => array<'a> = "reverse"
@send external arrayJoin: (array<string>, string) => string = "join"
@send external arrayFilter: (array<'a>, ('a, int) => bool) => array<'a> = "filter"
@send external arrayMap: (array<'a>, ('a, int) => 'b) => array<'b> = "map"
@send external arraySome: (array<'a>, ('a, int) => bool) => bool = "some"
@get external arrayLength: array<'a> => int = "length"
@send external stringSplit: (string, string) => array<string> = "split"
@send external stringSlice: (string, int) => string = "slice"
@send external stringIndexOf: (string, string) => int = "indexOf"
@send external stringReplace: (string, unknown, string) => string = "replace"
@send external stringCharCodeAt: (string, int) => int = "charCodeAt"
@send external stringStartsWith: (string, string) => bool = "startsWith"
@send external testRe: (unknown, string) => bool = "test"
@send external execRe: (unknown, string) => any = "exec"
@get external regexpLastIndex: unknown => int = "lastIndex"
@set external setRegexpLastIndex: (unknown, int) => unit = "lastIndex"
@send external toString: 'a => string = "toString"
@send external toStringRadix: (int, int) => string = "toString"
@send external jsCall0: ('fn, 'this) => 'r = "call"
@send external jsCall1: ('fn, 'this, 'a) => 'r = "call"
@send external jsCall2: ('fn, 'this, 'a, 'b) => 'r = "call"
@send external jsCall3: ('fn, 'this, 'a, 'b, 'c) => 'r = "call"
@send external jsCall4: ('fn, 'this, 'a, 'b, 'c, 'd) => 'r = "call"
@send external jsCall6: ('fn, 'this, 'a, 'b, 'c, 'd, 'e, 'f) => 'r = "call"
@send external jsBind1: ('fn, 'this, 'a) => 'r = "bind"
@send external createElement: (any, string) => any = "createElement"
@send external appendChild: (any, any) => any = "appendChild"
@send external removeChild: (any, any) => any = "removeChild"
@send external setAttribute: (any, string, string) => unit = "setAttribute"
@send external setProperty: (any, string, string, string) => unit = "setProperty"
@send external addEventListener: (any, string, 'fn) => unit = "addEventListener"
@send external getContext: (any, string) => any = "getContext"
@send external toDataURL: any => string = "toDataURL"
@send external getBoundingClientRect: any => any = "getBoundingClientRect"
@send external createEvent: (any, string) => any = "createEvent"
@new external newSet: array<'a> => unknown = "Set"
@send external setHas: (unknown, 'a) => bool = "has"
@val external jsInfinity: float = "Infinity"
external asPromise: 'a => promise<'b> = "%identity"
external fromPromise: 'a => unknown = "%identity"
@send external then_: ('p, 'ok, 'err) => unknown = "then"
@send external then1: ('p, 'ok) => unknown = "then"
@send external promiseAll: (unknown, array<unknown>) => unknown = "all"
@get external protoOf: 'a => any = "__proto__"

@val external setTimeout: ('a, int) => any = "setTimeout"
@val external clearTimeout: 'a => unit = "clearTimeout"
@val external atob: string => string = "atob"

@new external newObject: unit => any = "Object"
@new external newArray: int => array<'a> = "Array"
@new external newUint8Array: int => unknown = "Uint8Array"
@new external newUint8ArrayFromBuffer: unknown => unknown = "Uint8Array"
@new external newFloat32Array: int => unknown = "Float32Array"
@new external newTextEncoder: unit => unknown = "TextEncoder"
@new external newError: string => unknown = "Error"
@new external newDate: unit => unknown = "Date"
@new external newDateYMD: (int, int, int) => unknown = "Date"
@new external newRegExp: (string, string) => unknown = "RegExp"
@new external newMessageChannel: unit => unknown = "MessageChannel"
@send external encodeText: (unknown, string) => unknown = "encode"
@get external u8Length: 'a => int = "length"
@get external arrayBuffer: unknown => unknown = "buffer"
@get external getYear: unknown => int = "getFullYear"
@send external getTimezoneOffset: unknown => float = "getTimezoneOffset"
@send external resolvedOptions: 'a => any = "resolvedOptions"

let emptyObject: unit => any = %raw(`function() { return {} }`)
let isUndefined: 'a => bool = %raw(`function(value) { return value === undefined }`)
let isNull: 'a => bool = %raw(`function(value) { return value === null }`)
let isNullish: 'a => bool = %raw(`function(value) { return value === undefined || value === null }`)
let isTruthy: 'a => bool = %raw(`function(value) { return !!value }`)
let isFalsy: 'a => bool = %raw(`function(value) { return !value }`)
let isFunction: 'a => bool = %raw(`function(value) { return typeof value === "function" }`)
let isString: 'a => bool = %raw(`function(value) { return typeof value === "string" }`)
let isNumber: 'a => bool = %raw(`function(value) { return typeof value === "number" }`)
let isObject: 'a => bool = %raw(`function(value) { return !!value && typeof value === "object" }`)
let isJsArray: 'a => bool = %raw(`function(value) { return Array.isArray(value) }`)
let hasKey: ('a, string) => bool = %raw(`function(obj, key) { return key in obj }`)
let isError: 'a => bool = %raw(`function(value) { return value instanceof Error }`)
let isDOMException: 'a => bool = %raw(`function(value) {
  return typeof DOMException === "function" && value instanceof DOMException
}`)
let construct0: 'ctor => any = %raw(`function(C) { return new C() }`)
let construct1: ('ctor, 'a) => any = %raw(`function(C, a) { return new C(a) }`)
let construct3: ('ctor, 'a, 'b, 'c) => any = %raw(`function(C, a, b, c) { return new C(a, b, c) }`)
let apply0: 'fn => any = %raw(`function(fn) { return fn() }`)
let apply1: ('fn, 'a) => any = %raw(`function(fn, a) { return fn(a) }`)
let apply2: ('fn, 'a, 'b) => any = %raw(`function(fn, a, b) { return fn(a, b) }`)
let apply3: ('fn, 'a, 'b, 'c) => any = %raw(`function(fn, a, b, c) { return fn(a, b, c) }`)
let apply4: ('fn, 'a, 'b, 'c, 'd) => any = %raw(`function(fn, a, b, c, d) { return fn(a, b, c, d) }`)
let apply5: ('fn, 'a, 'b, 'c, 'd, 'e) => any = %raw(`function(fn, a, b, c, d, e) { return fn(a, b, c, d, e) }`)
let apply6: ('fn, 'a, 'b, 'c, 'd, 'e, 'f) => any = %raw(`function(fn, a, b, c, d, e, f) { return fn(a, b, c, d, e, f) }`)
let applyThis0: ('fn, 'this) => any = %raw(`function(fn, thisArg) { return fn.call(thisArg) }`)
let call0 = (obj, name) => jsCall0(get(obj, name), obj)
let call1 = (obj, name, a) => jsCall1(get(obj, name), obj, a)
let call2 = (obj, name, a, b) => jsCall2(get(obj, name), obj, a, b)
let call3 = (obj, name, a, b, c) => jsCall3(get(obj, name), obj, a, b, c)
let call4 = (obj, name, a, b, c, d) => jsCall4(get(obj, name), obj, a, b, c, d)
let call6 = (obj, name, a, b, c, d, e, f) => jsCall6(get(obj, name), obj, a, b, c, d, e, f)
let callMethod: (any, string, array<any>) => any = %raw(`function(obj, name, args) {
  return obj[name].apply(obj, args)
}`)
let orElse: ('a, 'a) => 'a = %raw(`function(a, b) { return a || b }`)
let coalesce: ('a, 'a) => 'a = %raw(`function(a, b) { return a ?? b }`)
let strictEq: ('a, 'b) => bool = %raw(`function(a, b) { return a === b }`)
let plusString: ('a, 'b) => string = %raw(`function(a, b) { return a + b }`)
let jsString: 'a => string = %raw(`function(value) { return String(value) }`)
let jsIndex: ('a, 'b) => any = %raw(`function(obj, key) { return obj[key] }`)
let jsSetIndex: ('a, 'b, 'c) => unit = %raw(`function(obj, key, value) { obj[key] = value }`)
let jsPromiseResolve: 'a => unknown = %raw(`function(value) { return Promise.resolve(value) }`)
let jsPromiseAll: array<unknown> => unknown = %raw(`function(values) { return Promise.all(values) }`)
let jsPromise: ((unknown => unit, unknown => unit) => unit) => unknown = %raw(`
function(executor) { return new Promise(executor) }
`)
let throwJs: 'a => 'b = %raw(`function(error) { throw error }`)
let makeGetterUsingThis: (any => any) => any = %raw(`function(compute) {
  return function() { return compute(this) }
}`)
let tryFinallyAsync: (unit => promise<any>, unit => unit) => unknown = %raw(`async function(action, cleanup) {
  try { return await action() }
  finally { cleanup() }
}`)
let charAt: (string, int) => string = %raw(`function(value, index) { return value[index] }`)
let sliceString: (string, int, int) => string = %raw(`function(value, start, end) { return value.slice(start, end) }`)
let stringFromCharCode2: (int, int) => string = %raw(`function(a, b) { return String.fromCharCode(a, b) }`)

@val external unsafeWindow: any = "window"
@val external unsafeDocument: any = "document"
@val external unsafeNavigator: any = "navigator"
@val external unsafeScreen: any = "screen"
@val external unsafeMatchMedia: string => any = "matchMedia"
let window = () => unsafeWindow
let document = () => unsafeDocument
let navigator = () => unsafeNavigator
let screen = () => unsafeScreen
let matchMedia = query => unsafeMatchMedia(query)

let algorithmVersion = "5.2.0"

let errorName = (error: any) => asString(get(error, "name"))
let errorMessage = (error: any) => asString(get(error, "message"))
let errorStack = (error: any) => get(error, "stack")
