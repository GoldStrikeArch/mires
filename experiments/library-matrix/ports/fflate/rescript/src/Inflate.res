// Raw DEFLATE decompressor (`inflt`, `inflateSync`) matching `fflate@0.8.3`.

open Tables

type decompressOptions = {
  dictionary?: Uint8Array.t,
  out?: Uint8Array.t,
}

type inflateState = {
  mutable i: int,
  mutable f: int,
  mutable p: int,
  mutable b: int,
  mutable l: option<Uint16Array.t>,
  mutable d: option<Uint16Array.t>,
  mutable m: int,
  mutable n: int,
}

let makeInflateState = (~i=0, ~b=0): inflateState => {
  i,
  f: 0,
  p: 0,
  b,
  l: None,
  d: None,
  m: 0,
  n: 0,
}

// Grow output buffer to at least `l` bytes (`cbuf`). Top-level so `buf` in `inflt` stays unboxed.
let growBuf = (buf: Uint8Array.t, l: int): Uint8Array.t => {
  let bl = buf->TypedArray.length
  if l > bl {
    let nbuf = Uint8Array.fromLength(Math.Int.max(bl * 2, l))
    nbuf->setU8Slice(buf)
    nbuf
  } else {
    buf
  }
}

// Core inflate state machine (`inflt`).
let inflt = (
  dat: Uint8Array.t,
  st: inflateState,
  ~buf: option<Uint8Array.t>=?,
  ~dict: option<Uint8Array.t>=?,
): Uint8Array.t => {
  let sl = dat->TypedArray.length
  let dl = switch dict {
  | Some(d) => d->TypedArray.length
  | None => 0
  }
  if sl == 0 || (st.f != 0 && st.l === None) {
    switch buf {
    | Some(b) => b
    | None => Uint8Array.fromLength(0)
    }
  } else {
    let noBuf = buf === None
    let resize = noBuf || st.i != 2
    let noSt = st.i
    let outBuf = ref(
      switch buf {
      | Some(b) => b
      | None => Uint8Array.fromLength(sl * 3)
      },
    )
    let dictBuf = switch dict {
    | Some(d) => d
    | None => et
    }
    let final = ref(st.f)
    let pos = ref(st.p)
    let bt = ref(st.b)
    let lm = ref(st.l)
    let dm = ref(st.d)
    let lbt = ref(st.m)
    let dbt = ref(st.n)
    let tbts = sl * 8
    let outerDone = ref(false)
    while !outerDone.contents {
      let skipDecode = ref(false)
      let truncated = ref(false)
      if lm.contents === None {
        final := bits(dat, pos.contents, 1)
        let btype = bits(dat, pos.contents + 1, 3)
        pos := pos.contents + 3
        if btype == 0 {
          let s = shft(pos.contents) + 4
          let l = bor(getU8(dat, s - 4), shl(getU8(dat, s - 3), 8))
          let t = s + l
          if t > sl {
            if noSt != 0 {
              fail(0)
            }
            truncated := true
          } else {
            if resize && bt.contents + l > outBuf.contents->TypedArray.length {
              outBuf := growBuf(outBuf.contents, bt.contents + l)
            }
            outBuf.contents->setU8SliceAt(dat->TypedArray.subarray(~start=s, ~end=t), bt.contents)
            bt := bt.contents + l
            st.b = bt.contents
            pos := t * 8
            st.p = pos.contents
            st.f = final.contents
            skipDecode := true
          }
        } else if btype == 1 {
          lm := Some(flrm)
          dm := Some(fdrm)
          lbt := 9
          dbt := 5
        } else if btype == 2 {
          let hLit = bits(dat, pos.contents, 31) + 257
          let hcLen = bits(dat, pos.contents + 10, 15) + 4
          let tl = hLit + bits(dat, pos.contents + 5, 31) + 1
          pos := pos.contents + 14
          let ldt = Uint8Array.fromLength(tl)
          let clt = Uint8Array.fromLength(19)
          for i in 0 to hcLen - 1 {
            setU8(clt, getU8(clim, i), bits(dat, pos.contents + i * 3, 7))
          }
          pos := pos.contents + hcLen * 3
          let clb = maxU8(clt)
          let clbmsk = shl(1, clb) - 1
          let clm = hMap(clt, clb, true)
          let i = ref(0)
          while i.contents < tl {
            let r = getU16(clm, bits(dat, pos.contents, clbmsk))
            pos := pos.contents + band(r, 15)
            let sym = shr(r, 4)
            if sym < 16 {
              setU8(ldt, i.contents, sym)
              i := i.contents + 1
            } else {
              let copyVal = ref(0)
              let count = ref(0)
              if sym == 16 {
                count := 3 + bits(dat, pos.contents, 3)
                pos := pos.contents + 2
                copyVal := (i.contents > 0 ? getU8(ldt, i.contents - 1) : 0)
              } else if sym == 17 {
                count := 3 + bits(dat, pos.contents, 7)
                pos := pos.contents + 3
              } else if sym == 18 {
                count := 11 + bits(dat, pos.contents, 127)
                pos := pos.contents + 7
              }
              while count.contents > 0 {
                setU8(ldt, i.contents, copyVal.contents)
                i := i.contents + 1
                count := count.contents - 1
              }
            }
          }
          let lt = ldt->TypedArray.subarray(~start=0, ~end=hLit)
          let dt = ldt->TypedArray.subarray(~start=hLit)
          lbt := maxU8(lt)
          dbt := maxU8(dt)
          lm := Some(hMap(lt, lbt.contents, true))
          dm := Some(hMap(dt, dbt.contents, true))
        } else {
          fail(1)
        }
        if !skipDecode.contents && !truncated.contents && pos.contents > tbts {
          if noSt != 0 {
            fail(0)
          }
          truncated := true
        }
      }
      if truncated.contents {
        outerDone := true
      } else if skipDecode.contents {
        if final.contents != 0 {
          outerDone := true
        }
      } else {
        switch (lm.contents, dm.contents) {
        | (Some(lmArr), Some(dmArr)) =>
          if resize && bt.contents + 131072 > outBuf.contents->TypedArray.length {
            outBuf := growBuf(outBuf.contents, bt.contents + 131072)
          }
          let lms = shl(1, lbt.contents) - 1
          let dms = shl(1, dbt.contents) - 1
          let lpos = ref(pos.contents)
          let innerDone = ref(false)
          while !innerDone.contents {
            lpos := pos.contents
            let c = getU16(lmArr, band(bits16(dat, pos.contents), lms))
            let sym = shr(c, 4)
            pos := pos.contents + band(c, 15)
            if pos.contents > tbts {
              if noSt != 0 {
                fail(0)
              }
              innerDone := true
            } else if c == 0 {
              fail(2)
            } else if sym < 256 {
              setU8(outBuf.contents, bt.contents, sym)
              bt := bt.contents + 1
            } else if sym == 256 {
              lpos := pos.contents
              lm := None
              innerDone := true
            } else {
              let add = ref(sym - 254)
              if sym > 264 {
                let idx = sym - 257
                let b = getU8(fleb, idx)
                add := bits(dat, pos.contents, shl(1, b) - 1) + getU16(fl, idx)
                pos := pos.contents + b
              }
              let d = getU16(dmArr, band(bits16(dat, pos.contents), dms))
              let dsym = shr(d, 4)
              if d == 0 {
                fail(3)
              }
              pos := pos.contents + band(d, 15)
              let dist = ref(getU16(fd, dsym))
              if dsym > 3 {
                let b = getU8(fdeb, dsym)
                dist := dist.contents + band(bits16(dat, pos.contents), shl(1, b) - 1)
                pos := pos.contents + b
              }
              if pos.contents > tbts {
                if noSt != 0 {
                  fail(0)
                }
                innerDone := true
              } else {
                if resize && bt.contents + 131072 > outBuf.contents->TypedArray.length {
                  outBuf := growBuf(outBuf.contents, bt.contents + 131072)
                }
                let endPos = bt.contents + add.contents
                let dt = dist.contents
                let curBuf = outBuf.contents
                if bt.contents < dt {
                  let shift = dl - dt
                  let dend = Math.Int.min(dt, endPos)
                  if shift + bt.contents < 0 {
                    fail(3)
                  }
                  while bt.contents < dend {
                    setU8(curBuf, bt.contents, getU8(dictBuf, shift + bt.contents))
                    bt := bt.contents + 1
                  }
                }
                while bt.contents < endPos {
                  setU8(curBuf, bt.contents, getU8(curBuf, bt.contents - dt))
                  bt := bt.contents + 1
                }
              }
            }
          }
          st.l = lm.contents
          st.p = lpos.contents
          st.b = bt.contents
          st.f = final.contents
          if lm.contents !== None {
            final := 1
            st.m = lbt.contents
            st.d = dm.contents
            st.n = dbt.contents
          }
          if final.contents != 0 {
            outerDone := true
          }
        | _ => outerDone := true
        }
      }
    }
    let finalBuf = outBuf.contents
    let finalBt = bt.contents
    if finalBt != finalBuf->TypedArray.length && noBuf {
      slc(finalBuf, 0, finalBt)
    } else {
      finalBuf->TypedArray.subarray(~start=0, ~end=finalBt)
    }
  }
}

let inflateSync = (data: Uint8Array.t, ~opts: decompressOptions={}): Uint8Array.t =>
  inflt(data, makeInflateState(~i=2), ~buf=?opts.out, ~dict=?opts.dictionary)
