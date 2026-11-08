// Canonical `key:value|key:value` assembly, including JSON.stringify of values.

import { murmurX64Hash128 } from './hash.js'

function escapeKey(key) {
  return String(key).replace(/[\\|:]/g, '\\$&')
}

export function componentsToCanonicalString(components) {
  const keys = Object.keys(components).sort()
  const parts = new Array(keys.length)
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i]
    const component = components[key]
    let value
    if ('error' in component) {
      value = 'error'
    } else {
      const serialized = JSON.stringify(component.value)
      value = serialized === undefined ? 'undefined' : serialized
    }
    parts[i] = escapeKey(key) + ':' + value
  }
  return parts.join('|')
}

export function hashComponents(components) {
  return murmurX64Hash128(componentsToCanonicalString(components), 0)
}
