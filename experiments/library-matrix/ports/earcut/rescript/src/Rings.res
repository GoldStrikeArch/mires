open Geometry

type block = {
  mutable head: node,
  mutable stop: node,
  mutable xmin: float,
  mutable ymin: float,
  mutable xmax: float,
  mutable ymax: float,
}
type state = {
  mutable blocks: option<array<block>>,
  mutable filtered: bool,
}

let grow = (b, n) => {
  if n.x < b.xmin {
    b.xmin = n.x
  }
  if n.x > b.xmax {
    b.xmax = n.x
  }
  if n.y < b.ymin {
    b.ymin = n.y
  }
  if n.y > b.ymax {
    b.ymax = n.y
  }
}

let remove = (st, p) => {
  p.next.prev = p.prev
  p.prev.next = p.next
  switch p.prevZ {
  | None => ()
  | Some(prev) => prev.nextZ = p.nextZ
  }
  switch p.nextZ {
  | None => ()
  | Some(next) => next.prevZ = p.prevZ
  }
  switch st.blocks {
  | None => ()
  | Some(blocks) => grow(Array.getUnsafe(blocks, p.prev.z), p.next)
  }
}

let linkedList = (st, data, first, limit, dim, clockwise) => {
  let last = ref(None)
  let insert = i => {
    let p = makeNode(i / dim, Array.getUnsafe(data, i), Array.getUnsafe(data, i + 1))
    switch last.contents {
    | None => ()
    | Some(n) =>
      p.next = n.next
      p.prev = n
      n.next.prev = p
      n.next = p
    }
    last := Some(p)
  }

  if clockwise == (signedArea(data, first, limit, dim) > 0.) {
    let i = ref(first)
    while i.contents < limit {
      insert(i.contents)
      i := i.contents + dim
    }
  } else {
    let i = ref(limit - dim)
    while i.contents >= first {
      insert(i.contents)
      i := i.contents - dim
    }
  }
  switch last.contents {
  | Some(p) if equals(p, p.next) =>
    remove(st, p)
    Some(p.next)
  | x => x
  }
}

let filter = (st, start, stop) => {
  let full = start === stop
  let rec walk = (p, bound) =>
    if p !== p.next && (!p.steiner && (equals(p, p.next) || area(p.prev, p, p.next) == 0.)) {
      let bound = if full || p === bound {
        p.prev
      } else {
        bound
      }
      st.filtered = true
      remove(st, p)
      walk(p.prev, bound)
    } else if full {
      if p.next === bound {
        bound
      } else {
        walk(p.next, bound)
      }
    } else if p === bound {
      bound
    } else {
      walk(p.next, bound)
    }
  walk(start, stop)
}

let indexSegment = (blocks, head, stop) => {
  let rec segment = head => {
    let block = {head, stop: head, xmin: head.x, ymin: head.y, xmax: head.x, ymax: head.y}
    let idx = Array.length(blocks)
    let rec edges = (p, k) => {
      let q = p.next
      p.z = idx
      grow(block, q)
      if k == 16 || q === stop {
        q
      } else {
        edges(q, k + 1)
      }
    }

    let bound = edges(head, 1)
    block.stop = bound
    Array.push(blocks, block)->ignore
    if bound !== stop {
      segment(bound)
    }
  }
  segment(head)
}

let rec live = p =>
  if p.prev.next === p {
    p
  } else {
    live(p.next)
  }

/* Each retained block can outlive a removed head or stop. Repair both before
 visiting its live edges, preserving the upstream append order. */
let scanBlock = (block, visit) => {
  let stop = live(block.stop)
  block.stop = stop
  let head = live(block.head)
  block.head = head
  let rec walk = p => {
    if p.prev.next === p {
      visit(p)
    }
    if p.next !== stop {
      walk(p.next)
    }
  }
  walk(head)
}

exception Touch(node)

