#!/usr/bin/env node
// Writes corpus.json for the earcut 3.2.4 library: every case is built deterministically (no
// clock, no Math.random: all pseudo-random data uses mulberry32 with fixed seeds), so corpus.json
// is a pure function of this script. CONTRACT.md documents every case.
//
//   node libraries/earcut/generate-corpus.mjs            # writes corpus.json
//   node libraries/earcut/generate-corpus.mjs --verify   # also runs reference.mjs on every case,
//                                                        # validates all invariants, bench-variant,
//                                                        # and prints bench timings.

import { createHash } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { performance } from 'node:perf_hooks'

const here = dirname(fileURLToPath(import.meta.url))

// ---------------------------------------------------------------------------------------------
// Deterministic PRNG & exact dyadic quantization.

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

// Quantize to 1/256 (exact binary float) so rigid D4 transforms + integer translations in
// bench-variant.mjs introduce zero IEEE-754 rounding error.
function q(v) {
  const r = Math.round(v * 256) / 256
  return r === 0 ? 0 : r
}

// Pack rings (array of [[x, y], ...]) into { data, holeIndices } with optional dim >= 2.
function packRings(rings, dim = 2, extraFn = null) {
  const data = []
  const holeIndices = []
  let vertCount = 0
  for (let r = 0; r < rings.length; r++) {
    if (r > 0) holeIndices.push(vertCount)
    const ring = rings[r]
    for (let i = 0; i < ring.length; i++) {
      const p = ring[i]
      data.push(p[0], p[1])
      for (let d = 2; d < dim; d++) {
        data.push(extraFn ? extraFn(r, i, d) : (p[d] ?? (vertCount * 0.5 + d)))
      }
      vertCount++
    }
  }
  const out = { data }
  if (holeIndices.length > 0) out.holeIndices = holeIndices
  if (dim !== 2) out.dim = dim
  return out
}

// ---------------------------------------------------------------------------------------------
// Procedural geometry builders (all guaranteed simple, non-self-intersecting, dyadic coords).

function makeStarRing(cx, cy, rOuter, rInner, points, phase = 0) {
  const ring = []
  const n = points * 2
  for (let i = 0; i < n; i++) {
    const angle = phase + (i * Math.PI * 2) / n
    const r = i % 2 === 0 ? rOuter : rInner
    ring.push([q(cx + Math.cos(angle) * r), q(cy + Math.sin(angle) * r)])
  }
  return ring
}

function makePerturbedRing(cx, cy, baseR, n, rnd, amp = 0.22, harmonics = 5, cw = false) {
  const freqs = []
  const amps = []
  const phases = []
  let ampSum = 0
  for (let h = 0; h < harmonics; h++) {
    const f = 2 + h * 2 + Math.floor(rnd() * 2)
    const a = (amp / (h + 1.2)) * (0.5 + rnd() * 0.5)
    freqs.push(f)
    amps.push(a)
    phases.push(rnd() * Math.PI * 2)
    ampSum += a
  }
  const scale = ampSum > amp ? amp / ampSum : 1
  const ring = []
  for (let i = 0; i < n; i++) {
    const idx = cw ? n - 1 - i : i
    const theta = (idx * Math.PI * 2) / n
    let r = baseR
    for (let h = 0; h < harmonics; h++) {
      r += baseR * amps[h] * scale * Math.sin(freqs[h] * theta + phases[h])
    }
    ring.push([q(cx + Math.cos(theta) * r), q(cy + Math.sin(theta) * r)])
  }
  return ring
}

function makeIslandWithLakes(seed, outerN, gridN, holeVerts, refine = false) {
  const rnd = mulberry32(seed)
  const baseR = 260
  const outer = makePerturbedRing(0, 0, baseR, outerN, rnd, 0.24, 6, false)
  const rings = [outer]

  // Place holes strictly inside radius <= 165 on a jittered grid so no hole can touch another
  // hole or the outer coastline (outer min radius >= 260 * 0.76 = 197.6).
  const span = 220
  const cell = span / gridN
  const half = span / 2
  for (let gx = 0; gx < gridN; gx++) {
    for (let gy = 0; gy < gridN; gy++) {
      const cx = -half + (gx + 0.5) * cell + (rnd() - 0.5) * cell * 0.18
      const cy = -half + (gy + 0.5) * cell + (rnd() - 0.5) * cell * 0.18
      if (cx * cx + cy * cy > 142 * 142) continue
      const hr = cell * (0.24 + rnd() * 0.1)
      const hN = holeVerts + Math.floor(rnd() * 5)
      rings.push(makePerturbedRing(cx, cy, hr, hN, rnd, 0.2, 3, true))
    }
  }
  const packed = packRings(rings)
  packed.refine = refine
  return packed
}

function makeCityBlock(seed, notches, cols, rows, refine = false) {
  const rnd = mulberry32(seed)
  const W = 400
  const H = 320
  const cornerPad = 28
  const outer = []
  // Bottom edge (left -> right) with inward rectangular bays kept clear of corners
  outer.push([q(-W / 2), q(-H / 2)])
  const spanX = W - 2 * cornerPad
  for (let i = 0; i < notches; i++) {
    const x0 = -W / 2 + cornerPad + (i * spanX) / notches
    const x1 = -W / 2 + cornerPad + ((i + 1) * spanX) / notches
    const bay = 10 + Math.floor(rnd() * 14)
    const pad = (x1 - x0) * 0.22
    outer.push([q(x0 + pad), q(-H / 2)])
    outer.push([q(x0 + pad), q(-H / 2 + bay)])
    outer.push([q(x1 - pad), q(-H / 2 + bay)])
    outer.push([q(x1 - pad), q(-H / 2)])
  }
  // Right edge (bottom -> top)
  outer.push([q(W / 2), q(-H / 2)])
  const spanY = H - 2 * cornerPad
  for (let i = 0; i < notches; i++) {
    const y0 = -H / 2 + cornerPad + (i * spanY) / notches
    const y1 = -H / 2 + cornerPad + ((i + 1) * spanY) / notches
    const bay = 10 + Math.floor(rnd() * 14)
    const pad = (y1 - y0) * 0.22
    outer.push([q(W / 2), q(y0 + pad)])
    outer.push([q(W / 2 - bay), q(y0 + pad)])
    outer.push([q(W / 2 - bay), q(y1 - pad)])
    outer.push([q(W / 2), q(y1 - pad)])
  }
  // Top edge (right -> left)
  outer.push([q(W / 2), q(H / 2)])
  for (let i = 0; i < notches; i++) {
    const x0 = W / 2 - cornerPad - (i * spanX) / notches
    const x1 = W / 2 - cornerPad - ((i + 1) * spanX) / notches
    const bay = 10 + Math.floor(rnd() * 14)
    const pad = (x0 - x1) * 0.22
    outer.push([q(x0 - pad), q(H / 2)])
    outer.push([q(x0 - pad), q(H / 2 - bay)])
    outer.push([q(x1 + pad), q(H / 2 - bay)])
    outer.push([q(x1 + pad), q(H / 2)])
  }
  // Left edge (top -> bottom)
  outer.push([q(-W / 2), q(H / 2)])
  for (let i = 0; i < notches; i++) {
    const y0 = H / 2 - cornerPad - (i * spanY) / notches
    const y1 = H / 2 - cornerPad - ((i + 1) * spanY) / notches
    const bay = 10 + Math.floor(rnd() * 14)
    const pad = (y0 - y1) * 0.22
    outer.push([q(-W / 2), q(y0 - pad)])
    outer.push([q(-W / 2 + bay), q(y0 - pad)])
    outer.push([q(-W / 2 + bay), q(y1 + pad)])
    outer.push([q(-W / 2), q(y1 + pad)])
  }

  const rings = [outer]
  const innerW = W - 76
  const innerH = H - 76
  const cw = innerW / cols
  const ch = innerH / rows
  for (let c = 0; c < cols; c++) {
    for (let r = 0; r < rows; r++) {
      const cx = -innerW / 2 + (c + 0.5) * cw
      const cy = -innerH / 2 + (r + 0.5) * ch
      const hw = cw * (0.28 + rnd() * 0.08)
      const hh = ch * (0.28 + rnd() * 0.08)
      const cut = Math.min(hw, hh) * 0.35
      // Chamfered courtyard (CW hole)
      rings.push([
        [q(cx - hw), q(cy - hh)],
        [q(cx - hw), q(cy + hh)],
        [q(cx + hw - cut), q(cy + hh)],
        [q(cx + hw), q(cy + hh - cut)],
        [q(cx + hw), q(cy - hh + cut)],
        [q(cx + hw - cut), q(cy - hh)],
      ])
    }
  }
  const packed = packRings(rings)
  packed.refine = refine
  return packed
}

