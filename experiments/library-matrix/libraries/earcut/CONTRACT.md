# Library contract: `earcut`

`earcut` is a fast 2D polygon triangulation library with hole elimination, z-order spatial
indexing, local self-intersection curing, fallback polygon splitting, area-deviation checking,
GeoJSON-style ring flattening, and constrained Delaunay edge-flipping refinement. Ports implement
`earcut 3.2.4`'s exact algorithms and data structures — circular doubly-linked vertex rings,
`K = 16` append-only block-bbox hole-bridge index, `compareXYSlope` hole ordering, Steiner-point
preservation, 30-bit Morton `zOrder` hashing with LSD radix sort (`sortNodes`), 3-stage
`earcutLinked` (`filterPoints` fixpoint → `cureLocalIntersections` → `splitEarcut`), `deviation`,
`flatten`, and Lawson-flip `refine` — and must match `earcut@3.2.4` **exactly** (exact triangle
index sequences and exact IEEE-754 `f64` values) on every corpus case.

| Field | Value |
| --- | --- |
| Package | `earcut@3.2.4`, entry `src/earcut.js` (`experiments/library-matrix/node_modules`, `npm.modules: "harness"`) |
| Reference adapter | `reference.mjs`: dispatches `input.op` onto `earcut`, `deviation`, `flatten`, `refine` (cloning `input.triangles` before refining in place), `earcutRefine`, and `batch` |
| API subset | **100% of `earcut@3.2.4`'s public API**: `earcut(data, holeIndices?, dim?)`, `deviation(data, holeIndices, dim, triangles)`, `flatten(rings)`, `refine(triangles, coords, dim?)`, plus `earcutRefine` and `batch` pipelines |
| Input | `{ op, ... }` pure JSON object; see "Input" |
| Output form | `json` (strict JSON: exact integer arrays, exact IEEE-754 `f64` including `Infinity`/`-0`, plain objects with exact key order), synchronous |
| Normalisation | none |
| Parity environments | node, chrome, firefox, safari (unavailable on this machine) |
| Corpus | `corpus.json`, 92 cases, written deterministically by `generate-corpus.mjs` (`--verify` validates all invariants, mutant killers, and bench variants) |
| Bench cases | `bench-earcut`, `bench-refine`; `baseIterations` 1; variant `bench-variant.mjs` (per-polygon exact D4 isometry + non-zero integer translation + cyclic ring rotation + hole-order permutation) |
| Caching | `earcut.js` keeps all of its working state in module-level variables: `steiners` (a `Set` of single-vertex hole nodes, cleared at the start of each `earcut` call), the flags `filteredOut` and `indexActive`, the hole-bridge block index `blockBBox`, `numBlocks`, `blockHead` and `blockStop` (rebuilt by `buildBlockIndex` in each `eliminateHoles`), the `sortNodes` scratch `sortArr`, `sortBuf`, `zArr`, `zBuf` and `counts`, and the `refine` scratch `edgeStack`, `he`, `hTable`, `hStamp`, `edgeStamp`, `hMask` and `gen`. Buffers grow on demand. A call writes each one before reading it, except two that carry over in a neutral state: incrementing `gen` makes every older `hStamp` entry read as an empty hash slot, and balanced pushes and pops leave `edgeStamp` all zero. The buffers only avoid GC churn: no result depends on an earlier call. `K = 16` is the only module-level constant. Ports allocate this state per call and keep none between calls; the Elm host keeps only its worker instance, whose model is `()`. |
| Files | none: neither the adapter nor a port reads files at run time |

## Input

Every corpus input is a pure JSON object. Neither the adapter nor any port may mutate `input`:

```text
BatchItem := {
  "data":         number[],
  "holeIndices"?: number[] | null,
  "dim"?:         number,          // default 2
  "refine"?:      boolean          // default false
}

Input :=
  { "op": "earcut",       "data": number[], "holeIndices"?: number[] | null, "dim"?: number }
| { "op": "deviation",    "data": number[], "holeIndices": number[] | null, "dim": number, "triangles": number[] }
| { "op": "flatten",      "rings": number[][][] }
| { "op": "refine",       "triangles": number[], "coords": number[], "dim"?: number }
| { "op": "earcutRefine", "data": number[], "holeIndices"?: number[] | null, "dim"?: number }
| { "op": "batch",        "items": BatchItem[] }
```

Unknown `op` throws a `TypeError` (or host error; checked by `both-threw`).

## Output

The harness compares the following return values with its `json` canonicalizer, which preserves `undefined`, signed zero and infinity.

