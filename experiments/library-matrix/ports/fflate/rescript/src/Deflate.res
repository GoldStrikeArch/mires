// Raw DEFLATE compressor (`wfblk`, `wblk`, `dflt`, `dopt`, `deflateSync`) matching `fflate@0.8.3`.

open Tables
open Huffman

type compressOptions = {
  level?: int,
  mem?: int,
  dictionary?: Uint8Array.t,
  mtime?: float,
  filename?: string,
}

type deflateState = {
  mutable l: int,
  mutable i: int,
  mutable w: int,
  mutable z: int,
  mutable r: int,
  mutable h: option<Uint16Array.t>,
  mutable p: option<Uint16Array.t>,
}

let makeDeflateState = (~l=0, ~i=0, ~w=0, ~z=0): deflateState => {
  l,
  i,
  w,
  z,
  r: 0,
  h: None,
  p: None,
}

// Write an uncompressed (stored, BTYPE=0) block (`wfblk`), returning the new bit position.
let wfblk = (out: Uint8Array.t, pos: int, dat: Uint8Array.t): int => {
  let s = dat->TypedArray.length
  let o = shft(pos + 2)
  let lo = band(s, 255)
  let hi = shr(s, 8)
  setU8(out, o, lo)
  setU8(out, o + 1, hi)
  setU8(out, o + 2, bxor(lo, 255))
  setU8(out, o + 3, bxor(hi, 255))
  for i in 0 to s - 1 {
    setU8(out, o + i + 4, getU8(dat, i))
  }
  (o + 4 + s) * 8
}

// Emit a run-length-encoded code-length sequence (`lclt` or `lcdt`), returning new bit position.
let emitClct = (
  out: Uint8Array.t,
  llm: Uint16Array.t,
  lct: Uint8Array.t,
  clct: Uint16Array.t,
  pInit: int,
): int => {
  let p = ref(pInit)
  let clLen = clct->TypedArray.length
  for i in 0 to clLen - 1 {
    let entry = getU16(clct, i)
    let symLen = band(entry, 31)
    wbits(out, p.contents, getU16(llm, symLen))
    p := p.contents + getU8(lct, symLen)
    if symLen > 15 {
      wbits(out, p.contents, band(shr(entry, 5), 127))
      p := p.contents + shr(entry, 12)
    }
  }
  p.contents
}

// Select optimal block encoding (stored / dynamic / fixed) and emit symbols (`wblk`).
let wblk = (
  dat: Uint8Array.t,
  out: Uint8Array.t,
  final: int,
  syms: Int32Array.t,
  lf: Uint16Array.t,
  df: Uint16Array.t,
  eb: int,
  li: int,
  bs: int,
  bl: int,
  pInit: int,
): int => {
  wbits(out, pInit, final)
  let p = ref(pInit + 1)
  setU16(lf, 256, getU16(lf, 256) + 1)
  let (dlt, mlb) = hTree(lf, 15)
  let (ddt, mdb) = hTree(df, 15)
  let (lclt, nlc) = lc(dlt)
  let (lcdt, ndc) = lc(ddt)
  let lcfreq = Uint16Array.fromLength(19)
  let lcltLen = lclt->TypedArray.length
  for i in 0 to lcltLen - 1 {
    let idx = band(getU16(lclt, i), 31)
    setU16(lcfreq, idx, getU16(lcfreq, idx) + 1)
  }
  let lcdtLen = lcdt->TypedArray.length
  for i in 0 to lcdtLen - 1 {
    let idx = band(getU16(lcdt, i), 31)
    setU16(lcfreq, idx, getU16(lcfreq, idx) + 1)
  }
  let (lct, mlcb) = hTree(lcfreq, 7)
  let lctLen = lct->TypedArray.length
  let nlcc = ref(19)
  let trimming = ref(true)
  while trimming.contents && nlcc.contents > 4 {
    let sym = getU8(clim, nlcc.contents - 1)
    if sym >= lctLen || getU8(lct, sym) == 0 {
      nlcc := nlcc.contents - 1
    } else {
      trimming := false
    }
  }
  let nlccVal = nlcc.contents
  let flen = shl(bl + 5, 3)
  let ftlen = clen(lf, flt) + clen(df, fdt) + eb
  let dtlen =
    clen(lf, dlt) +
    clen(df, ddt) +
    eb +
    14 +
    3 * nlccVal +
    clen(lcfreq, lct) +
    2 * getU16(lcfreq, 16) +
    3 * getU16(lcfreq, 17) +
    7 * getU16(lcfreq, 18)
  if bs >= 0 && flen <= ftlen && flen <= dtlen {
    wfblk(out, p.contents, dat->TypedArray.subarray(~start=bs, ~end=bs + bl))
  } else {
    let isDynamic = dtlen < ftlen
    wbits(out, p.contents, isDynamic ? 2 : 1)
    p := p.contents + 2
    let (lm, ll, dm, dl) = if isDynamic {
      let lm = hMap(dlt, mlb, false)
      let dm = hMap(ddt, mdb, false)
      let llm = hMap(lct, mlcb, false)
      wbits(out, p.contents, nlc - 257)
      wbits(out, p.contents + 5, ndc - 1)
      wbits(out, p.contents + 10, nlccVal - 4)
      p := p.contents + 14
      for i in 0 to nlccVal - 1 {
        let sym = getU8(clim, i)
        wbits(out, p.contents + 3 * i, sym < lctLen ? getU8(lct, sym) : 0)
      }
      p := p.contents + 3 * nlccVal
      p := emitClct(out, llm, lct, lclt, p.contents)
      p := emitClct(out, llm, lct, lcdt, p.contents)
      (lm, dlt, dm, ddt)
    } else {
      (flm, flt, fdm, fdt)
    }
    let curP = ref(p.contents)
    for i in 0 to li - 1 {
      let sym = getI32(syms, i)
      if sym > 255 {
        let lenIdx = band(shr(sym, 18), 31)
        wbits16(out, curP.contents, getU16(lm, lenIdx + 257))
        curP := curP.contents + getU8(ll, lenIdx + 257)
        if lenIdx > 7 {
          wbits(out, curP.contents, band(shr(sym, 23), 31))
          curP := curP.contents + getU8(fleb, lenIdx)
        }
        let dstIdx = band(sym, 31)
        wbits16(out, curP.contents, getU16(dm, dstIdx))
        curP := curP.contents + getU8(dl, dstIdx)
        if dstIdx > 3 {
          wbits16(out, curP.contents, band(shr(sym, 5), 8191))
          curP := curP.contents + getU8(fdeb, dstIdx)
        }
      } else {
        wbits16(out, curP.contents, getU16(lm, sym))
        curP := curP.contents + getU8(ll, sym)
      }
    }
    wbits16(out, curP.contents, getU16(lm, 256))
    curP.contents + getU8(ll, 256)
  }
}

