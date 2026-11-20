// d3-hierarchy 3.1.3 layout algorithms. Child and contour links are arena indices.
type node = {
  mutable id: string,
  mutable kids: array<int>,
  mutable parent: int,
  mutable depth: int,
  mutable height: int,
  mutable number: int,
  mutable ancestor: int,
  mutable defaultAncestor: int,
  mutable thread: int,
  mutable value: float,
  mutable x: float,
  mutable y: float,
  mutable r: float,
  mutable x0: float,
  mutable y0: float,
  mutable x1: float,
  mutable y1: float,
  mutable z: float,
  mutable m: float,
  mutable c: float,
  mutable s: float,
}
let circle = (x, y, r) => {
  id: "",
  kids: [],
  parent: -1,
  depth: 0,
  height: 0,
  number: 0,
  ancestor: 0,
  defaultAncestor: -1,
  thread: -1,
  value: 0.0,
  x0: 0.0,
  y0: 0.0,
  x1: 0.0,
  y1: 0.0,
  z: 0.0,
  m: 0.0,
  c: 0.0,
  s: 0.0,
  x,
  y,
  r,
}
let ints = n => Array.make(~length=n, 0)
let floats = n => Array.make(~length=n, 0.0)
let real = Int.toFloat
let integer = Float.toInt
@val external minf: (float, float) => float = "Math.min"
@val external maxf: (float, float) => float = "Math.max"
@val external floorf: float => float = "Math.floor"
@val external sqrtf: float => float = "Math.sqrt"
@val external roundf: float => float = "Math.round"
@val external absf: float => float = "Math.abs"
exception Return(int)
let invalidBasis = () => throw(Invalid_argument("invalid enclosing basis"))

let separation = (nodes: array<node>, a: int, b: int): float => {
  if Array.getUnsafe(nodes, a).parent == Array.getUnsafe(nodes, b).parent {
    1.0
  } else {
    2.0
  }
}

let contour = (nodes: array<node>, v: int, left: bool): int => {
  let kids = Array.getUnsafe(nodes, v).kids
  if Array.length(kids) == 0 {
    Array.getUnsafe(nodes, v).thread
  } else if left {
    Array.getUnsafe(kids, 0)
  } else {
    Array.getUnsafe(kids, Array.length(kids) - 1)
  }
}

let apportion = (nodes: array<node>, v: int, previous: int, fallback: int): int => {
  let ancestor = ref(fallback)
  if previous < 0 {
    ancestor.contents
  } else {
    let vip = ref(v)
    let vop = ref(v)
    let vim = ref(previous)
    let vom = ref(Array.getUnsafe(Array.getUnsafe(nodes, Array.getUnsafe(nodes, v).parent).kids, 0))
    let sip = ref(Array.getUnsafe(nodes, vip.contents).m)
    let sop = ref(Array.getUnsafe(nodes, vop.contents).m)
    let sim = ref(Array.getUnsafe(nodes, vim.contents).m)
    let som = ref(Array.getUnsafe(nodes, vom.contents).m)
    vim := contour(nodes, vim.contents, false)
    vip := contour(nodes, vip.contents, true)
    while vim.contents >= 0 && vip.contents >= 0 {
      vom := contour(nodes, vom.contents, true)
      vop := contour(nodes, vop.contents, false)
      Array.getUnsafe(nodes, vop.contents).ancestor = v
      let shift =
        Array.getUnsafe(nodes, vim.contents).z +.
        sim.contents -.
        Array.getUnsafe(nodes, vip.contents).z -.
        sip.contents +.
        separation(nodes, vim.contents, vip.contents)
      if shift > 0.0 {
        let anc = Array.getUnsafe(nodes, vim.contents).ancestor
        let wm = if Array.getUnsafe(nodes, anc).parent == Array.getUnsafe(nodes, v).parent {
          anc
        } else {
          ancestor.contents
        }
        let change =
          shift /. real(Array.getUnsafe(nodes, v).number - Array.getUnsafe(nodes, wm).number)
        Array.getUnsafe(nodes, v).c = Array.getUnsafe(nodes, v).c -. change
        Array.getUnsafe(nodes, v).s = Array.getUnsafe(nodes, v).s +. shift
        Array.getUnsafe(nodes, wm).c = Array.getUnsafe(nodes, wm).c +. change
        Array.getUnsafe(nodes, v).z = Array.getUnsafe(nodes, v).z +. shift
        Array.getUnsafe(nodes, v).m = Array.getUnsafe(nodes, v).m +. shift
        sip := sip.contents +. shift
        sop := sop.contents +. shift
      }
      sim := sim.contents +. Array.getUnsafe(nodes, vim.contents).m
      sip := sip.contents +. Array.getUnsafe(nodes, vip.contents).m
      som := som.contents +. Array.getUnsafe(nodes, vom.contents).m
      sop := sop.contents +. Array.getUnsafe(nodes, vop.contents).m
      vim := contour(nodes, vim.contents, false)
      vip := contour(nodes, vip.contents, true)
    }
    if vim.contents >= 0 && contour(nodes, vop.contents, false) < 0 {
      Array.getUnsafe(nodes, vop.contents).thread = vim.contents
      Array.getUnsafe(nodes, vop.contents).m =
        Array.getUnsafe(nodes, vop.contents).m +. (sim.contents -. sop.contents)
    }
    if vip.contents >= 0 && contour(nodes, vom.contents, true) < 0 {
      Array.getUnsafe(nodes, vom.contents).thread = vip.contents
      Array.getUnsafe(nodes, vom.contents).m =
        Array.getUnsafe(nodes, vom.contents).m +. (sip.contents -. som.contents)
      ancestor := v
    }
    ancestor.contents
  }
}