- **`earcut`**: `number[]` — flat array of vertex indices (`3 * T` integers).
- **`deviation`**: `number` — exact IEEE-754 `f64` relative area deviation (may be `0`, positive finite, or `Infinity`).
- **`flatten`**: `{ "vertices": (number | undefined)[], "holes": number[], "dimensions": number }` — **exact key order** `vertices`, `holes`, `dimensions`.
- **`refine`**: `number[]` — refined copy of `input.triangles` (`input.triangles` is never mutated).
- **`earcutRefine`**: `number[]` — `earcut(...)` followed by in-place `refine(..., data, dim)`.
- **`batch`**: `number[][]` — one triangle array per item in `input.items` (refined if `item.refine` is truthy).

## Behaviour

Everything below restates `earcut@3.2.4` (`experiments/library-matrix/node_modules/earcut/src/earcut.js`,
which is the final authority where this is silent). Because a polygon has many valid triangulations
(and bridge order, ring start vertex, and flip order determine the exact index sequence), **ports
must implement `earcut@3.2.4`'s exact algorithms, data structures, and floating-point expression
order**.

### Floating-point rules (exact IEEE-754 `f64`, no FMA)

`earcut@3.2.4` uses **zero transcendental functions** — only IEEE-754 `f64` `+`, `-`, `*`, `/`,
`abs`, `min`, `max`, and 32-bit integer bitwise/multiply operations. Node (V8), Chrome (V8) and Firefox (SpiderMonkey) produce bit-identical corpus outputs. Ports must
preserve the **exact parenthesisation** below and must not introduce fused multiply-add (FMA):

- `signedArea(data, start, end, dim)`:
  ```text
  sum = 0.0; j = end - dim
  for i = start; i < end; i += dim:
    sum += (data[j] - data[i]) * (data[i + 1] + data[j + 1])
    j = i
  return sum
  ```
- `area(p, q, r)`:
  `(q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)`
- `equals(p1, p2)`:
  `p1.x === p2.x && p1.y === p2.y`
- `pointInTriangle(ax, ay, bx, by, cx, cy, px, py)`:
  `(cx - px) * (ay - py) >= (ax - px) * (cy - py) && (ax - px) * (by - py) >= (bx - px) * (ay - py) && (bx - px) * (cy - py) >= (cx - px) * (by - py)`
- `orient(ax, ay, bx, by, cx, cy)`:
  `(bx - ax) * (cy - ay) - (by - ay) * (cx - ax)`
- `inCircle(ax, ay, bx, by, cx, cy, px, py)`:
  ```text
  dx = ax - px; dy = ay - py; ex = bx - px; ey = by - py; fx = cx - px; fy = cy - py
  ap = dx * dx + dy * dy; bp = ex * ex + ey * ey; cp = fx * fx + fy * fy
  s = ap + bp + cp
  return (dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx)) + ap * (ex * fy - ey * fx) <= (1e-13 * s) * s
  ```
  (Note the `1e-13 * s * s` tolerance margin from mapbox/earcut#205 guaranteeing termination on
  near-cocircular quads.)

### Vertex `Node` & circular doubly-linked list

Each vertex node holds:
`{ i: int, x: f64, y: f64, prev: Node, next: Node, z: i32, prevZ: Node?, nextZ: Node?, steiner: bool }`.

npm's `Node` has no `steiner` field. `earcut.js` keeps the Steiner nodes in the module-level
`Set` named `steiners`: `earcut` clears it on entry, `eliminateHoles` adds a single-vertex hole's
node, and `filterPoints` tests `steiners.size === 0 || !steiners.has(p)`. This contract, and every
port, uses a per-node `steiner` flag instead, set at the same point. The two are equivalent: the
set only ever holds nodes created in the current call, a node enters it at that one point and never
leaves it during the call, and membership is by node identity, so `steiners.has(p)` is
`p.steiner` for every node `filterPoints` sees.

- **`insertNode(i, x, y, last)`**: creates node `p` (`z = 0`, `prevZ = nextZ = null`, `steiner = false`).
  If `last == null`: `p.prev = p; p.next = p`. Else inserts `p` immediately after `last` (`p.next = last.next; p.prev = last; last.next.prev = p; last.next = p`). Returns `p`.
- **`removeNode(p)`**: unlinks `p` from both the ring (`p.next.prev = p.prev; p.prev.next = p.next`)
  and the z-list (`if p.prevZ: p.prevZ.nextZ = p.nextZ; if p.nextZ: p.nextZ.prevZ = p.prevZ`).
  If `indexActive`, calls `growBlock(p.prev, p.next)`.
  **Critical invariant**: `p.prev` and `p.next` on the removed node `p` are **NOT cleared** and
  node storage is never reused within a single `earcut` call. The hole-bridge block index detects
  dead nodes via `node.prev.next !== node` and walks forward via `node = node.next` until reaching
  the next live node.
