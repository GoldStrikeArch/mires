// MiniSearch's `SearchableMap` (minisearch/dist/es/index.js, 7.2.0): the radix tree behind the
// inverted index, with its prefix view, its tree iterator and its fuzzy (Levenshtein) walk.
//
// A node is a JavaScript `Map` (ReScript's `Map.t`), as in MiniSearch, so its keys keep the
// observable insertion order exactly: a new key is appended, a deleted key is removed in place, and
// setting an existing key keeps its place. A non-empty key is an edge label leading to a child node;
// the empty key (`leaf`) holds the value of the term ending at the node. Keys are JS strings, so
// labels, lengths and comparisons are in UTF-16 code units, lone surrogates included.

type rec node<'v> = Map.t<string, entry<'v>>
and entry<'v> =
  | Node(node<'v>)
  | Leaf('v)

type t<'v> = {
  tree: node<'v>,
  prefix: string,
  // `_size`: the number of terms, cached until the next change (MiniSearch's own cache).
  mutable size: option<int>,
}

let leaf = ""

// `new SearchableMap(tree, prefix)`; a missing tree is a new empty one.
let make = (~tree=Map.make(), ~prefix="") => {tree, prefix, size: None}

let child = (node: node<'v>, key) =>
  switch node->Map.get(key) {
  | Some(Node(child)) => child
  | _ => panic(`SearchableMap: no edge "${key}"`)
  }

let leafValue = (node: node<'v>) =>
  switch node->Map.get(leaf) {
  | Some(Leaf(value)) => Some(value)
  | _ => None
  }

let lastOf = array => array->Array.getUnsafe(array->Array.length - 1)

// `node.entries().next().value`: the oldest entry.
let firstEntry = (node: node<'v>) => (node->Map.entries->Iterator.next).value

// MiniSearch scans a node's keys with `for (const k of node.keys())` and returns at the first edge
// that fits; these do the same with the key iterator's `next` (no two edges share their first code
// unit, so at most one fits).

// The edge whose label `key` starts with.
let rec edgeStarting = (keys: Iterator.t<string>, key) =>
  switch keys->Iterator.next {
  | {done: true} => None
  | {value: Some(k)} if k != leaf && key->String.startsWith(k) => Some(k)
  | _ => edgeStarting(keys, key)
  }

// The edge whose label starts with `key`.
let rec edgeExtending = (keys: Iterator.t<string>, key) =>
  switch keys->Iterator.next {
  | {done: true} => None
  | {value: Some(k)} if k != leaf && k->String.startsWith(key) => Some(k)
  | _ => edgeExtending(keys, key)
  }

// The edge whose label starts with the code unit `first`.
let rec edgeStartingWith = (keys: Iterator.t<string>, first) =>
  switch keys->Iterator.next {
  | {done: true} => None
  | {value: Some(k)} if k != leaf && first == k->String.charCodeAtUnsafe(0) => Some(k)
  | _ => edgeStartingWith(keys, first)
  }

let rest = (key, label) => key->String.slice(~start=label->String.length)

// `trackDown`: the node `key` ends at (`None` when it ends inside an edge or leaves the tree), and
// the path of `(node, key)` pairs that led there.
let rec trackDown = (tree: option<node<'v>>, key, path: array<(node<'v>, string)>) =>
  switch tree {
  | Some(node) if key != "" =>
    switch edgeStarting(node->Map.keys, key) {
    | Some(k) =>
      path->Array.push((node, k))
      trackDown(Some(node->child(k)), rest(key, k), path)
    | None =>
      path->Array.push((node, key))
      trackDown(None, "", path)
    }
  | _ => (tree, path)
  }

let rec lookup = (tree: node<'v>, key) =>
  if key == "" {
    Some(tree)
  } else {
    switch edgeStarting(tree->Map.keys, key) {
    | Some(k) => lookup(tree->child(k), rest(key, k))
    | None => None
    }
  }

// `createPath`: the node for `key`, splitting an edge where the key leaves it. The intermediate
// node of a split is set under the shared part and the old edge deleted, so it moves to the end of
// its parent's keys.
let createPath = (node: node<'v>, key) => {
  let keyLength = key->String.length
  let rec descend = (node: node<'v>, pos) =>
    if pos >= keyLength {
      node
    } else {
      switch edgeStartingWith(node->Map.keys, key->String.charCodeAtUnsafe(pos)) {
      | Some(k) =>
        let len = Math.Int.min(keyLength - pos, k->String.length)
        let offset = ref(1)
        while (
          offset.contents < len &&
            key->String.charCodeAtUnsafe(pos + offset.contents) ==
              k->String.charCodeAtUnsafe(offset.contents)
        ) {
          offset := offset.contents + 1
        }
        let child = node->child(k)
        if offset.contents == k->String.length {
          descend(child, pos + offset.contents)
        } else {
          let intermediate = Map.make()
          intermediate->Map.set(k->String.slice(~start=offset.contents), Node(child))
          node->Map.set(
            key->String.slice(~start=pos, ~end=pos + offset.contents),
            Node(intermediate),
          )
          node->Map.delete(k)->ignore
          descend(intermediate, pos + offset.contents)
        }
      | None =>
        let child = Map.make()
        node->Map.set(key->String.slice(~start=pos), Node(child))
        child
      }
    }
  descend(node, 0)
}

// Merging the node reached by the last step of `path` into its parent: the parent sets the joined
// label and deletes the old one, so the merged edge moves to the end of the parent's keys.
let merge = (path: array<(node<'v>, string)>, key, value) =>
  switch path->Array.last {
  | None => ()
  | Some((node, nodeKey)) =>
    node->Map.set(nodeKey ++ key, value)
    node->Map.delete(nodeKey)->ignore
  }

let rec cleanup = (path: array<(node<'v>, string)>) =>
  switch path->Array.last {
  | None => ()
  | Some((node, key)) =>
    node->Map.delete(key)->ignore
    if node->Map.size == 0 {
      cleanup(path->Array.slice(~start=0, ~end=-1))
    } else if node->Map.size == 1 {
      switch firstEntry(node) {
      | Some((key, value)) if key != leaf => merge(path->Array.slice(~start=0, ~end=-1), key, value)
      | _ => ()
      }
    }
  }

let remove = (tree: node<'v>, key) => {
  let (node, path) = trackDown(Some(tree), key, [])
  switch node {
  | None => ()
  | Some(node) =>
    node->Map.delete(leaf)->ignore
    if node->Map.size == 0 {
      cleanup(path)
    } else if node->Map.size == 1 {
      switch firstEntry(node) {
      | Some((key, value)) => merge(path, key, value)
      | None => ()
      }
    }
  }
}

// A view of the entries under `prefix`: the node the prefix ends at, a one-entry node holding the
// rest of the edge it ends inside, or an empty tree.
let atPrefix = (set: t<'v>, prefix) => {
  if !(prefix->String.startsWith(set.prefix)) {
    JsError.throwWithMessage("Mismatched prefix")
  }
  switch trackDown(Some(set.tree), rest(prefix, set.prefix), []) {
  | (Some(node), _) => make(~tree=node, ~prefix)
  | (None, path) =>
    let (parentNode, key) = lastOf(path)
    switch edgeExtending(parentNode->Map.keys, key) {
    | Some(k) =>
      let node = Map.make()
      node->Map.set(rest(k, key), parentNode->Map.get(k)->Option.getUnsafe)
      make(~tree=node, ~prefix)
    | None => make(~prefix)
    }
  }
}

let get = (set: t<'v>, key) =>
  switch lookup(set.tree, key) {
  | Some(node) => node->leafValue
  | None => None
  }

let has = (set: t<'v>, key) =>
  switch lookup(set.tree, key) {
  | Some(node) => node->Map.has(leaf)
  | None => false
  }

let delete = (set: t<'v>, key) => {
  set.size = None
  remove(set.tree, key)
}

// The value of `key`, created by `initial` (and stored) when the key is new.
let fetch = (set: t<'v>, key, initial) => {
  set.size = None
  let node = createPath(set.tree, key)
  switch node->Map.get(leaf) {
  | Some(Leaf(value)) => value
  | _ =>
    let value = initial()
    node->Map.set(leaf, Leaf(value))
    value
  }
}

// `TreeIterator`: depth first, each node's keys from the most recently inserted to the oldest. The
// iterator works on a snapshot of each node's keys and looks the child of a key up when it gets
// there, so terms deleted (and nodes merged) behind it while it walks change nothing it has yet to
// visit.
type frame<'v> = {node: node<'v>, keys: array<string>}
type iterator<'v> = {prefix: string, path: array<frame<'v>>}

let entries = (set: t<'v>): iterator<'v> => {
  let keys = set.tree->Map.keys->Array.fromIterator
  {prefix: set.prefix, path: keys->Array.length > 0 ? [{node: set.tree, keys}] : []}
}

let key = (iterator: iterator<'v>) =>
  iterator.prefix ++
  iterator.path
  ->Array.map(({keys}) => lastOf(keys))
  ->Array.filter(key => key != leaf)
  ->Array.join("")

let value = (iterator: iterator<'v>) =>
  switch lastOf(iterator.path).node->Map.get(leaf) {
  | Some(Leaf(value)) => value
  | _ => panic("SearchableMap: iterator is not at a leaf")
  }

let rec dive = (iterator: iterator<'v>) =>
  switch iterator.path->Array.last {
  | None => None
  | Some({node, keys}) =>
    let last = lastOf(keys)
    if last == leaf {
      Some((key(iterator), value(iterator)))
    } else {
      let child = node->child(last)
      iterator.path->Array.push({node: child, keys: child->Map.keys->Array.fromIterator})
      dive(iterator)
    }
  }

let rec backtrack = (iterator: iterator<'v>) =>
  switch iterator.path->Array.last {
  | None => ()
  | Some({keys}) =>
    keys->Array.pop->ignore
    if keys->Array.length == 0 {
      iterator.path->Array.pop->ignore
      backtrack(iterator)
    }
  }

// The next `(term, value)`, or `None` at the end.
let next = (iterator: iterator<'v>) => {
  let entry = dive(iterator)
  backtrack(iterator)
  entry
}

let size = (set: t<'v>) =>
  switch set.size {
  | Some(size) if size > 0 => size
  | _ =>
    let iterator = entries(set)
    let rec count = size =>
      switch next(iterator) {
      | Some(_) => count(size + 1)
      | None => size
      }
    let size = count(0)
    set.size = Some(size)
    size
  }

// The Levenshtein matrix as MiniSearch keeps it: a `Uint8Array` read and written by index.
@get_index external cell: (Uint8Array.t, int) => int = ""
@set_index external setCell: (Uint8Array.t, int, int) => unit = ""

// `fuzzySearch`'s walk: each node's keys oldest first, one matrix row per label code unit over the
// band `max(0, i - maxDistance - 1) <= j < min(n - 1, i + maxDistance)`; a subtree is skipped when
// a row's minimum exceeds `maxDistance`. Rows past the matrix (terms longer than
// `query.length + maxDistance`) can never match: MiniSearch reads them as `undefined` and walks on
// without a match, this port stops there (CONTRACT.md, "The index").
let rec recurse = (node: node<'v>, query, maxDistance, results, matrix, m, n, prefix) => {
  let offset = m * n
  node->Map.forEachWithKey((entry, key) =>
    switch entry {
    | Leaf(value) =>
      let distance = matrix->cell(offset - 1)
      if distance <= maxDistance {
        results->Map.set(prefix, (value, distance))
      }
    | Node(child) =>
      let i = ref(m)
      let pos = ref(0)
      let within = ref(true)
      while within.contents && pos.contents < key->String.length {
        let thisRowOffset = n * i.contents
        if thisRowOffset >= matrix->TypedArray.length {
          within := false
        } else {
          let char = key->String.charCodeAtUnsafe(pos.contents)
          let prevRowOffset = thisRowOffset - n
          let minDistance = ref(matrix->cell(thisRowOffset))
          let jmin = Math.Int.max(0, i.contents - maxDistance - 1)
          let jmax = Math.Int.min(n - 1, i.contents + maxDistance)
          for j in jmin to jmax - 1 {
            let different = char != query->String.charCodeAtUnsafe(j)
            let rpl = matrix->cell(prevRowOffset + j) + (different ? 1 : 0)
            let del = matrix->cell(prevRowOffset + j + 1) + 1
            let ins = matrix->cell(thisRowOffset + j) + 1
            let dist = Math.Int.min(Math.Int.min(rpl, del), ins)
            matrix->setCell(thisRowOffset + j + 1, dist)
            if dist < minDistance.contents {
              minDistance := dist
            }
          }
          if minDistance.contents > maxDistance {
            within := false
          } else {
            pos := pos.contents + 1
            i := i.contents + 1
          }
        }
      }
      if within.contents {
        recurse(child, query, maxDistance, results, matrix, i.contents, n, prefix ++ key)
      }
    }
  )
}

// `fuzzyGet`: every term within `maxDistance` edits of `query`, in the order the walk meets them,
// with its value and distance.
let fuzzyGet = (set: t<'v>, query, maxDistance): Map.t<string, ('v, int)> => {
  let results = Map.make()
  let n = query->String.length + 1
  let m = n + maxDistance
  let matrix = Uint8Array.fromLength(m * n)->TypedArray.fillAll(maxDistance + 1)
  for j in 0 to n - 1 {
    matrix->setCell(j, j)
  }
  for i in 1 to m - 1 {
    matrix->setCell(i * n, i)
  }
  recurse(set.tree, query, maxDistance, results, matrix, 1, n, "")
  results
}
