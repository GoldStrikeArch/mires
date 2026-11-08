export function includes(haystack, needle) {
  for (let i = 0, l = haystack.length; i < l; ++i) {
    if (haystack[i] === needle) return true
  }
  return false
}

export function excludes(haystack, needle) {
  return !includes(haystack, needle)
}

export function toInt(value) {
  return parseInt(value)
}

export function toFloat(value) {
  return parseFloat(value)
}

export function replaceNaN(value, replacement) {
  return typeof value === 'number' && isNaN(value) ? replacement : value
}

export function countTruthy(values) {
  return values.reduce((sum, value) => sum + (value ? 1 : 0), 0)
}

export function round(value, base = 1) {
  if (Math.abs(base) >= 1) {
    return Math.round(value / base) * base
  }
  const counterBase = 1 / base
  return Math.round(value * counterBase) / counterBase
}

export function parseSimpleCssSelector(selector) {
  const errorMessage = `Unexpected syntax '${selector}'`
  const tagMatch = /^\s*([a-z-]*)(.*)$/i.exec(selector)
  const tag = tagMatch[1] || undefined
  const attributes = {}
  const partsRegex = /([.:#][\w-]+|\[.+?\])/gi

  const addAttribute = (name, value) => {
    attributes[name] = attributes[name] || []
    attributes[name].push(value)
  }

  for (;;) {
    const match = partsRegex.exec(tagMatch[2])
    if (!match) break
    const part = match[0]
    switch (part[0]) {
      case '.':
        addAttribute('class', part.slice(1))
        break
      case '#':
        addAttribute('id', part.slice(1))
        break
      case '[': {
        const attributeMatch = /^\[([\w-]+)([~|^$*]?=("(.*?)"|([\w-]+)))?(\s+[is])?\]$/.exec(part)
        if (attributeMatch) {
          addAttribute(attributeMatch[1], attributeMatch[4] ?? attributeMatch[5] ?? '')
        } else {
          throw new Error(errorMessage)
        }
        break
      }
      default:
        throw new Error(errorMessage)
    }
  }

  return [tag, attributes]
}
