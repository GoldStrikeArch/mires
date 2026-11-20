open Support
open Differ
open Host
type hunk = {old_start: int, old_lines: int, new_start: int, new_lines: int, lines: array<string>}
type patch = {meta: Host.value, hunks: array<hunk>}
let starts = (s, prefix) => Text.starts(~prefix, s)
let ends = (s, suffix) => Text.ends(~suffix, s)
let getn = (o, k) => integer(get(o, k))
let getstr = (o, k) => read(get(o, k))
let prop = (o, k, v) => set(o, k, str(v))
let length = js => Host.length(js)
let array_of_js = (f, v) => Array.fromInitializer(~length=length(v), i => f(at(v, i)))
let array_to_js = (f, a) => {
  let v = array(Array.length(a))
  eachIndex((i, e) => put(v, i, f(e)), a)
  v
}
let update_numbers = (h, x, reverse) => {
  let write = (k, v) => set(x, k, number(v))
  if reverse {
    write("oldLines", h.old_lines)
    write("oldStart", h.old_start)
    write("newLines", h.new_lines)
    write("newStart", h.new_start)
  } else {
    write("oldStart", h.old_start)
    write("oldLines", h.old_lines)
    write("newStart", h.new_start)
    write("newLines", h.new_lines)
  }
}
let write = (reversed, p) => {
  let hunk = h => {
    let x = obj()
    update_numbers(h, x, reversed)
    set(x, "lines", array_to_js(str, h.lines))
    x
  }
  set(p.meta, "hunks", array_to_js(hunk, p.hunks))
  p.meta
}
let read_patch = v => {
  let meta = obj()
  let ks = keys(v)
  for i in 0 to length(ks) - 1 {
    let k = at(ks, i)
    key_set(meta, k, key_get(v, k))
  }
  let h = x => {
    old_start: getn(x, "oldStart"),
    old_lines: getn(x, "oldLines"),
    new_start: getn(x, "newStart"),
    new_lines: getn(x, "newLines"),
    lines: array_of_js(read, get(x, "lines")),
  }
  {meta, hunks: array_of_js(h, get(v, "hunks"))}
}
let create = (input, context) => {
  let old = line_tokens(getstr(input, "before"), defaults)

  let newer = line_tokens(getstr(input, "after"), defaults)
  let spans = List.toArray(myers(old, newer, false))
  let hunks = ref(list{})

  let old_start = ref(0)

  let new_start = ref(0)

  let old_lines = ref(0)

  let new_lines = ref(0)

  let output = ref(list{})
  let emit = (kind, src, a, n) => {
    let mark = switch kind {
    | Keep => " "
    | Add => "+"
    | Remove => "-"
    }
    for i in a to a + n - 1 {
      let s = Array.getUnsafe(src, i)
      if ends(s, "\n") {
        output := list{mark ++ cut_r(s, 1), ...output.contents}
      } else {
        output := list{"\\ No newline at end of file", mark ++ s, ...output.contents}
      }
    }
    if kind != Add {
      old_lines := old_lines.contents + n
    }
    if kind != Remove {
      new_lines := new_lines.contents + n
    }
  }
  let flush = () => {
    hunks :=
      list{
        {
          old_start: old_start.contents,
          new_start: new_start.contents,
          old_lines: old_lines.contents,
          new_lines: new_lines.contents,
          lines: List.toArray(List.reverse(output.contents)),
        },
        ...hunks.contents,
      }
    old_start := 0
    old_lines := 0
    new_lines := 0
    output := list{}
  }
  let a = ref(0)
  let b = ref(0)
  let kept = ref(0)
  eachIndex((i, r) => {
    if r.kind == Keep {
      if old_start.contents != 0 {
        let close = i + 1 == Array.length(spans) || r.count > context * 2
        emit(
          Keep,
          newer,
          b.contents,
          if close {
            Math.Int.min(r.count, context)
          } else {
            r.count
          },
        )
        if close {
          flush()
        }
      }
      kept := r.count
    } else {
      if old_start.contents == 0 {
        let k = Math.Int.min(context, kept.contents)
        old_start := a.contents + 1 - k
        new_start := b.contents + 1 - k
        emit(Keep, newer, b.contents - k, k)
      }
      if r.kind == Add {
        emit(Add, newer, b.contents, r.count)
      } else {
        emit(Remove, old, a.contents, r.count)
      }
    }
    if r.kind != Add {
      a := a.contents + r.count
    }
    if r.kind != Remove {
      b := b.contents + r.count
    }
  }, spans)
  if old_start.contents != 0 {
    flush()
  }
  let meta = obj()
  eachList(
    k => set(meta, k, get(input, k)),
    list{"oldFileName", "newFileName", "oldHeader", "newHeader"},
  )
  {meta, hunks: List.toArray(List.reverse(hunks.contents))}
}

