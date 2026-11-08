export function normalizeCapture(value) {
  const result = structuredClone(value)
  for (const component of Object.values(result?.builtins?.first?.debug || {})) {
    if (component && typeof component === 'object') delete component.duration
  }
  return visit(result)
}

function visit(value) {
  if (Array.isArray(value)) return value.map(visit)
  if (!value || typeof value !== 'object') return value
  if (value.$type === 'error' && typeof value.stack === 'string') value.stack = normalizeStack(value.stack)
  if ('duration' in value && ('value' in value || 'error' in value)) delete value.duration
  for (const key of Object.keys(value)) {
    if (key === 'durations') delete value[key]
    else value[key] = visit(value[key])
  }
  return value
}

export function normalizeStack(stack) {
  const normalized = stack
    .replace(/data:text\/javascript[^\s)]+/g, '<generated>')
    .replace(/(?:https?|file):\/\/[^\s)]+:\d+:\d+/g, '<generated>')
    .replace(/\/?[^\s():]+(?:\/[^\s():]+)+:\d+:\d+/g, '<generated>')
  // Chromium includes an error header; Firefox and Safari start at the first frame. Keep the
  // header when present and two user-origin frames. The remainder is implementation-generated.
  const lines = normalized.split('\n')
  return lines.slice(0, lines[0].includes('@') ? 2 : 3).join('\n')
}

export function collectDurations(captures) {
  const result = {}
  for (const capture of captures) findDurations(capture, '', result)
  return Object.fromEntries(Object.entries(result).map(([path, values]) => [path, summarize(values)]))
}

function findDurations(value, path, result) {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; ++index) findDurations(value[index], `${path}[${index}]`, result)
  } else if (value && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) {
      if (key === 'durations') {
        for (const [source, duration] of Object.entries(item)) {
          ;(result[`${path}.durations.${source}`] ||= []).push(duration)
        }
      } else {
        findDurations(item, path ? `${path}.${key}` : key, result)
      }
    }
  }
}

function summarize(values) {
  const sorted = values.slice().sort((left, right) => left - right)
  const percentile = (fraction) => sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)]
  return { count: sorted.length, p50: percentile(0.5), p95: percentile(0.95) }
}