- **`linkedList(data, start, end, dim, clockwise)`**:
  - If `clockwise === (signedArea(data, start, end, dim) > 0)`:
    inserts `i = start, start + dim, ... < end` with vertex index `i / dim | 0`.
  - Else:
    inserts `i = end - dim, end - 2*dim, ... >= start` with vertex index `i / dim | 0`.
  - If `last != null && equals(last, last.next)`: `removeNode(last); last = last.next`.
    (For a 1-node ring, `last === last.next === last.prev`, so `removeNode` leaves it intact and
    `last === last.next` remains true — used to detect single-vertex Steiner holes!)
  - Returns `last` (which is the *last* inserted node when no closing duplicate was removed, or the
    *first* inserted node when the closing duplicate was removed).

### `earcut(data, holeIndices, dim = 2)`

1. `hasHoles = holeIndices != null && holeIndices.length > 0`.
2. `outerLen = hasHoles ? holeIndices[0] * dim : data.length`.
3. `outerNode = linkedList(data, 0, outerLen, dim, true)`.
4. If `!outerNode || outerNode.next === outerNode.prev`, return `[]`.
5. `minX = 0, minY = 0, invSize = 0`.
6. If `hasHoles`: `outerNode = eliminateHoles(data, holeIndices, outerNode, dim)`.
7. If `data.length > 80 * dim`:
   Compute bounding box `(minX, minY, maxX, maxY)` over the **outer ring only** (`i = 0, dim, ... < outerLen`).
   `invSize = max(maxX - minX, maxY - minY); invSize = invSize !== 0 ? 32767 / invSize : 0`.
8. `earcutLinked(outerNode, triangles, minX, minY, invSize)`.
9. Return `triangles`.

### `filterPoints(start, end = start)`

```text
full = (end === start)
p = start
do:
  again = false
  if p !== p.next && !p.steiner && (equals(p, p.next) || area(p.prev, p, p.next) === 0):
    if full || p === end: end = p.prev
    filteredOut = true
    removeNode(p)
    p = p.prev
    again = true
  else if full || p !== end:
    p = p.next
    again = !full
while again || p !== end
return end
```

### `eliminateHoles` & `K = 16` append-only block-bbox index

1. For each hole `i = 0..holeIndices.length - 1`:
   - `start = holeIndices[i] * dim`, `end = (i < len - 1) ? holeIndices[i + 1] * dim : data.length`.
   - `list = linkedList(data, start, end, dim, false)`.
   - If `list === list.next`: mark `list.steiner = true` (npm: `steiners.add(list)`), which preserves it through `filterPoints`.
   - Push `getLeftmost(list)` to `queue` (`p.x < leftmost.x || (p.x === leftmost.x && p.y < leftmost.y)`, starting at `list`).
2. **Stable sort** `queue` with `compareXYSlope(a, b)`:
   ```text
   d = a.x - b.x
   if d == 0: d = a.y - b.y
   if d == 0: d = (a.next.y - a.y) / (a.next.x - a.x) - (b.next.y - b.y) / (b.next.x - b.x)
   ```
   (`d < 0` → Less, `d > 0` → Greater, `0` or `NaN` → Equal, preserving original order).
3. Initialise block-bbox index (`K = 16` edges per block) and seed it with `indexSegment(outerNode, outerNode)`:
   - Each block `b` stores `blockHead[b]`, `blockStop[b]`, and `[minX, minY, maxX, maxY]` covering
     both endpoints of up to `K = 16` edges `p -> p.next` (stopping early if `p === stop`).
   - Sets `p.z = b` on each edge's starting node `p` (reusing `z` as owning block index during
     `eliminateHoles`).
4. Set `indexActive = true`. For each `hole` in `queue`:
   - `bridge = findHoleBridge(hole, outerNode)`. If `null`, skip this hole.
   - `bridgeReverse = splitPolygon(bridge, hole)`; `bridge2 = bridgeReverse.next`.
   - `indexSegment(bridge, bridge2.next)` (indexes merged hole + both slit edges *before* filtering).
   - `filterPoints(bridgeReverse, bridgeReverse.next)`.
   - `outerNode = filterPoints(bridge, bridge.next)`.
   During `removeNode(p)` while `indexActive` is true, `growBlock(p.prev, p.next)` expands
   `blockBBox[p.prev.z]` to include `(p.next.x, p.next.y)`.
5. Set `indexActive = false`, then return `filterPoints(outerNode)` (full-ring sweep).

**Exact `findHoleBridge(hole, outerNode)`** (important: blocks are scanned in **append order**
`b = 0..numBlocks - 1`, which determines tie-breaks when multiple bridges exist):

