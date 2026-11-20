open Support
/* Myers' furthest-reaching paths and the five jsdiff token policies. */
type kind = Keep | Remove | Add
type span = {kind: kind, count: int}
type options = {
  ignore_case: bool,
  individual: bool,
  strip_cr: bool,
  ignore_ws: bool,
  newline_token: bool,
  ignore_eof: bool,
}
let defaults = {
  ignore_case: false,
  individual: false,
  strip_cr: false,
  ignore_ws: false,
  newline_token: false,
  ignore_eof: false,
}
type path = {old: int, spans: list<span>}
exception Done(list<span>)
let myers = (old, newer, individual) => {
  let limit = Array.length(old) + Array.length(newer)
  let base = limit + 1
  let best = Array.make(~length=base * 2 + 1, {old: -2, spans: list{}})
  let add = (k, n, previous) =>
    if n == 0 {
      previous
    } else if individual {
      let spans = ref(previous)
      for _index in 1 to n {
        spans := list{{kind: k, count: 1}, ...spans.contents}
      }
      spans.contents
    } else {
      switch previous {
      | list{h, ...t} if h.kind == k => list{{kind: k, count: h.count + n}, ...t}
      | _ => list{{kind: k, count: n}, ...previous}
      }
    }
  let common = (k, p) => {
    let i = ref(p.old)
    let j = ref(p.old - k)
    while (
      i.contents + 1 < Array.length(old) &&
        (j.contents + 1 < Array.length(newer) &&
        Array.getUnsafe(old, i.contents + 1) == Array.getUnsafe(newer, j.contents + 1))
    ) {
      i := i.contents + 1
      j := j.contents + 1
    }
    ({old: i.contents, spans: add(Keep, i.contents - p.old, p.spans)}, j.contents)
  }
  let (first, j) = common(0, {old: -1, spans: list{}})
  let finished = (p, j) => p.old + 1 == Array.length(old) && j + 1 == Array.length(newer)
  if finished(first, j) {
    List.reverse(first.spans)
  } else {
    best[base] = first
    let low = ref(0 - limit)
    let high = ref(limit)

    try {
      for d in 1 to limit {
        let k = ref(Math.Int.max(low.contents, -d))
        let stop = Math.Int.min(high.contents, d)
        while k.contents <= stop {
          let i = base + k.contents
          let r = Array.getUnsafe(best, i - 1)
          let a = Array.getUnsafe(best, i + 1)
          best[i - 1] = {old: -2, spans: list{}}
          let can_remove = r.old >= -1 && r.old + 1 < Array.length(old)

          let can_add =
            a.old >= -1 && (a.old - k.contents >= 0 && a.old - k.contents < Array.length(newer))
          if can_add || can_remove {
            let p = if !can_remove || (can_add && r.old < a.old) {
              {...a, spans: add(Add, 1, a.spans)}
            } else {
              {old: r.old + 1, spans: add(Remove, 1, r.spans)}
            }
            let (p, j) = common(k.contents, p)
            if finished(p, j) {
              throw(Done(List.reverse(p.spans)))
            }
            best[i] = p
            if p.old + 1 == Array.length(old) {
              high := Math.Int.min(high.contents, k.contents - 1)
            }
            if j + 1 == Array.length(newer) {
              low := Math.Int.max(low.contents, k.contents + 1)
            }
          } else {
            best[i] = {old: -2, spans: list{}}
          }
          k := k.contents + 2
        }
      }
      list{}
    } catch {
    | Done(spans) => spans
    }
  }
}

let sub = (s, i, j) => Text.substring(s, i, j - i)
let len = String.length
let cp = Host.cp
let space = c =>
  (9 <= c && c <= 13) ||
    (c == 32 ||
    (c == 160 ||
      (c == 0x1680 ||
      ((0x2000 <= c && c <= 0x200a) ||
        (c == 0x2028 ||
        (c == 0x2029 || (c == 0x202f || (c == 0x205f || (c == 0x3000 || c == 0xfeff)))))))))
let word = c =>
  (65 <= c && c <= 90) ||
    ((97 <= c && c <= 122) ||
    ((48 <= c && c <= 57) ||
      (c == 95 ||
      (c == 0xad ||
        ((0xc0 <= c && c <= 0xd6) ||
        ((0xd8 <= c && c <= 0xf6) ||
          ((0xf8 <= c && c <= 0x2c6) ||
          ((0x2c8 <= c && c <= 0x2d7) ||
            ((0x2de <= c && c <= 0x2ff) ||
            0x1e00 <= c && c <= 0x1eff)))))))))
