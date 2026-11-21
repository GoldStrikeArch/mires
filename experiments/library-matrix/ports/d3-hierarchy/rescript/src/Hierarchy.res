// The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
// (`hierarchy`, `computeHeight`), eachBefore.js, eachAfter.js, iterator.js, sum.js, sort.js, and
// stratify.js with the default id and parentId accessors.

/** A record of the input, which is the JS object itself: a nested datum or a stratify row. */
type rec datum = {id?: string, parentId?: string, value?: float, children?: array<datum>}

/**
 * `Node` of hierarchy/index.js. `data` is None for an ancestor that stratify imputed from a
 * path, and `children` is empty for a leaf, where d3 leaves it undefined. The layouts write the
 * coordinates of their kind.
 */
type rec node = {
  data: option<datum>,
  mutable id: option<string>,
  mutable parent: option<node>,
  mutable children: array<node>,
  mutable depth: int,
  mutable height: int,
  mutable value: float,
  mutable x: float,
  mutable y: float,
  mutable r: float,
  mutable x0: float,
  mutable y0: float,
  mutable x1: float,
  mutable y1: float,
}

let make = data => {
  data,
  id: None,
  parent: None,
  children: [],
  depth: 0,
  height: 0,
  value: 0.,
  x: 0.,
  y: 0.,
  r: 0.,
  x0: 0.,
  y0: 0.,
  x1: 0.,
  y1: 0.,
}

/** JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way. */
let truthy = (x: float) => !(x == 0. || Float.isNaN(x))

/** `node.eachBefore(callback)` over any tree whose children `children` reads: pre-order. */
let eachBeforeBy = (root, children, callback) => {
  let nodes = [root]
  let rec visit = () =>
    switch nodes->Array.pop {
    | None => ()
    | Some(node) =>
      callback(node)
      let kids = children(node)
      for i in Array.length(kids) - 1 downto 0 {
        nodes->Array.push(kids->Array.getUnsafe(i))
      }
      visit()
    }
  visit()
  root
}

/** `node.eachAfter(callback)`: post-order, the reverse of a stack walk's visiting order. */
let eachAfterBy = (root, children, callback) => {
  let nodes = [root]
  let next = []
  let rec walk = () =>
    switch nodes->Array.pop {
    | None => ()
    | Some(node) =>
      next->Array.push(node)
      nodes->Array.pushMany(children(node))
      walk()
    }
  walk()
  for i in Array.length(next) - 1 downto 0 {
    callback(next->Array.getUnsafe(i))
  }
  root
}

let eachBefore = (root, callback) => eachBeforeBy(root, node => node.children, callback)
let eachAfter = (root, callback) => eachAfterBy(root, node => node.children, callback)

/** The order of `node[Symbol.iterator]`: breadth-first, each level from left to right. */
let breadthFirst = root => {
  let order = []
  let rec level = current =>
    if Array.length(current) > 0 {
      let next = []
      current->Array.forEach(node => {
        order->Array.push(node)
        next->Array.pushMany(node.children)
      })
      level(next)
    }
  level([root])
  order
}

/** `computeHeight`: raises the heights of the ancestors the node makes taller. */
let computeHeight = node => {
  let rec climb = (node, height) => {
    node.height = height
    switch node.parent {
    | Some(parent) if parent.height < height + 1 => climb(parent, height + 1)
    | _ => ()
    }
  }
  climb(node, 0)
}

/** `hierarchy(data)` with the default children accessor, `d.children`. */
let hierarchy = data => {
  let root = make(Some(data))
  let nodes = [root]
  let rec build = () =>
    switch nodes->Array.pop {
    | None => ()
    | Some(node) =>
      switch node.data->Option.flatMap(d => d.children) {
      | Some(childs) if Array.length(childs) > 0 =>
        let children = childs->Array.map(d => make(Some(d)))
        node.children = children
        for i in Array.length(children) - 1 downto 0 {
          let child = children->Array.getUnsafe(i)
          nodes->Array.push(child)
          child.parent = Some(node)
          child.depth = node.depth + 1
        }
      | _ => ()
      }
      build()
    }
  build()
  root->eachBefore(computeHeight)
}

/** `node.sum(value)`: each aggregate is `+value(data) || 0` plus the children's, last child first. */
let sum = (root, value) =>
  root->eachAfter(node => {
    let own = value(node.data)
    node.value =
      node.children->Array.reduceRight(truthy(own) ? own : 0., (sum, child) => sum +. child.value)
  })

/** `node.sort(compare)`: ReScript's Array.sort is Array.prototype.sort, which is stable. */
let sort = (root, compare) => root->eachBefore(node => node.children->Array.sort(compare))