- If `equals(hole, outerNode)`, return `outerNode`.
- `hx = hole.x, hy = hole.y, qx = -Infinity, m = null`.
- **Scan 1 (leftward ray intersection)**:
  For `b = 0..numBlocks - 1`:
  - Skip if `hy < minY || hy > maxY || minX > hx || maxX <= qx`.
  - Advance `stop = liveBlockStop(b)` (`while stop.prev.next !== stop: stop = stop.next`, updating `blockStop[b]`)
    and `p = liveBlockHead(b)` (`while head.prev.next !== head: head = head.next`, updating `blockHead[b]`).
  - `do` until `p === stop`:
    If `p.prev.next === p`:
    - If `equals(hole, p.next)`: return `p.next`.
    - Else if `hy <= p.y && hy >= p.next.y && p.next.y !== p.y`:
      `x = p.x + (hy - p.y) * (p.next.x - p.x) / (p.next.y - p.y)`.
      If `x <= hx && x > qx`: `qx = x`; `m = (p.x < p.next.x) ? p : p.next`; if `x === hx` return `m`.
- If `m == null`, return `null`.
- **Scan 2 (minimum-angle reflex vertex inside Eberly triangle)**:
  Capture `mx = m.x, my = m.y, tminY = min(hy, my), tmaxY = max(hy, my), tanMin = Infinity`.
  For `b = 0..numBlocks - 1`:
  - Skip if `maxX < mx || minX > hx || maxY < tminY || minY > tmaxY`.
  - `stop = liveBlockStop(b); p = liveBlockHead(b)`.
  - `do` until `p === stop`:
    If `p.prev.next === p && hx >= p.x && p.x >= mx && hx !== p.x && pointInTriangle(hy < my ? hx : qx, hy, mx, my, hy < my ? qx : hx, hy, p.x, p.y)`:
    - `tan = Math.abs(hy - p.y) / (hx - p.x)`.
    - If `(locallyInside(p, hole) || (p.y === hy && p.next.y === hy && p.next.x > hx))` and
      `(tan < tanMin || (tan === tanMin && (p.x > m.x || (p.x === m.x && sectorContainsSector(m, p)))))`:
      `m = p; tanMin = tan`.
      (Note: `p.x > m.x` compares against the *current* `m.x`, not the initial `mx`!)
- Return `m`.

### `zOrder`, `indexCurve`, `sortNodes`

- **`zOrder(x, y, minX, minY, invSize)`**:
  ```text
  ix = ToInt32((x - minX) * invSize)
  iy = ToInt32((y - minY) * invSize)
  ix = (ix | (ix << 8)) & 0x00FF00FF
  ix = (ix | (ix << 4)) & 0x0F0F0F0F
  ix = (ix | (ix << 2)) & 0x33333333
  ix = (ix | (ix << 1)) & 0x55555555
  iy = (iy | (iy << 8)) & 0x00FF00FF
  iy = (iy | (iy << 4)) & 0x0F0F0F0F
  iy = (iy | (iy << 2)) & 0x33333333
  iy = (iy | (iy << 1)) & 0x55555555
  return ix | (iy << 1)   // signed 32-bit int (non-negative 30-bit for in-bbox points)
  ```
- **`indexCurve(start, minX, minY, invSize)`**: walks the ring once computing `p.z = zOrder(...)`
  for every node (overwriting any leftover block index from `eliminateHoles`), sorts stably by `z`
  via `sortNodes(n)` (insertion sort for `n <= 32`; 4-pass 8-bit LSD radix sort using unsigned
  32-bit `z >>> shift & 0xff` for `n > 32`), and links `prevZ` / `nextZ` (`null` at ends).

### `earcutLinked`, `isEar`, `isEarHashed`, `cureLocalIntersections`, `splitEarcut`

- **`earcutLinked(ear, triangles, minX, minY, invSize)`**:
  - If `invSize !== 0`, call `indexCurve(ear, minX, minY, invSize)`.
  - `stop = ear, cured = false`.
  - `while ear.prev !== ear.next`:
    - `prev = ear.prev; next = ear.next`.
    - If `area(prev, ear, next) < 0 && (invSize !== 0 ? isEarHashed(ear, minX, minY, invSize) : isEar(ear))`:
      push `prev.i, ear.i, next.i`; `removeNode(ear)`; `ear = next; stop = next; continue`.
      *(Note: earcut v3.2.4 advances to `next`, not `next.next` as in v2.)*
    - `ear = next`.
    - If `ear === stop`:
      1. `filteredOut = false; ear = filterPoints(ear); if filteredOut { stop = ear; continue; }`
      2. `if !cured { ear = cureLocalIntersections(ear, triangles); stop = ear; cured = true; continue; }`
      3. `splitEarcut(ear, triangles, minX, minY, invSize); break;`
- **`isEar(ear)`**: bbox `[x0, x1] × [y0, y1]` of `(a=ear.prev, b=ear, c=ear.next)`; scan `p = c.next`
  while `p !== a`; return `false` if `p` is inside bbox, `!(ax === p.x && ay === p.y)`,
  `pointInTriangle(ax, ay, bx, by, cx, cy, p.x, p.y)`, and `area(p.prev, p, p.next) >= 0`.
