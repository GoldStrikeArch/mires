import { isDeepStrictEqual } from 'node:util'

import { collectDurations, normalizeCapture } from './normalize.mjs'

export function compareContracts(upstream, candidate) {
  const differences = []
  const count = Math.min(upstream.length, candidate.length)
  for (let index = 0; index < count; ++index) {
    diff(normalizeCapture(upstream[index]), normalizeCapture(candidate[index]), `$[${index}]`, differences)
  }
  if (upstream.length !== candidate.length) {
    differences.push({ path: '$.length', upstream: upstream.length, candidate: candidate.length })
  }
  return {
    differences,
    durationDistributions: {
      upstream: collectDurations(upstream),
      candidate: collectDurations(candidate),
    },
  }
}

function diff(left, right, path, output) {
  if (isDeepStrictEqual(left, right)) return
  if (Array.isArray(left) && Array.isArray(right)) {
    if (left.length !== right.length) output.push({ path: `${path}.length`, upstream: left.length, candidate: right.length })
    for (let index = 0; index < Math.min(left.length, right.length); ++index) diff(left[index], right[index], `${path}[${index}]`, output)
    return
  }
  if (left && right && typeof left === 'object' && typeof right === 'object') {
    const keys = [...new Set([...Object.keys(left), ...Object.keys(right)])]
    for (const key of keys) diff(left[key], right[key], `${path}.${key}`, output)
    return
  }
  output.push({ path, upstream: left, candidate: right })
}