let quote = text => {
  let s = Host.scalarText(text)
  let needed = c => {
    let n = Host.byteCode(c)
    n < 32 || (n > 126 || (n == 34 || n == 92))
  }
  if !Text.exists(needed, s) {
    s
  } else {
    let b = Buffer.create(len(s) + 5)
    Buffer.add_char(b, "\"")
    Text.each(c => {
      let n = Host.byteCode(c)
      if n >= 7 && n <= 13 {
        Buffer.add_char(b, "\\")
        Buffer.add_char(b, String.charAt("abtnvfr", n - 7))
      } else if n == 34 || n == 92 {
        Buffer.add_char(b, "\\")
        Buffer.add_char(b, c)
      } else if n < 32 || n > 126 {
        Buffer.add_char(b, "\\")
        eachList(
          k =>
            Buffer.add_char(
              b,
              String.fromCharCode(48 + Int.bitwiseAnd(Int.shiftRightUnsigned(n, k), 7)),
            ),
          list{6, 3, 0},
        )
      } else {
        Buffer.add_char(b, c)
      }
    }, s)
    Buffer.add_char(b, "\"")
    Buffer.contents(b)
  }
}
let quoted = s =>
  if !starts(s, "\"") {
    None
  } else {
    let b = Buffer.create(len(s))
    let oct = c => c >= "0" && c <= "7"
    let rec scan = i =>
      if i >= len(s) {
        None
      } else {
        switch String.charAt(s, i) {
        | "\"" => Some(Buffer.contents(b), i + 1)
        | "\\" if i + 1 < len(s) =>
          let c = String.charAt(s, i + 1)
          if oct(c) {
            let bytes = []
            let rec octets = j => {
              if j + 1 < len(s) && String.charAt(s, j) == "\\" && oct(String.charAt(s, j+1)) {
                if j + 3 >= len(s) || !oct(String.charAt(s, j+2)) || !oct(String.charAt(s, j+3)) {
                  None
                } else {
                  let d = n => Host.byteCode(String.charAt(s, j+n)) - 48
                  bytes->Array.push(Int.bitwiseAnd(d(1)*64 + d(2)*8 + d(3), 255))->ignore
                  octets(j+4)
                }
              } else {
                Buffer.add_string(b, Host.decodeOctets(bytes))
                scan(j)
              }
            }
            octets(i)
          } else if c == "\"" || c == "\\" {
            Buffer.add_char(b, c)
            scan(i + 2)
          } else {
            switch Text.locate("abtnvfr", c) {
            | None => None
            | Some(j) =>
              Buffer.add_char(b, String.fromCharCode(j + 7))
              scan(i + 2)
            }
          }
        | "\\" => None
        | c =>
          Buffer.add_char(b, c)
          scan(i + 1)
        }
      }
    scan(1)
  }
let unquote = s =>
  switch quoted(s) {
  | None => s
  | Some(n, _) => n
  }