let tidy = (
  nodes: array<node>,
  pre: array<int>,
  post: array<int>,
  w: float,
  h: float,
  fixed: bool,
): unit => {
  let root = Array.getUnsafe(pre, 0)
  Array.forEach(pre, v => {
    Array.getUnsafe(nodes, v).ancestor = v
    Array.getUnsafe(nodes, v).thread = -1
    Array.getUnsafe(nodes, v).defaultAncestor = -1
    let kids = Array.getUnsafe(nodes, v).kids
    for i in 0 to Array.length(kids) - 1 {
      Array.getUnsafe(nodes, Array.getUnsafe(kids, i)).number = i
    }
  })
  Array.forEach(post, v => {
    let n = Array.getUnsafe(nodes, v)
    let p = n.parent
    let prev = ref(-1)
    if n.number > 0 {
      prev := Array.getUnsafe(Array.getUnsafe(nodes, p).kids, n.number - 1)
    }
    let kids = n.kids
    if Array.length(kids) > 0 {
      let shift = ref(0.0)
      let change = ref(0.0)
      for i in Array.length(kids) - 1 downto 0 {
        let child = Array.getUnsafe(nodes, Array.getUnsafe(kids, i))
        child.z = child.z +. shift.contents
        child.m = child.m +. shift.contents
        change := change.contents +. child.c
        shift := shift.contents +. (child.s +. change.contents)
      }
      let mid =
        (Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).z +.
        Array.getUnsafe(nodes, Array.getUnsafe(kids, Array.length(kids) - 1)).z) /. 2.0
      if prev.contents < 0 {
        n.z = mid
      } else {
        n.z = Array.getUnsafe(nodes, prev.contents).z +. separation(nodes, v, prev.contents)
        n.m = n.z -. mid
      }
    } else if prev.contents >= 0 {
      n.z = Array.getUnsafe(nodes, prev.contents).z +. separation(nodes, v, prev.contents)
    }
    if p >= 0 {
      let ancestor = ref(Array.getUnsafe(nodes, p).defaultAncestor)
      if ancestor.contents < 0 {
        ancestor := Array.getUnsafe(Array.getUnsafe(nodes, p).kids, 0)
      }
      Array.getUnsafe(nodes, p).defaultAncestor = apportion(
        nodes,
        v,
        prev.contents,
        ancestor.contents,
      )
    }
  })
  let offset = -Array.getUnsafe(nodes, root).z
  let left = ref(root)
  let right = ref(root)
  let depth = ref(0)
  Array.forEach(pre, v => {
    let n = Array.getUnsafe(nodes, v)
    let modifier = if v == root {
      offset
    } else {
      Array.getUnsafe(nodes, n.parent).m
    }
    n.x = n.z +. modifier
    n.m = n.m +. modifier
    if n.x < Array.getUnsafe(nodes, left.contents).x {
      left := v
    }
    if n.x > Array.getUnsafe(nodes, right.contents).x {
      right := v
    }
    if n.depth > depth.contents {
      depth := n.depth
    }
  })
  let margin = if left.contents == right.contents {
    1.0
  } else {
    separation(nodes, left.contents, right.contents) /. 2.0
  }
  let tx = margin -. Array.getUnsafe(nodes, left.contents).x
  let kx = w /. (Array.getUnsafe(nodes, right.contents).x +. margin +. tx)
  let ky =
    h /.
    real(
      if depth.contents == 0 {
        1
      } else {
        depth.contents
      },
    )
  Array.forEach(pre, v => {
    let n = Array.getUnsafe(nodes, v)
    if fixed {
      n.x = n.x *. w
      n.y = real(n.depth) *. h
    } else {
      n.x = (n.x +. tx) *. kx
      n.y = real(n.depth) *. ky
    }
  })
}

