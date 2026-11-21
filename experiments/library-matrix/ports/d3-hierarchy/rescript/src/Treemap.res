// treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
// partition.js of d3-hierarchy 3.1.3.

open Hierarchy

/** round.js. Math.round rounds halves up, keeps -0 and returns -0 for [-0.5, 0). */
let roundNode = node => {
  node.x0 = Math.round(node.x0)
  node.y0 = Math.round(node.y0)
  node.x1 = Math.round(node.x1)
  node.y1 = Math.round(node.y1)
}

/**
 * dice.js on `nodes`, the children of a parent or of a squarified row, whose value is `value`:
 * columns from left to right.
 */
let treemapDice = (nodes: array<node>, value, x0, y0, x1, y1) => {
  let k = truthy(value) ? (x1 -. x0) /. value : value
  nodes
  ->Array.reduce(x0, (x0, node) => {
    node.y0 = y0
    node.y1 = y1
    node.x0 = x0
    node.x1 = x0 +. node.value *. k
    node.x1
  })
  ->ignore
}

/** slice.js, as `treemapDice`: rows from top to bottom. */
let treemapSlice = (nodes: array<node>, value, x0, y0, x1, y1) => {
  let k = truthy(value) ? (y1 -. y0) /. value : value
  nodes
  ->Array.reduce(y0, (y0, node) => {
    node.x0 = x0
    node.x1 = x1
    node.y0 = y0
    node.y1 = y0 +. node.value *. k
    node.y1
  })
  ->ignore
}

/** sliceDice.js: slice at odd depths, dice at even ones. */
let sliceDice = (parent, x0, y0, x1, y1) =>
  (Int.bitwiseAnd(parent.depth, 1) != 0 ? treemapSlice : treemapDice)(
    parent.children,
    parent.value,
    x0,
    y0,
    x1,
    y1,
  )

let phi = (1. +. Math.sqrt(5.)) /. 2.

/** `squarifyRatio`. d3 also collects the rows, for treemapResquarify; this port only lays them out. */
let squarifyRatio = (ratio, parent, x0, y0, x1, y1) => {
  let nodes = parent.children
  let n = Array.length(nodes)
  let valueOf = i => (nodes->Array.getUnsafe(i)).value

  // Find the next non-empty node, from i1: its value and the index after it.
  let rec nextNonEmpty = i1 => {
    let sumValue = valueOf(i1)
    !truthy(sumValue) && i1 + 1 < n ? nextNonEmpty(i1 + 1) : (sumValue, i1 + 1)
  }

  // Keep adding nodes while the aspect ratio maintains or improves: the row's value and end.
  let rec grow = (i1, sumValue, minValue, maxValue, minRatio, alpha) =>
    if i1 < n {
      let nodeValue = valueOf(i1)
      let total = sumValue +. nodeValue
      let minValue = nodeValue < minValue ? nodeValue : minValue
      let maxValue = nodeValue > maxValue ? nodeValue : maxValue
      let beta = total *. total *. alpha
      let newRatio = Math.max(maxValue /. beta, beta /. minValue)
      // d3 adds the value and takes it back off, which rounds as it does here.
      newRatio > minRatio
        ? (total -. nodeValue, i1)
        : grow(i1 + 1, total, minValue, maxValue, newRatio, alpha)
    } else {
      (sumValue, i1)
    }

  let rec rows = (i0, value, x0, y0) =>
    if i0 < n {
      let dx = x1 -. x0
      let dy = y1 -. y0
      let (sumValue, i1) = nextNonEmpty(i0)
      let alpha = Math.max(dy /. dx, dx /. dy) /. (value *. ratio)
      let beta = sumValue *. sumValue *. alpha
      let minRatio = Math.max(sumValue /. beta, beta /. sumValue)
      let (sumValue, i1) = grow(i1, sumValue, sumValue, sumValue, minRatio, alpha)

      // Position the row in its orientation.
      let row = nodes->Array.slice(~start=i0, ~end=i1)
      if dx < dy {
        let bottom = truthy(value) ? y0 +. dy *. sumValue /. value : y1
        treemapDice(row, sumValue, x0, y0, x1, bottom)
        rows(i1, value -. sumValue, x0, truthy(value) ? bottom : y0)
      } else {
        let right = truthy(value) ? x0 +. dx *. sumValue /. value : x1
        treemapSlice(row, sumValue, x0, y0, right, y1)
        rows(i1, value -. sumValue, truthy(value) ? right : x0, y0)
      }
    }
  rows(0, parent.value, x0, y0)
}

/** squarify.js with the default ratio, the golden ratio. */
let squarify = (parent, x0, y0, x1, y1) => squarifyRatio(phi, parent, x0, y0, x1, y1)

