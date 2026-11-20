open Host

exception Error(string)
let syntax = () => throw(Error("syntax"))
type token = {kind: string, value: value}
type parser = {tokens: array<token>, mutable pos: int}
let node = t => {
  let n = obj()
  set(n, "type", str(t))
  n
}
let named = (t, v) => {
  let n = node(t)
  set(n, "name", v)
  n
}
let leaf = (t, v) => {
  let n = node(t)
  set(n, "value", v)
  n
}
let list = xs => {
  let a = array()
  List.forEach(xs, v => push(a, v))
  a
}
let children = (t, xs) => {
  let n = node(t)
  set(n, "children", list(xs))
  n
}
let letter = c => (c >= 65 && c <= 90) || ((c >= 97 && c <= 122) || c == 95)
let digit = c => c >= 48 && c <= 57
let json_value = v =>
  try parse(v) catch {
  | _ => syntax()
  }
let lex = source => {
  let n = length(source)
  let i = ref(0)
  let tokens = ref(list{})
  let add = (k, v) => tokens := list{{kind: k, value: v}, ...tokens.contents}
  while i.contents < n {
    let start = i.contents
    let c = unit_at(source, i.contents)
    i := i.contents + 1
    let next = if i.contents < n {
      unit_at(source, i.contents)
    } else {
      0
    }
    if letter(c) {
      while (
        i.contents < n && {
            let ch = unit_at(source, i.contents)
            letter(ch) || digit(ch)
          }
      ) {
        i := i.contents + 1
      }
      add("Field", slice(source, start, i.contents))
    } else if c == 45 || digit(c) {
      while (
        i.contents < n && {
            let ch = unit_at(source, i.contents)
            ch == 45 || digit(ch)
          }
      ) {
        i := i.contents + 1
      }
      add("Number", num(parse_int(slice(source, start, i.contents))))
    } else if c == 34 || (c == 39 || c == 96) {
      while i.contents < n && unit_at(source, i.contents) != c {
        if (
          unit_at(source, i.contents) == 92 &&
            (i.contents + 1 < n &&
            (unit_at(source, i.contents + 1) == c || unit_at(source, i.contents + 1) == 92))
        ) {
          i := i.contents + 1
        }
        i := i.contents + 1
      }
      let raw = slice(source, start + 1, i.contents)
      i := i.contents + 1
      if c == 34 {
        add("Quoted", json_value(slice(source, start, i.contents)))
      } else if c == 39 {
        add("Literal", replace(raw, "\\'", "'"))
      } else {
        let v = replace(trim_start(raw), "\\`", "`")
        let first = if length(v) == 0 {
          0
        } else {
          unit_at(v, 0)
        }
        let quoted = () => json_value(concat(concat(str("\""), v), str("\"")))
        let x = if (
          first == 91 ||
            (first == 123 ||
            (first == 34 || List.some(list{"true", "false", "null"}, x => x == text(v))))
        ) {
          json_value(v)
        } else if digit(first) || first == 45 {
          try parse(v) catch {
          | _ => quoted()
          }
        } else {
          quoted()
        }
        add("Literal", x)
      }
    } else if c != 32 && (c != 9 && c != 10) {
      let two = switch (c, next) {
      | (91, 63) => "Filter"
      | (91, 93) => "Flatten"
      | (38, 38) => "And"
      | (124, 124) => "Or"
      | (33, 61) => "NE"
      | (61, 61) => "EQ"
      | (60, 61) => "LTE"
      | (62, 61) => "GTE"
      | _ => ""
      }
      let k = if two != "" {
        i := i.contents + 1
        two
      } else {
        switch c {
        | 60 => "LT"
        | 62 => "GT"
        | 33 => "Not"
        | 38 => "Expref"
        | 124 => "Pipe"
        | 91 => "Lbracket"
        | 93 => "Rbracket"
        | 123 => "Lbrace"
        | 125 => "Rbrace"
        | 40 => "Lparen"
        | 41 => "Rparen"
        | 42 => "Star"
        | 64 => "Current"
        | 44 => "Comma"
        | 46 => "Dot"
        | 58 => "Colon"
        | _ => syntax()
        }
      }
      add(k, und)
    }
  }
  add("EOF", und)
  List.toArray(List.reverse(tokens.contents))
}
let power = x =>
  switch x {
  | "Pipe" => 1
  | "Or" => 2
  | "And" => 3
  | "EQ" | "NE" | "GT" | "LT" | "GTE" | "LTE" => 5
  | "Flatten" => 9
  | "Star" => 20
  | "Filter" => 21
  | "Dot" => 40
  | "Not" => 45
  | "Lbrace" => 50
  | "Lbracket" => 55
  | "Lparen" => 60
  | _ => 0
  }
let look = (p, n) =>
  if p.pos + n < Array.length(p.tokens) {
    Array.getUnsafe(p.tokens, p.pos + n).kind
  } else {
    "EOF"
  }
let next = p => {
  if p.pos >= Array.length(p.tokens) {
    syntax()
  }
  let t = Array.getUnsafe(p.tokens, p.pos)
  p.pos = p.pos + 1
  t
}
let take = (p, k) =>
  if next(p).kind != k {
    syntax()
  }