- **`isEarHashed(ear, minX, minY, invSize)`**: `minZ = zOrder(x0, y0, ...)`, `maxZ = zOrder(x1, y1, ...)`;
  walk `p = ear.prevZ` while `p && p.z >= minZ`, then `n = ear.nextZ` while `n && n.z <= maxZ`,
  with the same predicate plus `p !== c` (and `n !== c`).
- **`cureLocalIntersections(start, triangles)`**: walks `p` around ring; if
  `intersects(p.prev, p, p.next, p.next.next, false) && locallyInside(a, b) && locallyInside(b, a)`,
  pushes `(a.i, p.i, b.i)`, calls `removeNode(p)` and `removeNode(p.next)`, sets `p = start = b`,
  `cured = true`. Returns `cured ? filterPoints(p) : p`.
- **`splitEarcut(start, triangles, minX, minY, invSize)`**: nested loops `a` around ring,
  `b = a.next.next` while `b !== a.prev`; first pair with `a.i !== b.i && isValidDiagonal(a, b)`
  splits via `c = splitPolygon(a, b)`, heals `a = filterPoints(a, a.next)` and `c = filterPoints(c, c.next)`,
  recursively calls `earcutLinked(a, ...)` then `earcutLinked(c, ...)`, and returns.
- **`isValidDiagonal(a, b)`**:
  ```text
  zeroLength = equals(a, b) && area(a.prev, a, a.next) > 0 && area(b.prev, b, b.next) > 0
  return a.next.i !== b.i &&
    (zeroLength || (locallyInside(a, b) && locallyInside(b, a) &&
                    (area(a.prev, a, b.prev) !== 0 || area(a, b.prev, b) !== 0))) &&
    !intersectsPolygon(a, b) && (zeroLength || middleInside(a, b))
  ```
- **`intersects(p1, q1, p2, q2, includeBoundary = true)`**: strict crossing test on `o1..o4 = area(...)`;
  if `includeBoundary`, also tests `o_k === 0 && onSegment(...)` for all four endpoints.
- **`intersectsPolygon(a, b)`**: bbox fast skip (`> maxX`, `< minX`, `> maxY`, `< minY`), skips
  edges sharing vertex index `a.i` or `b.i`, tests `intersects(p, n, a, b, true)`.
- **`locallyInside(a, b)`**: if `area(a.prev, a, a.next) < 0` then `area(a, b, a.next) >= 0 && area(a, a.prev, b) >= 0`
  else `area(a, b, a.prev) < 0 || area(a, a.next, b) < 0`.
- **`middleInside(a, b)`**: odd-even ray-cast from midpoint `((a.x + b.x) / 2, (a.y + b.y) / 2)`
  checking `((p.y > py) !== (n.y > py)) && (px < (n.x - p.x) * (py - p.y) / (n.y - p.y) + p.x)`.
- **`sectorContainsSector(m, p)`**: `area(m.prev, m, p.prev) < 0 && area(p.next, m, m.next) < 0`.

### `deviation(data, holeIndices, dim, triangles)`

1. `polygonArea = |signedArea(data, 0, outerLen, dim)|`, minus `|signedArea(data, start, end, dim)|`
   for each hole.
2. `trianglesArea = sum_{i = 0, 3, ...} |(data[a] - data[c]) * (data[b + 1] - data[a + 1]) - (data[a] - data[b]) * (data[c + 1] - data[a + 1])|`
   where `a = triangles[i] * dim`, `b = triangles[i + 1] * dim`, `c = triangles[i + 2] * dim`.
3. If `trianglesArea === 0`:
   `max = max(0, |data[i]|, |data[i + 1]| for i = 0, dim, ... < data.length)`.
   Return `|polygonArea| <= ((data.length * max) * max) * 2.220446049250313e-16 ? 0 : 1`.
4. Return `|(trianglesArea - polygonArea) / polygonArea|`.

### `flatten(rings)`

- `dimensions = rings[0][0].length` (throws if `rings` or `rings[0]` is empty).
- Iterates `ring` of `rings`: pushes `p[0..dimensions-1]` for each `p` of `ring`; if `prevLen != 0`,
  `holeIndex += prevLen; holes.push(holeIndex)`; `prevLen = ring.length`.
- A short point contributes JavaScript `undefined` for each missing component. Components beyond the first point’s dimension are skipped.
- Returns `{ vertices, holes, dimensions }` in that exact key order.

### `refine(triangles, coords, dim = 2)`

1. `n = triangles.length`; if `n < 6`, return unchanged.
2. Allocate half-edge twins `he[0..n] = -1`, `edgeStack[0..n]`, `edgeStamp[0..n] = 0`, and open-addressing
   hash table of power-of-two size `size >= 4 * n` (`hMask = size - 1`).