let format = p => {
  let b = Buffer.create(256)
  let push = s => Buffer.add_string(b, s)
  let append = k => {
    push(k)
    push("\n")
  }
  let old = get(p.meta, "oldFileName")
  let newer = get(p.meta, "newFileName")
  if !(absent(old) || absent(newer)) && read(old) == read(newer) {
    append("Index: " ++ read(old))
  }
  append("===================================================================")
  if !(absent(old) || absent(newer)) {
    eachList(((tag, k, h)) => {
      let header = get(p.meta, h)
      append(
        tag ++
        (quote(getstr(p.meta, k)) ++
        if truth(header) {
          "\t" ++ read(header)
        } else {
          ""
        }),
      )
    }, list{("--- ", "oldFileName", "oldHeader"), ("+++ ", "newFileName", "newHeader")})
  }
  each(h => {
    let range = (start, n) =>
      Int.toString(
        start - if n == 0 {
          1
        } else {
          0
        },
      ) ++
      ("," ++
      Int.toString(n))
    append(
      "@@ -" ++
      (range(h.old_start, h.old_lines) ++
      (" +" ++ (range(h.new_start, h.new_lines) ++ " @@"))),
    )
    each(append, h.lines)
  }, p.hunks)
  Buffer.contents(b)
}
let git_names = (p, s) => {
  let names = (a, b) => {
    prop(p, "oldFileName", a)
    prop(p, "newFileName", b)
  }
  switch quoted(s) {
  | Some(a, n) =>
    let b = trim(cut_l(s, n))
    if b != "" {
      names(a, unquote(b))
    }
  | None =>
    switch Text.locate(s, "\"") {
    | Some(j) if j > 0 =>
      switch quoted(cut_l(s, j)) {
      | Some(b, _) => names(trim(sub(s, 0, j)), b)
      | _ => ()
      }
    | _ =>
      if starts(s, "a/") {
        let splits = ref(list{})
        for i in 0 to len(s) - 3 {
          if sub(s, i, i + 3) == " b/" {
            splits := list{i, ...splits.contents}
          }
        }
        let a = List.toArray(List.reverse(splits.contents))
        if Array.length(a) > 0 {
          let i = Array.getUnsafe(a, Array.length(a) / 2)
          names(sub(s, 0, i), cut_l(s, i + 1))
        }
      }
    }
  }
}
let extended = (p, l) => {
  eachList(((tag, name, flag, part)) =>
    if starts(l, tag) {
      prop(p, name, part ++ unquote(cut_l(l, len(tag))))
      set(p, flag, boolean(true))
    }
  , list{
    ("rename from ", "oldFileName", "isRename", "a/"),
    ("rename to ", "newFileName", "isRename", "b/"),
    ("copy from ", "oldFileName", "isCopy", "a/"),
    ("copy to ", "newFileName", "isCopy", "b/"),
  })
  eachList(((tag, flag, name)) =>
    if starts(l, tag) {
      if flag != "" {
        set(p, flag, boolean(true))
      }
      prop(p, name, cut_l(l, len(tag)))
    }
  , list{
    ("new file mode ", "isCreate", "newMode"),
    ("deleted file mode ", "isDelete", "oldMode"),
    ("old mode ", "", "oldMode"),
    ("new mode ", "", "newMode"),
  })
  if starts(l, "Binary files ") {
    set(p, "isBinary", boolean(true))
  }
}
let file = l =>
  len(l) > 3 &&
    ((starts(l, "---") || starts(l, "+++")) &&
    (String.charAt(l, 3) == " " || String.charAt(l, 3) == "\t"))
let header = l => starts(l, "diff --git ") || (starts(l, "Index: ") || starts(l, "diff -r "))
let line_mark = l =>
  if l == "" {
    " "
  } else {
    String.charAt(l, 0)
  }
