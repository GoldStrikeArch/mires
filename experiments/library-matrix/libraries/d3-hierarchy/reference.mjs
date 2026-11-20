import { hierarchy, stratify, tree, cluster, partition, pack, treemap,
  treemapSquarify, treemapBinary, treemapSliceDice } from '../../node_modules/d3-hierarchy/dist/d3-hierarchy.mjs'

export function run(input) {
  const root = input.source === 'id' ? stratify()(input.rows)
    : input.source === 'path' ? stratify().path(d => d.id)(input.rows)
    : hierarchy(input.data)
  root.sum(d => d?.value ?? 0)
  if (input.sort) root.sort((a, b) => b.value - a.value)
  const w = input.width, h = input.height, p = input.padding
  switch (input.op) {
    case 'tree': case 'cluster': {
      const layout = input.op === 'tree' ? tree() : cluster()
      if (input.nodeSize) layout.nodeSize([w, h])
      else layout.size([w,h])
      layout(root)
      break
    }
    case 'partition': partition().size([w,h]).padding(p).round(input.round)(root); break
    case 'pack': pack().size([w,h]).padding(p)(root); break
    case 'treemap': {
      const tiles = {binary: treemapBinary, sliceDice: treemapSliceDice, squarify: treemapSquarify}
      treemap().size([w,h]).tile(tiles[input.tile]).padding(p).round(input.round)(root)
      break
    }
    case 'hierarchy': case 'iterator': case 'stratify': break
    default: throw new Error('unknown layout')
  }
  const id = n => n.id ?? n.data.id
  const output = []
  function visit(n) {
    const record = [id(n), n.parent ? id(n.parent) : null, n.depth, n.height, n.value]
    if (input.op === 'tree' || input.op === 'cluster') record.push(n.x, n.y)
    if (input.op === 'pack') record.push(n.x, n.y, n.r)
    if (input.op === 'partition' || input.op === 'treemap') record.push(n.x0,n.y0,n.x1,n.y1)
    output.push(record)
  }
  if (input.op === 'iterator') for (const n of root) visit(n)
  else root.eachBefore(visit)
  return output
}
