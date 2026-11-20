// Huffman tree construction (`hTree`), depth clamping, RLE code-length encoding (`lc`), and
// bit-cost computation (`clen`) matching `fflate@0.8.3`.

open Tables

type rec hNode = {
  s: int,
  f: int,
  l?: hNode,
  r?: hNode,
}

// Walk tree assigning leaf depths into `l`, returning max depth (`ln`).
let rec ln = (n: hNode, l: Uint16Array.t, d: int): int =>
  switch (n.l, n.r) {
  | (Some(nl), Some(nr)) => Math.Int.max(ln(nl, l, d + 1), ln(nr, l, d + 1))
  | _ =>
    setU16(l, n.s, d)
    d
  }

// Build length-limited canonical Huffman code lengths from symbol frequencies (`hTree`).
let hTree = (d: Uint16Array.t, mb: int): (Uint8Array.t, int) => {
  let dLen = d->TypedArray.length
  let t: array<hNode> = []
  for i in 0 to dLen - 1 {
    let freq = getU16(d, i)
    if freq != 0 {
      t->Array.push({s: i, f: freq})
    }
  }
  let s = t->Array.length
  if s == 0 {
    (et, 0)
  } else if s == 1 {
    let sym = (t->Array.getUnsafe(0)).s
    let v = Uint8Array.fromLength(sym + 1)
    setU8(v, sym, 1)
    (v, 1)
  } else {
    let t2 = t->Array.copy
    let maxSym = (t2->Array.getUnsafe(s - 1)).s
    // Stable sort by ascending frequency (ties keep ascending symbol index `s`).
    t->Array.sort((a, b) => Int.toFloat(a.f - b.f))
    // Sentinel exceeding max symbol count per block (25000).
    t->Array.push({s: -1, f: 25001})
    let l0 = t->Array.getUnsafe(0)
    let r0 = t->Array.getUnsafe(1)
    t->Array.setUnsafe(0, {s: -1, f: l0.f + r0.f, l: l0, r: r0})
    let i0 = ref(0)
    let i1 = ref(1)
    let i2 = ref(2)
    while i1.contents != s - 1 {
      let a = t->Array.getUnsafe(i0.contents)
      let b = t->Array.getUnsafe(i2.contents)
      let l = if a.f < b.f {
        i0 := i0.contents + 1
        a
      } else {
        i2 := i2.contents + 1
        b
      }
      let r = if (
        i0.contents != i1.contents &&
          (t->Array.getUnsafe(i0.contents)).f < (t->Array.getUnsafe(i2.contents)).f
      ) {
        let node = t->Array.getUnsafe(i0.contents)
        i0 := i0.contents + 1
        node
      } else {
        let node = t->Array.getUnsafe(i2.contents)
        i2 := i2.contents + 1
        node
      }
      t->Array.setUnsafe(i1.contents, {s: -1, f: l.f + r.f, l, r})
      i1 := i1.contents + 1
    }
    let tr = Uint16Array.fromLength(maxSym + 1)
    let mbt = ref(ln(t->Array.getUnsafe(i1.contents - 1), tr, 0))
    if mbt.contents > mb {
      let curMbt = mbt.contents
      let lft = curMbt - mb
      let cst = shl(1, lft)
      // Stable sort by descending depth, then ascending frequency, then ascending symbol.
      t2->Array.sort((a, b) => {
        let diff = getU16(tr, b.s) - getU16(tr, a.s)
        Int.toFloat(diff != 0 ? diff : a.f - b.f)
      })
      let i = ref(0)
      let dt = ref(0)
      while i.contents < s && getU16(tr, (t2->Array.getUnsafe(i.contents)).s) > mb {
        let sym = (t2->Array.getUnsafe(i.contents)).s
        dt := dt.contents + cst - shl(1, curMbt - getU16(tr, sym))
        setU16(tr, sym, mb)
        i := i.contents + 1
      }
      dt := shr(dt.contents, lft)
      while dt.contents > 0 {
        let sym = (t2->Array.getUnsafe(i.contents)).s
        let curLen = getU16(tr, sym)
        if curLen < mb {
          dt := dt.contents - shl(1, mb - curLen - 1)
          setU16(tr, sym, curLen + 1)
        } else {
          i := i.contents + 1
        }
      }
      while i.contents >= 0 && dt.contents != 0 {
        let sym = (t2->Array.getUnsafe(i.contents)).s
        let curLen = getU16(tr, sym)
        if curLen == mb {
          setU16(tr, sym, curLen - 1)
          dt := dt.contents + 1
        }
        i := i.contents - 1
      }
      mbt := mb
    }
    (copyU8FromU16(tr), mbt.contents)
  }
}

// Run-length encode code lengths (`lc`). Empty input returns `([0], 1)`.
let lc = (c: Uint8Array.t): (Uint16Array.t, int) => {
  let len = c->TypedArray.length
  let s = ref(len)
  let trimming = ref(true)
  while trimming.contents && s.contents > 0 {
    s := s.contents - 1
    if getU8(c, s.contents) != 0 {
      trimming := false
    }
  }
  let sVal = s.contents + 1
  let cl = Uint16Array.fromLength(sVal)
  let cli = ref(0)
  let cln = ref(len > 0 ? getU8(c, 0) : 0)
  let cls = ref(1)
  for i in 1 to sVal {
    let ci = i < len ? getU8(c, i) : 0
    if i != sVal && ci == cln.contents {
      cls := cls.contents + 1
    } else {
      let curCln = cln.contents
      if curCln == 0 && cls.contents > 2 {
        while cls.contents > 138 {
          setU16(cl, cli.contents, 32754)
          cli := cli.contents + 1
          cls := cls.contents - 138
        }
        if cls.contents > 2 {
          let code =
            cls.contents > 10
              ? bor(shl(cls.contents - 11, 5), 28690)
              : bor(shl(cls.contents - 3, 5), 12305)
          setU16(cl, cli.contents, code)
          cli := cli.contents + 1
          cls := 0
        }
      } else if cls.contents > 3 {
        setU16(cl, cli.contents, curCln)
        cli := cli.contents + 1
        cls := cls.contents - 1
        while cls.contents > 6 {
          setU16(cl, cli.contents, 8304)
          cli := cli.contents + 1
          cls := cls.contents - 6
        }
        if cls.contents > 2 {
          setU16(cl, cli.contents, bor(shl(cls.contents - 3, 5), 8208))
          cli := cli.contents + 1
          cls := 0
        }
      }
      while cls.contents > 0 {
        setU16(cl, cli.contents, curCln)
        cli := cli.contents + 1
        cls := cls.contents - 1
      }
      cls := 1
      cln := ci
    }
  }
  (cl->TypedArray.subarray(~start=0, ~end=cli.contents), sVal)
}

// Compute total encoded bits from symbol frequencies and code lengths (`clen`).
let clen = (cf: Uint16Array.t, cl: Uint8Array.t): int => {
  let len = cl->TypedArray.length
  let total = ref(0)
  for i in 0 to len - 1 {
    total := total.contents + getU16(cf, i) * getU8(cl, i)
  }
  total.contents
}