let cluster = (
  nodes: array<node>,
  pre: array<int>,
  post: array<int>,
  w: float,
  h: float,
  fixed: bool,
): unit => {
  let previous = ref(-1)
  let x = ref(0.0)
  Array.forEach(post, v => {
    let n = Array.getUnsafe(nodes, v)
    if Array.length(n.kids) == 0 {
      if previous.contents >= 0 {
        x := x.contents +. separation(nodes, v, previous.contents)
      }
      n.x = x.contents
      previous := v
    } else {
      let height = ref(0.0)
      let total = ref(0.0)
      Array.forEach(n.kids, c => {
        total := total.contents +. Array.getUnsafe(nodes, c).x
        height := maxf(height.contents, Array.getUnsafe(nodes, c).y)
      })
      n.x = total.contents /. real(Array.length(n.kids))
      n.y = 1.0 +. height.contents
    }
  })
  let left = Array.getUnsafe(post, 0)
  let right = previous.contents
  let x0 = Array.getUnsafe(nodes, left).x -. separation(nodes, left, right) /. 2.0
  let x1 = Array.getUnsafe(nodes, right).x +. separation(nodes, right, left) /. 2.0
  let rx = Array.getUnsafe(nodes, Array.getUnsafe(pre, 0)).x
  let ry = Array.getUnsafe(nodes, Array.getUnsafe(pre, 0)).y
  Array.forEach(post, v => {
    let n = Array.getUnsafe(nodes, v)
    if fixed {
      n.x = (n.x -. rx) *. w
      n.y = (ry -. n.y) *. h
    } else {
      n.x = (n.x -. x0) /. (x1 -. x0) *. w
      n.y =
        (1.0 -. if ry != 0.0 {
          n.y /. ry
        } else {
          1.0
        }) *. h
    }
  })
}

let tileSlice = (
  nodes: array<node>,
  kids: array<int>,
  start: int,
  stop: int,
  total: float,
  x0: float,
  y0: float,
  x1: float,
  y1: float,
  dice: bool,
): unit => {
  let x = ref(x0)
  let y = ref(y0)
  let scale = if total == 0.0 {
    total
  } else if dice {
    (x1 -. x.contents) /. total
  } else {
    (y1 -. y.contents) /. total
  }
  for i in start to stop - 1 {
    let n = Array.getUnsafe(nodes, Array.getUnsafe(kids, i))
    n.x0 = x.contents
    n.y0 = y.contents
    n.x1 = x1
    n.y1 = y1
    if dice {
      x := x.contents +. n.value *. scale
      n.x1 = x.contents
    } else {
      y := y.contents +. n.value *. scale
      n.y1 = y.contents
    }
  }
}

