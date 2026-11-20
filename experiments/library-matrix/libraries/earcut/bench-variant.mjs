// Bench variant for earcut (registry `bench.variant`; README "Adding a library", step 5).
// Browser-safe, zero imports.
//
// For each polygon in a `batch` input, the k-th variant (k >= 1) applies a k-seeded exact rigid
// isometry from D4 (one of 8 axis-aligned rotations/reflections) plus a non-zero integer
// translation (tx, ty), cyclically rotates the vertex order of the outer ring and every hole ring,
// and permutes the hole ring order.
//
// Because every bench polygon uses dyadic coordinates (multiples of 2^-8 within [-1000, 1000]),
// the D4 transform and integer translation are 100% exact in IEEE-754 f64 (zero rounding error),
// preserving polygon simplicity, area, hole nesting, vertex/hole counts, and exact triangle count
// while varying the coordinate and triangle-index sequences.

function mulberry32(seed) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let value = state
    value = Math.imul(value ^ (value >>> 15), value | 1)
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296
  }
}

function transformXY(x, y, sym, tx, ty) {
  let nx
  let ny
  switch (sym) {
    case 0: nx = x; ny = y; break
    case 1: nx = -y; ny = x; break
    case 2: nx = -x; ny = -y; break
    case 3: nx = y; ny = -x; break
    case 4: nx = -x; ny = y; break
    case 5: nx = y; ny = x; break
    case 6: nx = x; ny = -y; break
    default: nx = -y; ny = -x; break
  }
  return [nx + tx + 0, ny + ty + 0]
}

function varyItem(item, rnd) {
  const data = item.data
  const dim = item.dim ?? 2
  const numVerts = Math.floor(data.length / dim)
  const holesIn = item.holeIndices && item.holeIndices.length ? item.holeIndices : null

  // Slice into rings [startVert, endVert).
  const rings = []
  if (holesIn) {
    rings.push([0, holesIn[0]])
    for (let h = 0; h < holesIn.length; h++) {
      const start = holesIn[h]
      const end = h + 1 < holesIn.length ? holesIn[h + 1] : numVerts
      rings.push([start, end])
    }
  } else {
    rings.push([0, numVerts])
  }

  // Permute hole ring order (rings[1..]).
  if (rings.length > 2) {
    for (let i = rings.length - 1; i > 1; i--) {
      const j = 1 + Math.floor(rnd() * i)
      const tmp = rings[i]
      rings[i] = rings[j]
      rings[j] = tmp
    }
  }

  const sym = Math.floor(rnd() * 8)
  const tx = (1 + Math.floor(rnd() * 1024)) * (rnd() < 0.5 ? -1 : 1)
  const ty = (1 + Math.floor(rnd() * 1024)) * (rnd() < 0.5 ? -1 : 1)

  const outData = new Array(data.length)
  const outHoles = rings.length > 1 ? new Array(rings.length - 1) : null
  let cursorVert = 0

  for (let r = 0; r < rings.length; r++) {
    const [start, end] = rings[r]
    const len = end - start
    if (r > 0) outHoles[r - 1] = cursorVert
    const shift = len > 1 ? 1 + Math.floor(rnd() * (len - 1)) : 0
    for (let i = 0; i < len; i++) {
      const srcVert = start + ((i + shift) % len)
      const srcOff = srcVert * dim
      const dstOff = (cursorVert + i) * dim
      const [nx, ny] = transformXY(data[srcOff], data[srcOff + 1], sym, tx, ty)
      outData[dstOff] = nx
      outData[dstOff + 1] = ny
      for (let d = 2; d < dim; d++) outData[dstOff + d] = data[srcOff + d]
    }
    cursorVert += len
  }

  const out = { data: outData }
  if (outHoles) out.holeIndices = outHoles
  if (item.dim !== undefined) out.dim = item.dim
  if (item.refine !== undefined) out.refine = item.refine
  return out
}

export function variant(input, k) {
  if (!Number.isInteger(k) || k < 1) {
    throw new Error(`bench variant: k must be an integer >= 1 (got ${k})`)
  }
  if (!input || input.op !== 'batch' || !Array.isArray(input.items)) {
    throw new Error(`bench variant: unexpected operation ${input?.op}`)
  }
  const rnd = mulberry32((0x9e3779b9 ^ Math.imul(k, 0x85ebca6b)) >>> 0)
  const items = new Array(input.items.length)
  for (let i = 0; i < input.items.length; i++) {
    items[i] = varyItem(input.items[i], rnd)
  }
  return { op: 'batch', items }
}