/** binary.js: splits the children in two of about equal value, recursively. */
let binary = (parent, x0, y0, x1, y1) => {
  let nodes = parent.children
  let n = Array.length(nodes)
  let sums = [0.]
  nodes->Array.forEachWithIndex((node, i) => sums->Array.push(sums->Array.getUnsafe(i) +. node.value))
  let sum = i => sums->Array.getUnsafe(i)

  let rec partition = (i, j, value, x0, y0, x1, y1) =>
    if i >= j - 1 {
      let node = nodes->Array.getUnsafe(i)
      node.x0 = x0
      node.y0 = y0
      node.x1 = x1
      node.y1 = y1
    } else {
      let valueOffset = sum(i)
      let valueTarget = value /. 2. +. valueOffset
      let rec search = (k, hi) =>
        if k < hi {
          let mid = Int.shiftRightUnsigned(k + hi, 1)
          sum(mid) < valueTarget ? search(mid + 1, hi) : search(k, mid)
        } else {
          k
        }
      let k = search(i + 1, j - 1)
      let k = valueTarget -. sum(k - 1) < sum(k) -. valueTarget && i + 1 < k ? k - 1 : k

      let valueLeft = sum(k) -. valueOffset
      let valueRight = value -. valueLeft
      if x1 -. x0 > y1 -. y0 {
        let xk = truthy(value) ? (x0 *. valueRight +. x1 *. valueLeft) /. value : x1
        partition(i, k, valueLeft, x0, y0, xk, y1)
        partition(k, j, valueRight, xk, y0, x1, y1)
      } else {
        let yk = truthy(value) ? (y0 *. valueRight +. y1 *. valueLeft) /. value : y1
        partition(i, k, valueLeft, x0, y0, x1, yk)
        partition(k, j, valueRight, x0, yk, x1, y1)
      }
    }
  partition(0, n, parent.value, x0, y0, x1, y1)
}

/** The treemap's and partition's `if (x1 < x0) x0 = x1 = (x0 + x1) / 2`, and the same for y. */
let collapse = (x0, y0, x1, y1) => {
  let (x0, x1) = x1 < x0 ? ((x0 +. x1) /. 2., (x0 +. x1) /. 2.) : (x0, x1)
  let (y0, y1) = y1 < y0 ? ((y0 +. y1) /. 2., (y0 +. y1) /. 2.) : (y0, y1)
  (x0, y0, x1, y1)
}

/**
 * `treemap().size([dx, dy]).tile(tile).padding(padding).round(round)`: the inner and every outer
 * padding are `padding`.
 */
let treemap = (root, dx, dy, tile, padding, round) => {
  // d3 keeps the stack between calls and resets it to [0]; depth d reads index d.
  let paddingStack = Array.make(~length=root.height + 1, 0.)
  root.x0 = 0.
  root.y0 = 0.
  root.x1 = dx
  root.y1 = dy
  root
  ->eachBefore(node => {
    let p = paddingStack->Array.getUnsafe(node.depth)
    let (x0, y0, x1, y1) = collapse(node.x0 +. p, node.y0 +. p, node.x1 -. p, node.y1 -. p)
    node.x0 = x0
    node.y0 = y0
    node.x1 = x1
    node.y1 = y1
    if Array.length(node.children) > 0 {
      let p = padding /. 2.
      paddingStack->Array.setUnsafe(node.depth + 1, p)
      let (x0, y0, x1, y1) = collapse(
        x0 +. (padding -. p),
        y0 +. (padding -. p),
        x1 -. (padding -. p),
        y1 -. (padding -. p),
      )
      tile(node, x0, y0, x1, y1)
    }
  })
  ->ignore
  if round {
    root->eachBefore(roundNode)->ignore
  }
}

/** `partition().size([dx, dy]).padding(padding).round(round)`. */
let partition = (root, dx, dy, padding, round) => {
  let n = Int.toFloat(root.height + 1)
  root.x0 = padding
  root.y0 = padding
  root.x1 = dx
  root.y1 = dy /. n
  root
  ->eachBefore(node => {
    if Array.length(node.children) > 0 {
      treemapDice(
        node.children,
        node.value,
        node.x0,
        dy *. Int.toFloat(node.depth + 1) /. n,
        node.x1,
        dy *. Int.toFloat(node.depth + 2) /. n,
      )
    }
    let (x0, y0, x1, y1) = collapse(node.x0, node.y0, node.x1 -. padding, node.y1 -. padding)
    node.x0 = x0
    node.y0 = y0
    node.x1 = x1
    node.y1 = y1
  })
  ->ignore
  if round {
    root->eachBefore(roundNode)->ignore
  }
}