let rec binary = (
  nodes: array<node>,
  kids: array<int>,
  sums: array<float>,
  start: int,
  stop: int,
  total: float,
  x0: float,
  y0: float,
  x1: float,
  y1: float,
): unit => {
  if start >= stop - 1 {
    let n = Array.getUnsafe(nodes, Array.getUnsafe(kids, start))
    n.x0 = x0
    n.y0 = y0
    n.x1 = x1
    n.y1 = y1
  } else {
    let off = Array.getUnsafe(sums, start)
    let target = total /. 2.0 +. off
    let k = ref(start + 1)
    let hi = ref(stop - 1)
    while k.contents < hi.contents {
      let mid = (k.contents + hi.contents) / 2
      if Array.getUnsafe(sums, mid) < target {
        k := mid + 1
      } else {
        hi := mid
      }
    }
    if (
      target -. Array.getUnsafe(sums, k.contents - 1) <
        Array.getUnsafe(sums, k.contents) -. target && start + 1 < k.contents
    ) {
      k := k.contents - 1
    }
    let left = Array.getUnsafe(sums, k.contents) -. off
    let right = total -. left
    if x1 -. x0 > y1 -. y0 {
      let x = if total != 0.0 {
        (x0 *. right +. x1 *. left) /. total
      } else {
        x1
      }
      binary(nodes, kids, sums, start, k.contents, left, x0, y0, x, y1)
      binary(nodes, kids, sums, k.contents, stop, right, x, y0, x1, y1)
    } else {
      let y = if total != 0.0 {
        (y0 *. right +. y1 *. left) /. total
      } else {
        y1
      }
      binary(nodes, kids, sums, start, k.contents, left, x0, y0, x1, y)
      binary(nodes, kids, sums, k.contents, stop, right, x0, y, x1, y1)
    }
  }
}

let squarify = (
  nodes: array<node>,
  kids: array<int>,
  value: float,
  left: float,
  top: float,
  x1: float,
  y1: float,
): unit => {
  let total = ref(value)
  let x = ref(left)
  let y = ref(top)
  let i = ref(0)
  let j = ref(0)
  let ratio = (1.0 +. sqrtf(5.0)) /. 2.0
  while i.contents < Array.length(kids) {
    let dx = x1 -. x.contents
    let dy = y1 -. y.contents
    let row = ref(Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)).value)
    j := j.contents + 1
    while row.contents == 0.0 && j.contents < Array.length(kids) {
      row := Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)).value
      j := j.contents + 1
    }
    let low = ref(row.contents)
    let high = ref(row.contents)
    let alpha = maxf(dy /. dx, dx /. dy) /. (total.contents *. ratio)
    let beta = ref(row.contents *. row.contents *. alpha)
    let best = ref(maxf(high.contents /. beta.contents, beta.contents /. low.contents))
    let finished = ref(false)
    while j.contents < Array.length(kids) && !finished.contents {
      let v = Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)).value
      row := row.contents +. v
      low := minf(low.contents, v)
      high := maxf(high.contents, v)
      beta := row.contents *. row.contents *. alpha
      let score = maxf(high.contents /. beta.contents, beta.contents /. low.contents)
      if score > best.contents {
        row := row.contents -. v
        finished := true
      } else {
        best := score
        j := j.contents + 1
      }
    }
    if dx < dy {
      let bottom = if total.contents != 0.0 {
        y.contents +. dy *. row.contents /. total.contents
      } else {
        y1
      }
      tileSlice(
        nodes,
        kids,
        i.contents,
        j.contents,
        row.contents,
        x.contents,
        y.contents,
        x1,
        bottom,
        true,
      )
      if total.contents != 0.0 {
        y := bottom
      }
    } else {
      let right = if total.contents != 0.0 {
        x.contents +. dx *. row.contents /. total.contents
      } else {
        x1
      }
      tileSlice(
        nodes,
        kids,
        i.contents,
        j.contents,
        row.contents,
        x.contents,
        y.contents,
        right,
        y1,
        false,
      )
      if total.contents != 0.0 {
        x := right
      }
    }
    total := total.contents -. row.contents
    i := j.contents
  }
}

