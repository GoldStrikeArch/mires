// JavaScript reference for the Marked-shaped workload: the published `marked` library rendering
// the same Markdown string the Rust build renders. The import is a relative path to the package's
// published ESM entry (its `exports` "." target), so Node and browsers (tools/run.mjs serves
// repository files at their repository-relative URLs) load these exact bytes the same way.
//
// One configuration differs from marked's defaults, and it is the shared behaviour, not a speedup:
// the Rust port has no inline/block HTML, so it escapes `<`, `>` and `&` in raw HTML text. marked
// passes raw HTML through. The `html` renderer below escapes those three characters the way the
// Rust port does (src/marked.rs `escape_slice`); marked still lexes and tokenizes the HTML.
import { Marked } from '../../../node_modules/marked/lib/marked.esm.js'

const escapeHtml = (text) => text.replace(/[&<>]/g, (character) =>
  character === '&' ? '&amp;' : character === '<' ? '&lt;' : '&gt;')

const marked = new Marked({
  async: false,
  gfm: true,
  breaks: false,
  renderer: {
    html({ text }) {
      return escapeHtml(text)
    },
  },
})

export function run({ markdown }) {
  return marked.parse(markdown)
}
