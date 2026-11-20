// Optional closure for applications composed from other ports. Ordinary ports retain their
// original source inventories, language builders, check receipts and benchmark receipts.
import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  fileRecord,
  freshness,
  harnessRoot,
  readJson,
  sourceInventory,
  stableJson,
  walkFiles
} from './registry.mjs'

export const compositeBuilder = resolve(harnessRoot, 'languages/composite/build.mjs')

export function hasComponents(port) {
  const manifest = resolve(port.dir, 'port.json')
  return existsSync(manifest) && Object.hasOwn(readJson(manifest), 'components')
}

export function componentsOf(registry, port) {
  const manifestPath = resolve(port.dir, 'port.json')
  if (!existsSync(manifestPath)) return []
  const names = readJson(manifestPath).components
  if (names === undefined) return []
  if (
    !Array.isArray(names) ||
    !names.length ||
    names.includes(port.library) ||
    new Set(names).size !== names.length
  ) {
    throw new Error(`${port.key}: expected distinct component library names`)
  }
  return names.map((library) => {
    const component = registry.ports.find(
      (p) => p.library === library && p.language === port.language && p.id === port.id
    )
    if (!component)
      throw new Error(`${port.key}: missing same-language component ${library}/${port.id} in registry`)
    if (readJson(resolve(component.dir, 'port.json')).components)
      throw new Error(`${port.key}: expected a standalone component: ${component.key}`)
    return component
  })
}

export const lockPath = (port) => resolve(port.dir, 'components.lock.json')

export function componentSources(registry, port, components) {
  const files = [
    ...walkFiles(resolve(harnessRoot, 'languages/composite')),
    resolve(harnessRoot, 'tools/lib/composite.mjs')
  ]
  const linker = Object.fromEntries(
    files.map((path) => {
      const record = fileRecord(path)
      return [record.path, record.sha256]
    })
  )
  const sources = Object.fromEntries(
    components.map((dep) => [dep.library, sourceInventory(registry, dep).digest])
  )
  return { schema: 1, language: port.language, linker, sources }
}

export function verifyComponentLock(port, actual) {
  const path = lockPath(port)
  if (!existsSync(path))
    throw new Error(`${port.key}: missing components.lock.json; refresh with tools/lock-components.mjs`)
  const expected = readJson(path)
  if (stableJson(expected) !== stableJson(actual)) {
    const changed = new Set()
    for (const key of ['sources', 'artifacts']) {
      for (const name of Object.keys(actual[key] ?? {})) {
        if (stableJson(expected[key]?.[name]) !== stableJson(actual[key][name])) changed.add(name)
      }
    }
    if (stableJson(expected.linker) !== stableJson(actual.linker)) changed.add('component linker')
    if (stableJson(expected.toolchain) !== stableJson(actual.toolchain)) changed.add('compiler toolchain')
    throw new Error(
      `${port.key}: components.lock.json mismatch (${[...changed].join(', ') || 'metadata'}); review the changes and run tools/lock-components.mjs`
    )
  }
}

export function compositeFreshness(registry, paths, port) {
  const state = freshness(registry, paths, port)
  if (!state.fresh) return state
  try {
    const components = componentsOf(registry, port)
    if (components.length) {
      const meta = readJson(resolve(port.dir, 'dist/meta.json'))
      verifyComponentLock(port, {
        ...componentSources(registry, port, components),
        toolchain: meta.toolchain,
        artifacts: meta.componentArtifacts
      })
    }
  } catch (error) {
    return { ...state, fresh: false, status: 'stale', reasons: [error.message] }
  }
  return state
}