let left_ws = s => {
  let rec loop = i =>
    if i == len(s) {
      i
    } else {
      let (c, n) = cp(s, i)
      if space(c) {
        loop(i + n)
      } else {
        i
      }
    }
  loop(0)
}
let right_ws = s => {
  let last = ref(0)
  let i = ref(0)
  while i.contents < len(s) {
    let (c, n) = cp(s, i.contents)
    i := i.contents + n
    if !space(c) {
      last := i.contents
    }
  }
  len(s) - last.contents
}
let cut_l = (s, n) => sub(s, n, len(s))
let cut_r = (s, n) => sub(s, 0, len(s) - n)
let left = s => sub(s, 0, left_ws(s))
let right = s => cut_l(s, len(s) - right_ws(s))
let trim = s => {
  let s = cut_l(s, left_ws(s))
  cut_r(s, right_ws(s))
}
let words = s => {
  let parts = ref(list{})
  let start = ref(0)
  let prior = ref(-1)
  let i = ref(0)
  while i.contents < len(s) {
    let (c, n) = cp(s, i.contents)
    let cls = if space(c) {
      0
    } else if word(c) {
      1
    } else {
      2
    }
    if cls != prior.contents || cls == 2 {
      if i.contents > start.contents {
        parts := list{sub(s, start.contents, i.contents), ...parts.contents}
      }
      start := i.contents
    }
    prior := cls
    i := i.contents + n
  }
  if start.contents < len(s) {
    parts := list{cut_l(s, start.contents), ...parts.contents}
  }
  let out = ref(list{})
  let prev = ref("")
  eachList(part => {
    let (c, _) = cp(part, 0)
    if space(c) {
      switch out.contents {
      | list{} => out := list{part}
      | list{h, ...t} => out := list{h ++ part, ...t}
      }
      prev := part
    } else {
      switch out.contents {
      | list{h} if prev.contents != "" && h == prev.contents => out := list{h ++ part}
      | _ => out := list{prev.contents ++ part, ...out.contents}
      }
      prev := ""
    }
  }, List.reverse(parts.contents))
  List.toArray(List.reverse(out.contents))
}
let line_tokens = (s, o) => {
  let out = ref(list{})
  let start = ref(0)
  for i in 0 to len(s) - 1 {
    if String.charAt(s, i) == "\n" {
      let e = if i > start.contents && String.charAt(s, i - 1) == "\r" {
        i - 1
      } else {
        i
      }
      if o.newline_token {
        if e > start.contents {
          out := list{sub(s, start.contents, e), ...out.contents}
        }
        out :=
          list{
            if o.strip_cr {
              "\n"
            } else {
              sub(s, e, i + 1)
            },
            ...out.contents,
          }
      } else {
        out :=
          list{
            if o.strip_cr && e < i {
              sub(s, start.contents, e) ++ "\n"
            } else {
              sub(s, start.contents, i + 1)
            },
            ...out.contents,
          }
      }
      start := i + 1
    }
  }
  if start.contents < len(s) {
    out := list{cut_l(s, start.contents), ...out.contents}
  }
  List.toArray(List.reverse(out.contents))
}
let tokenize = (op, s, o) =>
  if op == "diffWords" {
    words(s)
  } else if op == "diffLines" || op == "diffJson" {
    line_tokens(s, o)
  } else {
    let out = ref(list{})
    let i = ref(0)
    while i.contents < len(s) {
      let (_, n) = cp(s, i.contents)
      out := list{sub(s, i.contents, i.contents + n), ...out.contents}
      i := i.contents + n
    }
    List.toArray(List.reverse(out.contents))
  }
