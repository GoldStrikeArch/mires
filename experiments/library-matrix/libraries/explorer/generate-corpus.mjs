import { writeFileSync } from 'node:fs'

const cases = []
function chartFrame(ys, size=[64,48]) {
  const xs = Array.from(ys, (_, i) => i)
  return {
    size,
    xRange: [0, Math.max(1, xs.length - 1)],
    yRange: [-2, 12],
    data: [xs, ys],
    probes: [
      [0, 0, 0, 0],
      [xs.length - 1, 9, 64, 48],
      [1.25, 4, 22, 23]
    ]
  }
}
function add(id, records, query, ys = [0, 3, null, 9], spanGaps = false, ...nextFrames) {
  cases.push({ id, input: { records, query, chart: { spanGaps, frames: [chartFrame(ys), ...nextFrames] } } })
}
const doc = (id, title, body) => ({ id, title, body })
const records = [
  doc('c', 'Indexed storage', 'Persist a document in **IndexedDB**.'),
  doc('a', 'Rust compiler', '# Rust browser compiler\n\nFind **Rust** programs.'),
  doc('b', 'Browser search', 'A [browser](https://example.com/) can search offline.')
]
add('offline-library', records, 'compiler')
add('ranked-hits', records, 'browser')
add('no-hit', records, 'absent')
add('empty-store', [], 'rust')
add('overwrite', [...records, doc('a', 'Updated entry', '## Updated\n\n- local\n- copy')], 'updated')
add(
  'unicode',
  [doc('ø', 'Café été', '# Café\n\nnaïve 🌍'), doc('東京', '旅', '旅ものがたり')],
  'café',
  [1, null, 8, -1],
  true
)
add('empty-query', records, '')
add('empty-chart',records,'browser',[])
const empty = chartFrame([], [95.5,55.5])
add('clear-chart',records,'compiler',[0,3,null,9],false,empty)
add('restore-chart',records,'compiler',[],true,{...empty,probes:[]},chartFrame([1,7],[64.25,40.75]))
add(
  'markdown-blocks',
  [doc('x', 'Archive sample', '> Reader\n\n```js\nconst x = 42\n```\n\n~~old~~ **new**')],
  'archive'
)
const many = Array.from({ length: 96 }, (_, i) =>
  doc(
    'doc-' + i,
    'Offline note ' + (i % 7),
    '## Note ' +
      i +
      '\n\nStore **group ' +
      (i % 7) +
      '** records and ' +
      (i % 5 === 0 ? 'render an archive.' : 'index a document.')
  )
)
// WebDriver sends document keys in lexical order to the benchmark. The archive
// preserves insertion order, so the file-based checker must see those same bytes.
const timedRecords = many.map(({ body, id, title }) => ({ body, id, title }))
add(
  'bench-offline',
  timedRecords,
  'archive',
  Array.from({ length: 320 }, (_, i) => (i % 79 === 1 ? null : ((i * 17) % 13) - 1))
)
writeFileSync(
  new URL('corpus.json', import.meta.url),
  JSON.stringify({ schema: 1, library: 'explorer', cases }) + '\n'
)
