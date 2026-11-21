// tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
// 3.1.3, with the default separation.

open Hierarchy

/** `defaultSeparation`, the same function in tree.js and cluster.js. */
let separation = (a: node, b: node) =>
  switch (a.parent, b.parent) {
  | (Some(p), Some(q)) if p === q => 1.
  | (None, None) => 1.
  | _ => 2.
  }

/**
 * tree.js's `TreeNode`: the layout's bookkeeping for `node` (`_`), the child number `i`. Every
 * node has a parent; d3's extra parent of the root has none.
 */
type rec treeNode = {
  node: node,
  i: int,
  mutable parent: option<treeNode>,
  mutable children: array<treeNode>,
  mutable defaultAncestor: option<treeNode>, // A
  mutable ancestor: treeNode, // a
  mutable prelim: float, // z
  mutable modifier: float, // m
  mutable change: float, // c
  mutable shift: float, // s
  mutable thread: option<treeNode>, // t
}

let treeNode = (node, i) => {
  let rec t = {
    node,
    i,
    parent: None,
    children: [],
    defaultAncestor: None,
    ancestor: t,
    prelim: 0.,
    modifier: 0.,
    change: 0.,
    shift: 0.,
    thread: None,
  }
  t
}

let parentOf = v => v.parent->Option.getOrThrow

/** The left contour's successor of `v`: its first child, or its thread. */
let nextLeft = v => Array.length(v.children) > 0 ? v.children[0] : v.thread

/** The right contour's successor of `v`: its last child, or its thread. */
let nextRight = v => Array.length(v.children) > 0 ? v.children->Array.at(-1) : v.thread

/** Shifts the subtree rooted at `wp` and prepares the shifts of the smaller subtrees in between. */
let moveSubtree = (wm, wp, shift) => {
  let change = shift /. Int.toFloat(wp.i - wm.i)
  wp.change = wp.change -. change
  wp.shift = wp.shift +. shift
  wm.change = wm.change +. change
  wp.prelim = wp.prelim +. shift
  wp.modifier = wp.modifier +. shift
}

/** Applies the shifts that `moveSubtree` prepared to the children of `v`. */
let executeShifts = v =>
  v.children
  ->Array.reduceRight((0., 0.), ((shift, change), w) => {
    w.prelim = w.prelim +. shift
    w.modifier = w.modifier +. shift
    let change = change +. w.change
    (shift +. (w.shift +. change), change)
  })
  ->ignore

/** `vim`'s ancestor if it is a sibling of `v`, otherwise the default `ancestor`. */
let nextAncestor = (vim, v, ancestor) =>
  parentOf(vim.ancestor) === parentOf(v) ? vim.ancestor : ancestor

/** `treeRoot`: wraps every node in a `treeNode`, under an extra parent of the root. */
let treeRoot = root => {
  let tree = treeNode(root, 0)
  let nodes = [tree]
  let rec wrap = () =>
    switch nodes->Array.pop {
    | None => ()
    | Some(node) =>
      let children = node.node.children->Array.mapWithIndex(treeNode)
      children->Array.forEach(child => child.parent = Some(node))
      node.children = children
      for i in Array.length(children) - 1 downto 0 {
        nodes->Array.push(children->Array.getUnsafe(i))
      }
      wrap()
    }
  wrap()
  // d3's extra parent wraps null; it is never read, so it wraps the root here.
  let extra = treeNode(root, 0)
  extra.children = [tree]
  tree.parent = Some(extra)
  tree
}

/**
 * The core of the algorithm: combines the subtree of `v` with those of its left siblings, walking
 * the inside and outside contours, and returns the new default ancestor.
 */
let apportion = (v, w, ancestor) =>
  switch w {
  | None => ancestor
  | Some(w) =>
    let vom = (parentOf(v).children)->Array.getUnsafe(0)
    let rec contours = (vim, vip, vom, vop, sim, sip, som, sop, ancestor) =>
      switch (nextRight(vim), nextLeft(vip)) {
      | (Some(im), Some(ip)) =>
        let om = nextLeft(vom)->Option.getOrThrow
        let op = nextRight(vop)->Option.getOrThrow
        op.ancestor = v
        let shift = im.prelim +. sim -. ip.prelim -. sip +. separation(im.node, ip.node)
        let (sip, sop) = if shift > 0. {
          moveSubtree(nextAncestor(im, v, ancestor), v, shift)
          (sip +. shift, sop +. shift)
        } else {
          (sip, sop)
        }
        contours(
          im,
          ip,
          om,
          op,
          sim +. im.modifier,
          sip +. ip.modifier,
          som +. om.modifier,
          sop +. op.modifier,
          ancestor,
        )
      | (im, ip) =>
        switch im {
        | Some(im) if nextRight(vop)->Option.isNone =>
          vop.thread = Some(im)
          vop.modifier = vop.modifier +. (sim -. sop)
        | _ => ()
        }
        switch ip {
        | Some(ip) if nextLeft(vom)->Option.isNone =>
          vom.thread = Some(ip)
          vom.modifier = vom.modifier +. (sip -. som)
          v
        | _ => ancestor
        }
      }
    contours(w, v, vom, v, w.modifier, v.modifier, vom.modifier, v.modifier, ancestor)
  }

