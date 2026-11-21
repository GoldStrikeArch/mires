// pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy 3.1.3.

open Hierarchy

/**
 * lcg.js: a seeded generator of [0, 1). Every product stays below 2^53, so floats are exact.
 * https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use
 */
let lcg = () => {
  let a = 1664525.
  let c = 1013904223.
  let m = 4294967296. // 2^32
  let s = ref(1.)
  () => {
    s.contents = Float.mod(a *. s.contents +. c, m)
    s.contents /. m
  }
}

/**
 * A circle as enclose.js reads one. The front chain's nodes are copied in: they do not move while
 * their enclosure is computed.
 */
type circle = {cx: float, cy: float, cr: float}

let circleOf = node => {cx: node.x, cy: node.y, cr: node.r}

/** array.js's `shuffle`: Fisher-Yates from the end, with `random() * m-- | 0`. */
let shuffle = (array, random) =>
  for m in Array.length(array) downto 1 {
    let i = Float.toInt(random() *. Int.toFloat(m))
    let t = array->Array.getUnsafe(m - 1)
    array->Array.setUnsafe(m - 1, array->Array.getUnsafe(i))
    array->Array.setUnsafe(i, t)
  }

let enclosesNot = (a, b) => {
  let dr = a.cr -. b.cr
  let dx = b.cx -. a.cx
  let dy = b.cy -. a.cy
  dr < 0. || dr *. dr < dx *. dx +. dy *. dy
}

let enclosesWeak = (a, b) => {
  let dr = a.cr -. b.cr +. Math.max(Math.max(a.cr, b.cr), 1.) *. 1e-9
  let dx = b.cx -. a.cx
  let dy = b.cy -. a.cy
  dr > 0. && dr *. dr > dx *. dx +. dy *. dy
}

let enclosesWeakAll = (a, basis) => basis->List.every(p => enclosesWeak(a, p))

let encloseBasis2 = (a, b) => {
  let (x1, y1, r1) = (a.cx, a.cy, a.cr)
  let (x2, y2, r2) = (b.cx, b.cy, b.cr)
  let x21 = x2 -. x1
  let y21 = y2 -. y1
  let r21 = r2 -. r1
  let l = Math.sqrt(x21 *. x21 +. y21 *. y21)
  {
    cx: (x1 +. x2 +. x21 /. l *. r21) /. 2.,
    cy: (y1 +. y2 +. y21 /. l *. r21) /. 2.,
    cr: (l +. r1 +. r2) /. 2.,
  }
}

let encloseBasis3 = (a, b, c) => {
  let (x1, y1, r1) = (a.cx, a.cy, a.cr)
  let (x2, y2, r2) = (b.cx, b.cy, b.cr)
  let (x3, y3, r3) = (c.cx, c.cy, c.cr)
  let a2 = x1 -. x2
  let a3 = x1 -. x3
  let b2 = y1 -. y2
  let b3 = y1 -. y3
  let c2 = r2 -. r1
  let c3 = r3 -. r1
  let d1 = x1 *. x1 +. y1 *. y1 -. r1 *. r1
  let d2 = d1 -. x2 *. x2 -. y2 *. y2 +. r2 *. r2
  let d3 = d1 -. x3 *. x3 -. y3 *. y3 +. r3 *. r3
  let ab = a3 *. b2 -. a2 *. b3
  let xa = (b2 *. d3 -. b3 *. d2) /. (ab *. 2.) -. x1
  let xb = (b3 *. c2 -. b2 *. c3) /. ab
  let ya = (a3 *. d2 -. a2 *. d3) /. (ab *. 2.) -. y1
  let yb = (a2 *. c3 -. a3 *. c2) /. ab
  let qa = xb *. xb +. yb *. yb -. 1.
  let qb = 2. *. (r1 +. xa *. xb +. ya *. yb)
  let qc = xa *. xa +. ya *. ya -. r1 *. r1
  let r = -.(
    Math.abs(qa) > 1e-6 ? (qb +. Math.sqrt(qb *. qb -. 4. *. qa *. qc)) /. (2. *. qa) : qc /. qb
  )
  {cx: x1 +. xa +. xb *. r, cy: y1 +. ya +. yb *. r, cr: r}
}

let encloseBasis = basis =>
  switch basis {
  | list{a} => a
  | list{a, b} => encloseBasis2(a, b)
  | list{a, b, c} => encloseBasis3(a, b, c)
  | _ => JsError.throwWithMessage("no enclosing basis")
  }

