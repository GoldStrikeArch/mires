type value
external cast: 'a => 'b = "%identity"
@new external obj: unit => value = "Object"
@scope("Object") @val external null_object: value => value = "create"
@new external array: int => value = "Array"
@get_index external get: (value, string) => value = ""
@set_index external set: (value, string, value) => unit = ""
@get_index external at: (value, int) => value = ""
@set_index external put: (value, int, value) => unit = ""
@get_index external key_get: (value, value) => value = ""
@set_index external key_set: (value, value, value) => unit = ""
@get external length: value => int = "length"
@scope("Object") @val external keys: value => value = "keys"
@send external sorted: value => value = "sort"
@scope("Array") @val external is_array: value => bool = "isArray"
@scope("JSON") @val external json: value => value = "stringify"
@scope("JSON") @val external pretty: (value, value, int) => value = "stringify"
@val external truth: value => bool = "Boolean"
external typeof: value => string = "#typeof"
let undef: value = cast(Nullable.undefined)
let null: value = cast(Nullable.null)
let absent = v => Nullable.isNullable(cast(v))
let integer = (v: value): int => cast(v)
let boolean = (b: bool): value => cast(b)
let number = (n: int): value => cast(n)
let is_string = v => typeof(v) == "string"
let object_value = v => typeof(v) == "object" && !absent(v)
@send external codePointAt: (string, int) => int = "codePointAt"
@scope("String") @val external fromCodePoint: int => string = "fromCodePoint"
@send external code: (string, int) => int = "charCodeAt"
let byteCode = s => code(s, 0)
let ascii = s => {
  let rec test = i => i >= String.length(s) || (code(s, i) < 128 && test(i + 1))
  test(0)
}
let cp = (s, i) => {
  let byte = n => code(s, i + n)
  let a = byte(0)
  if a < 128 {
    (a, 1)
  } else if a < 224 {
    (Int.bitwiseOr(Int.shiftLeft(Int.bitwiseAnd(a, 31), 6), Int.bitwiseAnd(byte(1), 63)), 2)
  } else if a < 240 {
    (
      Int.bitwiseOr(
        Int.bitwiseOr(
          Int.shiftLeft(Int.bitwiseAnd(a, 15), 12),
          Int.shiftLeft(Int.bitwiseAnd(byte(1), 63), 6),
        ),
        Int.bitwiseAnd(byte(2), 63),
      ),
      3,
    )
  } else {
    (
      Int.bitwiseOr(
        Int.bitwiseOr(
          Int.bitwiseOr(
            Int.shiftLeft(Int.bitwiseAnd(a, 7), 18),
            Int.shiftLeft(Int.bitwiseAnd(byte(1), 63), 12),
          ),
          Int.shiftLeft(Int.bitwiseAnd(byte(2), 63), 6),
        ),
        Int.bitwiseAnd(byte(3), 63),
      ),
      4,
    )
  }
}
// Preserve isolated UTF-16 units alongside ordinary UTF-8 code points.
let read = v => {
  let s = cast(v)
  if ascii(s) { s } else {
    let bytes = []
    let byte = c => bytes->Array.push(String.fromCharCode(c))->ignore
    let cont = c => byte(128 + Int.bitwiseAnd(c, 63))
    let i = ref(0)
    while i.contents < String.length(s) {
      let c = codePointAt(s, i.contents)
      if c < 128 { byte(c) }
      else if c < 0x800 { byte(192 + c / 64); cont(c) }
      else if c < 0x10000 { byte(224 + c / 4096); cont(c / 64); cont(c) }
      else { byte(240 + c / 262144); cont(c / 4096); cont(c / 64); cont(c) }
      i := i.contents + (if c > 0xffff { 2 } else { 1 })
    }
    bytes->Array.join("")
  }
}
let str = s => {
  if ascii(s) { cast(s) } else {
    let parts = []
    let i = ref(0)
    while i.contents < String.length(s) {
      let (c, n) = cp(s, i.contents)
      parts->Array.push(fromCodePoint(c))->ignore
      i := i.contents + n
    }
    cast(parts->Array.join(""))
  }
}
let lower = s => read(cast(String.toLowerCase(cast(str(s)))))

@send external toWellFormed: value => value = "toWellFormed"
@new external byteArray: int => value = "Uint8Array"
@new external decoder: unit => value = "TextDecoder"
@send external decodeBytes: (value, value) => value = "decode"
let scalarText = s => read(toWellFormed(str(s)))
let decodeOctets = bytes => {
  let data = byteArray(Array.length(bytes))
  bytes->Array.forEachWithIndex((x, i) => put(data, i, number(x)))
  read(decodeBytes(decoder(), data))
}
