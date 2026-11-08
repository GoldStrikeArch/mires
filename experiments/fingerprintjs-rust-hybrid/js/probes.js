// Host probes that only read navigator.X / screen.X / matchMedia. No FP control flow.

export function navigatorGet(name) {
  return navigator[name]
}

export function screenGet(name) {
  return screen[name]
}

export function matchMediaMatches(query) {
  return matchMedia(query).matches
}
