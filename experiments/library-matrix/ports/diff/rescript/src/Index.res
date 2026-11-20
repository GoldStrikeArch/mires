open Support
open Host
open Differ
let canonical = v => {
  let rec visit = v =>
    if is_array(v) {
      let a = array(length(v))
      for i in 0 to length(v) - 1 {
        put(a, i, visit(at(v, i)))
      }
      a
    } else if object_value(v) {
      let o = null_object(Host.null)
      let ks = sorted(keys(v))
      for i in 0 to length(ks) - 1 {
        let k = at(ks, i)
        key_set(o, k, visit(key_get(v, k)))
      }
      o
    } else {
      v
    }
  visit(v)
}
let document = (op, v) =>
  if op == "diffJson" && !is_string(v) {
    read(pretty(canonical(v), Host.null, 2))
  } else {
    read(v)
  }
let write_change = (span, v) => {
  let o = obj()
  set(o, "count", number(span.count))
  set(o, "added", boolean(span.kind == Add))
  set(o, "removed", boolean(span.kind == Remove))
  set(o, "value", v)
  o
}
let run = input => {
  let op = read(get(input, "op"))

  let a = get(input, "before")

  let b = get(input, "after")

  let arg = get(input, "patch")

  let opts = get(input, "options")
  let flag = key => !absent(opts) && truth(get(opts, key))
  let o = {
    ignore_case: flag("ignoreCase"),
    individual: flag("oneChangePerToken"),
    strip_cr: flag("stripTrailingCr"),
    ignore_ws: flag("ignoreWhitespace"),
    newline_token: flag("newlineIsToken"),
    ignore_eof: flag("ignoreNewlineAtEof"),
  }
  switch op {
  | "diffArrays" =>
    let values = v => Patch.array_of_js(x => read(json(x)), v)
    let spans = List.toArray(myers(values(a), values(b), o.individual))
    let old = ref(0)
    let newer = ref(0)
    Patch.array_to_js(span => {
      let value = array(span.count)
      for i in 0 to span.count - 1 {
        put(
          value,
          i,
          if span.kind == Remove {
            at(a, old.contents + i)
          } else {
            at(b, newer.contents + i)
          },
        )
      }
      if span.kind != Add {
        old := old.contents + span.count
      }
      if span.kind != Remove {
        newer := newer.contents + span.count
      }
      write_change(span, value)
    }, spans)
  | "diffChars" | "diffWords" | "diffLines" | "diffJson" =>
    let tokens = v => tokenize(op, document(op, v), o)
    Patch.array_to_js(c => write_change(c.span, str(c.value)), changes(op, tokens(a), tokens(b), o))
  | "structuredPatch" | "createTwoFilesPatch" =>
    let ctx = if absent(opts) || absent(get(opts, "context")) {
      4
    } else {
      integer(get(opts, "context"))
    }
    let p = Patch.create(input, ctx)
    if op == "structuredPatch" {
      Patch.write(false, p)
    } else {
      str(Patch.format(p))
    }
  | "parsePatch" => Patch.array_to_js(p => Patch.write(false, p), Patch.parse(read(arg)))
  | "reversePatch" =>
    if is_array(arg) {
      let n = length(arg)
      let v = array(n)
      for i in 0 to n - 1 {
        put(v, n - i - 1, Patch.reverse(at(arg, i)))
      }
      v
    } else {
      Patch.reverse(arg)
    }
  | "applyPatch" =>
    let p = if is_string(arg) {
      let ps = Patch.parse(read(arg))
      if Array.length(ps) != 1 {
        invalid("multiple patches")
      }
      Array.getUnsafe(ps, 0)
    } else if is_array(arg) {
      if length(arg) != 1 {
        invalid("multiple patches")
      }
      Patch.read_patch(at(arg, 0))
    } else {
      Patch.read_patch(arg)
    }
    Patch.apply(read(a), p)
  | _ => invalid("unknown operation")
  }
}