/** Computes the preliminary x of `v`, after its children; then combines it with its left siblings. */
let firstWalk = v => {
  let parent = parentOf(v)
  let siblings = parent.children
  let w = v.i != 0 ? siblings[v.i - 1] : None
  if Array.length(v.children) > 0 {
    executeShifts(v)
    let midpoint =
      ((v.children->Array.getUnsafe(0)).prelim +.
        (v.children->Array.at(-1)->Option.getOrThrow).prelim) /. 2.
    switch w {
    | Some(w) =>
      v.prelim = w.prelim +. separation(v.node, w.node)
      v.modifier = v.prelim -. midpoint
    | None => v.prelim = midpoint
    }
  } else {
    w->Option.forEach(w => v.prelim = w.prelim +. separation(v.node, w.node))
  }
  parent.defaultAncestor = Some(
    apportion(v, w, parent.defaultAncestor->Option.getOr(siblings->Array.getUnsafe(0))),
  )
}

/** Computes the real x of `v` by summing the modifiers of its ancestors. */
let secondWalk = v => {
  let parent = parentOf(v)
  v.node.x = v.prelim +. parent.modifier
  v.modifier = v.modifier +. parent.modifier
}

/** `tree().size([dx, dy])`, or `tree().nodeSize([dx, dy])` when `nodeSize` is set. */
let tree = (root, dx, dy, nodeSize) => {
  let t = treeRoot(root)
  let children = v => v.children

  // Compute the layout using Buchheim et al.'s algorithm.
  t->eachAfterBy(children, firstWalk)->ignore
  parentOf(t).modifier = -.t.prelim
  t->eachBeforeBy(children, secondWalk)->ignore

  if nodeSize {
    // A fixed node size scales x and y.
    root
    ->eachBefore(node => {
      node.x = node.x *. dx
      node.y = Int.toFloat(node.depth) *. dy
    })
    ->ignore
  } else {
    // A fixed tree size scales x and y by the extent of the left-most, right-most and deepest
    // nodes.
    let left = ref(root)
    let right = ref(root)
    let bottom = ref(root)
    root
    ->eachBefore(node => {
      if node.x < left.contents.x {
        left.contents = node
      }
      if node.x > right.contents.x {
        right.contents = node
      }
      if node.depth > bottom.contents.depth {
        bottom.contents = node
      }
    })
    ->ignore
    let s = left.contents === right.contents ? 1. : separation(left.contents, right.contents) /. 2.
    let tx = s -. left.contents.x
    let kx = dx /. (right.contents.x +. s +. tx)
    let ky = dy /. Int.toFloat(bottom.contents.depth != 0 ? bottom.contents.depth : 1)
    root
    ->eachBefore(node => {
      node.x = (node.x +. tx) *. kx
      node.y = Int.toFloat(node.depth) *. ky
    })
    ->ignore
  }
}

let rec leafLeft = (node: node) =>
  Array.length(node.children) > 0 ? leafLeft(node.children->Array.getUnsafe(0)) : node

let rec leafRight = (node: node) =>
  switch node.children->Array.at(-1) {
  | Some(child) => leafRight(child)
  | None => node
  }

/** `cluster().size([dx, dy])`, or `cluster().nodeSize([dx, dy])` when `nodeSize` is set. */
let cluster = (root, dx, dy, nodeSize) => {
  let previousNode = ref(None)
  let x = ref(0.)

  // First walk, computing the initial x and y values.
  root
  ->eachAfter(node => {
    let children = node.children
    if Array.length(children) > 0 {
      node.x =
        children->Array.reduce(0., (sum, c) => sum +. c.x) /. Int.toFloat(Array.length(children)) // meanX
      node.y = 1. +. children->Array.reduce(0., (y, c) => Math.max(y, c.y)) // maxY
    } else {
      switch previousNode.contents {
      | Some(previous) =>
        x.contents = x.contents +. separation(node, previous)
        node.x = x.contents
      | None => node.x = 0.
      }
      node.y = 0.
      previousNode.contents = Some(node)
    }
  })
  ->ignore

  let left = leafLeft(root)
  let right = leafRight(root)
  let x0 = left.x -. separation(left, right) /. 2.
  let x1 = right.x +. separation(right, left) /. 2.

  // Second walk, normalizing x and y to the desired size. The root comes last, so the walk reads
  // the root's unnormalized position throughout.
  root
  ->eachAfter(
    nodeSize
      ? node => {
          node.x = (node.x -. root.x) *. dx
          node.y = (root.y -. node.y) *. dy
        }
      : node => {
          node.x = (node.x -. x0) /. (x1 -. x0) *. dx
          node.y = (1. -. (truthy(root.y) ? node.y /. root.y : 1.)) *. dy
        },
  )
  ->ignore
}