// Three-byte rolling hash (`hsh`).
let hsh = (dat: Uint8Array.t, idx: int, bs1: int, bs2: int, msk: int): int =>
  band(
    bxor(bxor(getU8(dat, idx), shl(getU8(dat, idx + 1), bs1)), shl(getU8(dat, idx + 2), bs2)),
    msk,
  )

// Core LZ77 + DEFLATE compressor (`dflt`).
let dflt = (
  dat: Uint8Array.t,
  lvl: int,
  plvl: int,
  pre: int,
  post: int,
  st: deflateState,
): Uint8Array.t => {
  let s = st.z != 0 ? st.z : dat->TypedArray.length
  let o = Uint8Array.fromLength(
    pre + s + 5 * (1 + Math.ceil(s->Int.toFloat /. 7000.)->Int.fromFloat) + post,
  )
  let w = o->TypedArray.subarray(~start=pre, ~end=o->TypedArray.length - post)
  let lst = st.l
  let pos = ref(band(st.r, 7))
  if lvl != 0 {
    if pos.contents != 0 {
      setU8(w, 0, shr(st.r, 3))
    }
    let opt = getI32(deo, lvl - 1)
    let n = shr(opt, 13)
    let c = band(opt, 8191)
    let msk = shl(1, plvl) - 1
    let prev = switch st.p {
    | Some(p) => p
    | None => Uint16Array.fromLength(32768)
    }
    let head = switch st.h {
    | Some(h) => h
    | None => Uint16Array.fromLength(msk + 1)
    }
    let bs1 = Math.ceil(plvl->Int.toFloat /. 3.)->Int.fromFloat
    let bs2 = 2 * bs1
    let syms = Int32Array.fromLength(25000)
    let lf = Uint16Array.fromLength(288)
    let df = Uint16Array.fromLength(32)
    let lcCnt = ref(0)
    let eb = ref(0)
    let i = ref(st.i)
    let li = ref(0)
    let wi = ref(st.w)
    let bs = ref(0)
    while i.contents + 2 < s {
      let curI = i.contents
      let hv = hsh(dat, curI, bs1, bs2, msk)
      let imod = ref(band(curI, 32767))
      let pimod = ref(getU16(head, hv))
      setU16(prev, imod.contents, pimod.contents)
      setU16(head, hv, imod.contents)
      if wi.contents <= curI {
        let rem = s - curI
        if (lcCnt.contents > 7000 || li.contents > 24576) && (rem > 423 || lst == 0) {
          pos :=
            wblk(
              dat,
              w,
              0,
              syms,
              lf,
              df,
              eb.contents,
              li.contents,
              bs.contents,
              curI - bs.contents,
              pos.contents,
            )
          li := 0
          lcCnt := 0
          eb := 0
          bs := curI
          for j in 0 to 285 {
            setU16(lf, j, 0)
          }
          for j in 0 to 29 {
            setU16(df, j, 0)
          }
        }
        let l = ref(2)
        let d = ref(0)
        let ch = ref(c)
        let dif = ref(band(imod.contents - pimod.contents, 32767))
        if rem > 2 && hv == hsh(dat, curI - dif.contents, bs1, bs2, msk) {
          let maxn = Math.Int.min(n, rem) - 1
          let maxd = Math.Int.min(32767, curI)
          let ml = Math.Int.min(258, rem)
          let satisfied = ref(false)
          while (
            !satisfied.contents &&
            dif.contents <= maxd &&
            ch.contents > 1 &&
            imod.contents != pimod.contents
          ) {
            ch := ch.contents - 1
            let curDif = dif.contents
            let curL = l.contents
            if getU8(dat, curI + curL) == getU8(dat, curI + curL - curDif) {
              let nl = ref(0)
              let base = curI - curDif
              while (
                nl.contents < ml &&
                  getU8(dat, curI + nl.contents) == getU8(dat, base + nl.contents)
              ) {
                nl := nl.contents + 1
              }
              let nlVal = nl.contents
              if nlVal > curL {
                l := nlVal
                d := curDif
                if nlVal > maxn {
                  satisfied := true
                } else {
                  let mmd = Math.Int.min(curDif, nlVal - 2)
                  let md = ref(0)
                  for j in 0 to mmd - 1 {
                    let ti = band(base + j, 32767)
                    let pti = getU16(prev, ti)
                    let cd = band(ti - pti, 32767)
                    if cd > md.contents {
                      md := cd
                      pimod := ti
                    }
                  }
                }
              }
            }
            if !satisfied.contents {
              let nextImod = pimod.contents
              let nextPimod = getU16(prev, nextImod)
              imod := nextImod
              pimod := nextPimod
              dif := curDif + band(nextImod - nextPimod, 32767)
            }
          }
        }
        if d.contents != 0 {
          let rfl = getI32(revfl, l.contents)
          let rfd = getI32(revfd, d.contents)
          setI32(syms, li.contents, bor(bor(268435456, shl(rfl, 18)), rfd))
          li := li.contents + 1
          let lin = band(rfl, 31)
          let din = band(rfd, 31)
          eb := eb.contents + getU8(fleb, lin) + getU8(fdeb, din)
          setU16(lf, 257 + lin, getU16(lf, 257 + lin) + 1)
          setU16(df, din, getU16(df, din) + 1)
          wi := curI + l.contents
          lcCnt := lcCnt.contents + 1
        } else {
          let b = getU8(dat, curI)
          setI32(syms, li.contents, b)
          li := li.contents + 1
          setU16(lf, b, getU16(lf, b) + 1)
        }
      }
      i := curI + 1
    }
    let iEnd = ref(Math.Int.max(i.contents, wi.contents))
    while iEnd.contents < s {
      let b = getU8(dat, iEnd.contents)
      setI32(syms, li.contents, b)
      li := li.contents + 1
      setU16(lf, b, getU16(lf, b) + 1)
      iEnd := iEnd.contents + 1
    }
    pos :=
      wblk(
        dat,
        w,
        lst,
        syms,
        lf,
        df,
        eb.contents,
        li.contents,
        bs.contents,
        iEnd.contents - bs.contents,
        pos.contents,
      )
    if lst == 0 {
      st.r = bor(band(pos.contents, 7), shl(getU8(w, shr(pos.contents, 3)), 3))
      pos := pos.contents - 7
      st.h = Some(head)
      st.p = Some(prev)
      st.i = iEnd.contents
      st.w = wi.contents
    }
  } else {
    let i = ref(st.w)
    while i.contents < s + lst {
      let e = ref(i.contents + 65535)
      if e.contents >= s {
        setU8(w, shr(pos.contents, 3), lst)
        e := s
      }
      pos := wfblk(w, pos.contents + 1, dat->TypedArray.subarray(~start=i.contents, ~end=e.contents))
      i := i.contents + 65535
    }
    st.i = s
  }
  slc(o, 0, pre + shft(pos.contents) + post)
}