let rectangles = (
  nodes: array<node>,
  pre: array<int>,
  w: float,
  h: float,
  p: float,
  partition: bool,
  tile: string,
  rounded: bool,
): unit => {
  let root = Array.getUnsafe(nodes, Array.getUnsafe(pre, 0))
  let levels = real(root.height + 1)
  root.x1 = w
  root.y1 = h
  if partition {
    root.x0 = p
    root.y0 = p
    root.y1 = h /. levels
  }
  Array.forEach(pre, v => {
    let n = Array.getUnsafe(nodes, v)
    let kids = n.kids
    let inset = if n.parent >= 0 {
      p /. 2.0
    } else {
      0.0
    }
    if partition {
      if Array.length(kids) > 0 {
        tileSlice(
          nodes,
          kids,
          0,
          Array.length(kids),
          n.value,
          n.x0,
          h *. real(n.depth + 1) /. levels,
          n.x1,
          h *. real(n.depth + 2) /. levels,
          true,
        )
      }
      n.x1 = n.x1 -. p
      n.y1 = n.y1 -. p
    } else {
      n.x0 = n.x0 +. inset
      n.y0 = n.y0 +. inset
      n.x1 = n.x1 -. inset
      n.y1 = n.y1 -. inset
    }
    if n.x1 < n.x0 {
      n.x0 = (n.x0 +. n.x1) /. 2.0
      n.x1 = n.x0
    }
    if n.y1 < n.y0 {
      n.y0 = (n.y0 +. n.y1) /. 2.0
      n.y1 = n.y0
    }
    if !partition && Array.length(kids) > 0 {
      let gap = p -. p /. 2.0
      let x = ref(n.x0 +. gap)
      let y = ref(n.y0 +. gap)
      let xx = ref(n.x1 -. gap)
      let yy = ref(n.y1 -. gap)
      if xx.contents < x.contents {
        x := (x.contents +. xx.contents) /. 2.0
        xx := x.contents
      }
      if yy.contents < y.contents {
        y := (y.contents +. yy.contents) /. 2.0
        yy := y.contents
      }
      if tile == "binary" {
        let sums = floats(Array.length(kids) + 1)
        for j in 0 to Array.length(kids) - 1 {
          sums[
            j + 1
          ] =
            Array.getUnsafe(sums, j) +. Array.getUnsafe(nodes, Array.getUnsafe(kids, j)).value
        }
        binary(
          nodes,
          kids,
          sums,
          0,
          Array.length(kids),
          n.value,
          x.contents,
          y.contents,
          xx.contents,
          yy.contents,
        )
      } else if tile == "sliceDice" {
        tileSlice(
          nodes,
          kids,
          0,
          Array.length(kids),
          n.value,
          x.contents,
          y.contents,
          xx.contents,
          yy.contents,
          Int.mod(n.depth, 2) == 0,
        )
      } else {
        squarify(nodes, kids, n.value, x.contents, y.contents, xx.contents, yy.contents)
      }
    }
  })
  if rounded {
    Array.forEach(pre, v => {
      let n = Array.getUnsafe(nodes, v)
      n.x0 = roundf(n.x0)
      n.x1 = roundf(n.x1)
      n.y0 = roundf(n.y0)
      n.y1 = roundf(n.y1)
    })
  }
}

let weak = (a: node, b: node): bool => {
  let dr = a.r -. b.r +. maxf(maxf(a.r, b.r), 1.0) *. 1e-09
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  dr > 0.0 && dr *. dr > dx *. dx +. dy *. dy
}

let outside = (a: node, b: node): bool => {
  let dr = a.r -. b.r
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  dr < 0.0 || dr *. dr < dx *. dx +. dy *. dy
}

let pair = (a: node, b: node): node => {
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  let dr = b.r -. a.r
  let distance = sqrtf(dx *. dx +. dy *. dy)
  circle(
    (a.x +. b.x +. dx /. distance *. dr) /. 2.0,
    (a.y +. b.y +. dy /. distance *. dr) /. 2.0,
    (distance +. a.r +. b.r) /. 2.0,
  )
}

let triple = (a: node, b: node, c: node): node => {
  let a2 = a.x -. b.x
  let a3 = a.x -. c.x
  let b2 = a.y -. b.y
  let b3 = a.y -. c.y
  let c2 = b.r -. a.r
  let c3 = c.r -. a.r
  let d1 = a.x *. a.x +. a.y *. a.y -. a.r *. a.r
  let d2 = d1 -. b.x *. b.x -. b.y *. b.y +. b.r *. b.r
  let d3 = d1 -. c.x *. c.x -. c.y *. c.y +. c.r *. c.r
  let ab = a3 *. b2 -. a2 *. b3
  let xa = (b2 *. d3 -. b3 *. d2) /. (ab *. 2.0) -. a.x
  let xb = (b3 *. c2 -. b2 *. c3) /. ab
  let ya = (a3 *. d2 -. a2 *. d3) /. (ab *. 2.0) -. a.y
  let yb = (a2 *. c3 -. a3 *. c2) /. ab
  let qa = xb *. xb +. yb *. yb -. 1.0
  let qb = 2.0 *. (a.r +. xa *. xb +. ya *. yb)
  let qc = xa *. xa +. ya *. ya -. a.r *. a.r
  let r = -(
    if absf(qa) > 1e-06 {
      (qb +. sqrtf(qb *. qb -. 4.0 *. qa *. qc)) /. (2.0 *. qa)
    } else {
      qc /. qb
    }
  )
  circle(a.x +. xa +. xb *. r, a.y +. ya +. yb *. r, r)
}

