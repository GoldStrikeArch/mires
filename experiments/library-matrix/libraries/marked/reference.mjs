// Reference adapter for marked 18.1.1: the published ESM entry, imported by a relative path (no
// bare specifier, no import map), so Node and the browsers load the same file.
//
// `run(input)` executes one marked operation (`parse` or `parseInline`) with the options in
// `input.options` (`gfm`, `breaks`, `pedantic`; always `async: false`, `silent: false`) and
// returns the rendered HTML string directly with zero framing overhead.
import { Marked } from '../../../../node_modules/marked/lib/marked.esm.js'

function makeOptions(json) {
  const opts = { async: false, silent: false }
  if (json !== undefined) {
    if (json.gfm !== undefined) opts.gfm = json.gfm
    if (json.breaks !== undefined) opts.breaks = json.breaks
    if (json.pedantic !== undefined) opts.pedantic = json.pedantic
  }
  return opts
}

export function run(input) {
  const marked = new Marked(makeOptions(input.options))
  if (input.op === 'parse') return marked.parse(input.markdown)
  if (input.op === 'parseInline') return marked.parseInline(input.markdown)
  throw new TypeError(`unknown operation ${input.op}`)
}
