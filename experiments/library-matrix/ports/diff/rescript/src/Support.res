let each = (f, a) => Array.forEach(a, f)
let eachIndex = (f, a) => Array.forEachWithIndex(a, (v, i) => f(i, v))
let arraySlice = (a, start, length) => Array.slice(a, ~start, ~end=start + length)
let eachList = (f, l) => List.forEach(l, f)
let mapArray = (f, a) => Array.map(a, f)
let mapList = (f, l) => List.map(l, f)
let inList = (s, l) => List.some(l, x => x == s)
let onSome = (f, v) =>
  switch v {
  | Some(x) => f(x)
  | None => ()
  }
let invalid = message => JsError.throw(JsError.TypeError.make(message))
let parseInt = s =>
  switch Int.fromString(s) {
  | Some(n) => n
  | None => invalid("invalid hunk range")
  }
module Text = {
  let substring = (s, start, n) => String.slice(s, ~start, ~end=start + n)
  let starts = (~prefix, s) => String.startsWith(s, prefix)
  let ends = (~suffix, s) => String.endsWith(s, suffix)
  let split = (c, s) => String.split(s, c)->List.fromArray
  let eachIndex = (f, s) => {
    for i in 0 to String.length(s) - 1 {
      f(i, String.charAt(s, i))
    }
  }
  let each = (f, s) => eachIndex((_, c) => f(c), s)
  let locate = (s, c) => {
    let i = String.indexOf(s, c)
    i < 0 ? None : Some(i)
  }
  let exists = (f, s) => {
    let rec search = i => i < String.length(s) && (f(String.charAt(s, i)) || search(i + 1))
    search(0)
  }
  let concat = (sep, lines) => lines->List.toArray->Array.join(sep)
}
module Buffer = {
  let create = _ => []
  let add_string = (b, s: string) => {b->Array.push(s)->ignore}
  let add_char = add_string
  let contents = b => Array.join(b, "")
}