let extendBasis = (b, p) =>
  if enclosesWeakAll(p, b) {
    list{p}
  } else {
    // If we get here then B must have at least one element.
    switch b->List.find(x => enclosesNot(p, x) && enclosesWeakAll(encloseBasis2(x, p), b)) {
    | Some(x) => list{x, p}
    | None =>
      // If we get here then B must have at least two elements.
      let triple = (x, y) =>
        enclosesNot(encloseBasis2(x, y), p) &&
        enclosesNot(encloseBasis2(x, p), y) &&
        enclosesNot(encloseBasis2(y, p), x) &&
        enclosesWeakAll(encloseBasis3(x, y, p), b)
      let rec firstPair = list =>
        switch list {
        | list{} => None
        | list{x, ...rest} =>
          switch rest->List.find(y => triple(x, y)) {
          | Some(y) => Some((x, y))
          | None => firstPair(rest)
          }
        }
      switch firstPair(b) {
      | Some((x, y)) => list{x, y, p}
      // If we get here then something is very wrong.
      | None => JsError.throwWithMessage("no enclosing basis")
      }
    }
  }

/** `packEncloseRandom`: the smallest circle enclosing `circles`, by Welzl's algorithm. */
let packEncloseRandom = (circles, random) => {
  shuffle(circles, random)
  let n = Array.length(circles)
  let rec scan = (i, basis, e) =>
    if i >= n {
      e
    } else {
      let p = circles->Array.getUnsafe(i)
      switch e {
      | Some(enclosure) if enclosesWeak(enclosure, p) => scan(i + 1, basis, e)
      | _ =>
        let basis = extendBasis(basis, p)
        scan(0, basis, Some(encloseBasis(basis)))
      }
    }
  scan(0, list{}, None)->Option.getOrThrow
}

/** Places `c` tangent to `a` and `b`. */
let place = (b, a, c) => {
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  let d2 = dx *. dx +. dy *. dy
  if truthy(d2) {
    let a2 = a.r +. c.r
    let a2 = a2 *. a2
    let b2 = b.r +. c.r
    let b2 = b2 *. b2
    if a2 > b2 {
      let x = (d2 +. b2 -. a2) /. (2. *. d2)
      let y = Math.sqrt(Math.max(0., b2 /. d2 -. x *. x))
      c.x = b.x -. x *. dx -. y *. dy
      c.y = b.y -. x *. dy +. y *. dx
    } else {
      let x = (d2 +. a2 -. b2) /. (2. *. d2)
      let y = Math.sqrt(Math.max(0., a2 /. d2 -. x *. x))
      c.x = a.x +. x *. dx -. y *. dy
      c.y = a.y +. x *. dy +. y *. dx
    }
  } else {
    c.x = a.x +. c.r
    c.y = a.y
  }
}

let intersects = (a, b) => {
  let dr = a.r +. b.r -. 1e-6
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  dr > 0. && dr *. dr > dx *. dx +. dy *. dy
}

/**
 * siblings.js's front-chain `Node`: a circle (`_`) and its neighbours on the chain. A link is its
 * own neighbour until it joins the chain.
 */
type rec link = {circle: node, mutable next: link, mutable previous: link}

let link = circle => {
  let rec l = {circle, next: l, previous: l}
  l
}

/** The squared distance from the origin to the weighted centre of `node` and its successor. */
let score = node => {
  let a = node.circle
  let b = node.next.circle
  let ab = a.r +. b.r
  let dx = (a.x *. b.r +. b.x *. a.r) /. ab
  let dy = (a.y *. b.r +. b.y *. a.r) /. ab
  dx *. dx +. dy *. dy
}

/**
 * The circle on the front chain between `a` and `b` that circle `c` would intersect first, if
 * any: `Behind(k)` replaces `a`, `Ahead(j)` replaces `b`. "Closeness" is determined by linear
 * distance along the front-chain, and so are "ahead" and "behind".
 */
type intersection = Ahead(link) | Behind(link)

let findIntersection = (a, b, c) => {
  let rec scan = (j, k, sj, sk) =>
    if sj <= sk {
      if intersects(j.circle, c) {
        Some(Ahead(j))
      } else if j.next === k.next {
        None
      } else {
        scan(j.next, k, sj +. j.circle.r, sk)
      }
    } else if intersects(k.circle, c) {
      Some(Behind(k))
    } else if j === k.previous.next {
      None
    } else {
      scan(j, k.previous, sj, sk +. k.circle.r)
    }
  scan(b.next, a.previous, b.circle.r, a.circle.r)
}