let covers = (nodes: array<node>, basis: array<int>, count: int, a: node): bool => {
  let i = ref(0)
  while i.contents < count && weak(a, Array.getUnsafe(nodes, Array.getUnsafe(basis, i.contents))) {
    i := i.contents + 1
  }
  i.contents == count
}

let extend = (nodes: array<node>, basis: array<int>, count: int, p: int): int => {
  try {
    if covers(nodes, basis, count, Array.getUnsafe(nodes, p)) {
      basis[0] = p
      throw(Return(1))
    }
    for i in 0 to count - 1 {
      let x = Array.getUnsafe(basis, i)
      if (
        outside(Array.getUnsafe(nodes, p), Array.getUnsafe(nodes, x)) &&
        covers(nodes, basis, count, pair(Array.getUnsafe(nodes, x), Array.getUnsafe(nodes, p)))
      ) {
        basis[0] = x
        basis[1] = p
        throw(Return(2))
      }
    }
    for i in 0 to count - 1 {
      let x = Array.getUnsafe(basis, i)
      for j in i + 1 to count - 1 {
        let y = Array.getUnsafe(basis, j)
        if (
          outside(
            pair(Array.getUnsafe(nodes, x), Array.getUnsafe(nodes, y)),
            Array.getUnsafe(nodes, p),
          ) &&
          outside(
            pair(Array.getUnsafe(nodes, x), Array.getUnsafe(nodes, p)),
            Array.getUnsafe(nodes, y),
          ) &&
          outside(
            pair(Array.getUnsafe(nodes, y), Array.getUnsafe(nodes, p)),
            Array.getUnsafe(nodes, x),
          ) &&
          covers(
            nodes,
            basis,
            count,
            triple(Array.getUnsafe(nodes, x), Array.getUnsafe(nodes, y), Array.getUnsafe(nodes, p)),
          )
        ) {
          basis[0] = x
          basis[1] = y
          basis[2] = p
          throw(Return(3))
        }
      }
    }
    invalidBasis()
  } catch {
  | Return(result) => result
  }
}

let enclose = (nodes: array<node>, front: array<int>, n: int, seed: array<float>): node => {
  for j in n - 1 downto 0 {
    let step = 1664525.0 *. Array.getUnsafe(seed, 0) +. 1013904223.0
    seed[0] = step -. floorf(step /. 4294967296.0) *. 4294967296.0
    let k = integer(Array.getUnsafe(seed, 0) /. 4294967296.0 *. real(j + 1))
    let save = Array.getUnsafe(front, j)
    front[j] = Array.getUnsafe(front, k)
    front[k] = save
  }
  let basis = ints(3)
  let count = ref(0)
  let i = ref(0)
  let result = ref(circle(0.0, 0.0, 0.0))
  while i.contents < n {
    let p = Array.getUnsafe(front, i.contents)
    if count.contents > 0 && weak(result.contents, Array.getUnsafe(nodes, p)) {
      i := i.contents + 1
    } else {
      count := extend(nodes, basis, count.contents, p)
      if count.contents == 1 {
        result :=
          circle(
            Array.getUnsafe(nodes, p).x,
            Array.getUnsafe(nodes, p).y,
            Array.getUnsafe(nodes, p).r,
          )
      } else if count.contents == 2 {
        result :=
          pair(
            Array.getUnsafe(nodes, Array.getUnsafe(basis, 0)),
            Array.getUnsafe(nodes, Array.getUnsafe(basis, 1)),
          )
      } else {
        result :=
          triple(
            Array.getUnsafe(nodes, Array.getUnsafe(basis, 0)),
            Array.getUnsafe(nodes, Array.getUnsafe(basis, 1)),
            Array.getUnsafe(nodes, Array.getUnsafe(basis, 2)),
          )
      }
      i := 0
    }
  }
  result.contents
}