/** Whether `path[i]` is a delimiting slash: one preceded by an even number of backslashes. */
let slash = (path, i) =>
  path->String.charAt(i) == "/" && {
      let rec backslashes = (j, k) =>
        j > 0 && path->String.charAt(j - 1) == "\\" ? backslashes(j - 1, k + 1) : k
      Int.bitwiseAnd(backslashes(i, 0), 1) == 0
    }

/**
 * Coerces nothing (ids are strings here), strips a trailing slash unless another slash precedes
 * it, and adds a leading slash if missing.
 */
let normalize = path => {
  let i = String.length(path)
  let path = slash(path, i - 1) && !slash(path, i - 2) ? path->String.slice(~start=0, ~end=-1) : path
  path->String.startsWith("/") ? path : "/" ++ path
}

/** The path up to its last delimiting slash after the first character: "/a/b" to "/a", "/" to "". */
let parentof = path => {
  let rec back = i => i > 1 && !slash(path, i) ? back(i - 1) : i
  let n = String.length(path)
  n < 2 ? "" : path->String.slice(~start=0, ~end=back(n - 1))
}

/** An entry of stratify's `nodeByKey`: the one node with that id, or d3's `ambiguous`. */
type key = Unique(node) | Ambiguous

/**
 * `stratify()(data)`, or `stratify().path(path)(data)` when `path` is given. d3 tests ids as
 * `id != null && (id += "")`: an undefined, null or empty id names nothing, and "" stands for the
 * three here. An imputed ancestor is a node whose data is None; d3 marks one with its `imputed`
 * object and replaces that by null before it returns.
 */
let stratify = (~path=?, data: array<datum>) => {
  let rows = data->Array.map(d => Some(d))
  let (ids, parentIds) = switch path {
  | None => (
      data->Array.map(d => d.id->Option.getOr("")),
      data->Array.map(d => d.parentId->Option.getOr("")),
    )
  | Some(path) =>
    let i = data->Array.map(d => normalize(path(d)))
    let p = i->Array.map(parentof)
    let s = Set.fromArray(i)
    s->Set.add("")
    // d3 appends each missing parent to I and P while it iterates P.
    let rec impute = k =>
      if k < Array.length(p) {
        let id = p->Array.getUnsafe(k)
        if !(s->Set.has(id)) {
          s->Set.add(id)
          i->Array.push(id)
          p->Array.push(parentof(id))
          rows->Array.push(None)
        }
        impute(k + 1)
      }
    impute(0)
    (i, p)
  }

  let n = Array.length(rows)
  let nodes = rows->Array.map(make)
  let nodeByKey = Map.make()
  nodes->Array.forEachWithIndex((node, i) => {
    let nodeId = ids->Array.getUnsafe(i)
    if nodeId != "" {
      node.id = Some(nodeId)
      nodeByKey->Map.set(nodeId, nodeByKey->Map.has(nodeId) ? Ambiguous : Unique(node))
    }
  })

  let root = ref(None)
  nodes->Array.forEachWithIndex((node, i) => {
    let key = parentIds->Array.getUnsafe(i)
    if key != "" {
      switch nodeByKey->Map.get(key) {
      | None => JsError.throwWithMessage("missing: " ++ key)
      | Some(Ambiguous) => JsError.throwWithMessage("ambiguous: " ++ key)
      | Some(Unique(parent)) =>
        parent.children->Array.push(node)
        node.parent = Some(parent)
      }
    } else {
      if root.contents->Option.isSome {
        JsError.throwWithMessage("multiple roots")
      }
      root.contents = Some(node)
    }
  })
  let root = switch root.contents {
  | Some(root) => root
  | None => JsError.throwWithMessage("no root")
  }

  // When imputing internal nodes, only introduce roots if needed.
  let rec trim = (root, n) =>
    root.data->Option.isNone && Array.length(root.children) == 1
      ? trim(root.children->Array.getUnsafe(0), n - 1)
      : (root, n)
  let (root, n) = path->Option.isSome ? trim(root, n) : (root, n)

  // d3 parents the root to `preroot`, whose depth is -1, while it walks.
  root.parent = None
  let remaining = ref(n)
  root
  ->eachBefore(node => {
    node.depth = switch node.parent {
    | Some(parent) => parent.depth + 1
    | None => 0
    }
    remaining.contents = remaining.contents - 1
  })
  ->eachBefore(computeHeight)
  ->ignore
  if remaining.contents > 0 {
    JsError.throwWithMessage("cycle")
  }
  root
}
