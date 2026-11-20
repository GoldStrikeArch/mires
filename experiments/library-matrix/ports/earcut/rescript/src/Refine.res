/* Build half-edge twins with a linear-probing hash, then legalize interior
 edges with a LIFO worklist. All scratch arrays belong to one invocation. */
let orient = (ax, ay, bx, by_, cx, cy) => (bx -. ax) *. (cy -. ay) -. (by_ -. ay) *. (cx -. ax)

let inCircle = (ax, ay, bx, by_, cx, cy, px, py) => {
  let dx = ax -. px
  let dy = ay -. py
  let ex = bx -. px
  let ey = by_ -. py
  let fx = cx -. px
  let fy = cy -. py
  let ap = dx *. dx +. dy *. dy
  let bp = ex *. ex +. ey *. ey
  let cp = fx *. fx +. fy *. fy
  let s = ap +. bp +. cp
  dx *. (ey *. cp -. bp *. fy) -. dy *. (ex *. cp -. bp *. fx) +. ap *. (ex *. fy -. ey *. fx) <=
    1e-13 *. s *. s
}

let nextEdge = e => e - Int.mod(e, 3) + Int.mod(e + 1, 3)

let refine = (triangles, coords, dim) => {
  let n = Array.length(triangles)
  if n >= 6 {
    let size = ref(1)
    while size.contents < 4 * n {
      size := Int.shiftLeft(size.contents, 1)
    }
    let mask = size.contents - 1

    /* -2 is empty, -1 is a linked (consumed) pair. */
    let table = Array.make(~length=size.contents, -2)

    let twin = Array.make(~length=n, -1)
    let stack = Array.make(~length=n, 0)
    let queued = Array.make(~length=n, false)
    let count = ref(0)
    let push = e =>
      if Array.getUnsafe(twin, e) != -1 && !Array.getUnsafe(queued, e) {
        queued[e] = true
        stack[count.contents] = e
        count := count.contents + 1
      }

    for edge in 0 to n - 1 {
      let a = Array.getUnsafe(triangles, edge)
      let b = Array.getUnsafe(triangles, nextEdge(edge))
      let lo = if a < b {
        a
      } else {
        b
      }
      let hi = if a < b {
        b
      } else {
        a
      }
      let hash = Int.bitwiseAnd(
        Int.bitwiseXor(Math.Int.imul(lo, -1640531535), Math.Int.imul(hi, -2048144789)),
        mask,
      )
      let rec probe = slot => {
        let s = Array.getUnsafe(table, slot)
        if s == -2 {
          table[slot] = edge
        } else if (
          s >= 0 &&
            ((Array.getUnsafe(triangles, s) == lo &&
              Array.getUnsafe(triangles, nextEdge(s)) == hi) ||
              (Array.getUnsafe(triangles, s) == hi &&
                Array.getUnsafe(triangles, nextEdge(s)) == lo))
        ) {
          twin[edge] = s
          twin[s] = edge
          table[slot] = -1
          push(s)
        } else {
          probe(Int.bitwiseAnd(slot + 1, mask))
        }
      }
      probe(hash)
    }
    while count.contents > 0 {
      count := count.contents - 1
      let a = Array.getUnsafe(stack, count.contents)
      queued[a] = false
      let b = Array.getUnsafe(twin, a)
      if b != -1 {
        let a0 = a - Int.mod(a, 3)
        let b0 = b - Int.mod(b, 3)
        let ar = a0 + Int.mod(a + 2, 3)
        let al = a0 + Int.mod(a + 1, 3)
        let bl = b0 + Int.mod(b + 2, 3)
        let br = b0 + Int.mod(b + 1, 3)
        let p0 = Array.getUnsafe(triangles, ar)
        let pr = Array.getUnsafe(triangles, a)
        let pl = Array.getUnsafe(triangles, al)
        let p1 = Array.getUnsafe(triangles, bl)
        let x = p => Array.getUnsafe(coords, p * dim)
        let y = p => Array.getUnsafe(coords, p * dim + 1)
        let x0 = x(p0)
        let y0 = y(p0)
        let xr = x(pr)
        let yr = y(pr)
        let xl = x(pl)
        let yl = y(pl)
        let x1 = x(p1)
        let y1 = y(p1)
        if (
          !inCircle(x0, y0, xr, yr, xl, yl, x1, y1) &&
          (orient(x0, y0, xr, yr, x1, y1) > 0. &&
          orient(x0, y0, x1, y1, xl, yl) > 0.)
        ) {
          triangles[a] = p1
          triangles[b] = p0
          let hbl = Array.getUnsafe(twin, bl)
          let har = Array.getUnsafe(twin, ar)
          twin[a] = hbl
          if hbl != -1 {
            twin[hbl] = a
          }
          twin[b] = har
          if har != -1 {
            twin[har] = b
          }
          twin[ar] = bl
          twin[bl] = ar
          push(a)
          push(b)
          push(al)
          push(br)
        }
      }
    }
  }
}
