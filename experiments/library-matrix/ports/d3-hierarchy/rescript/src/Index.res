type rec datum = {id?: string, parentId?: string, value?: float, children?: array<datum>}
type input = {
  op: string,
  source: string,
  data?: datum,
  rows?: array<datum>,
  width: float,
  height: float,
  padding: float,
  sort: bool,
  nodeSize: bool,
  round: bool,
  tile: string,
}
type value
external cast: 'a => value = "%identity"
let fail = msg => throw(Invalid_argument(msg))
let elt = Array.getUnsafe
let slash = (s, i) => {
  if i < 0 || i >= String.length(s) || String.charCodeAtUnsafe(s, i) != 47 {
    false
  } else {
    let k = ref(i)
    while k.contents > 0 && String.charCodeAtUnsafe(s, k.contents - 1) == 92 {
      k := k.contents - 1
    }
    Int.mod(i - k.contents, 2) == 0
  }
}
let normalize = s => {
  let n = String.length(s)
  let path = if slash(s, n - 1) && !slash(s, n - 2) {
    String.slice(s, ~start=0, ~end=n - 1)
  } else {
    s
  }
  if String.startsWith(path, "/") {
    path
  } else {
    "/" ++ path
  }
}
let parentPath = s => {
  if String.length(s) < 2 {
    ""
  } else {
    let i = ref(String.length(s) - 1)
    while i.contents > 1 && !slash(s, i.contents) {
      i := i.contents - 1
    }
    String.slice(s, ~start=0, ~end=i.contents)
  }
}
let walk = (nodes: array<Geometry.node>, root, post) => {
  let stack = [root]
  let order = []
  while Array.length(stack) > 0 {
    let i = stack->Array.pop->Option.getOrThrow
    Array.push(order, i)
    let kids = elt(nodes, i).kids
    if post {
      Array.forEach(kids, c => Array.push(stack, c))
    } else {
      for j in Array.length(kids) - 1 downto 0 {
        Array.push(stack, elt(kids, j))
      }
    }
  }
  if post {
    Array.reverse(order)
  }
  order
}
let read = input => {
  let nodes: array<Geometry.node> = []
  if input.source != "id" && input.source != "path" {
    let rec nested = (d: datum, p) => {
      let i = Array.length(nodes)
      let n = Geometry.circle(0., 0., 0.)
      n.id = d.id->Option.getOr("")
      n.value = d.value->Option.getOr(0.)
      n.parent = p
      Array.push(nodes, n)
      n.kids = d.children->Option.getOr([])->Array.map(c => nested(c, i))
      i
    }
    ignore(nested(input.data->Option.getOrThrow, -1))
    (nodes, 0)
  } else {
    let rows = input.rows->Option.getOrThrow
    let paths = input.source == "path"
    let byId = Map.make()
    let parents = []
    Array.forEachWithIndex(rows, (d, i) => {
      let n = Geometry.circle(0., 0., 0.)
      let id = d.id->Option.getOr("")
      n.id = if paths {
        normalize(id)
      } else {
        id
      }
      n.value = d.value->Option.getOr(0.)
      if n.id != "" {
        Map.set(
          byId,
          n.id,
          if Map.has(byId, n.id) {
            -1
          } else {
            i
          },
        )
      }
      Array.push(nodes, n)
      Array.push(
        parents,
        if paths {
          parentPath(n.id)
        } else {
          d.parentId->Option.getOr("")
        },
      )
    })
    let j = ref(0)
    while paths && j.contents < Array.length(parents) {
      let p = elt(parents, j.contents)
      if p != "" && !Map.has(byId, p) {
        Map.set(byId, p, Array.length(nodes))
        let n = Geometry.circle(0., 0., 0.)
        n.id = p
        Array.push(nodes, n)
        Array.push(parents, parentPath(p))
      }
      j := j.contents + 1
    }
    let root = ref(-1)
    Array.forEachWithIndex(parents, (key, i) => {
      if key == "" {
        if root.contents >= 0 {
          fail("multiple roots")
        }
        root := i
      } else {
        let p = Map.get(byId, key)->Option.getOr(-1)
        if p < 0 {
          fail("missing or ambiguous parent")
        }
        elt(nodes, i).parent = p
        Array.push(elt(nodes, p).kids, i)
      }
    })
    if root.contents < 0 {
      fail("no root")
    }
    let expected = ref(Array.length(nodes))
    while (
      paths &&
      root.contents >= Array.length(rows) &&
      Array.length(elt(nodes, root.contents).kids) == 1
    ) {
      root := elt(elt(nodes, root.contents).kids, 0)
      expected := expected.contents - 1
    }
    elt(nodes, root.contents).parent = -1
    if Array.length(walk(nodes, root.contents, false)) != expected.contents {
      fail("cycle")
    }
    (nodes, root.contents)
  }
}
let run = (input: input) => {
  let (ns, root) = read(input)
  walk(ns, root, true)->Array.forEach(i => {
    let n = elt(ns, i)
    if n.value == 0.0 { n.value = 0.0 }
    for j in Array.length(n.kids) - 1 downto 0 {
      let c = elt(ns, elt(n.kids, j))
      n.value = n.value +. c.value
      n.height = max(n.height, c.height + 1)
    }
    if input.sort {
      Array.sort(n.kids, (a, b) => Float.compare(elt(ns, b).value, elt(ns, a).value))
    }
  })
  let pre = walk(ns, root, false)
  let post = walk(ns, root, true)
  pre->Array.forEach(i => {
    let n = elt(ns, i)
    if n.parent >= 0 {
      n.depth = elt(ns, n.parent).depth + 1
    }
  })
  switch input.op {
  | "tree" => Geometry.tidy(ns, pre, post, input.width, input.height, input.nodeSize)
  | "cluster" => Geometry.cluster(ns, pre, post, input.width, input.height, input.nodeSize)
  | "pack" => Geometry.pack(ns, pre, post, input.width, input.height, input.padding)
  | "treemap" | "partition" =>
    Geometry.rectangles(
      ns,
      pre,
      input.width,
      input.height,
      input.padding,
      input.op == "partition",
      input.tile,
      input.round,
    )
  | "iterator" =>
    pre[0] = root
    let j = ref(1)
    for i in 0 to Array.length(pre) - 1 {
      elt(ns, elt(pre, i)).kids->Array.forEach(c => {
        pre[j.contents] = c
        j := j.contents + 1
      })
    }
  | "hierarchy" | "stratify" => ()
  | _ => fail("operation")
  }
  pre->Array.map(i => {
    let n = elt(ns, i)
    let parent = if n.parent < 0 {
      cast(Nullable.null)
    } else {
      cast(elt(ns, n.parent).id)
    }
    let row = [cast(n.id), parent, cast(n.depth), cast(n.height), cast(n.value)]
    let coords = switch input.op {
    | "tree" | "cluster" => [n.x, n.y]
    | "pack" => [n.x, n.y, n.r]
    | "treemap" | "partition" => [n.x0, n.y0, n.x1, n.y1]
    | _ => []
    }
    Array.forEach(coords, x => Array.push(row, cast(x)))
    row
  })
}