let place = (b: node, a: node, c: node): unit => {
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  let d2 = dx *. dx +. dy *. dy
  if d2 != 0.0 {
    let a2 = (a.r +. c.r) *. (a.r +. c.r)
    let b2 = (b.r +. c.r) *. (b.r +. c.r)
    if a2 > b2 {
      let x = (d2 +. b2 -. a2) /. (2.0 *. d2)
      let y = sqrtf(maxf(0.0, b2 /. d2 -. x *. x))
      c.x = b.x -. x *. dx -. y *. dy
      c.y = b.y -. x *. dy +. y *. dx
    } else {
      let x = (d2 +. a2 -. b2) /. (2.0 *. d2)
      let y = sqrtf(maxf(0.0, a2 /. d2 -. x *. x))
      c.x = a.x +. x *. dx -. y *. dy
      c.y = a.y +. x *. dy +. y *. dx
    }
  } else {
    c.x = a.x +. c.r
    c.y = a.y
  }
}

let intersects = (a: node, b: node): bool => {
  let dr = a.r +. b.r -. 1e-06
  let dx = b.x -. a.x
  let dy = b.y -. a.y
  dr > 0.0 && dr *. dr > dx *. dx +. dy *. dy
}

let score = (a: node, b: node): float => {
  let ab = a.r +. b.r
  let dx = (a.x *. b.r +. b.x *. a.r) /. ab
  let dy = (a.y *. b.r +. b.y *. a.r) /. ab
  dx *. dx +. dy *. dy
}

let siblings = (nodes: array<node>, kids: array<int>, seed: array<float>): float => {
  let n = Array.length(kids)
  let a = ref(0)
  let b = ref(1)
  Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).x = 0.0
  Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).y = 0.0
  if n == 1 {
    Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).r
  } else {
    Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).x = -Array.getUnsafe(
      nodes,
      Array.getUnsafe(kids, 1),
    ).r
    Array.getUnsafe(nodes, Array.getUnsafe(kids, 1)).x = Array.getUnsafe(
      nodes,
      Array.getUnsafe(kids, 0),
    ).r
    Array.getUnsafe(nodes, Array.getUnsafe(kids, 1)).y = 0.0
    if n == 2 {
      Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)).r +.
      Array.getUnsafe(nodes, Array.getUnsafe(kids, 1)).r
    } else {
      place(
        Array.getUnsafe(nodes, Array.getUnsafe(kids, 1)),
        Array.getUnsafe(nodes, Array.getUnsafe(kids, 0)),
        Array.getUnsafe(nodes, Array.getUnsafe(kids, 2)),
      )
      let after = ints(n)
      let before = ints(n)
      after[0] = 1
      before[2] = 1
      after[1] = 2
      before[0] = 2
      after[2] = 0
      before[1] = 0
      let i = ref(3)
      while i.contents < n {
        place(
          Array.getUnsafe(nodes, Array.getUnsafe(kids, a.contents)),
          Array.getUnsafe(nodes, Array.getUnsafe(kids, b.contents)),
          Array.getUnsafe(nodes, Array.getUnsafe(kids, i.contents)),
        )
        let j = ref(Array.getUnsafe(after, b.contents))
        let k = ref(Array.getUnsafe(before, a.contents))
        let sj = ref(Array.getUnsafe(nodes, Array.getUnsafe(kids, b.contents)).r)
        let sk = ref(Array.getUnsafe(nodes, Array.getUnsafe(kids, a.contents)).r)
        let retry = ref(false)
        let more = ref(true)
        while more.contents && !retry.contents {
          if sj.contents <= sk.contents {
            if (
              intersects(
                Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)),
                Array.getUnsafe(nodes, Array.getUnsafe(kids, i.contents)),
              )
            ) {
              b := j.contents
              after[a.contents] = b.contents
              before[b.contents] = a.contents
              retry := true
            } else {
              sj := sj.contents +. Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)).r
              j := Array.getUnsafe(after, j.contents)
            }
          } else if (
            intersects(
              Array.getUnsafe(nodes, Array.getUnsafe(kids, k.contents)),
              Array.getUnsafe(nodes, Array.getUnsafe(kids, i.contents)),
            )
          ) {
            a := k.contents
            after[a.contents] = b.contents
            before[b.contents] = a.contents
            retry := true
          } else {
            sk := sk.contents +. Array.getUnsafe(nodes, Array.getUnsafe(kids, k.contents)).r
            k := Array.getUnsafe(before, k.contents)
          }
          more := j.contents != Array.getUnsafe(after, k.contents)
        }
        if !retry.contents {
          before[i.contents] = a.contents
          after[i.contents] = b.contents
          after[a.contents] = i.contents
          before[b.contents] = i.contents
          b := i.contents
          let best = ref(
            score(
              Array.getUnsafe(nodes, Array.getUnsafe(kids, a.contents)),
              Array.getUnsafe(nodes, Array.getUnsafe(kids, Array.getUnsafe(after, a.contents))),
            ),
          )
          j := Array.getUnsafe(after, i.contents)
          while j.contents != b.contents {
            let candidateScore = score(
              Array.getUnsafe(nodes, Array.getUnsafe(kids, j.contents)),
              Array.getUnsafe(nodes, Array.getUnsafe(kids, Array.getUnsafe(after, j.contents))),
            )
            if candidateScore < best.contents {
              best := candidateScore
              a := j.contents
            }
            j := Array.getUnsafe(after, j.contents)
          }
          b := Array.getUnsafe(after, a.contents)
          i := i.contents + 1
        }
      }
      let front = ints(n)
      front[0] = Array.getUnsafe(kids, b.contents)
      let count = ref(1)
      let cursor = ref(Array.getUnsafe(after, b.contents))
      while cursor.contents != b.contents {
        front[count.contents] = Array.getUnsafe(kids, cursor.contents)
        count := count.contents + 1
        cursor := Array.getUnsafe(after, cursor.contents)
      }
      let enclosure = enclose(nodes, front, count.contents, seed)
      Array.forEach(kids, id => {
        Array.getUnsafe(nodes, id).x = Array.getUnsafe(nodes, id).x -. enclosure.x
        Array.getUnsafe(nodes, id).y = Array.getUnsafe(nodes, id).y -. enclosure.y
      })
      enclosure.r
    }
  }
}

