export function getColorGamut() {
  for (const gamut of ['rec2020', 'p3', 'srgb']) {
    if (matchMedia(`(color-gamut: ${gamut})`).matches) return gamut
  }
  return undefined
}

export function areColorsInverted() {
  if (matchMedia('(inverted-colors: inverted)').matches) return true
  if (matchMedia('(inverted-colors: none)').matches) return false
  return undefined
}

export function areColorsForced() {
  if (matchMedia('(forced-colors: active)').matches) return true
  if (matchMedia('(forced-colors: none)').matches) return false
  return undefined
}

export function getMonochromeDepth() {
  if (!matchMedia('(min-monochrome: 0)').matches) return undefined
  for (let i = 0; i <= 100; ++i) {
    if (matchMedia(`(max-monochrome: ${i})`).matches) return i
  }
  throw new Error('Too high value')
}

export function getContrastPreference() {
  if (matchMedia('(prefers-contrast: no-preference)').matches) return 0
  if (matchMedia('(prefers-contrast: high)').matches || matchMedia('(prefers-contrast: more)').matches) return 1
  if (matchMedia('(prefers-contrast: low)').matches || matchMedia('(prefers-contrast: less)').matches) return -1
  if (matchMedia('(prefers-contrast: forced)').matches) return 10
  return undefined
}

export function isMotionReduced() {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return true
  if (matchMedia('(prefers-reduced-motion: no-preference)').matches) return false
  return undefined
}

export function isTransparencyReduced() {
  if (matchMedia('(prefers-reduced-transparency: reduce)').matches) return true
  if (matchMedia('(prefers-reduced-transparency: no-preference)').matches) return false
  return undefined
}

export function isHDR() {
  if (matchMedia('(dynamic-range: high)').matches) return true
  if (matchMedia('(dynamic-range: standard)').matches) return false
  return undefined
}
