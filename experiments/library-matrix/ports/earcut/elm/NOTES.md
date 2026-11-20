# earcut 3.2.4 in Elm

The geometry runs in Elm. Each request allocates its own arena of numbered nodes in a persistent
`Array`. Removing a node updates its neighbors while retaining its old links so a hole-bridge
scan can skip past a removed block head or stop. `Holes.elm` builds 16-edge blocks in encounter
order. `Morton.elm` links the surviving vertices by signed insertion sort for small rings and four
stable unsigned-byte radix passes for larger ones.

`Refine.elm` pairs half-edges by linear probing, marks each consumed slot, and seeds a LIFO stack
in input order. Rechecking four outer edges after each flip preserves the upstream triangle order.

The JavaScript host forwards requests and replies through Elm ports. When a point has fewer
coordinates than the first point in a `flatten` request, Elm emits `null` for each missing
coordinate. The host converts those elements to JavaScript `undefined`. All finite coordinate
values, including negative zero, pass through as numbers.
