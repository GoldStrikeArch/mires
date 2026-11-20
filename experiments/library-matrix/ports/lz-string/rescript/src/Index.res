type input = {op: string, format: string, data: Null.t<array<int>>, repeat?: int}
type width = {mutable bits: int, mutable left: int}
let advance = w => {
  w.left = w.left - 1
  if w.left == 0 {
    w.left = Int.shiftLeft(1, w.bits)
    w.bits = w.bits + 1
  }
}
let size = format => switch format {
| "raw" | "bytes" => 16
| "utf16" => 15
| "base64" | "uri" => 6
| _ => JsError.TypeError.make("unknown format")->JsError.throw
}
let alphabet = format => format == "uri"
  ? "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$"
  : "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
let numberAt = (s, i) => String.charCodeAt(s, i)->Option.getOr(0)

let compress = (text, format) => {
  let dict = Map.make()
  let pending = Set.make()
  let words = []
  let w = {bits: 2, left: 2}
  let width = size(format)
  let used = ref(0)
  let word = ref(0)
  let next = ref(3)
  let write = (n, value) => {
    for bit in 0 to n - 1 {
      word := word.contents * 2 + Int.bitwiseAnd(Int.shiftRightUnsigned(value, bit), 1)
      used := used.contents + 1
      if used.contents == width {
        Array.push(words, word.contents)
        used := 0
        word := 0
      }
    }
  }
  let emit = phrase => {
    if pending->Set.delete(phrase) {
      let c = numberAt(phrase, 0)
      write(w.bits, c < 256 ? 0 : 1)
      write(c < 256 ? 8 : 16, c)
      advance(w)
    } else {
      write(w.bits, dict->Map.get(phrase)->Option.getOrThrow)
    }
    advance(w)
  }
  let prefix = ref("")
  for i in 0 to String.length(text) - 1 {
    let ch = String.charAt(text, i)
    if !Map.has(dict, ch) {
      dict->Map.set(ch, next.contents)
      next := next.contents + 1
      pending->Set.add(ch)
    }
    let phrase = prefix.contents ++ ch
    if dict->Map.has(phrase) {
      prefix := phrase
    } else {
      emit(prefix.contents)
      if phrase != "__proto__" {dict->Map.set(phrase, next.contents)}
      next := next.contents + 1
      prefix := ch
    }
  }
  if prefix.contents != "" {emit(prefix.contents)}
  write(w.bits, 2)
  write(width - used.contents, 0)
  switch format {
  | "raw" => words
  | "utf16" =>
    let a = words->Array.map(n => n + 32)
    a->Array.push(32)
    a
  | "bytes" => words->Array.flatMap(n => [Int.shiftRightUnsigned(n, 8), Int.bitwiseAnd(n, 255)])
  | _ =>
    let chars = alphabet(format)
    let a = words->Array.map(n => numberAt(chars, n))
    if format == "base64" {
      while mod(Array.length(a), 4) != 0 {a->Array.push(61)}
    }
    a
  }
}

let decompress = (data, format) => {
  let width = size(format)
  let words = switch format {
  | "raw" => data
  | "utf16" => data->Array.map(n => n - 32)
  | "bytes" =>
    if mod(Array.length(data), 2) != 0 {JsError.TypeError.make("odd byte count")->JsError.throw}
    Array.fromInitializer(~length=Array.length(data) / 2, i =>
      Array.getUnsafe(data, 2*i) * 256 + Array.getUnsafe(data, 2*i+1))
  | _ =>
    let chars = alphabet(format)
    data->Array.map(n => {
      let c = String.fromCharCode(format == "uri" && n == 32 ? 43 : n)
      let k = String.indexOf(chars, c)
      k < 0 ? 0 : k
    })
  }
  let index = ref(0)
  let position = ref(width)
  let read = n => {
    let value = ref(0)
    for bit in 0 to n-1 {
      let word = words->Array.get(index.contents)->Option.getOr(0)
      position := position.contents - 1
      let digit = Int.bitwiseAnd(Int.shiftRightUnsigned(word, position.contents), 1)
      value := value.contents + Int.shiftLeft(digit, bit)
      if position.contents == 0 {position := width; index := index.contents + 1}
    }
    value.contents
  }
  if Array.length(data) == 0 {None} else {
    let header = read(2)
    if header == 2 {Some("")} else {
      let previous = ref(header == 3 ? "" : String.fromCharCode(read(header == 0 ? 8 : 16)))
      let dict = ["", "", "", previous.contents]
      let output = [previous.contents]
      let codes = {bits: 3, left: 4}
      let rec loop = () => {
        if index.contents >= Array.length(words) {Some("")} else {
          let code = read(codes.bits)
          if code == 2 {Some(output->Array.join(""))} else {
            let actual = if code <= 1 {
              let k = Array.length(dict)
              dict->Array.push(String.fromCharCode(read(code == 0 ? 8 : 16)))
              advance(codes)
              k
            } else {code}
            let entry = if actual == Array.length(dict) {
              if previous.contents == "" {JsError.TypeError.make("undefined phrase")->JsError.throw}
              previous.contents ++ String.charAt(previous.contents, 0)
            } else {dict->Array.get(actual)->Option.getOr("")}
            if entry == "" {None} else {
              output->Array.push(entry)
              let prefix = previous.contents == "" ? "undefined" : previous.contents
              dict->Array.push(prefix ++ String.charAt(entry, 0))
              previous := entry
              advance(codes)
              loop()
            }
          }
        }
      }
      loop()
    }
  }
}

let run = input => switch input.data->Null.toOption {
| None => Null.make([])
| Some(data) =>
  if input.op == "decompress" {
    switch decompress(data, input.format) {
    | None => Null.null
    | Some(text) => Null.make(Array.fromInitializer(~length=String.length(text), i => numberAt(text, i)))
    }
  } else {
    let text = data->Array.map(String.fromCharCode)->Array.join("")->String.repeat(input.repeat->Option.getOr(1))
    let packed = compress(text, input.format)
    if input.op == "compress" {Null.make(packed)} else {
      if input.op != "roundtrip" || decompress(packed, input.format) != Some(text) {
        JsError.TypeError.make("roundtrip mismatch")->JsError.throw
      }
      Null.make([Array.length(packed), String.length(text)])
    }
  }
}