/** `packSiblingsRandom`: packs `circles` around the origin and returns the enclosing radius. */
let packSiblingsRandom = (circles, random) => {
  let n = Array.length(circles)
  if n == 0 {
    0.
  } else {
    // Place the first circle.
    let first = circles->Array.getUnsafe(0)
    first.x = 0.
    first.y = 0.
    if n <= 1 {
      first.r
    } else {
      // Place the second circle.
      let second = circles->Array.getUnsafe(1)
      first.x = -.second.r
      second.x = first.r
      second.y = 0.
      if n <= 2 {
        first.r +. second.r
      } else {
        // Place the third circle.
        place(second, first, circles->Array.getUnsafe(2))

        // Initialize the front-chain using the first three circles a, b and c.
        let a = link(first)
        let b = link(second)
        let c = link(circles->Array.getUnsafe(2))
        a.next = b
        c.previous = b
        b.next = c
        a.previous = c
        c.next = a
        b.previous = a

        // Attempt to place each remaining circle…
        let rec placeFrom = (i, a, b) =>
          if i >= n {
            b
          } else {
            let circle = circles->Array.getUnsafe(i)
            place(a.circle, b.circle, circle)
            switch findIntersection(a, b, circle) {
            | Some(Ahead(j)) =>
              a.next = j
              j.previous = a
              placeFrom(i, a, j)
            | Some(Behind(k)) =>
              k.next = b
              b.previous = k
              placeFrom(i, k, b)
            | None =>
              // Success! Insert the new circle c between a and b.
              let c = link(circle)
              c.previous = a
              c.next = b
              a.next = c
              b.previous = c

              // Compute the new closest circle pair to the centroid.
              let rec closest = (node, best, bestScore) =>
                if node === c {
                  best
                } else {
                  let s = score(node)
                  s < bestScore ? closest(node.next, node, s) : closest(node.next, best, bestScore)
                }
              let a = closest(b, a, score(a))
              placeFrom(i + 1, a, a.next)
            }
          }
        let b = placeFrom(3, a, b)

        // Compute the enclosing circle of the front chain.
        let front = [circleOf(b.circle)]
        let rec collect = node =>
          if node !== b {
            front->Array.push(circleOf(node.circle))
            collect(node.next)
          }
        collect(b.next)
        let e = packEncloseRandom(front, random)

        // Translate the circles to put the enclosing circle around the origin.
        circles->Array.forEach(node => {
          node.x = node.x -. e.cx
          node.y = node.y -. e.cy
        })
        e.cr
      }
    }
  }
}

/** `radiusLeaf(defaultRadius)`: a leaf's radius is the square root of its value. */
let radiusLeaf = node =>
  if Array.length(node.children) == 0 {
    let r = Math.sqrt(node.value)
    node.r = Math.max(0., truthy(r) ? r : 0.)
  }

/** `packChildrenRandom(padding, k, random)` with a constant padding. */
let packChildrenRandom = (padding, k, random, node) =>
  if Array.length(node.children) > 0 {
    let r = padding *. k
    let r = truthy(r) ? r : 0.
    if truthy(r) {
      node.children->Array.forEach(child => child.r = child.r +. r)
    }
    let e = packSiblingsRandom(node.children, random)
    if truthy(r) {
      node.children->Array.forEach(child => child.r = child.r -. r)
    }
    node.r = e +. r
  }

/** `translateChild(k)`. */
let translateChild = (k, node) => {
  node.r = node.r *. k
  switch node.parent {
  | Some(parent) =>
    node.x = parent.x +. k *. node.x
    node.y = parent.y +. k *. node.y
  | None => ()
  }
}

/** `pack().size([dx, dy]).padding(padding)`: one generator serves both packing passes. */
let pack = (root, dx, dy, padding) => {
  let random = lcg()
  root.x = dx /. 2.
  root.y = dy /. 2.
  root->eachBefore(radiusLeaf)->eachAfter(node => packChildrenRandom(0., 1., random, node))->ignore
  let k = root.r /. Math.min(dx, dy)
  root->eachAfter(node => packChildrenRandom(padding, k, random, node))->ignore
  let scale = Math.min(dx, dy) /. (2. *. root.r)
  root->eachBefore(node => translateChild(scale, node))->ignore
}