function makeSpiralRibbon(turns, stepsPerTurn, rStart, pitch, width, refine = false) {
  const total = turns * stepsPerTurn
  const outer = []
  const inner = []
  for (let i = 0; i <= total; i++) {
    const t = i / stepsPerTurn
    const angle = t * Math.PI * 2
    const r0 = rStart + pitch * t
    const r1 = r0 + width
    outer.push([q(Math.cos(angle) * r1), q(Math.sin(angle) * r1)])
    inner.push([q(Math.cos(angle) * r0), q(Math.sin(angle) * r0)])
  }
  inner.reverse()
  const ring = outer.concat(inner)
  const packed = packRings([ring])
  packed.refine = refine
  return packed
}

function makeCombPolygon(teeth, width, height, toothDepth, refine = false) {
  const ring = []
  // Bottom spine with slight tilt so no unintended collinearities
  ring.push([q(0), q(0)])
  ring.push([q(width), q(0)])
  ring.push([q(width), q(height)])
  const step = width / teeth
  for (let i = teeth - 1; i >= 0; i--) {
    const xR = (i + 1) * step - step * 0.15
    const xM = (i + 0.5) * step
    const xL = i * step + step * 0.15
    ring.push([q(xR), q(height)])
    ring.push([q(xM + step * 0.12), q(height - toothDepth)])
    ring.push([q(xM - step * 0.12), q(height - toothDepth)])
    ring.push([q(xL), q(height)])
  }
  ring.push([q(0), q(height)])
  const packed = packRings([ring])
  packed.refine = refine
  return packed
}

function makeHilbertCorridor(order, stepSize, halfWidth) {
  // Generate Hilbert curve centerline of 4^order points, then offset to a simple ribbon.
  const n = 1 << order
  function d2xy(s, d) {
    let x = 0
    let y = 0
    let t = d
    for (let m = 1; m < s; m <<= 1) {
      const rx = 1 & (t >> 1)
      const ry = 1 & (t ^ rx)
      if (ry === 0) {
        if (rx === 1) {
          x = m - 1 - x
          y = m - 1 - y
        }
        const tmp = x
        x = y
        y = tmp
      }
      x += m * rx
      y += m * ry
      t >>= 2
    }
    return [x, y]
  }
  const pts = []
  for (let d = 0; d < n * n; d++) {
    const [gx, gy] = d2xy(n, d)
    pts.push([gx * stepSize, gy * stepSize])
  }
  // Build mitered left & right walls around the orthogonal Hilbert path.
  const left = []
  const right = []
  for (let i = 0; i < pts.length; i++) {
    const [x, y] = pts[i]
    if (i === 0) {
      const dx = Math.sign(pts[1][0] - x)
      const dy = Math.sign(pts[1][1] - y)
      left.push([q(x - dy * halfWidth - dx * halfWidth), q(y + dx * halfWidth - dy * halfWidth)])
      right.push([q(x + dy * halfWidth - dx * halfWidth), q(y - dx * halfWidth - dy * halfWidth)])
    } else if (i === pts.length - 1) {
      const dx = Math.sign(x - pts[i - 1][0])
      const dy = Math.sign(y - pts[i - 1][1])
      left.push([q(x - dy * halfWidth + dx * halfWidth), q(y + dx * halfWidth + dy * halfWidth)])
      right.push([q(x + dy * halfWidth + dx * halfWidth), q(y - dx * halfWidth + dy * halfWidth)])
    } else {
      const dx1 = Math.sign(x - pts[i - 1][0])
      const dy1 = Math.sign(y - pts[i - 1][1])
      const dx2 = Math.sign(pts[i + 1][0] - x)
      const dy2 = Math.sign(pts[i + 1][1] - y)
      if (dx1 === dx2 && dy1 === dy2) {
        // straight segment: skip interior collinear point or keep offset
        left.push([q(x - dy1 * halfWidth), q(y + dx1 * halfWidth)])
        right.push([q(x + dy1 * halfWidth), q(y - dx1 * halfWidth)])
      } else {
        // 90-degree corner: left normal averages (-dy1, dx1) and (-dy2, dx2)
        const lx = -(dy1 + dy2) * halfWidth
        const ly = (dx1 + dx2) * halfWidth
        left.push([q(x + lx), q(y + ly)])
        right.push([q(x - lx), q(y - ly)])
      }
    }
  }
  right.reverse()
  return left.concat(right)
}

function makeGearWithHoles(seed, teeth, holeCount, refine = false) {
  const rnd = mulberry32(seed)
  const outer = []
  const rOut = 240
  const rIn = 185
  for (let i = 0; i < teeth; i++) {
    const a0 = (i / teeth) * Math.PI * 2
    const a1 = ((i + 0.25) / teeth) * Math.PI * 2
    const a2 = ((i + 0.5) / teeth) * Math.PI * 2
    const a3 = ((i + 0.75) / teeth) * Math.PI * 2
    outer.push([q(Math.cos(a0) * rIn), q(Math.sin(a0) * rIn)])
    outer.push([q(Math.cos(a1) * rOut), q(Math.sin(a1) * rOut)])
    outer.push([q(Math.cos(a2) * rOut), q(Math.sin(a2) * rOut)])
    outer.push([q(Math.cos(a3) * rIn), q(Math.sin(a3) * rIn)])
  }
  const rings = [outer]
  // Central hub hole
  rings.push(makePerturbedRing(0, 0, 42, 16, rnd, 0.08, 2, true))
  // Ring of bolt holes
  for (let h = 0; h < holeCount; h++) {
    const a = (h / holeCount) * Math.PI * 2 + 0.13
    const cx = Math.cos(a) * 115
    const cy = Math.sin(a) * 115
    rings.push(makePerturbedRing(cx, cy, 22, 10, rnd, 0.12, 2, true))
  }
  const packed = packRings(rings)
  packed.refine = refine
  return packed
}

// ---------------------------------------------------------------------------------------------
// Bench workload builders (aiming for ~2-3.5 ms per call in Node JS).

function buildBenchEarcut() {
  const items = []
  // 10 complex multi-hole coastlines (>80 verts, block-bbox index + z-order + LSD radix sort)
  for (let i = 0; i < 10; i++) {
    items.push(makeIslandWithLakes(1000 + i * 37, 260 + (i % 4) * 50, 5, 10 + (i % 4) * 3, false))
  }
  // 6 city blocks with many orthogonal bays and courtyards (>80 verts + many holes)
  for (let i = 0; i < 6; i++) {
    items.push(makeCityBlock(2000 + i * 53, 8 + (i % 3) * 2, 4, 3, false))
  }
  // 4 spirals (>240 verts)
  for (let i = 0; i < 4; i++) {
    items.push(makeSpiralRibbon(4, 32 + i * 4, 24, 36, 15, false))
  }
  // 4 gears with ring of holes (>160 verts)
  for (let i = 0; i < 4; i++) {
    items.push(makeGearWithHoles(3000 + i * 41, 24 + i * 4, 8, false))
  }
  // 8 medium simple polygons (<=80 verts, exercising unhashed `isEar` alongside `isEarHashed`)
  for (let i = 0; i < 8; i++) {
    const rnd = mulberry32(4000 + i * 29)
    const ring = makePerturbedRing(0, 0, 180, 44 + i * 4, rnd, 0.34, 6, i % 2 === 0)
    items.push({ data: packRings([ring]).data, refine: false })
  }
  return { op: 'batch', items }
}

