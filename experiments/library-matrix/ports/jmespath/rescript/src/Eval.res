open Host
open Parser

let kind = v =>
  if strict(v, nil) {
    "null"
  } else if is_array(v) {
    "array"
  } else {
    switch typeof(v) {
    | "object" if strict(get(v, "jmespathType"), str("Expref")) => "expref"
    | t => t
    }
  }
let object_kind = v => {
  let t = kind(v)
  t == "object" || t == "expref"
}
let falsy = v =>
  switch kind(v) {
  | "null" => true
  | "boolean" => strict(v, bool(false))
  | "array" | "string" => length(v) == 0
  | "object" | "expref" => length(keys(v)) == 0
  | _ => false
  }
let every = (n, f) => {
  let rec loop = i => i == n || (f(i) && loop(i + 1))
  loop(0)
}
let rec equal = (a, b) =>
  if strict(a, b) {
    true
  } else {
    let t = kind(a)
    if t != kind(b) {
      false
    } else if t == "array" {
      length(a) == length(b) && every(length(a), i => equal(at(a, i), at(b, i)))
    } else if t == "object" {
      let ks = keys(a)
      length(ks) == length(keys(b)) &&
        every(length(ks), i => {
          let k = at(ks, i)
          equal(prop(a, k), prop(b, k))
        })
    } else {
      false
    }
  }
let compare = (x, y) => x < y ? -1 : x > y ? 1 : 0
let cmp = (a, b) => {
  let primitive = v =>
    if typeof(v) == "object" && !strict(v, nil) {
      as_string(v)
    } else {
      v
    }
  let x = primitive(a)
  let y = primitive(b)
  if typeof(x) == "string" && typeof(y) == "string" {
    let bound = Math.Int.min(length(x), length(y))
    let rec loop = i =>
      if i == bound {
        compare(length(x), length(y))
      } else {
        let d = compare(unit_at(x, i), unit_at(y, i))
        if d == 0 {
          loop(i + 1)
        } else {
          d
        }
      }
    Some(loop(0))
  } else {
    let x = number(x)
    let y = number(y)
    if Float.isNaN(x) || Float.isNaN(y) {
      None
    } else {
      Some(compare(x, y))
    }
  }
}
let signature = x =>
  switch x {
  | "abs" | "floor" | "ceil" => list{"number"}
  | "avg" | "sum" => list{"numbers"}
  | "contains" => list{"array|string", "any"}
  | "starts_with" | "ends_with" => list{"string", "string"}
  | "length" => list{"object|string|array"}
  | "map" => list{"expref", "array"}
  | "max" | "min" | "sort" => list{"numbers|strings"}
  | "keys" | "values" | "merge" => list{"object"}
  | "max_by" | "min_by" | "sort_by" => list{"array", "expref"}
  | "join" => list{"string", "strings"}
  | "reverse" => list{"string|array"}
  | "to_array" | "to_number" | "to_string" | "type" | "not_null" => list{"any"}
  | _ => throw(Error("unknown-function"))
  }