let findBridge = (blocks, hole, outer) =>
  if equals(hole, outer) {
    Some(outer)
  } else {
    let hx = hole.x
    let hy = hole.y
    let qx = ref(Float.Constants.negativeInfinity)
    let candidate = ref(None)
    try {
      for i in 0 to Array.length(blocks) - 1 {
        let b = Array.getUnsafe(blocks, i)
        if !(hy < b.ymin || (hy > b.ymax || (b.xmin > hx || b.xmax <= qx.contents))) {
          scanBlock(b, p => {
            let n = p.next
            if equals(hole, n) {
              throw(Touch(n))
            }
            if hy <= p.y && (hy >= n.y && n.y != p.y) {
              let x = p.x +. (hy -. p.y) *. (n.x -. p.x) /. (n.y -. p.y)
              if x <= hx && x > qx.contents {
                qx := x
                let m = if p.x < n.x {
                  p
                } else {
                  n
                }
                candidate := Some(m)
                if x == hx {
                  throw(Touch(m))
                }
              }
            }
          })
        }
      }
      switch candidate.contents {
      | None => None
      | Some(initial) =>
        let mx = initial.x
        let my = initial.y
        let ymin = Math.min(hy, my)
        let ymax = Math.max(hy, my)
        let ax = if hy < my {
          hx
        } else {
          qx.contents
        }

        let cx = if hy < my {
          qx.contents
        } else {
          hx
        }
        let best = ref(initial)
        let angle = ref(Float.Constants.positiveInfinity)
        for i in 0 to Array.length(blocks) - 1 {
          let b = Array.getUnsafe(blocks, i)
          if !(b.xmax < mx || (b.xmin > hx || (b.ymax < ymin || b.ymin > ymax))) {
            scanBlock(b, p =>
              if (
                hx >= p.x &&
                  (p.x >= mx &&
                  (hx != p.x && insideTriangle(ax, hy, mx, my, cx, hy, p.x, p.y)))
              ) {
                let tan = Math.abs(hy -. p.y) /. (hx -. p.x)
                let m = best.contents
                if (
                  (locallyInside(p, hole) || (p.y == hy && (p.next.y == hy && p.next.x > hx))) &&
                    (tan < angle.contents ||
                      (tan == angle.contents &&
                        (p.x > m.x ||
                          (p.x == m.x &&
                            (area(m.prev, m, p.prev) < 0. &&
                            area(p.next, m, m.next) < 0.)))))
                ) {
                  best := p
                  angle := tan
                }
              }
            )
          }
        }
        Some(best.contents)
      }
    } catch {
    | Touch(n) => Some(n)
    }
  }

let leftmost = start => {
  let rec walk = (p, best) => {
    let best = if p.x < best.x || (p.x == best.x && p.y < best.y) {
      p
    } else {
      best
    }
    if p.next === start {
      best
    } else {
      walk(p.next, best)
    }
  }
  walk(start, start)
}

let holeOrder = (a, b) => {
  let cmp = x =>
    if x < 0. {
      -1
    } else if x > 0. {
      1
    } else {
      0
    }
  if a.x != b.x {
    cmp(a.x -. b.x)
  } else if a.y != b.y {
    cmp(a.y -. b.y)
  } else {
    cmp((a.next.y -. a.y) /. (a.next.x -. a.x) -. (b.next.y -. b.y) /. (b.next.x -. b.x))
  }
}

let eliminateHoles = (st, data, holes, dim, outer) => {
  let nh = Array.length(holes)
  let queue = Array.fromInitializer(~length=nh, i => {
    let limit = if i + 1 < nh {
      Array.getUnsafe(holes, i + 1) * dim
    } else {
      Array.length(data)
    }
    switch linkedList(st, data, Array.getUnsafe(holes, i) * dim, limit, dim, false) {
    | None => JsError.throw(JsError.TypeError.make("empty hole"))
    | Some(ring) =>
      if ring === ring.next {
        ring.steiner = true
      }
      leftmost(ring)
    }
  })

  Array.sort(queue, (a, b) => Int.toFloat(holeOrder(a, b)))
  let blocks = []
  indexSegment(blocks, outer, outer)
  st.blocks = Some(blocks)
  let root = ref(outer)
  for i in 0 to nh - 1 {
    let hole = Array.getUnsafe(queue, i)
    switch findBridge(blocks, hole, root.contents) {
    | None => ()
    | Some(bridge) =>
      let rev = splitPolygon(bridge, hole)
      indexSegment(blocks, bridge, rev.next.next)
      ignore(filter(st, rev, rev.next))
      root := filter(st, bridge, bridge.next)
    }
  }
  st.blocks = None
  filter(st, root.contents, root.contents)
}