function buildBenchRefine() {
  const items = []
  // 8 complex multi-hole coastlines with Lawson Delaunay refinement
  for (let i = 0; i < 8; i++) {
    items.push(makeIslandWithLakes(5000 + i * 43, 220 + (i % 3) * 40, 4, 10 + (i % 3) * 3, true))
  }
  // 6 city blocks with long skinny corridor ears (heavy Lawson cascades!)
  for (let i = 0; i < 6; i++) {
    items.push(makeCityBlock(6000 + i * 59, 7 + (i % 2) * 2, 4, 3, true))
  }
  // 4 spirals with skinny ribbon triangles
  for (let i = 0; i < 4; i++) {
    items.push(makeSpiralRibbon(3, 32 + i * 4, 20, 34, 14, true))
  }
  // 3 combs with long teeth
  for (let i = 0; i < 3; i++) {
    items.push(makeCombPolygon(24 + i * 6, 420, 220, 175, true))
  }
  // 3 gears with holes
  for (let i = 0; i < 3; i++) {
    items.push(makeGearWithHoles(7000 + i * 61, 22 + i * 4, 7, true))
  }
  return { op: 'batch', items }
}

// ---------------------------------------------------------------------------------------------
// Corpus assembly (every code path in earcut@3.2.4).

function buildCases() {
  const cases = []
  const add = (id, note, input) => cases.push({ id, note, input })

  // === 1. Empty, degenerate, tiny & basic outer rings ===
  add('earcut-empty', 'empty coordinate array returns []', {
    op: 'earcut',
    data: [],
  })

  add('earcut-1-vertex', 'single vertex (outerNode.next === outerNode.prev) returns []', {
    op: 'earcut',
    data: [10, 20],
  })

  add('earcut-2-vertices', 'two vertices (outerNode.next === outerNode.prev) returns []', {
    op: 'earcut',
    data: [0, 0, 10, 5],
  })

  add('earcut-closed-2-vertices', '3 vertices with duplicate closing vertex collapse to 2 nodes -> []', {
    op: 'earcut',
    data: [0, 0, 10, 0, 0, 0],
  })

  add('earcut-triangle-ccw', 'single CCW triangle (signedArea > 0, forward linkedList)', {
    op: 'earcut',
    data: [0, 0, 10, 0, 5, 10],
  })

  add('earcut-triangle-cw', 'single CW triangle (signedArea < 0, reversed linkedList)', {
    op: 'earcut',
    data: [0, 0, 5, 10, 10, 0],
  })

  add('earcut-convex-quad', 'README example quad with omitted holeIndices and dim', {
    op: 'earcut',
    data: [10, 0, 0, 50, 60, 60, 70, 10],
  })

  add('earcut-closed-ring', 'closed GeoJSON-style square (last === first removed by linkedList)', {
    op: 'earcut',
    data: [0, 0, 10, 0, 10, 10, 0, 10, 0, 0],
  })

  add('earcut-collinear-zero-area', 'all vertices collinear -> filterPoints removes ring -> []', {
    op: 'earcut',
    data: [0, 0, 5, 0, 10, 0, 15, 0, 20, 0],
  })

  add('earcut-duplicate-consecutive', 'polygon with repeated consecutive vertices healed by filterPoints', {
    op: 'earcut',
    data: [0, 0, 0, 0, 10, 0, 10, 0, 10, 0, 10, 10, 5, 15, 5, 15, 0, 10],
  })

  add('earcut-collinear-edges', 'rectangle with extra collinear midpoints triggering filterPoints stall & re-slice', {
    op: 'earcut',
    data: [0, 0, 5, 0, 10, 0, 15, 0, 20, 0, 20, 5, 20, 10, 10, 10, 0, 10, 0, 5],
  })

  add('earcut-zero-bbox-degenerate', 'all vertices identical (zero area, zero bbox) returns []', {
    op: 'earcut',
    data: [5, 5, 5, 5, 5, 5, 5, 5],
  })

  // === 2. Concave, star, comb, spiral, pinched (unhashed isEar path, <=80 verts) ===
  add('earcut-concave-lshape', 'concave L/C-shaped polygon with reflex vertices blocking naive ears', {
    op: 'earcut',
    data: [0, 0, 40, 0, 40, 10, 10, 10, 10, 30, 40, 30, 40, 40, 0, 40],
  })

  add('earcut-star-10', '10-pointed alternating convex/reflex star (20 vertices)', {
    op: 'earcut',
    data: packRings([makeStarRing(0, 0, 100, 35, 10, 0.1)]).data,
  })

  add('earcut-comb', 'comb polygon with deep narrow fingers stressing pointInTriangle checks', {
    op: 'earcut',
    data: makeCombPolygon(12, 240, 120, 95).data,
  })

  add('earcut-spiral-small', '44-vertex non-hashed Archimedean spiral ribbon', {
    op: 'earcut',
    data: makeSpiralRibbon(2, 10, 12, 24, 8).data,
  })

  add('earcut-pinched-outer', 'outer ring pinching to a shared vertex (zero-area pinch & !(ax===px&&ay===py))', {
    op: 'earcut',
    data: [0, 0, 20, 0, 10, 10, 20, 20, 0, 20, 10, 10],
  })

  // === 3. Dimensions (dim = 3, dim = 4) ===
  add('earcut-dim-3', '3D polygon with hole (dim = 3, Z coordinate ignored, indices divided by 3)', {
    op: 'earcut',
    data: [
      0, 0, 1, 100, 0, 2, 100, 100, 3, 0, 100, 4,
      20, 20, 5, 80, 20, 6, 80, 80, 7, 20, 80, 8,
    ],
    holeIndices: [4],
    dim: 3,
  })

  {
    const island = makeIslandWithLakes(901, 90, 2, 8)
    const data4 = []
    for (let i = 0; i < island.data.length; i += 2) {
      data4.push(island.data[i], island.data[i + 1], (i / 2) * 0.25, -1.5)
    }
    add('earcut-dim-4-large', '4D multi-hole polygon >80 vertices (dim = 4, z-order threshold 80*dim)', {
      op: 'earcut',
      data: data4,
      holeIndices: island.holeIndices,
      dim: 4,
    })
  }

  // === 4. Large polygons (>80 vertices -> indexCurve, zOrder, sortNodes LSD radix, isEarHashed) ===
  {
    const rnd80 = mulberry32(101)
    const ring80 = makePerturbedRing(0, 0, 150, 80, rnd80, 0.3, 6, false)
    add('earcut-threshold-80-unhashed', 'exact 80 vertices (data.length === 80*dim: stays unhashed)', {
      op: 'earcut',
      data: packRings([ring80]).data,
    })
    const rnd81 = mulberry32(102)
    const ring81 = makePerturbedRing(0, 0, 150, 81, rnd81, 0.3, 6, false)
    add('earcut-threshold-81-hashed', 'exact 81 vertices (data.length > 80*dim: triggers indexCurve & LSD radix sort)', {
      op: 'earcut',
      data: packRings([ring81]).data,
    })
  }

  {
    const dup85 = []
    for (let i = 0; i < 85; i++) dup85.push(42.5, -17.25)
    add('earcut-large-zero-bbox', '85 coincident vertices (>80 verts but invSize === 0 fallback)', {
      op: 'earcut',
      data: dup85,
    })
  }

  add('earcut-large-hilbert', 'Hilbert-curve corridor (>240 vertices, stresses zOrder & prevZ/nextZ walks)', {
    op: 'earcut',
    data: packRings([makeHilbertCorridor(3, 20, 4)]).data,
  })

  add('earcut-large-spiral-150', '162-vertex high-aspect spiral triggering z-order hashing & LSD radix sort', {
    op: 'earcut',
    data: makeSpiralRibbon(4, 20, 10, 26, 9).data,
  })

  // The repeated vertex puts the polygon over the z-order threshold. Its two
  // holes have coordinates far outside the outer triangle's bounding box.
  add('earcut-zorder-negative-wrap', 'negative fractional Morton coordinates truncate before 32-bit wrapping', {
    op: 'earcut',
    data: [
      ...Array(72).fill([0, 0]), [210, 0], [0, 210],
      [0, 0], [200, -27525960.028], [0, 1],
      [0, 100], [1, -41288940], [1, 0], [34407450, 0],
    ].flat(),
    holeIndices: [74, 77],
  })

  {
    // Polygon >80 vertices that shrinks to <=32 nodes in eliminateHoles's filterPoints BEFORE
    // earcutLinked -> indexCurve -> sortNodes(n <= 32) insertion sort!
    const corners = [[0, 0], [100, 0], [100, 20], [20, 20], [20, 80], [100, 80], [100, 100], [0, 100]]
    const outer = []
    for (let i = 0; i < corners.length; i++) {
      const [x, y] = corners[i]
      const [nx, ny] = corners[(i + 1) % corners.length]
      for (let k = 0; k < 12; k++) outer.push([x + (nx - x) * k / 12, y + (ny - y) * k / 12])
    }
    const hole = [[4, 40], [12, 40], [12, 60], [4, 60]]
    add('earcut-zorder-insertion-sort', 'concave C-shaped ring and hole shrink before indexCurve; skipping insertion-sort index 1 changes clipping order', {
      op: 'earcut',
      ...packRings([outer, hole]),
    })
  }

  {
    const rnd = mulberry32(109)
    const ring = makePerturbedRing(-1e6, 2.5e6, 500, 96, rnd, 0.25, 5, false)
    add('earcut-large-coords', '96-vertex polygon at coordinates ~1e6 with negative X', {
      op: 'earcut',
      data: packRings([ring]).data,
    })
  }

  {
    const rnd = mulberry32(110)
    const raw = makePerturbedRing(0, 0, 100, 92, rnd, 0.25, 5, false)
    const tiny = raw.map(([x, y]) => [x * 1e-11, y * 1e-11])
    add('earcut-tiny-scale', '92-vertex polygon at 1e-9 coordinate scale (invSize ~ 1e13)', {
      op: 'earcut',
      data: packRings([tiny]).data,
    })
  }

  // A tiny outer square sets the z-order scale, and an 88-vertex spiky hole far to its right
  // supplies the vertices. The hole's leftmost vertex sits inside the square's y-range so it
  // bridges. Its other vertices straddle y = minY, so a saturating ToInt32 in zOrder drops
  // reflex vertices from isEarHashed's z-range walk and changes the triangles.
  const tinyBoxWithSpikyHole = (seed, s, cx, r) => {
    const rnd = mulberry32(seed)
    const cy = Math.round((rnd() - 0.5) * r)
    const angles = Array.from({ length: 88 }, () => rnd() * 2 * Math.PI).sort((a, b) => a - b)
    const hole = []
    let left = 0
    for (let j = 0; j < angles.length; j++) {
      const radius = r * (0.1 + 0.9 * rnd())
      hole.push(Math.round(cx + radius * Math.cos(angles[j])), Math.round(cy + radius * Math.sin(angles[j])))
      if (hole[2 * j] < hole[2 * left]) left = j
    }
    hole[2 * left] = cx - 2 * r
    hole[2 * left + 1] = s / 2
    return { op: 'earcut', data: [0, 0, s, 0, s, s, 0, s, ...hole], holeIndices: [4] }
  }
  add('earcut-zorder-infinite-scale', 'subnormal 1e-310 outer box: invSize = 32767 / 1e-310 is Infinity, every Morton argument is +-Infinity or NaN, and ToInt32 maps each to 0',
    tinyBoxWithSpikyHole(8, 1e-310, 1000, 400))
  add('earcut-zorder-beyond-2-64', '1e-6 outer box and a hole near x = 4e9: Morton arguments of magnitude 2^64 to 2^68 wrap modulo 2^32 exactly',
    tinyBoxWithSpikyHole(7, 1e-6, 4e9, 1.5e9))

  // === 5. Holes, compareXYSlope, block-bbox index (K=16), Steiner points ===
  add('earcut-single-hole', 'README square with single hole', {
    op: 'earcut',
    data: [0, 0, 100, 0, 100, 100, 0, 100, 20, 20, 80, 20, 80, 80, 20, 80],
    holeIndices: [4],
  })

  add('earcut-empty-hole-indices', 'holeIndices: [] treated identically to no holes', {
    op: 'earcut',
    data: [0, 0, 50, 0, 50, 50, 0, 50],
    holeIndices: [],
  })

  add('earcut-null-hole-indices', 'holeIndices: null with explicit dim: 2', {
    op: 'earcut',
    data: [0, 0, 50, 0, 50, 50, 0, 50],
    holeIndices: null,
    dim: 2,
  })

  add('earcut-hole-leftmost-y-tie', 'the top-left vertex is encountered before the equally leftmost bottom-left bridge candidate', {
    op: 'earcut',
    ...packRings([[[0, 0], [100, 0], [100, 100], [0, 100]], [[20, 80], [20, 20], [80, 20], [80, 80]]]),
  })

  add('earcut-hole-windings', 'outer CW with one CCW hole and one CW hole (linkedList winding normalization)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [0, 100], [100, 100], [100, 0]], // CW outer
      [[15, 15], [40, 15], [40, 40], [15, 40]], // CCW hole
      [[60, 60], [60, 85], [85, 85], [85, 60]], // CW hole
    ]),
  })

  add('earcut-multiple-holes-unsorted', '6 holes in scrambled X order exercising compareXYSlope primary sort', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [120, 0], [120, 100], [0, 100]],
      [[90, 60], [110, 60], [110, 80], [90, 80]],
      [[10, 10], [25, 10], [25, 25], [10, 25]],
      [[55, 20], [75, 20], [75, 40], [55, 40]],
      [[30, 65], [48, 65], [48, 85], [30, 85]],
      [[85, 12], [108, 12], [108, 35], [85, 35]],
      [[12, 60], [26, 60], [26, 82], [12, 82]],
    ]),
  })

  add('earcut-holes-same-x-diff-y', '4 holes sharing leftmost x=20 with distinct y (compareXYSlope y tie-breaker)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 120], [0, 120]],
      [[20, 85], [50, 85], [50, 105], [20, 105]],
      [[20, 10], [50, 10], [50, 25], [20, 25]],
      [[20, 55], [50, 55], [50, 75], [20, 75]],
      [[20, 32], [50, 32], [50, 48], [20, 48]],
    ]),
  })

  add('earcut-holes-shared-leftmost-slope', '3 holes meeting at exact same leftmost vertex (30,50) with different slopes (compareXYSlope slope tie-breaker & sectorContainsSector)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      // Upper diamond sharing (30,50)
      [[30, 50], [60, 85], [80, 75]],
      // Lower diamond sharing (30,50)
      [[30, 50], [80, 25], [60, 15]],
      // Middle wedge sharing (30,50)
      [[30, 50], [80, 55], [80, 45]],
    ]),
  })

  add('earcut-holes-vertical-edge-slope', '2 holes meeting at (30,50) where one outgoing edge is vertical (Infinity slope in compareXYSlope)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[30, 50], [70, 40], [70, 50]],
      [[30, 50], [30, 80], [70, 80]],
    ]),
  })

  add('earcut-hole-touches-outer-vertex', 'hole sharing a vertex with the outer ring (equals(hole, p) / equals(hole, p.next))', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[0, 0], [40, 20], [20, 40]],
      [[0, 100], [30, 80], [50, 90]],
    ]),
  })

  add('earcut-hole-touches-outer-edge', 'hole whose leftmost vertex lies directly on an outer edge (x === hx early return in findHoleBridge)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[0, 50], [40, 30], [40, 70]],
    ]),
  })

  add('earcut-hole-t-junction-horizontal', 'hole leftmost vertex sitting on a reflex horizontal edge (T-junction touch clause p.y===hy && p.next.y===hy && p.next.x>hx)', {
    op: 'earcut',
    ...packRings([
      // Reflex corner at p=(10,50) with p.prev=(10,30) and p.next=(60,50) so locallyInside(p, hole)
      // is false while p.y===hy && p.next.y===hy && p.next.x>hx is true!
      [[0, 0], [100, 0], [100, 100], [0, 100], [0, 20], [10, 30], [10, 50], [60, 50], [60, 60], [10, 60], [0, 80]],
      [[30, 50], [50, 30], [70, 40]],
    ]),
  })

  add('earcut-hole-sector-contains', 'two holes meeting at (30,50) with nested sectors + third hole at y=50 (exercises both halves of sectorContainsSector returning true)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [120, 0], [120, 120], [10, 120], [10, 100], [0, 20]],
      [[20, 65], [35, 65], [30, 50]],
      [[26, 35], [30, 50], [35, 35]],
      [[45, 50], [55, 45], [55, 55]],
    ]),
  })

  add('earcut-hole-reflex-bridge-angle', 'concave outer ring with multiple reflex vertices inside Eberly bridge triangle (tanMin, p.x > m.x, sectorContainsSector)', {
    op: 'earcut',
    ...packRings([
      // Left wall has protruding spikes inside the bridge triangle toward hole at (70,50)
      [[0, 0], [120, 0], [120, 100], [0, 100], [0, 80], [40, 65], [15, 55], [40, 50], [15, 45], [35, 35], [0, 20]],
      [[70, 50], [95, 35], [95, 65]],
    ]),
  })

  add('earcut-hole-steiner-single', 'single-vertex hole treated as a Steiner point (steiners set preserves it through filterPoints)', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[50, 50]],
    ]),
  })

  add('earcut-hole-steiner-multiple', 'multiple Steiner points combined with normal polygonal holes', {
    op: 'earcut',
    ...packRings([
      [[0, 0], [120, 0], [120, 120], [0, 120]],
      [[30, 60]],
      [[60, 30], [90, 30], [90, 55], [60, 55]],
      [[60, 90]],
      [[95, 85]],
    ]),
  })

  add('earcut-hole-collinear-growblock', 'multi-block outer ring & 31-vertex hole with collinear bridge removals growing all 4 blockBBox bounds (growBlock, liveBlockHead, liveBlockStop)', {
    op: 'earcut',
    ...(() => {
      const rFixed = [
        [8, 55], [9, 54], [8.5, 53], [9.5, 52], [9, 51], [10, 50], [9.5, 49], [10.5, 48],
        [10, 47], [11, 46], [10.5, 45], [11.5, 44], [12, 43.5], [13, 43.25],
        [15, 42.5], // vertex 14 (z = block 0)
        [20, 40],   // vertex 15 (bridge! collinear with 14 and hole (40,30) -> grows maxX & minY)
        [22, 25], [0, 0], [100, 0], [100, 150], [-10, 150], [-10, 60], [8, 60],
      ]
      const hole31 = [[40, 30], [60, 34]]
      for (let i = 0; i < 28; i++) hole31.push([75, q(34 - i * 0.5)])
      hole31.push([60, 20]) // 31 verts: last vert collinear with (40,30) & (20,40) -> grows minX & maxY
      // Second hole exercises liveBlockHead / liveBlockStop skipping the removed boundary nodes!
      const hole2 = [[45, 90], [65, 90], [65, 110], [45, 110]]
      return packRings([rFixed, hole31, hole2])
    })(),
  })

  add('earcut-many-holes-block-index', '16-hole island (>180 verts, multi-block K=16 bbox index + z-order)', {
    op: 'earcut',
    ...(() => {
      const m = makeIslandWithLakes(808, 120, 5, 7, false)
      return { data: m.data, holeIndices: m.holeIndices }
    })(),
  })

  add('earcut-hole-outside-left', 'degenerate hole completely left of outer ring (findHoleBridge returns null)', {
    op: 'earcut',
    ...packRings([
      [[50, 0], [100, 0], [100, 100], [50, 100]],
      [[0, 40], [20, 40], [20, 60], [0, 60]],
    ]),
  })

  // === 6. Recovery passes: cureLocalIntersections & splitEarcut ===
  add('earcut-cure-local-hourglass', 'self-intersecting hourglass triggering cureLocalIntersections (cured=true, triangles.push, removeNode x2, filterPoints)', {
    op: 'earcut',
    data: [0, 0, 10, 0, 10, 10, 0, 10, 5, 5, 10, 10, 5, 0, 0, 10, 5, 5],
  })

  add('earcut-cure-local-intersection', 'self-intersecting internal bowtie', {
    op: 'earcut',
    data: [0, 0, 100, 0, 100, 100, 0, 100, 40, 60, 60, 40, 60, 60, 40, 40],
  })

  add('earcut-cure-local-ribbon', 'self-crossing 6-vertex folded ribbon triggering cureLocalIntersections', {
    op: 'earcut',
    data: [0, 0, 10, 10, 20, 0, 10, 20, 0, 10, 20, 10],
  })

  add('earcut-self-intersecting-pentagram', '5-pointed self-intersecting pentagram triggering cureLocalIntersections', {
    op: 'earcut',
    data: [
      0, 100,
      58.78, -80.9,
      -95.11, 30.9,
      95.11, 30.9,
      -58.78, -80.9,
    ],
  })

  add('earcut-self-intersecting-local-loop', 'multi-crossing star polygon hitting cureLocalIntersections and intersects(true)', {
    op: 'earcut',
    data: Array.from({ length: 12 }, (_, i) => {
      const a = (i * Math.PI * 2 * 5) / 12
      return [Math.round(Math.cos(a) * 100), Math.round(Math.sin(a) * 100)]
    }).flat(),
  })

  // Overlapping opposite-wound figure / twisted polygon that exhausts filterPoints and
  // cureLocalIntersections and falls through to splitEarcut (isValidDiagonal, intersectsPolygon,
  // middleInside, splitPolygon, recursive earcutLinked).
  add('earcut-split-earcut-twisted', 'twisted double-loop polygon forcing splitEarcut & intersectsPolygon/onSegment', {
    op: 'earcut',
    data: [
      0, 0, 40, 0, 40, 40, 0, 40,
      20, 20, 30, 10, 10, 10, 20, 20, 10, 30, 30, 30, 20, 20, 0, 20,
    ],
  })

  // Self-intersecting double-wound polygon with zero-length pinch vertex forcing splitEarcut
  // with zeroLength diagonal check and intersectsPolygon bbox checks.
  add('earcut-split-earcut-pinched', 'pinched overlapping corridor forcing splitEarcut with zeroLength & intersectsPolygon', {
    op: 'earcut',
    data: [
      0, 0, 100, 0, 100, 100, 0, 100, 0, 50,
      50, 50, 80, 20, 20, 20, 80, 80, 20, 80, 50, 50,
      0, 50,
    ],
  })

  // Larger twisted polygon (>80 vertices) reaching splitEarcut with invSize != 0.
  {
    const ring = []
    // Build a 90-vertex ring with a pinched self-touching slit that stalls into splitEarcut
    for (let i = 0; i < 20; i++) ring.push([i * 5, 0])
    for (let i = 0; i < 20; i++) ring.push([100, i * 5])
    for (let i = 0; i < 20; i++) ring.push([100 - i * 5, 100])
    for (let i = 0; i < 10; i++) ring.push([0, 100 - i * 5])
    // Slit into center and loop back wrong-way (creates a stall that requires splitEarcut)
    ring.push([50, 50], [70, 30], [70, 70], [30, 70], [30, 30], [70, 30], [50, 50])
    for (let i = 10; i < 20; i++) ring.push([0, 100 - i * 5])
    add('earcut-split-earcut-hashed', '>80-vertex polygon reaching splitEarcut with z-order hashing enabled', {
      op: 'earcut',
      data: packRings([ring]).data,
    })
  }

  // Self-touching twisted polygon with two reflex vertices at the exact same (x,y) plus
  // collinear edges on candidate diagonals (exercises zeroLength in isValidDiagonal & onSegment in intersects).
  add('earcut-split-earcut-zerolen-collinear', 'degenerate self-touching polygon exercising zeroLength diagonal and onSegment boundary intersects', {
    op: 'earcut',
    data: [
      0, 0, 100, 0, 100, 100, 0, 100,
      0, 50, 50, 50, 20, 80, 80, 80, 50, 50, 80, 20, 20, 20, 50, 50, 0, 50,
    ],
  })

  // === 7. deviation() ===
  add('deviation-exact-zero', 'deviation on exact square triangulation returns 0', {
    op: 'deviation',
    data: [0, 0, 10, 0, 10, 10, 0, 10],
    holeIndices: null,
    dim: 2,
    triangles: [2, 3, 0, 0, 1, 2],
  })

  add('deviation-with-holes', 'deviation on polygon with 2 holes (exercises holeIndices[i+1] loop branch)', {
    op: 'deviation',
    data: [
      0, 0, 100, 0, 100, 100, 0, 100,
      10, 10, 40, 10, 40, 40, 10, 40,
      60, 60, 90, 60, 90, 90, 60, 90,
    ],
    holeIndices: [4, 8],
    dim: 2,
    triangles: [
      0, 4, 7, 5, 4, 0, 5, 0, 1, 8, 11, 5, 9, 8, 5, 9, 5, 1,
      9, 1, 2, 3, 0, 7, 3, 7, 6, 11, 6, 5, 11, 3, 6, 10, 9, 2,
      10, 2, 3, 10, 3, 11,
    ],
  })

  add('deviation-dim-3', 'deviation with dim = 3 and non-trivial area ratio', {
    op: 'deviation',
    data: [0, 0, 99, 10, 0, 88, 10, 7, 77, 0, 7, 66],
    holeIndices: null,
    dim: 3,
    triangles: [1, 2, 0],
  })

  add('deviation-missing-triangle', 'deviation when triangulation omits one triangle (exact non-zero f64)', {
    op: 'deviation',
    data: [0, 0, 13, 0, 17, 11, 2, 19],
    holeIndices: null,
    dim: 2,
    triangles: [0, 1, 2],
  })

  add('deviation-extra-overlap', 'deviation when triangles overlap beyond polygon area (> 0)', {
    op: 'deviation',
    data: [0, 0, 10, 0, 10, 10, 0, 10],
    holeIndices: null,
    dim: 2,
    triangles: [0, 1, 2, 0, 2, 3, 0, 1, 3],
  })

  add('deviation-empty-degenerate', 'deviation with empty triangles on zero-area collinear polygon -> 0', {
    op: 'deviation',
    data: [0, 0, 5, 0, 10, 0],
    holeIndices: null,
    dim: 2,
    triangles: [],
  })

  add('deviation-empty-nonzero', 'deviation with empty triangles on non-zero area polygon -> 1', {
    op: 'deviation',
    data: [0, 0, 10, 0, 10, 10, 0, 10],
    holeIndices: null,
    dim: 2,
    triangles: [],
  })

  add('deviation-roundoff-zero-area', 'deviation with empty triangles on near-collinear large-coord triangle within shoelace EPSILON -> 0', {
    op: 'deviation',
    data: [1e6, 1e6, 1e6 + 1, 1e6 + 1, 1e6 + 2, 1e6 + 2 + 1e-6],
    holeIndices: null,
    dim: 2,
    triangles: [],
  })

  add('deviation-zero-polygon-nonzero-tris', 'deviation with zero-area polygon and positive triangle area -> Infinity', {
    op: 'deviation',
    data: [0, 0, 10, 0, 0, 0, 0, 10],
    holeIndices: [3],
    dim: 2,
    triangles: [0, 1, 3],
  })

  // === 8. flatten() ===
  add('flatten-simple-2d', 'flatten single 2D outer ring', {
    op: 'flatten',
    rings: [[[0, 0], [10, 0], [10, 10], [0, 10]]],
  })

  add('flatten-multi-hole-2d', 'flatten outer ring + 3 holes', {
    op: 'flatten',
    rings: [
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[10, 10], [20, 10], [20, 20], [10, 20]],
      [[40, 40], [50, 40], [50, 50]],
      [[70, 70], [85, 70], [85, 85], [75, 90], [70, 85]],
    ],
  })

  add('flatten-3d', 'flatten 3D coordinates with 2 holes', {
    op: 'flatten',
    rings: [
      [[0, 0, 1], [10, 0, 2], [10, 10, 3], [0, 10, 4]],
      [[2, 2, 5], [4, 2, 6], [4, 4, 7]],
      [[6, 6, 8], [8, 6, 9], [8, 8, 10]],
    ],
  })

  add('flatten-4d', 'flatten 4D coordinates', {
    op: 'flatten',
    rings: [
      [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
    ],
  })

  add('flatten-with-empty-ring', 'flatten with empty middle hole ring exercising prevLen guard', {
    op: 'flatten',
    rings: [
      [[0, 0], [10, 0], [10, 10], [0, 10]],
      [],
      [[2, 2], [4, 2], [4, 4]],
    ],
  })

  add('flatten-mixed-dimensions', 'first point fixes the stride across short, long, and empty points', {
    op: 'flatten',
    rings: [[[1, 2, 3], [4, 5]], [[6, 7, 8, 9], []]],
  })

  add('flatten-empty-throws', 'flatten([]) throws TypeError (both-threw)', {
    op: 'flatten',
    rings: [],
  })

  // === 9. refine() and earcutRefine ===
  add('refine-empty', 'refine on empty triangles (n < 6 early return)', {
    op: 'refine',
    triangles: [],
    coords: [0, 0, 10, 0, 10, 10],
  })

  add('refine-single-triangle', 'refine on single triangle (n = 3 < 6 early return)', {
    op: 'refine',
    triangles: [0, 1, 2],
    coords: [0, 0, 10, 0, 5, 10],
  })

  add('refine-sliver-quad', 'refine flips skinny diagonal of a convex quad to Delaunay (without mutating input.triangles)', {
    op: 'refine',
    // Skinny diamond (0,0), (10,1), (20,0), (10,-1) split along long diagonal (0,2) -> flips to (1,3)!
    coords: [0, 0, 10, -1, 20, 0, 10, 1],
    triangles: [0, 2, 3, 0, 1, 2],
  })

  add('refine-already-delaunay', 'refine leaves already-Delaunay quad untouched', {
    op: 'refine',
    coords: [0, 0, 10, -1, 20, 0, 10, 1],
    triangles: [0, 1, 3, 1, 2, 3],
  })

  add('refine-orient1-guard', 'refine refuses flip when !inCircle is true but first orient(p0,pr,p1) <= 0', {
    op: 'refine',
    // p0=(0,0), pr=(10,0), pl=(5,5): circumcircle center (5,0) r=5; p1=(5,-2) is inside (!inCircle true)
    // but orient(p0, pr, p1) = -20 <= 0 blocks the flip!
    coords: [0, 0, 10, 0, 5, 5, 5, -2],
    triangles: [1, 2, 0, 2, 1, 3],
  })

  add('refine-orient2-guard', 'refine refuses flip when !inCircle and first orient pass but second orient(p0,p1,pl) <= 0', {
    op: 'refine',
    // p0=(0,0), pr=(10,0), pl=(5,5): p1=(2,3.5) is inside circumcircle (!inCircle true) and
    // orient(p0,pr,p1) = +35 > 0, but orient(p0,p1,pl) = -7.5 <= 0 blocks the flip!
    coords: [0, 0, 10, 0, 5, 5, 2, 3.5],
    triangles: [1, 2, 0, 2, 1, 3],
  })

  add('refine-cocircular-square', 'refine treats exact cocircular square as legal tie (inCircle <= margin -> no flip)', {
    op: 'refine',
    coords: [0, 0, 10, 0, 10, 10, 0, 10],
    triangles: [0, 1, 2, 0, 2, 3],
  })

  add('refine-near-cocircular-eps', 'refine treats 1e-14 perturbed cocircular quad as legal tie (issue #205 1e-13*s*s margin)', {
    op: 'refine',
    coords: [0, 0, 10, 0, 10 + 1e-14, 10 + 1e-14, 0, 10],
    triangles: [0, 1, 2, 0, 2, 3],
  })

  {
    // Fan triangulation of a convex 20-gon on an elongated ellipse -> multi-hop cascading Lawson flips
    const n = 20
    const coords = []
    for (let i = 0; i < n; i++) {
      const a = (i * Math.PI * 2) / n
      coords.push(q(Math.cos(a) * 120), q(Math.sin(a) * 25))
    }
    const triangles = []
    for (let i = 1; i < n - 1; i++) {
      triangles.push(0, i, i + 1)
    }
    add('refine-cascade-ellipse', 'fan triangulation of elongated 20-gon triggering multi-hop cascading Lawson flips & edgeStamp dedup', {
      op: 'refine',
      coords,
      triangles,
    })
  }

  add('refine-boundary-preserved', 'earcutRefine on square with hole preserves constrained hole boundary edges', {
    op: 'earcutRefine',
    ...packRings([
      [[0, 0], [100, 0], [100, 100], [0, 100]],
      [[45, 10], [55, 10], [55, 90], [45, 90]],
    ]),
  })

  add('refine-dim-3', 'refine with dim = 3 on 3D coordinates', {
    op: 'refine',
    coords: [0, 0, 1.5, 10, -1, 2.5, 20, 0, 3.5, 10, 1, 4.5],
    triangles: [0, 2, 3, 0, 1, 2],
    dim: 3,
  })

  add('refine-non-manifold', 'refine on non-manifold mesh (3 triangles sharing one edge -> tests hTable consumed -1 slot skipping)', {
    op: 'refine',
    coords: [0, 0, 10, -1, 20, 0, 10, 1, 10, 5],
    triangles: [0, 2, 3, 0, 1, 2, 0, 2, 4],
  })

  add('earcut-refine-multi-hole', 'earcutRefine end-to-end on complex 120-vertex multi-hole island', {
    op: 'earcutRefine',
    ...(() => {
      const m = makeIslandWithLakes(919, 100, 3, 8, true)
      return { data: m.data, holeIndices: m.holeIndices }
    })(),
  })

  add('earcut-refine-city-block', 'earcutRefine on bayed city block with 12 courtyards (large hash table & heavy Lawson flips)', {
    op: 'earcutRefine',
    ...(() => {
      const m = makeCityBlock(929, 6, 4, 3, true)
      return { data: m.data, holeIndices: m.holeIndices }
    })(),
  })

  // === 10. batch & error handling ===
  add('batch-mixed', 'batch mixing dim=2, dim=3, holes, and refine=true/false', {
    op: 'batch',
    items: [
      { data: [10, 0, 0, 50, 60, 60, 70, 10] },
      { data: [0, 0, 100, 0, 100, 100, 0, 100, 20, 20, 80, 20, 80, 80, 20, 80], holeIndices: [4], refine: true },
      { data: [0, 0, 1, 10, -1, 2, 20, 0, 3, 10, 1, 4], dim: 3, refine: true },
      { data: [] },
    ],
  })

  add('error-unknown-op', 'unknown op throws TypeError (both-threw)', {
    op: 'notAnEarcutOp',
  })

  // === 11. Bench cases ===
  add('bench-earcut', 'batch of 32 realistic multi-hole islands, city blocks, spirals, gears, and medium polygons (~2-3 ms in JS)', buildBenchEarcut())
  add('bench-refine', 'batch of 24 realistic multi-hole polygons with earcut + Lawson Delaunay refinement (~2-3 ms in JS)', buildBenchRefine())

  return cases
}