let packChildren = (
  nodes: array<node>,
  post: array<int>,
  padding: float,
  seed: array<float>,
): unit => {
  let pad = if padding != padding {
    0.0
  } else {
    padding
  }
  Array.forEach(post, i => {
    let kids = Array.getUnsafe(nodes, i).kids
    if Array.length(kids) > 0 {
      if pad != 0.0 {
        Array.forEach(kids, k => {
          Array.getUnsafe(nodes, k).r = Array.getUnsafe(nodes, k).r +. pad
        })
      }
      let radius = siblings(nodes, kids, seed)
      if pad != 0.0 {
        Array.forEach(kids, k => {
          Array.getUnsafe(nodes, k).r = Array.getUnsafe(nodes, k).r -. pad
        })
      }
      Array.getUnsafe(nodes, i).r = radius +. pad
    }
  })
}

let pack = (
  nodes: array<node>,
  pre: array<int>,
  post: array<int>,
  w: float,
  h: float,
  p: float,
): unit => {
  let root = Array.getUnsafe(nodes, Array.getUnsafe(pre, 0))
  root.x = w /. 2.0
  root.y = h /. 2.0
  Array.forEach(pre, i => {
    let n = Array.getUnsafe(nodes, i)
    if Array.length(n.kids) == 0 {
      let raw = sqrtf(n.value)
      n.r = if raw == raw {
        maxf(0.0, raw)
      } else {
        0.0
      }
    }
  })
  let seed = floats(1)
  seed[0] = 1.0
  packChildren(nodes, post, 0.0, seed)
  packChildren(nodes, post, p *. (root.r /. minf(w, h)), seed)
  let k = minf(w, h) /. (2.0 *. root.r)
  Array.forEach(pre, i => {
    let n = Array.getUnsafe(nodes, i)
    n.r = n.r *. k
    if n.parent >= 0 {
      n.x = Array.getUnsafe(nodes, n.parent).x +. k *. n.x
      n.y = Array.getUnsafe(nodes, n.parent).y +. k *. n.y
    }
  })
}
