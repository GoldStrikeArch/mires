// Reference adapter for earcut 3.2.4: the published ESM entry, imported by a relative path (no
// bare specifier, no import map), so Node and the browsers load the same file.
//
// `run(input)` dispatches `input.op` onto earcut 3.2.4's full public API (`earcut`, `deviation`,
// `flatten`, `refine`) plus the convenience pipelines `earcutRefine` and `batch` (CONTRACT.md,
// "Input" / "Output"). `refine` clones `input.triangles` before refining in place so `input` is
// never mutated.
import earcut, { deviation, flatten, refine } from '../../node_modules/earcut/src/earcut.js'

export function run(input) {
  switch (input.op) {
    case 'earcut':
      return earcut(input.data, input.holeIndices, input.dim)
    case 'deviation':
      return deviation(input.data, input.holeIndices, input.dim, input.triangles)
    case 'flatten':
      return flatten(input.rings)
    case 'refine': {
      const triangles = input.triangles.slice()
      refine(triangles, input.coords, input.dim)
      return triangles
    }
    case 'earcutRefine': {
      const triangles = earcut(input.data, input.holeIndices, input.dim)
      refine(triangles, input.data, input.dim)
      return triangles
    }
    case 'batch': {
      const items = input.items
      const results = new Array(items.length)
      for (let i = 0; i < items.length; i++) {
        const item = items[i]
        const triangles = earcut(item.data, item.holeIndices, item.dim)
        if (item.refine) refine(triangles, item.data, item.dim)
        results[i] = triangles
      }
      return results
    }
    default:
      throw new TypeError(`unknown operation ${input.op}`)
  }
}