3. For `e = 0..n - 1`:
   - `a = t[e], b = t[nextHE(e)]` where `nextHE(e) = e - (e % 3) + ((e + 1) % 3)`.
   - `lo = min(a, b), hi = max(a, b)`.
   - `h = (imul(lo, 0x9e3779b1) ^ imul(hi, 0x85ebca6b)) & hMask`.
   - Linear probe: skip consumed slots (`-1`); on first live slot `s` matching `{lo, hi}`, link
     `he[e] = s; he[s] = e`, mark slot consumed (`-1`), push `s` (`edgeStamp[s] = 1; edgeStack[i++] = s`),
     and break. If empty slot reached, insert `e`.
4. While `i > 0`:
   - Pop `a = edgeStack[--i]; edgeStamp[a] = 0`; `b = he[a]`; if `b == -1`, continue.
   - `a0 = a - a % 3`, `b0 = b - b % 3`, `ar = a0 + (a + 2) % 3`, `al = a0 + (a + 1) % 3`,
     `bl = b0 + (b + 2) % 3`, `br = b0 + (b + 1) % 3`.
   - `p0 = t[ar], pr = t[a], pl = t[al], p1 = t[bl]`.
   - If `!inCircle(p0, pr, pl, p1) && orient(p0, pr, p1) > 0 && orient(p0, p1, pl) > 0`:
     - Flip: `t[a] = p1; t[b] = p0`.
     - Relink twins: `hbl = he[bl]; har = he[ar]; he[a] = hbl; if hbl != -1 { he[hbl] = a }; he[b] = har; if har != -1 { he[har] = b }; he[ar] = bl; he[bl] = ar`.
     - Re-queue up to 4 outer edges in exact order `a`, `b`, `al`, `br` whenever twin `!= -1` and `edgeStamp == 0`.

## Parity domain

- All coordinates are finite JSON numbers (`-1e12 <= c <= 1e12`).
- `dim` is an integer `>= 2`, and `data.length % dim === 0` (and `coords.length % dim === 0`).
- `holeIndices`, when non-null and non-empty, is a **strictly increasing** list of integer vertex indices with `0 < holeIndices[0]` and every entry `< data.length / dim`. The outer ring and every hole slice therefore have at least one vertex.
- **Out of domain: empty hole slices.** Two equal entries (`holeIndices: [4, 4, 7]`), or a last entry equal to `data.length / dim` (`[4, 7, 10]` on 10 vertices), give a hole slice with no vertices. npm's `linkedList` returns `undefined` for it, and `eliminateHoles` throws a `TypeError` reading `list.next`. Ports differ: ReScript, Melange, js_of_ocaml and GopherJS throw, Scala.js, Kotlin and Dart put their sentinel `-1` vertex index into the triangles, Rust skips the hole, and Elm does not terminate. The rule above excludes both shapes.
- `compareXYSlope` never receives a hole triple sharing the same leftmost `(x, y)` where one has `0/0 = NaN` slope and another has a finite slope (which is not a strict weak order in JS).
- `triangles` passed to `deviation` and `refine` have length divisible by 3 and indices in `[0, coords.length / dim)`.

## Per-language porting notes

- **32-bit integer bitwise ops (`zOrder`, `refine` hash):** `zOrder` uses JS 32-bit signed bitwise
  shifts/masks (`| 0`, `<<`, `&`) and `radixPass` shifts unsigned 32-bit (`>>>`). `refine` uses
  wrapping 32-bit `imul` (`0x9e3779b1`, `0x85ebca6b`).
  - **Dart (`dart2js`):** use `.toUnsigned(32)` / `>>>` and `(a * b).toUnsigned(32)` (or split 16×16
    multiply if needed, since JS double multiplication of two 32-bit ints loses precision above 2^53 —
    use `Math.imul` or 16-bit split `(((lo & 0xffff) * C) + (((lo >>> 16) * C & 0xffff) << 16)) | 0`!).
  - **Elm:** `Bitwise` operates on 32-bit signed ints (`Bitwise.shiftRightZfBy` for `>>>`), and
    `imul` must use a 16-bit split helper (Elm `Int * Int` is IEEE-754 double multiplication).
  - **OCaml (`melange`, `jsoo`), ReScript:** OCaml `int` on JS is 32-bit signed (`lsr` is `>>>`),
    `Int32.mul` or `Math.imul` gives exact wrapping 32-bit multiply.
  - **Rust (`mires`), Scala.js, Kotlin/JS, Go (GopherJS):** native `i32`/`u32` wrapping arithmetic
    (`wrapping_mul`, `>>` on `u32`). For Morton coordinates outside the signed 32-bit range, truncate toward zero before reducing modulo 2³², and map NaN and ±Infinity to 0, as ToInt32 does. Flooring instead of truncating changes the triangles in `earcut-zorder-negative-wrap`. A cast that saturates, at 32 bits or at 64 bits before wrapping, changes them in `earcut-zorder-infinite-scale` (a subnormal outer box makes `invSize` Infinity) and `earcut-zorder-beyond-2-64` (Morton arguments of magnitude 2^64 to 2^68).
