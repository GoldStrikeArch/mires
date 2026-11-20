open Geometry
open Rings

let spread = x => {
  let x = Int.bitwiseAnd(Int.bitwiseOr(x, Int.shiftLeft(x, 8)), 0x00ff00ff)
  let x = Int.bitwiseAnd(Int.bitwiseOr(x, Int.shiftLeft(x, 4)), 0x0f0f0f0f)
  let x = Int.bitwiseAnd(Int.bitwiseOr(x, Int.shiftLeft(x, 2)), 0x33333333)
  Int.bitwiseAnd(Int.bitwiseOr(x, Int.shiftLeft(x, 1)), 0x55555555)
}

let zOrder = (x, y, xmin, ymin, inv) =>
  Int.bitwiseOr(
    spread(Int.fromFloat((x -. xmin) *. inv)),
    Int.shiftLeft(spread(Int.fromFloat((y -. ymin) *. inv)), 1),
  )

let sortNodes = (nodes, n) =>
  if n <= 32 {
    for i in 1 to n - 1 {
      let p = Array.getUnsafe(nodes, i)
      let j = ref(i - 1)
      while j.contents >= 0 && Array.getUnsafe(nodes, j.contents).z > p.z {
        nodes[j.contents + 1] = Array.getUnsafe(nodes, j.contents)
        j := j.contents - 1
      }
      nodes[j.contents + 1] = p
    }
  } else {
    let tmp = Array.make(~length=n, Array.getUnsafe(nodes, 0))
    let keys = Array.fromInitializer(~length=n, i => Array.getUnsafe(nodes, i).z)
    let nextKeys = Array.make(~length=n, 0)
    let counts = Array.make(~length=256, 0)
    let pass = (src, srcKeys, dst, dstKeys, shift) => {
      Array.fill(counts, 0)
      for i in 0 to n - 1 {
        let bucket = Int.bitwiseAnd(Int.shiftRightUnsigned(Array.getUnsafe(srcKeys, i), shift), 255)
        counts[bucket] = Array.getUnsafe(counts, bucket) + 1
      }
      let total = ref(0)
      for bucket in 0 to 255 {
        let count = Array.getUnsafe(counts, bucket)
        counts[bucket] = total.contents
        total := total.contents + count
      }
      for i in 0 to n - 1 {
        let key = Array.getUnsafe(srcKeys, i)
        let bucket = Int.bitwiseAnd(Int.shiftRightUnsigned(key, shift), 255)
        let offset = Array.getUnsafe(counts, bucket)
        dst[offset] = Array.getUnsafe(src, i)
        dstKeys[offset] = key
        counts[bucket] = offset + 1
      }
    }

    pass(nodes, keys, tmp, nextKeys, 0)
    pass(tmp, nextKeys, nodes, keys, 8)
    pass(nodes, keys, tmp, nextKeys, 16)
    pass(tmp, nextKeys, nodes, keys, 24)
  }

let indexCurve = (root, xmin, ymin, inv) => {
  let nodes = []
  let rec collect = p => {
    p.z = zOrder(p.x, p.y, xmin, ymin, inv)
    Array.push(nodes, p)->ignore
    if p.next !== root {
      collect(p.next)
    }
  }
  collect(root)
  sortNodes(nodes, Array.length(nodes))
  let prev = ref(None)
  for i in 0 to Array.length(nodes) - 1 {
    let p = Array.getUnsafe(nodes, i)
    p.prevZ = prev.contents
    switch prev.contents {
    | None => ()
    | Some(before) => before.nextZ = Some(p)
    }
    prev := Some(p)
  }
  Array.getUnsafe(nodes, Array.length(nodes) - 1).nextZ = None
}

let isEar = (b, xmin, ymin, inv) => {
  let a = b.prev
  let c = b.next
  let x0 = Math.min(a.x, Math.min(b.x, c.x))
  let y0 = Math.min(a.y, Math.min(b.y, c.y))
  let x1 = Math.max(a.x, Math.max(b.x, c.x))
  let y1 = Math.max(a.y, Math.max(b.y, c.y))
  let blocks = p =>
    p.x >= x0 &&
      (p.x <= x1 &&
      (p.y >= y0 &&
        (p.y <= y1 &&
        (!(a.x == p.x && a.y == p.y) &&
        (insideTriangle(a.x, a.y, b.x, b.y, c.x, c.y, p.x, p.y) &&
        area(p.prev, p, p.next) >= 0.)))))

  if inv == 0. {
    let rec walk = p =>
      if p === a {
        true
      } else if blocks(p) {
        false
      } else {
        walk(p.next)
      }
    walk(c.next)
  } else {
    let low = zOrder(x0, y0, xmin, ymin, inv)
    let high = zOrder(x1, y1, xmin, ymin, inv)
    let rec backward = link =>
      switch link {
      | Some(p) if p.z >= low =>
        if p !== c && blocks(p) {
          false
        } else {
          backward(p.prevZ)
        }
      | _ => true
      }

    let rec forward = link =>
      switch link {
      | Some(p) if p.z <= high =>
        if p !== c && blocks(p) {
          false
        } else {
          forward(p.nextZ)
        }
      | _ => true
      }
    backward(b.prevZ) && forward(b.nextZ)
  }
}

