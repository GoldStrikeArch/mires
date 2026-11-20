/* Ear clipping and polygon predicates, adapted from Mapbox earcut 3.2.4 (ISC). */

type rec node = {
  i: int,
  x: float,
  y: float,
  mutable prev: node,
  mutable next: node,
  mutable z: int,
  mutable prevZ: option<node>,
  mutable nextZ: option<node>,
  mutable steiner: bool,
}

let makeNode = (i, x, y) => {
  let rec node = {
    i,
    x,
    y,
    prev: node,
    next: node,
    z: 0,
    prevZ: None,
    nextZ: None,
    steiner: false,
  }
  node
}

let equals = (a, b) => a.x == b.x && a.y == b.y
let area = (p, q, r) => (q.y -. p.y) *. (r.x -. q.x) -. (q.x -. p.x) *. (r.y -. q.y)
let insideTriangle = (ax, ay, bx, by, cx, cy, px, py) =>
  (cx -. px) *. (ay -. py) >= (ax -. px) *. (cy -. py) &&
    ((ax -. px) *. (by -. py) >= (bx -. px) *. (ay -. py) &&
    (bx -. px) *. (cy -. py) >= (cx -. px) *. (by -. py))

let onSegment = (p, q, r) =>
  q.x <= Math.max(p.x, r.x) &&
    (q.x >= Math.min(p.x, r.x) &&
    (q.y <= Math.max(p.y, r.y) && q.y >= Math.min(p.y, r.y)))

let crosses = (x, y) => (x > 0. && y < 0.) || (x < 0. && y > 0.)
let intersects = (a, b, c, d, boundary) => {
  let abC = area(a, b, c)
  let abD = area(a, b, d)
  let cdA = area(c, d, a)
  let cdB = area(c, d, b)
  (crosses(abC, abD) && crosses(cdA, cdB)) ||
    (boundary &&
    ((abC == 0. && onSegment(a, c, b)) ||
      ((abD == 0. && onSegment(a, d, b)) ||
      ((cdA == 0. && onSegment(c, a, d)) || (cdB == 0. && onSegment(c, b, d))))))
}

let locallyInside = (a, b) =>
  if area(a.prev, a, a.next) < 0. {
    area(a, b, a.next) >= 0. && area(a, a.prev, b) >= 0.
  } else {
    area(a, b, a.prev) < 0. || area(a, a.next, b) < 0.
  }

let intersectsPolygon = (a, b) => {
  let xmin = Math.min(a.x, b.x)
  let xmax = Math.max(a.x, b.x)
  let ymin = Math.min(a.y, b.y)
  let ymax = Math.max(a.y, b.y)
  let rec walk = p => {
    let n = p.next
    let disjoint =
      (p.x > xmax && n.x > xmax) ||
        ((p.x < xmin && n.x < xmin) ||
        ((p.y > ymax && n.y > ymax) || (p.y < ymin && n.y < ymin)))
    if (
      !disjoint &&
      (p.i != a.i &&
      (n.i != a.i && (p.i != b.i && (n.i != b.i && intersects(p, n, a, b, true)))))
    ) {
      true
    } else if n === a {
      false
    } else {
      walk(n)
    }
  }
  walk(a)
}

let middleInside = (a, b) => {
  let x = (a.x +. b.x) /. 2.
  let y = (a.y +. b.y) /. 2.
  let rec walk = (p, inside) => {
    let n = p.next
    let crossing = p.y > y != (n.y > y) && x < (n.x -. p.x) *. (y -. p.y) /. (n.y -. p.y) +. p.x
    let inside = inside != crossing
    if n === a {
      inside
    } else {
      walk(n, inside)
    }
  }
  walk(a, false)
}

let validDiagonal = (a, b) => {
  let zero = equals(a, b) && (area(a.prev, a, a.next) > 0. && area(b.prev, b, b.next) > 0.)
  a.next.i != b.i &&
    ((zero ||
    (locallyInside(a, b) &&
    (locallyInside(b, a) &&
    (area(a.prev, a, b.prev) != 0. || area(a, b.prev, b) != 0.)))) &&
    (!intersectsPolygon(a, b) && (zero || middleInside(a, b))))
}

let splitPolygon = (a, b) => {
  let a2 = makeNode(a.i, a.x, a.y)
  let b2 = makeNode(b.i, b.x, b.y)
  let an = a.next
  let bp = b.prev
  a.next = b
  b.prev = a
  a2.next = an
  an.prev = a2
  b2.next = a2
  a2.prev = b2
  bp.next = b2
  b2.prev = bp
  b2
}

let signedArea = (data, first, limit, dim) => {
  let rec sum = (i, j, total) =>
    if i >= limit {
      total
    } else {
      sum(
        i + dim,
        i,
        total +.
        (Array.getUnsafe(data, j) -. Array.getUnsafe(data, i)) *.
          (Array.getUnsafe(data, i + 1) +. Array.getUnsafe(data, j + 1)),
      )
    }
  sum(first, limit - dim, 0.)
}

let deviation = (data, holes, dim, triangles) => {
  let len = Array.length(data)
  let nh = Array.length(holes)
  let outer = if nh == 0 {
    len
  } else {
    Array.getUnsafe(holes, 0) * dim
  }
  let polygon = ref(Math.abs(signedArea(data, 0, outer, dim)))
  for i in 0 to nh - 1 {
    let limit = if i + 1 < nh {
      Array.getUnsafe(holes, i + 1) * dim
    } else {
      len
    }
    polygon :=
      polygon.contents -. Math.abs(signedArea(data, Array.getUnsafe(holes, i) * dim, limit, dim))
  }
  let rec sum = (i, total) =>
    if i >= Array.length(triangles) {
      total
    } else {
      let a = Array.getUnsafe(triangles, i) * dim
      let b = Array.getUnsafe(triangles, i + 1) * dim
      let c = Array.getUnsafe(triangles, i + 2) * dim
      sum(
        i + 3,
        total +.
        Math.abs(
          (Array.getUnsafe(data, a) -. Array.getUnsafe(data, c)) *.
            (Array.getUnsafe(data, b + 1) -. Array.getUnsafe(data, a + 1)) -.
            (Array.getUnsafe(data, a) -. Array.getUnsafe(data, b)) *.
              (Array.getUnsafe(data, c + 1) -. Array.getUnsafe(data, a + 1)),
        ),
      )
    }

  let total = sum(0, 0.)
  if total != 0. {
    Math.abs((total -. polygon.contents) /. polygon.contents)
  } else {
    let rec extent = (i, bound) =>
      if i >= len {
        bound
      } else {
        extent(
          i + dim,
          Math.max(
            bound,
            Math.max(Math.abs(Array.getUnsafe(data, i)), Math.abs(Array.getUnsafe(data, i + 1))),
          ),
        )
      }

    let bound = extent(0, 0.)
    if Math.abs(polygon.contents) <= Int.toFloat(len) *. bound *. bound *. 2.220446049250313e-16 {
      0.
    } else {
      1.
    }
  }
}