- **Arena-based circular doubly-linked lists (Rust, Elm):** Store `Node`s in a flat append-only
  array/vector (`Vec<Node>` in Rust, `Array Node` in Elm) indexed by `u32`/`Int`, with `NONE = u32::MAX`
  (or `-1`) for `prevZ`/`nextZ`. Never recycle removed node indices during an `earcut` call so dead
  nodes retain their `prev`/`next` links for `p.prev.next !== p`, `liveBlockHead`, and `liveBlockStop`.
- **No FMA:** Keep `f64` multiplications and additions separate in the exact order shown above.
- **Stable sort for `compareXYSlope`:** Use a stable sort so ties (including `0` and `NaN` slope
  differences) preserve original `holeIndices` order identically to JS `Array.prototype.sort`.
- **Object key order in `flatten`:** Construct `{ vertices, holes, dimensions }` in that exact order.

## Bench cases and variants

- **`bench-earcut`**: `batch` of 32 realistic dyadic-coordinate polygons (`refine: false`) totaling
  11,182 vertices, 358 holes, and 11,834 output triangles (~2.1 ms per call in Node JS):
  - 10 multi-hole perturbed coastlines (260–410 outer vertices + 12–19 holes each; exercises `K = 16`
    block-bbox index, z-order hashing, and 4-pass LSD radix sort);
  - 6 bayed architectural city blocks with 12 chamfered courtyards each;
  - 4 multi-turn Archimedean spiral ribbons (258–354 vertices);
  - 4 toothed gears with central hub + 8 bolt holes;
  - 8 medium perturbed polygons (44–72 vertices, below the 80-vertex z-order threshold, exercising
    the unhashed `isEar` loop alongside `isEarHashed`).
- **`bench-refine`**: `batch` of 24 realistic multi-hole polygons (`refine: true`) totaling 6,992
  vertices, 224 holes, and 7,392 triangles (~1.8–2.0 ms per call in Node JS): 8 multi-hole islands,
  6 bayed city blocks, 4 spirals, 3 long-toothed combs, and 3 gears with holes — heavily exercising
  both `earcut` and cascading Lawson Delaunay flips in `refine`.

**Bench variant** (`bench-variant.mjs`, browser-safe, zero imports): for each polygon in `items`,
applies a `k`-seeded exact `D4` rigid isometry (one of 8 axis-aligned rotations/reflections) plus a
non-zero integer translation `(tx, ty)` in `[-1024, 1024]`, cyclically rotates the outer ring and
every hole ring by a non-zero offset, and permutes the hole order. Because all bench coordinates are
exact binary multiples of `2^-8` within `[-1000, 1000]`, the transform introduces **zero** IEEE-754
roundoff error while varying the input coordinates and output indices, preserving
exact triangle counts and work equivalence (`51/51` distinct inputs and outputs verified for `k = 1..50`).

## Corpus

92 deterministic cases generated by `generate-corpus.mjs` (covering **44/44** functions in
`earcut@3.2.4`):