let triangle = (out, a, b, c) => {
  Array.push(out, a.i)->ignore
  Array.push(out, b.i)->ignore
  Array.push(out, c.i)->ignore
}

let cure = (st, root, out) => {
  let rec scan = (p, start, cured) => {
    let a = p.prev
    let b = p.next.next
    let (p, start, cured) = if (
      intersects(a, p, p.next, b, false) && (locallyInside(a, b) && locallyInside(b, a))
    ) {
      triangle(out, a, p, b)
      remove(st, p)
      remove(st, p.next)
      (b, b, true)
    } else {
      (p, start, cured)
    }

    if p.next === start {
      if cured {
        filter(st, start, start)
      } else {
        start
      }
    } else {
      scan(p.next, start, cured)
    }
  }
  scan(root, root, false)
}

let rec clipRing = (st, root, out, xmin, ymin, inv) => {
  if inv != 0. {
    indexCurve(root, xmin, ymin, inv)
  }
  let rec slice = (p, stop, cured) => {
    let prev = p.prev
    let next = p.next
    if prev !== next {
      if area(prev, p, next) < 0. && isEar(p, xmin, ymin, inv) {
        triangle(out, prev, p, next)
        remove(st, p)
        slice(next, next, cured)
      } else if next !== stop {
        slice(next, stop, cured)
      } else {
        st.filtered = false
        let p = filter(st, next, next)
        if st.filtered {
          slice(p, p, cured)
        } else if !cured {
          let p = cure(st, p, out)
          slice(p, p, true)
        } else {
          splitRing(st, p, out, xmin, ymin, inv)
        }
      }
    }
  }
  slice(root, root, false)
}

and splitRing = (st, root, out, xmin, ymin, inv) => {
  let rec findB = (a, b) =>
    if b === a.prev {
      None
    } else if a.i != b.i && validDiagonal(a, b) {
      Some(b)
    } else {
      findB(a, b.next)
    }

  let rec findA = a =>
    switch findB(a, a.next.next) {
    | Some(b) =>
      let c = splitPolygon(a, b)
      let a = filter(st, a, a.next)
      let c = filter(st, c, c.next)
      clipRing(st, a, out, xmin, ymin, inv)
      clipRing(st, c, out, xmin, ymin, inv)
    | None =>
      if a.next !== root {
        findA(a.next)
      }
    }
  findA(root)
}

let earcut = (data, holes, dim) => {
  let st = {blocks: None, filtered: false}
  let outerLen = if Array.length(holes) == 0 {
    Array.length(data)
  } else {
    Array.getUnsafe(holes, 0) * dim
  }
  switch linkedList(st, data, 0, outerLen, dim, true) {
  | None => []
  | Some(outer) if outer.next === outer.prev => []
  | Some(outer) =>
    let outer = if Array.length(holes) == 0 {
      outer
    } else {
      eliminateHoles(st, data, holes, dim, outer)
    }
    let xmin = ref(0.)
    let ymin = ref(0.)
    let inv = ref(0.)
    if Array.length(data) > 80 * dim {
      xmin := Array.getUnsafe(data, 0)
      ymin := Array.getUnsafe(data, 1)
      let xmax = ref(xmin.contents)
      let ymax = ref(ymin.contents)
      let i = ref(dim)
      while i.contents < outerLen {
        let x = Array.getUnsafe(data, i.contents)
        let y = Array.getUnsafe(data, i.contents + 1)
        xmin := Math.min(xmin.contents, x)
        ymin := Math.min(ymin.contents, y)
        xmax := Math.max(xmax.contents, x)
        ymax := Math.max(ymax.contents, y)
        i := i.contents + dim
      }
      let extent = Math.max(xmax.contents -. xmin.contents, ymax.contents -. ymin.contents)
      if extent != 0. {
        inv := 32767. /. extent
      }
    }
    let out = []
    clipRing(st, outer, out, xmin.contents, ymin.contents, inv.contents)
    out
  }
}