let valid = (v, t) => {
  let subtype = k => is_array(v) && every(length(v), i => kind(at(v, i)) == k)
  Array.some(String.split(t, "|"), x =>
    switch x {
    | "any" => true
    | "strings" => subtype("string")
    | "numbers" => subtype("number")
    | tag => kind(v) == tag
    }
  )
}
let child = (n, i) => at(get(n, "children"), i)
let rec eval = (n, v) => {
  if strict(n, nil) || strict(n, und) {
    syntax()
  }
  let k = text(get(n, "type"))
  let arg = i => eval(child(n, i), v)
  switch k {
  | "Field" =>
    if !object_kind(v) {
      nil
    } else {
      let x = prop(v, get(n, "name"))
      if strict(x, und) {
        nil
      } else {
        x
      }
    }
  | "Identity" | "Current" => v
  | "Literal" => get(n, "value")
  | "Pipe" | "IndexExpression" | "Subexpression" => eval(child(n, 1), arg(0))
  | "Index" =>
    if !is_array(v) {
      nil
    } else {
      let f = number(get(n, "value"))
      let p = if f < 0. {
        f +. Int.toFloat(length(v))
      } else {
        f
      }
      if p >= 0. && p < Int.toFloat(length(v)) {
        at(v, Float.toInt(p))
      } else {
        nil
      }
    }
  | "Slice" =>
    if !is_array(v) {
      nil
    } else {
      let p = child(n, 2)
      let step = if strict(p, nil) { 1. } else { number(p) }
      if step == 0. {
        throw(Error("invalid-value"))
      }
      let len = Int.toFloat(length(v))
      let (low, high) = if step < 0. { (-1., len -. 1.) } else { (0., len) }
      let bound = (x, fallback) =>
        if strict(x, nil) {
          fallback
        } else {
          let raw = number(x)
          let y = if raw < 0. { raw +. len } else { raw }
          if y < low { low } else if y > high { high } else { y }
        }
      let i = ref(bound(child(n, 0), if step < 0. { high } else { low }))
      let stop = bound(child(n, 1), if step < 0. { low } else { high })
      let out = array()
      while (if step < 0. { i.contents > stop } else { i.contents < stop }) {
        push(out, at(v, Float.toInt(i.contents)))
        i := i.contents +. step
      }
      out
    }
  | "Projection" | "ValueProjection" | "FilterProjection" =>
    let base = arg(0)
    let src = if k != "ValueProjection" {
      base
    } else if object_kind(base) {
      values(base)
    } else {
      nil
    }
    if !is_array(src) {
      nil
    } else {
      let out = array()
      for i in 0 to length(src) - 1 {
        let x = at(src, i)
        if k != "FilterProjection" || !falsy(eval(child(n, 2), x)) {
          let y = eval(child(n, 1), x)
          if !strict(y, nil) {
            push(out, y)
          }
        }
      }
      out
    }
  | "Flatten" =>
    let src = arg(0)
    if !is_array(src) {
      nil
    } else {
      let out = array()
      for i in 0 to length(src) - 1 {
        let x = at(src, i)
        if is_array(x) {
          for j in 0 to length(x) - 1 {
            push(out, at(x, j))
          }
        } else {
          push(out, x)
        }
      }
      out
    }
  | "Comparator" =>
    let a = arg(0)
    let b = arg(1)
    let op = text(get(n, "name"))
    bool(
      switch op {
      | "EQ" => equal(a, b)
      | "NE" => !equal(a, b)
      | _ =>
        switch cmp(a, b) {
        | None => false
        | Some(c) =>
          switch op {
          | "LT" => c < 0
          | "GT" => c > 0
          | "LTE" => c <= 0
          | _ => c >= 0
          }
        }
      },
    )
  | "AndExpression" | "OrExpression" =>
    let a = arg(0)
    if falsy(a) == (k == "OrExpression") {
      arg(1)
    } else {
      a
    }
  | "NotExpression" => bool(falsy(arg(0)))
  | "MultiSelectHash" | "MultiSelectList" =>
    if strict(v, nil) {
      nil
    } else {
      let hash = k == "MultiSelectHash"
      let out = if hash {
        obj()
      } else {
        array()
      }
      let cs = get(n, "children")
      for i in 0 to length(cs) - 1 {
        let c = at(cs, i)
        if hash {
          put(out, get(c, "name"), eval(get(c, "value"), v))
        } else {
          push(out, eval(c, v))
        }
      }
      out
    }
  | "ExpressionReference" =>
    let c = child(n, 0)
    set(c, "jmespathType", str("Expref"))
    c
  | "Function" =>
    let args = array()
    let cs = get(n, "children")
    for i in 0 to length(cs) - 1 {
      push(args, eval(at(cs, i), v))
    }
    let name = get(n, "name")
    if typeof(name) != "string" {
      throw(Error("unknown-function"))
    }
    call(text(name), args)
  | _ => syntax()
  }
}
and call = (name, args) => {
  let spec = signature(name)
  let arity = List.length(spec)
  if length(args) < arity || (name != "merge" && name != "not_null" && length(args) != arity) {
    throw(Error("invalid-arity"))
  }
  Array.forEachWithIndex(List.toArray(spec), (t, i) =>
    if !valid(at(args, i), t) {
      throw(Error("invalid-type"))
    }
  )
  let a = at(args, 0)
  let b = at(args, 1)
  switch name {
  | "abs" => num(Math.abs(number(a)))
  | "floor" => num(Math.floor(number(a)))
  | "ceil" => num(Math.ceil(number(a)))
  | "avg" | "sum" =>
    let total = ref(0.)
    for i in 0 to length(a) - 1 {
      total := total.contents +. number(at(a, i))
    }
    num(
      if name == "avg" {
        total.contents /. Int.toFloat(length(a))
      } else {
        total.contents
      },
    )
  | "contains" => bool(index(a, b) >= 0)
  | "starts_with" => bool(last_index(a, b) == 0)
  | "ends_with" => bool(index_from(a, b, length(a) - length(b)) >= 0)
  | "length" =>
    num(
      Int.toFloat(
        if kind(a) == "object" {
          length(keys(a))
        } else {
          length(a)
        },
      ),
    )
  | "join" => join(b, a)
  | "type" => str(kind(a))
  | "keys" => keys(a)
  | "values" => values(a)
  | "to_array" =>
    if is_array(a) {
      a
    } else {
      list(list{a})
    }
  | "to_number" =>
    if typeof(a) == "number" {
      a
    } else if typeof(a) == "string" && !Float.isNaN(number(a)) {
      num(number(a))
    } else {
      nil
    }
  | "to_string" =>
    if typeof(a) == "string" {
      a
    } else {
      json(a)
    }
  | "not_null" =>
    let rec loop = i =>
      if i == length(args) {
        nil
      } else {
        let x = at(args, i)
        if strict(x, nil) {
          loop(i + 1)
        } else {
          x
        }
      }
    loop(0)
  | "reverse" =>
    if typeof(a) == "string" {
      let x = ref(str(""))
      for i in length(a) - 1 downto 0 {
        x := concat(x.contents, slice(a, i, i + 1))
      }
      x.contents
    } else {
      reversed(slice(a, 0, length(a)))
    }
  | "sort" => sorted(slice(a, 0, length(a)))
  | "merge" =>
    let out = obj()
    for i in 0 to length(args) - 1 {
      let x = at(args, i)
      if !(strict(x, nil) || strict(x, und)) {
        let ks = keys(x)
        for j in 0 to length(ks) - 1 {
          let k = at(ks, j)
          put(out, k, prop(x, k))
        }
      }
    }
    out
  | "max" | "min" =>
    if length(a) == 0 {
      nil
    } else {
      let best = ref(at(a, 0))
      for i in 1 to length(a) - 1 {
        let x = at(a, i)
        let d = if typeof(x) == "string" {
          locale_cmp(x, best.contents)
        } else {
          Option.getOr(cmp(x, best.contents), 0)
        }
        if (
          if name == "min" {
            d < 0
          } else {
            d > 0
          }
        ) {
          best := x
        }
      }
      best.contents
    }
  | "map" =>
    let out = array()
    for i in 0 to length(b) - 1 {
      push(out, eval(a, at(b, i)))
    }
    out
  | "min_by" | "max_by" =>
    let max_ = name == "max_by"
    let extreme = ref(
      num(
        if max_ {
          Float.Constants.negativeInfinity
        } else {
          Float.Constants.positiveInfinity
        },
      ),
    )

    let out = ref(und)
    for i in 0 to length(a) - 1 {
      let x = at(a, i)
      let key = eval(b, x)
      if kind(key) != "number" && kind(key) != "string" {
        throw(Error("invalid-type"))
      }
      switch cmp(key, extreme.contents) {
      | Some(d)
        if if max_ {
          d > 0
        } else {
          d < 0
        } =>
        extreme := key
        out := x
      | _ => ()
      }
    }
    out.contents
  | "sort_by" =>
    let out = array()
    if length(a) > 0 {
      let ty = kind(eval(b, at(a, 0)))
      if ty != "number" && ty != "string" {
        throw(Error("invalid-type"))
      }
      let pairs = Array.fromInitializer(~length=length(a), i => {
        let item = at(a, i)
        let key = eval(b, item)
        if kind(key) != ty {
          throw(Error("invalid-type"))
        }
        (key, item)
      })
      Array.sort(pairs, ((x, _), (y, _)) => Int.toFloat(Option.getOr(cmp(x, y), 0)))->ignore
      Array.forEach(pairs, ((_, item)) => push(out, item))
    }
    out
  | _ => throw(Error("unknown-function"))
  }
}