// Deflate with options (`dopt`).
let dopt = (
  dat: Uint8Array.t,
  opts: compressOptions,
  pre: int,
  post: int,
  ~st: option<deflateState>=?,
): Uint8Array.t => {
  let (dat, st) = switch st {
  | Some(s) => (dat, s)
  | None =>
    let s = makeDeflateState(~l=1)
    let dat = switch opts.dictionary {
    | Some(d) =>
      let dict = d->TypedArray.subarray(~start=-32768)
      let dictLen = dict->TypedArray.length
      let newDat = Uint8Array.fromLength(dictLen + dat->TypedArray.length)
      newDat->setU8Slice(dict)
      newDat->setU8SliceAt(dat, dictLen)
      s.w = dictLen
      newDat
    | None => dat
    }
    (dat, s)
  }
  let lvl = switch opts.level {
  | Some(l) => l
  | None => 6
  }
  let plvl = switch opts.mem {
  | Some(m) => 12 + m
  | None =>
    if st.l != 0 {
      let clamped = Math.max(8., Math.min(13., Math.log(dat->TypedArray.length->Int.toFloat)))
      Math.ceil(clamped *. 1.5)->Int.fromFloat
    } else {
      20
    }
  }
  dflt(dat, lvl, plvl, pre, post, st)
}

let deflateSync = (data: Uint8Array.t, ~opts: compressOptions={}): Uint8Array.t =>
  dopt(data, opts, 0, 0)
