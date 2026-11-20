// Native JSON and UTF-16 string operations.
type value
external box: 'a => value = "%identity"
external text: value => string = "%identity"
external float_value: value => float = "%identity"
@new external obj: unit => value = "Object"
@new external array: unit => value = "Array"
@get_index external get: (value, string) => value = ""
@get_index external prop: (value, value) => value = ""
@set_index external set: (value, string, value) => unit = ""
@set_index external put: (value, value, value) => unit = ""
@get_index external at: (value, int) => value = ""
@send external push: (value, value) => unit = "push"
@get external length: value => int = "length"
@scope("Object") @val external keys: value => value = "keys"
@scope("Object") @val external values: value => value = "values"
@scope("Array") @val external is_array: value => bool = "isArray"
@scope("JSON") @val external json: value => value = "stringify"
@scope("JSON") @val external parse: value => value = "parse"
@val external number: value => float = "Number"
@val external as_string: value => value = "String"
@val external parse_int: value => float = "parseInt"
@send external concat: (value, value) => value = "concat"
@send external unit_at: (value, int) => int = "charCodeAt"
@send external slice: (value, int, int) => value = "slice"
@send external replace: (value, string, string) => value = "replace"
@send external trim_start: value => value = "trimStart"
@send external index: (value, value) => int = "indexOf"
@send external index_from: (value, value, int) => int = "indexOf"
@send external last_index: (value, value) => int = "lastIndexOf"
@send external locale_cmp: (value, value) => int = "localeCompare"
@send external join: (value, value) => value = "join"
@send external sorted: value => value = "sort"
@send external reversed: value => value = "reverse"
external typeof: value => string = "#typeof"
let strict = (x: value, y: value) => x === y
let nil = box(Nullable.null)
let und = box(Nullable.undefined)
let str = (s: string) => box(s)
let num = (n: float) => box(n)
let bool = (b: bool) => box(b)