let parse_file = (p, l) => {
  let parts = Text.split("\t", trim(cut_l(l, 3)))
  let s = List.headOrThrow(parts)
  let name = if starts(s, "\"") {
    unquote(s)
  } else {
    let b = Buffer.create(len(s))
    let i = ref(0)
    while i.contents < len(s) {
      Buffer.add_char(b, String.charAt(s, i.contents))
      if (
        String.charAt(s, i.contents) == "\\" &&
          (i.contents + 1 < len(s) &&
          String.charAt(s, i.contents + 1) == "\\")
      ) {
        i := i.contents + 1
      }
      i := i.contents + 1
    }
    Buffer.contents(b)
  }
  let side = if String.charAt(l, 0) == "-" {
    "old"
  } else {
    "new"
  }
  prop(p, side ++ "FileName", name)
  prop(
    p,
    side ++ "Header",
    switch parts {
    | list{_, h, ..._} => trim(h)
    | _ => ""
    },
  )
}
let parse_hunk = (body, i) => {
  let l = Array.getUnsafe(body, i.contents)
  i := i.contents + 1
  let range = s =>
    switch Text.split(",", cut_l(s, 1)) {
    | list{a} => (parseInt(a), 1)
    | list{a, b} =>
      let n = parseInt(b)
      (
        parseInt(a) + if n == 0 {
          1
        } else {
          0
        },
        n,
      )
    | _ => invalid("hunk range")
    }
  let (old_start, o, new_start, n) = switch Text.split(" ", l) {
  | list{"@@", a, b, "@@", ..._} if starts(a, "-") && starts(b, "+") =>
    let (x, y) = range(a)
    let (z, t) = range(b)
    (x, y, z, t)
  | _ => invalid("hunk header")
  }
  let old = ref(0)
  let newer = ref(0)
  let lines = ref(list{})
  while (
    i.contents < Array.length(body) &&
      (old.contents < o || (newer.contents < n || starts(Array.getUnsafe(body, i.contents), "\\")))
  ) {
    let s = Array.getUnsafe(body, i.contents)
    if s == "" && i.contents + 1 == Array.length(body) {
      invalid("line")
    }
    switch line_mark(s) {
    | "+" => newer := newer.contents + 1
    | "-" => old := old.contents + 1
    | " " =>
      old := old.contents + 1
      newer := newer.contents + 1
    | "\\" => ()
    | _ => invalid("line")
    }
    lines := list{s, ...lines.contents}
    i := i.contents + 1
  }
  let old_lines = if old.contents == 0 && o == 1 {
    0
  } else {
    o
  }

  let new_lines = if newer.contents == 0 && n == 1 {
    0
  } else {
    n
  }
  if old_lines != old.contents || new_lines != newer.contents {
    invalid("line count")
  }
  if (
    i.contents < Array.length(body) &&
      (Array.getUnsafe(body, i.contents) != "" &&
      (String.includes("+ -", line_mark(Array.getUnsafe(body, i.contents))) &&
      !file(Array.getUnsafe(body, i.contents))))
  ) {
    invalid("extra line")
  }
  {old_start, old_lines, new_start, new_lines, lines: List.toArray(List.reverse(lines.contents))}
}
let parse = s => {
  let body = List.toArray(Text.split("\n", s))
  let out = ref(list{})
  let i = ref(0)
  while i.contents < Array.length(body) {
    let meta = obj()
    let hunks = ref(list{})
    let seen = ref(false)
    set(meta, "hunks", undef)
    while (
      i.contents < Array.length(body) &&
        !(
          file(Array.getUnsafe(body, i.contents)) ||
          (starts(Array.getUnsafe(body, i.contents), "@@ ") ||
          seen.contents && header(Array.getUnsafe(body, i.contents)))
        )
    ) {
      let l = Array.getUnsafe(body, i.contents)
      if header(l) {
        seen := true
        if starts(l, "diff --git ") {
          set(meta, "isGit", boolean(true))
          git_names(meta, cut_l(l, 11))
        } else if starts(l, "Index: ") {
          prop(meta, "index", trim(cut_l(l, 7)))
        } else {
          prop(meta, "index", cut_l(l, 1 + String.lastIndexOf(l, " ")))
        }
      } else if truth(get(meta, "isGit")) {
        extended(meta, l)
      }
      i := i.contents + 1
    }
    for _index in 0 to 1 {
      if i.contents < Array.length(body) && file(Array.getUnsafe(body, i.contents)) {
        parse_file(meta, Array.getUnsafe(body, i.contents))
        i := i.contents + 1
      }
    }
    if absent(get(meta, "oldFileName")) != absent(get(meta, "newFileName")) {
      invalid("file header")
    }
    while (
      i.contents < Array.length(body) &&
        !(
          header(Array.getUnsafe(body, i.contents)) ||
          (file(Array.getUnsafe(body, i.contents)) ||
          starts(
            Array.getUnsafe(body, i.contents),
            "===================================================================",
          ))
        )
    ) {
      if starts(Array.getUnsafe(body, i.contents), "@@ ") {
        hunks := list{parse_hunk(body, i), ...hunks.contents}
      } else {
        i := i.contents + 1
      }
    }
    out := list{{meta, hunks: List.toArray(List.reverse(hunks.contents))}, ...out.contents}
  }
  List.toArray(List.reverse(out.contents))
}
let reverse = v => {
  let p = read_patch(v)
  let copy = truth(get(v, "isCopy"))
  let meta = obj()
  let ks = keys(v)
  for i in 0 to length(ks) - 1 {
    let k = at(ks, i)
    let name = read(k)
    if !(copy && inList(name, list{"isCreate", "isCopy", "isRename"})) {
      key_set(meta, k, key_get(v, k))
    }
  }
  eachList(((k, other)) => {
    let a = get(v, other)
    if Text.ends(~suffix="FileName", k) && (truth(get(v, "isGit")) && !absent(a)) {
      let n = read(a)
      prop(
        meta,
        k,
        if starts(n, "a/") {
          "b/" ++ cut_l(n, 2)
        } else if starts(n, "b/") {
          "a/" ++ cut_l(n, 2)
        } else {
          n
        },
      )
    } else {
      set(meta, k, a)
    }
  }, list{
    ("oldFileName", "newFileName"),
    ("oldHeader", "newHeader"),
    ("newFileName", "oldFileName"),
    ("newHeader", "oldHeader"),
    ("oldMode", "newMode"),
    ("newMode", "oldMode"),
  })
  if !copy {
    set(meta, "isCreate", get(v, "isDelete"))
  }
  set(
    meta,
    "isDelete",
    if copy {
      boolean(true)
    } else {
      get(v, "isCreate")
    },
  )
  let hunks = if copy {
    prop(meta, "newFileName", "/dev/null")
    set(meta, "newHeader", undef)
    []
  } else {
    mapArray(h => {
      old_lines: h.new_lines,
      old_start: h.new_start,
      new_lines: h.old_lines,
      new_start: h.old_start,
      lines: mapArray(l =>
        if starts(l, "+") {
          "-" ++ cut_l(l, 1)
        } else if starts(l, "-") {
          "+" ++ cut_l(l, 1)
        } else {
          l
        }
      , h.lines),
    }, p.hunks)
  }
  write(true, {meta, hunks})
}
let convert = (source, hunks) => {
  let win = ref(false)
  let unix = ref(false)
  Text.eachIndex((i, c) =>
    if c == "\n" {
      if i > 0 && String.charAt(source, i - 1) == "\r" {
        win := true
      } else {
        unix := true
      }
    }
  , source)
  if win.contents != unix.contents {
    let pw = ref(false)
    let pu = ref(false)
    let marker = (h, i) =>
      i + 1 < Array.length(h.lines) && starts(Array.getUnsafe(h.lines, i + 1), "\\")
    each(h => eachIndex((i, l) =>
        if !starts(l, "\\") {
          if ends(l, "\r") {
            pw := true
          } else if !marker(h, i) {
            pu := true
          }
        }
      , h.lines), hunks)
    each(h => eachIndex((i, l) =>
        if win.contents && (!pw.contents && !(starts(l, "\\") || marker(h, i))) {
          h.lines[i] = l ++ "\r"
        } else if unix.contents && (pw.contents && (!pu.contents && ends(l, "\r"))) {
          h.lines[i] = cut_r(l, 1)
        }
      , h.lines), hunks)
  }
}
exception Mismatch
let apply = (source, p) => {
  if Array.length(p.hunks) == 0 {
    str(source)
  } else {
    try {
      convert(source, p.hunks)
      let data = List.toArray(Text.split("\n", source))
      let ae = ref(false)
      let re = ref(false)
      let prev = ref(" ")
      each(l => {
        if starts(l, "\\") {
          if prev.contents == "-" {
            ae := true
          }
          if prev.contents == "+" {
            re := true
          }
        }
        prev := line_mark(l)
      }, Array.getUnsafe(p.hunks, Array.length(p.hunks) - 1).lines)
      let eof = Array.getUnsafe(data, Array.length(data) - 1) == ""
      let data = switch (re.contents, ae.contents) {
      | (true, true) if eof => throw(Mismatch)
      | (true, false) =>
        if !eof {
          throw(Mismatch)
        }
        arraySlice(data, 0, Array.length(data) - 1)
      | (false, true) =>
        if eof {
          throw(Mismatch)
        }
        Array.concat(data, [""])
      | _ => data
      }
      let min_line = ref(0)
      let offset = ref(0)
      let result = ref(list{})
      each(h => {
        let max_line = Array.length(data) - h.old_lines
        if max_line < min_line.contents {
          throw(Mismatch)
        }
        let first = Math.Int.max(
          min_line.contents,
          Math.Int.min(max_line, h.old_start + offset.contents - 1),
        )
        let fits = at => {
          let i = ref(at)
          let ok = ref(true)
          each(l =>
            if ok.contents && (line_mark(l) == "-" || line_mark(l) == " ") {
              if (
                i.contents >= Array.length(data) ||
                  Array.getUnsafe(data, i.contents) != if l == "" {
                      ""
                    } else {
                      cut_l(l, 1)
                    }
              ) {
                ok := false
              }
              i := i.contents + 1
            }
          , h.lines)
          ok.contents
        }
        let d = ref(0)
        let pos = ref(-1)
        while (
          pos.contents < 0 &&
            d.contents <= Math.Int.max(first - min_line.contents, max_line - first)
        ) {
          if first + d.contents <= max_line && fits(first + d.contents) {
            pos := first + d.contents
          } else if (
            first - d.contents >= min_line.contents && (d.contents > 0 && fits(first - d.contents))
          ) {
            pos := first - d.contents
          }
          d := d.contents + 1
        }
        if pos.contents < 0 {
          throw(Mismatch)
        }
        for i in min_line.contents to pos.contents - 1 {
          result := list{Array.getUnsafe(data, i), ...result.contents}
        }
        let cur = ref(pos.contents)
        let ctx = ref(0)
        each(l =>
          switch line_mark(l) {
          | " " =>
            result := list{Array.getUnsafe(data, cur.contents), ...result.contents}
            ctx := ctx.contents + 1
            cur := cur.contents + 1
          | "-" =>
            cur := cur.contents + 1
            ctx := 0
          | "+" =>
            result := list{cut_l(l, 1), ...result.contents}
            ctx := 0
          | _ => ()
          }
        , h.lines)
        for _index in 1 to ctx.contents {
          result := List.tailOrThrow(result.contents)
        }
        min_line := cur.contents - ctx.contents
        offset := pos.contents + 1 - h.old_start
      }, p.hunks)
      for i in min_line.contents to Array.length(data) - 1 {
        result := list{Array.getUnsafe(data, i), ...result.contents}
      }
      str(Text.concat("\n", List.reverse(result.contents)))
    } catch {
    | Mismatch => boolean(false)
    }
  }
}