let rec expr = (p, rbp) => {
  let lhs = ref(nud(p, next(p)))
  while rbp < power(look(p, 0)) {
    let k = next(p).kind
    lhs := led(p, k, lhs.contents)
  }
  lhs.contents
}
and projection = (p, rbp) => {
  let k = look(p, 0)
  if power(k) < 10 {
    node("Identity")
  } else {
    switch k {
    | "Lbracket" | "Filter" => expr(p, rbp)
    | "Dot" =>
      ignore(next(p))
      dot(p, rbp)
    | _ => syntax()
    }
  }
}
and dot = (p, rbp) =>
  switch look(p, 0) {
  | "Field" | "Quoted" | "Star" => expr(p, rbp)
  | "Lbracket" =>
    ignore(next(p))
    multi(p)
  | "Lbrace" =>
    ignore(next(p))
    hash(p)
  | _ => und
  }
and multi = p => {
  let acc = ref(list{})
  while look(p, 0) != "Rbracket" {
    acc := list{expr(p, 0), ...acc.contents}
    if look(p, 0) == "Comma" {
      ignore(next(p))
      if look(p, 0) == "Rbracket" {
        syntax()
      }
    }
  }
  take(p, "Rbracket")
  children("MultiSelectList", List.reverse(acc.contents))
}
and hash = p => {
  let rec loop = acc => {
    let n = next(p)
    if n.kind != "Field" && n.kind != "Quoted" {
      syntax()
    }
    take(p, "Colon")
    let pair = named("KeyValuePair", n.value)
    set(pair, "value", expr(p, 0))
    if look(p, 0) == "Comma" {
      ignore(next(p))
      loop(list{pair, ...acc})
    } else if look(p, 0) == "Rbrace" {
      ignore(next(p))
      children("MultiSelectHash", List.reverse(list{pair, ...acc}))
    } else {
      loop(list{pair, ...acc})
    }
  }
  loop(list{})
}
and bracket = (p, lhs) =>
  if look(p, 0) == "Colon" || look(p, 1) == "Colon" {
    let parts = Array.make(~length=3, nil)
    let i = ref(0)
    while look(p, 0) != "Rbracket" && i.contents < 3 {
      let t = next(p)
      if t.kind == "Colon" {
        i := i.contents + 1
      } else if t.kind == "Number" {
        parts[i.contents] = t.value
      } else {
        syntax()
      }
    }
    take(p, "Rbracket")
    let ix = children("IndexExpression", list{lhs, children("Slice", List.fromArray(parts))})
    children("Projection", list{ix, projection(p, 20)})
  } else {
    let t = next(p)
    take(p, "Rbracket")
    children("IndexExpression", list{lhs, leaf("Index", t.value)})
  }
and args = (p, commas) => {
  let acc = ref(list{})
  while look(p, 0) != "Rparen" {
    let v = if look(p, 0) == "Current" {
      ignore(next(p))
      node("Current")
    } else {
      expr(p, 0)
    }
    acc := list{v, ...acc.contents}
    if commas && look(p, 0) == "Comma" {
      ignore(next(p))
    }
  }
  take(p, "Rparen")
  List.reverse(acc.contents)
}
and nud = (p, t) =>
  switch t.kind {
  | "Literal" => leaf("Literal", t.value)
  | "Quoted" if look(p, 0) == "Lparen" => syntax()
  | "Field" | "Quoted" => named("Field", t.value)
  | "Not" => children("NotExpression", list{expr(p, 45)})
  | "Current" => node("Current")
  | "Expref" => children("ExpressionReference", list{expr(p, 0)})
  | "Star" => children("ValueProjection", list{node("Identity"), projection(p, 20)})
  | "Filter" | "Flatten" => led(p, t.kind, node("Identity"))
  | "Lbrace" => hash(p)
  | "Lbracket" =>
    if look(p, 0) == "Colon" || look(p, 0) == "Number" {
      bracket(p, node("Identity"))
    } else if look(p, 0) == "Star" && look(p, 1) == "Rbracket" {
      p.pos = p.pos + 2
      children("Projection", list{node("Identity"), projection(p, 20)})
    } else {
      multi(p)
    }
  | "Lparen" =>
    switch args(p, false) {
    | list{h, ..._} => h
    | list{} => und
    }
  | _ => syntax()
  }
and led = (p, k, lhs) =>
  switch k {
  | "Dot" if look(p, 0) == "Star" =>
    ignore(next(p))
    children("ValueProjection", list{lhs, projection(p, 40)})
  | "Dot" => children("Subexpression", list{lhs, dot(p, 40)})
  | "Or" | "And" | "Pipe" =>
    children(
      if k == "Pipe" {
        k
      } else {
        k ++ "Expression"
      },
      list{lhs, expr(p, power(k))},
    )
  | "Lparen" =>
    if strict(lhs, und) {
      syntax()
    }
    let n = named("Function", get(lhs, "name"))
    set(n, "children", list(args(p, true)))
    n
  | "Filter" =>
    let condition = expr(p, 0)
    take(p, "Rbracket")
    let right = if look(p, 0) == "Flatten" {
      node("Identity")
    } else {
      projection(p, 21)
    }
    children("FilterProjection", list{lhs, right, condition})
  | "Flatten" => children("Projection", list{children("Flatten", list{lhs}), projection(p, 9)})
  | "EQ" | "NE" | "LT" | "GT" | "LTE" | "GTE" =>
    let n = named("Comparator", str(k))
    let rhs = expr(p, 5)
    set(n, "children", list(list{lhs, rhs}))
    n
  | "Lbracket" =>
    if look(p, 0) == "Colon" || look(p, 0) == "Number" {
      bracket(p, lhs)
    } else {
      take(p, "Star")
      take(p, "Rbracket")
      children("Projection", list{lhs, projection(p, 20)})
    }
  | _ => syntax()
  }
let compile = s => {
  let p = {tokens: lex(s), pos: 0}
  let v = expr(p, 0)
  if look(p, 0) != "EOF" {
    syntax()
  }
  v
}