| Group | Cases | What they exercise |
| --- | --- | --- |
| Empty, tiny & degenerate (`12`) | `earcut-empty`, `earcut-1-vertex`, `earcut-2-vertices`, `earcut-closed-2-vertices`, `earcut-triangle-ccw`, `earcut-triangle-cw`, `earcut-convex-quad`, `earcut-closed-ring`, `earcut-collinear-zero-area`, `earcut-duplicate-consecutive`, `earcut-collinear-edges`, `earcut-zero-bbox-degenerate` | `< 3` vertices early exits, CW/CCW `linkedList` winding normalization, closing-vertex removal, collinear/duplicate healing via `filterPoints`, `filteredOut` stall re-slice, zero-bbox collapse |
| Concave & unhashed (`5`) | `earcut-concave-lshape`, `earcut-star-10`, `earcut-comb`, `earcut-spiral-small`, `earcut-pinched-outer` | `isEar` unhashed reflex checks, narrow fingers, `pointInTriangle`, pinched shared vertex |
| Higher dimensions (`2`) | `earcut-dim-3`, `earcut-dim-4-large` | `dim = 3` and `dim = 4` strides (including `data.length > 80 * dim` threshold and holes) |
| Large & z-order (`11`) | `earcut-threshold-80-unhashed`, `earcut-threshold-81-hashed`, `earcut-large-zero-bbox`, `earcut-large-hilbert`, `earcut-large-spiral-150`, `earcut-zorder-insertion-sort`, `earcut-zorder-negative-wrap`, `earcut-large-coords`, `earcut-tiny-scale`, `earcut-zorder-infinite-scale`, `earcut-zorder-beyond-2-64` | Exact `80 * dim` vs `81 * dim` threshold, `invSize === 0` fallback, `zOrder`, `sortNodes` both `n > 32` 4-pass LSD radix sort and `n <= 32` insertion sort, `isEarHashed` `prevZ`/`nextZ`, `1e6` and `1e-9` coordinate scales, negative Morton overflow, ToInt32 of an Infinity scale and of Morton arguments past 2^64 (a saturating conversion at 32 or 64 bits fails both) |
| Holes, block-bbox & Steiner (`19`) | `earcut-hole-leftmost-y-tie`, `earcut-single-hole`, `earcut-empty-hole-indices`, `earcut-null-hole-indices`, `earcut-hole-windings`, `earcut-multiple-holes-unsorted`, `earcut-holes-same-x-diff-y`, `earcut-holes-shared-leftmost-slope`, `earcut-holes-vertical-edge-slope`, `earcut-hole-touches-outer-vertex`, `earcut-hole-touches-outer-edge`, `earcut-hole-t-junction-horizontal`, `earcut-hole-sector-contains`, `earcut-hole-reflex-bridge-angle`, `earcut-hole-steiner-single`, `earcut-hole-steiner-multiple`, `earcut-hole-collinear-growblock`, `earcut-many-holes-block-index`, `earcut-hole-outside-left` | `compareXYSlope` (`x`, `y`, finite slope, `±Infinity` vertical slope), `findHoleBridge` (`equals`, `x === hx`, horizontal reflex T-junction, Eberly triangle `tanMin`/`p.x`/`sectorContainsSector` returning `true`), `steiners` preservation, `K = 16` block-bbox index, `indexActive` + `growBlock` all 4 bounds + `liveBlockHead`/`liveBlockStop`, `null` bridge fallback |
| Recovery stages (`9`) | `earcut-cure-local-hourglass`, `earcut-cure-local-intersection`, `earcut-cure-local-ribbon`, `earcut-self-intersecting-pentagram`, `earcut-self-intersecting-local-loop`, `earcut-split-earcut-twisted`, `earcut-split-earcut-pinched`, `earcut-split-earcut-hashed`, `earcut-split-earcut-zerolen-collinear` | `cureLocalIntersections` (`cured = true`, `filterPoints(p)`), `splitEarcut`, `isValidDiagonal`, `intersects` (both crossing and `onSegment` collinear), `intersectsPolygon`, `middleInside`, `splitPolygon`, recursive hashed `earcutLinked` |
| `deviation` (`9`) | `deviation-exact-zero`, `deviation-with-holes`, `deviation-dim-3`, `deviation-missing-triangle`, `deviation-extra-overlap`, `deviation-empty-degenerate`, `deviation-empty-nonzero`, `deviation-roundoff-zero-area`, `deviation-zero-polygon-nonzero-tris` | Exact `0`, multi-hole `holeIndices[i + 1]`, `dim = 3`, fractional & overlapping areas, empty-triangles `EPSILON` threshold (`0` vs `1`), zero-area polygon → `Infinity` |
| `flatten` (`7`) | `flatten-simple-2d`, `flatten-multi-hole-2d`, `flatten-3d`, `flatten-4d`, `flatten-with-empty-ring`, `flatten-mixed-dimensions`, `flatten-empty-throws` | 2D/3D/4D flattening, multi-hole index accumulation, `{ vertices, holes, dimensions }` key order, `prevLen` empty-ring quirk, mixed point dimensions, empty input throw |
| `refine` & `earcutRefine` (`14`) | `refine-empty`, `refine-single-triangle`, `refine-sliver-quad`, `refine-already-delaunay`, `refine-orient1-guard`, `refine-orient2-guard`, `refine-cocircular-square`, `refine-near-cocircular-eps`, `refine-cascade-ellipse`, `refine-boundary-preserved`, `refine-dim-3`, `refine-non-manifold`, `earcut-refine-multi-hole`, `earcut-refine-city-block` | `n < 6` skip, sliver flip without input mutation, already-Delaunay no-op, both `orient > 0` guards with `!inCircle === true`, exact cocircular & `1e-13 * s * s` near-cocircular ties (#205), cascading flips + `edgeStamp` dedup, constrained hole boundary (`he === -1`), `dim = 3`, non-manifold consumed `-1` hash slot, large multi-hole end-to-end refinement |
| `batch` & errors (`2`) | `batch-mixed`, `error-unknown-op` | Mixed `dim`/holes/`refine` batch dispatch, unknown op `TypeError` |
| Benchmarks (`2`) | `bench-earcut`, `bench-refine` | Primary timing workloads (`batch`, varied by `bench-variant.mjs`) |