let key = (op, o, raw) => {
  let s = if (
    op == "diffWords" ||
      (op == "diffLines" && (o.ignore_ws && (!o.newline_token || !String.includes(raw, "\n"))))
  ) {
    trim(raw)
  } else if (
    op == "diffLines" && (o.ignore_eof && (!o.newline_token && Text.ends(~suffix="\n", raw)))
  ) {
    cut_r(raw, 1)
  } else {
    raw
  }
  let s = if op == "diffJson" {
    let b = Buffer.create(len(s))
    for i in 0 to len(s) - 1 {
      if (
        String.charAt(s, i) != "," ||
          (i + 1 == len(s) ||
          String.charAt(s, i + 1) != "\n" && String.charAt(s, i + 1) != "\r")
      ) {
        Buffer.add_char(b, String.charAt(s, i))
      }
    }
    Buffer.contents(b)
  } else {
    s
  }
  if o.ignore_case {
    Host.lower(s)
  } else {
    s
  }
}
type change = {span: span, mutable value: string}
let common_len = (a, b, suffix) => {
  let i = ref(0)
  let end_ = Math.Int.min(len(a), len(b))
  while (
    i.contents < end_ && if suffix {
        String.charAt(a, len(a) - 1 - i.contents) == String.charAt(b, len(b) - 1 - i.contents)
      } else {
        String.charAt(a, i.contents) == String.charAt(b, i.contents)
      }
  ) {
    i := i.contents + 1
  }
  if suffix {
    while (
      i.contents > 0 &&
        (i.contents < len(a) &&
        Int.bitwiseAnd(Host.byteCode(String.charAt(a, len(a) - i.contents)), 192) == 128)
    ) {
      i := i.contents - 1
    }
  } else {
    while (
      i.contents > 0 &&
        (i.contents < len(a) &&
        Int.bitwiseAnd(Host.byteCode(String.charAt(a, i.contents)), 192) == 128)
    ) {
      i := i.contents - 1
    }
  }
  i.contents
}
let overlap = (a, b) => {
  let rec loop = i =>
    if i == len(a) {
      0
    } else if Text.starts(~prefix=cut_l(a, i), b) {
      len(a) - i
    } else {
      let (_, n) = cp(a, i)
      loop(i + n)
    }
  loop(0)
}
let clean = changes => {
  let start = ref(None)
  let del = ref(None)
  let ins = ref(None)
  let adjust = ending =>
    switch (del.contents, ins.contents) {
    | (Some(d), Some(i)) =>
      let op = left(d.value)
      let np = left(i.value)
      let os = right(d.value)
      let ns = right(i.value)
      onSome(s => {
        let n = common_len(op, np, false)
        s.value = cut_r(s.value, len(np)) ++ sub(np, 0, n)
        d.value = cut_l(d.value, n)
        i.value = cut_l(i.value, n)
      }, start.contents)
      onSome(e => {
        let n = common_len(os, ns, true)
        e.value = cut_l(ns, len(ns) - n) ++ cut_l(e.value, len(ns))
        d.value = cut_r(d.value, n)
        i.value = cut_r(i.value, n)
      }, ending)
    | (None, Some(i)) =>
      if Option.isSome(start.contents) {
        i.value = cut_l(i.value, left_ws(i.value))
      }
      onSome(e => e.value = cut_l(e.value, left_ws(e.value)), ending)
    | (Some(d), None) =>
      switch (start.contents, ending) {
      | (Some(s), Some(e)) =>
        let w = left(e.value)
        let a = common_len(w, left(d.value), false)
        let b = common_len(cut_l(w, a), right(d.value), true)
        d.value = cut_r(cut_l(d.value, a), b)
        e.value = cut_l(e.value, len(w) - b)
        s.value = cut_r(s.value, b)
      | (Some(s), None) => d.value = cut_l(d.value, overlap(right(s.value), left(d.value)))
      | (None, Some(e)) => d.value = cut_r(d.value, overlap(right(d.value), left(e.value)))
      | _ => ()
      }
    | _ => ()
    }
  each(c =>
    switch c.span.kind {
    | Add => ins := Some(c)
    | Remove => del := Some(c)
    | Keep =>
      adjust(Some(c))
      start := Some(c)
      del := None
      ins := None
    }
  , changes)
  adjust(None)
}
let changes = (op, old, newer, o) => {
  let spans = myers(
    mapArray(x => key(op, o, x), old),
    mapArray(x => key(op, o, x), newer),
    o.individual,
  )
  let a = ref(0)
  let b = ref(0)
  let out = List.toArray(mapList(span => {
      let buf = Buffer.create(24)
      for j in 0 to span.count - 1 {
        let s = if (
          span.kind == Remove ||
            (op == "diffJson" &&
              (span.kind == Keep &&
              len(Array.getUnsafe(old, a.contents + j)) >
              len(Array.getUnsafe(newer, b.contents + j))))
        ) {
          Array.getUnsafe(old, a.contents + j)
        } else {
          Array.getUnsafe(newer, b.contents + j)
        }
        Buffer.add_string(
          buf,
          if op == "diffWords" && j > 0 {
            cut_l(s, left_ws(s))
          } else {
            s
          },
        )
      }
      if span.kind != Add {
        a := a.contents + span.count
      }
      if span.kind != Remove {
        b := b.contents + span.count
      }
      {span, value: Buffer.contents(buf)}
    }, spans))
  if op == "diffWords" && !o.individual {
    clean(out)
  }
  out
}
