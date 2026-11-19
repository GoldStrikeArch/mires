// Reference adapter for the pilot library: the stand-in package's entry, imported by a relative
// path exactly as an npm library's adapter imports `../../../../node_modules/<package>/<entry>`.
// The adapter only maps the corpus input onto the package API; it adds no behaviour.
import { analyse } from './package/index.mjs'

export function run(input) {
  return analyse(input.text, { top: input.top })
}
