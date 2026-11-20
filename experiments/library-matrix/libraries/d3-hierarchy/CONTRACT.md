# d3-hierarchy 3.1.3

Inputs with `source: "hierarchy"` supply a nested `data` object with a string `id`, optional numeric `value` and optional `children` array. Flat `rows` with `source: "id"` use the default stratify id and parentId accessors. With `source: "path"`, each id is a path. Stratification preserves input order, escaping, implicit ancestors, root trimming and upstream errors. Each node aggregates its own weight and its children’s values. A weight of negative zero contributes positive zero. Setting `sort` to true sorts children stably by descending aggregate value.

Operations are hierarchy, stratify, iterator, tree, cluster, treemap, pack and partition. The numeric width and height specify layout size. For tree and cluster the boolean nodeSize selects fixed node spacing. Treemap supports the squarify, binary and sliceDice tilers. Numeric padding applies to treemap, pack and partition. The boolean round selects integer rounding for treemap and partition.

Each output row starts with the id, parent id (null for the root), depth, height and aggregate value. Tree and cluster append x and y, pack appends x, y and radius, and rectangular layouts append x0, y0, x1 and y1. The iterator operation returns breadth-first order. All others use pre-order. Equality includes every coordinate as an exact JavaScript number, including NaN and signed zero.

The aggregate of a leaf with weight `-0` is positive zero. The browser checker reads literal `-0` from the corpus. The standalone Node verifier reads the same file directly and uses `Object.is` on both the input weight and the output aggregate. This preserves the sign through the test, which ordinary JSON serialization would erase.

The two timing cases lay out all 10,000 nodes as a treemap and as packed circles. Each timing variant changes the data in every subtree while preserving the tree shape. Run `node generate-corpus.mjs` to regenerate the corpus. The upstream flare hierarchy is included with its original node names and weights. [Upstream fixture](https://github.com/d3/d3-hierarchy/blob/41f11f82647566c90150272210c50188dec4071b/test/data/flare.json)

After building a port, run `node verify.mjs <language>` here to check the official 252-node circle-pack fixture and 576 further exact layout comparisons on independently generated trees. The published fixture rounds coordinates to hundredths. The randomized checks compare each unrounded coordinate directly. [Upstream pack answers](https://github.com/d3/d3-hierarchy/blob/41f11f82647566c90150272210c50188dec4071b/test/data/flare-pack.json)