// ---------------------------------------------------------------------------------------------
// Write & verify.

const corpus = {
  schema: 1,
  library: 'earcut',
  cases: buildCases(),
}

const outPath = resolve(here, 'corpus.json')
writeFileSync(outPath, `${JSON.stringify(corpus, null, 2)}\n`)
console.log(`wrote ${outPath} (${corpus.cases.length} cases)`)

if (process.argv.includes('--verify')) {
  const { run } = await import('./reference.mjs')
  const { variant } = await import('./bench-variant.mjs')
  const earcutMod = await import('../../node_modules/earcut/src/earcut.js')
  const { deviation } = earcutMod
  const { canonicalText } = await import('../../tools/lib/canonical.mjs')

  const assert = (cond, msg) => {
    if (!cond) throw new Error(`VERIFY FAILED: ${msg}`)
  }

  const byId = new Map()
  for (const c of corpus.cases) {
    assert(!byId.has(c.id), `duplicate id ${c.id}`)
    const clone = structuredClone(c.input)
    let out
    let threw = false
    try {
      out = run(clone)
    } catch (e) {
      threw = true
      out = e
    }
    // Check no input mutation
    assert(JSON.stringify(clone) === JSON.stringify(c.input), `${c.id}: input was mutated!`)
    byId.set(c.id, { c, out, threw })
  }

  // Verify expected throw cases
  assert(byId.get('flatten-empty-throws').threw, 'flatten-empty-throws should throw')
  assert(byId.get('error-unknown-op').threw, 'error-unknown-op should throw')
  for (const [id, rec] of byId) {
    if (id === 'flatten-empty-throws' || id === 'error-unknown-op') continue
    assert(!rec.threw, `${id}: unexpected throw: ${rec.out}`)
  }

  // Verify specific semantic invariants & mutant-killers
  const mixed = byId.get('flatten-mixed-dimensions').out
  const expectedMixed = { vertices: [1, 2, 3, 4, 5, undefined, 6, 7, 8, undefined, undefined, undefined], holes: [2], dimensions: 3 }
  assert(canonicalText(mixed, 'json') === canonicalText(expectedMixed, 'json'), 'flatten mixed dimensions')
  const wrappedTriangles = [71, 72, 73, 73, 77, 80, 78, 77, 76, 78, 76, 71, 80, 79, 78]
  assert(canonicalText(byId.get('earcut-zorder-negative-wrap').out, 'json') === canonicalText(wrappedTriangles, 'json'),
    'negative Morton wrap must preserve ear order')
  assert(byId.get('earcut-empty').out.length === 0, 'earcut-empty')
  assert(byId.get('earcut-1-vertex').out.length === 0, 'earcut-1-vertex')
  assert(byId.get('earcut-2-vertices').out.length === 0, 'earcut-2-vertices')
  assert(byId.get('earcut-closed-2-vertices').out.length === 0, 'earcut-closed-2-vertices')
  assert(byId.get('earcut-collinear-zero-area').out.length === 0, 'earcut-collinear-zero-area')
  assert(byId.get('earcut-zero-bbox-degenerate').out.length === 0, 'earcut-zero-bbox-degenerate')
  assert(byId.get('earcut-large-zero-bbox').out.length === 0, 'earcut-large-zero-bbox')

  // Check deviation === 0 (or < 1e-10) on valid non-self-intersecting polygons
  const validPolygonIds = [
    'earcut-triangle-ccw', 'earcut-triangle-cw', 'earcut-convex-quad', 'earcut-closed-ring',
    'earcut-duplicate-consecutive', 'earcut-collinear-edges', 'earcut-concave-lshape',
    'earcut-star-10', 'earcut-comb', 'earcut-spiral-small', 'earcut-dim-3', 'earcut-dim-4-large',
    'earcut-threshold-80-unhashed', 'earcut-threshold-81-hashed', 'earcut-large-hilbert',
    'earcut-large-spiral-150', 'earcut-zorder-insertion-sort', 'earcut-large-coords',
    'earcut-tiny-scale', 'earcut-single-hole', 'earcut-empty-hole-indices',
    'earcut-null-hole-indices', 'earcut-hole-windings', 'earcut-multiple-holes-unsorted',
    'earcut-holes-same-x-diff-y', 'earcut-holes-shared-leftmost-slope',
    'earcut-holes-vertical-edge-slope', 'earcut-hole-touches-outer-vertex',
    'earcut-hole-touches-outer-edge', 'earcut-hole-t-junction-horizontal',
    'earcut-hole-reflex-bridge-angle', 'earcut-hole-steiner-single',
    'earcut-hole-steiner-multiple', 'earcut-hole-collinear-growblock',
    'earcut-many-holes-block-index', 'earcut-refine-multi-hole', 'earcut-refine-city-block',
  ]
  for (const id of validPolygonIds) {
    const { c, out } = byId.get(id)
    assert(out.length > 0 && out.length % 3 === 0, `${id}: expected non-empty triangles, got ${out.length}`)
    const d = deviation(c.input.data, c.input.holeIndices ?? null, c.input.dim ?? 2, out)
    assert(d < 1e-10, `${id}: deviation too high: ${d}`)
  }

  // Check Steiner point actually appears in output triangles
  const steinerOut = byId.get('earcut-hole-steiner-single').out
  assert(steinerOut.includes(4), `earcut-hole-steiner-single: Steiner vertex 4 missing from ${steinerOut}`)
  const steinerMultiOut = byId.get('earcut-hole-steiner-multiple').out
  assert(steinerMultiOut.includes(4) && steinerMultiOut.includes(9) && steinerMultiOut.includes(10),
    `earcut-hole-steiner-multiple: Steiner vertices missing`)

  // Check refine mutant-killers
  const sliverIn = byId.get('refine-sliver-quad').c.input.triangles
  const sliverOut = byId.get('refine-sliver-quad').out
  assert(JSON.stringify(sliverOut) !== JSON.stringify(sliverIn), 'refine-sliver-quad must flip')
  assert(JSON.stringify(byId.get('refine-already-delaunay').out) === JSON.stringify(byId.get('refine-already-delaunay').c.input.triangles),
    'refine-already-delaunay must not flip')
  assert(JSON.stringify(byId.get('refine-orient1-guard').out) === JSON.stringify(byId.get('refine-orient1-guard').c.input.triangles),
    'refine-orient1-guard must not flip')
  assert(JSON.stringify(byId.get('refine-orient2-guard').out) === JSON.stringify(byId.get('refine-orient2-guard').c.input.triangles),
    'refine-orient2-guard must not flip')
  assert(JSON.stringify(byId.get('refine-cocircular-square').out) === JSON.stringify(byId.get('refine-cocircular-square').c.input.triangles),
    'refine-cocircular-square must not flip')
  assert(JSON.stringify(byId.get('refine-near-cocircular-eps').out) === JSON.stringify(byId.get('refine-near-cocircular-eps').c.input.triangles),
    'refine-near-cocircular-eps must not flip')
  const cascadeIn = byId.get('refine-cascade-ellipse').c.input.triangles
  const cascadeOut = byId.get('refine-cascade-ellipse').out
  assert(JSON.stringify(cascadeOut) !== JSON.stringify(cascadeIn), 'refine-cascade-ellipse must flip')

  // Verify both orient guards ACTUALLY have !inCircle true (so orient guards are genuinely tested!)
  for (const id of ['refine-orient1-guard', 'refine-orient2-guard']) {
    const coords = byId.get(id).c.input.coords
    const p0 = 0, pr = 1, pl = 2, p1 = 3
    const ax = coords[p0 * 2], ay = coords[p0 * 2 + 1]
    const bx = coords[pr * 2], by = coords[pr * 2 + 1]
    const cx = coords[pl * 2], cy = coords[pl * 2 + 1]
    const px = coords[p1 * 2], py = coords[p1 * 2 + 1]
    const dx = ax - px, dy = ay - py, ex = bx - px, ey = by - py, fx = cx - px, fy = cy - py
    const ap = dx * dx + dy * dy, bp = ex * ex + ey * ey, cp = fx * fx + fy * fy
    const s = ap + bp + cp
    const det = dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx) + ap * (ex * fy - ey * fx)
    assert(det > 1e-13 * s * s, `${id}: expected !inCircle (det=${det} > ${1e-13 * s * s}) so orient guard is actually tested!`)
  }

  // Verify deviation cases
  assert(byId.get('deviation-exact-zero').out === 0, 'deviation-exact-zero')
  assert(byId.get('deviation-with-holes').out === 0, 'deviation-with-holes')
  assert(byId.get('deviation-missing-triangle').out > 0 && byId.get('deviation-missing-triangle').out < 1, 'deviation-missing-triangle')
  assert(byId.get('deviation-extra-overlap').out === 0.5, `deviation-extra-overlap: got ${byId.get('deviation-extra-overlap').out}`)
  assert(byId.get('deviation-empty-degenerate').out === 0, 'deviation-empty-degenerate')
  assert(byId.get('deviation-empty-nonzero').out === 1, 'deviation-empty-nonzero')
  assert(byId.get('deviation-roundoff-zero-area').out === 0, 'deviation-roundoff-zero-area')
  assert(byId.get('deviation-zero-polygon-nonzero-tris').out === Infinity, 'deviation-zero-polygon-nonzero-tris')

  // Verify bench cases: zero deviation, no duplicate consecutive/closing vertices, bench-variant 51/51 distinct
  for (const benchId of ['bench-earcut', 'bench-refine']) {
    const { c, out } = byId.get(benchId)
    let totalVerts = 0
    let totalHoles = 0
    let totalTris = 0
    const baseTriCounts = []
    for (let i = 0; i < c.input.items.length; i++) {
      const item = c.input.items[i]
      const dim = item.dim ?? 2
      totalVerts += item.data.length / dim
      totalHoles += item.holeIndices ? item.holeIndices.length : 0
      totalTris += out[i].length / 3
      baseTriCounts.push(out[i].length / 3)
      const d = deviation(item.data, item.holeIndices ?? null, dim, out[i])
      assert(d < 1e-10, `${benchId}[${i}]: deviation=${d}`)
    }

    // Check bench-variant across k = 1..50
    const inputHashes = new Set([createHash('sha256').update(JSON.stringify(c.input)).digest('hex')])
    const outputHashes = new Set([createHash('sha256').update(canonicalText(out, 'json')).digest('hex')])
    for (let k = 1; k <= 50; k++) {
      const v = variant(c.input, k)
      inputHashes.add(createHash('sha256').update(JSON.stringify(v)).digest('hex'))
      const vOut = run(v)
      outputHashes.add(createHash('sha256').update(canonicalText(vOut, 'json')).digest('hex'))
      assert(vOut.length === out.length, `${benchId} k=${k}: batch length mismatch`)
      for (let i = 0; i < v.items.length; i++) {
        assert(vOut[i].length / 3 === baseTriCounts[i], `${benchId} k=${k} item ${i}: tri count ${vOut[i].length / 3} != ${baseTriCounts[i]}`)
        const d = deviation(v.items[i].data, v.items[i].holeIndices ?? null, v.items[i].dim ?? 2, vOut[i])
        assert(d < 1e-10, `${benchId} k=${k} item ${i}: deviation=${d}`)
      }
    }
    assert(inputHashes.size === 51, `${benchId}: expected 51 distinct variant inputs, got ${inputHashes.size}`)
    assert(outputHashes.size === 51, `${benchId}: expected 51 distinct variant outputs, got ${outputHashes.size}`)

    // Quick timing
    for (let w = 0; w < 5; w++) run(c.input)
    const t0 = performance.now()
    const reps = 20
    for (let r = 0; r < reps; r++) run(c.input)
    const ms = (performance.now() - t0) / reps
    console.log(`${benchId}: ${c.input.items.length} polys, ${totalVerts} verts, ${totalHoles} holes, ${totalTris} tris, ~${ms.toFixed(2)} ms/call (51/51 variants ok)`)
  }
  console.log('ALL VERIFY CHECKS PASSED!')
}
