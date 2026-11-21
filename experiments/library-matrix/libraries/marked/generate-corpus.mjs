#!/usr/bin/env node
// Writes corpus.json and unicode-classes.json for the marked 18.1.1 library: every case is built
// deterministically (no clock, no Math.random: all pseudo-random data uses mulberry32 with fixed
// seeds), so corpus.json and unicode-classes.json are pure functions of this script.
// CONTRACT.md documents every case.
//
//   node libraries/marked/generate-corpus.mjs            # writes corpus.json & unicode-classes.json
//   node libraries/marked/generate-corpus.mjs --verify   # also runs reference.mjs on every case,
//                                                        # validates all invariants & mutant killers,
//                                                        # bench-variant, and prints bench timings.

import { createHash } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { performance } from 'node:perf_hooks'

const here = dirname(fileURLToPath(import.meta.url))

// ---------------------------------------------------------------------------------------------
// Deterministic PRNG and helpers.

function mulberry32(seed) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let value = state
    value = Math.imul(value ^ (value >>> 15), value | 1)
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296
  }
}

function extractRanges(re) {
  const ranges = []
  let start = -1
  let prev = -1
  let count = 0
  for (let cp = 0; cp <= 0x10ffff; cp++) {
    if (cp >= 0xd800 && cp <= 0xdfff) continue
    if (re.test(String.fromCodePoint(cp))) {
      count++
      if (start === -1) {
        start = cp
      } else if (cp !== prev + 1) {
        ranges.push([start, prev])
        start = cp
      }
      prev = cp
    }
  }
  if (start !== -1) ranges.push([start, prev])
  return { codePoints: count, ranges }
}

function buildUnicodeClasses() {
  return {
    description: 'Unicode code-point classes used by marked 18.1.1 inline delimiter and unescape rules, read off host RegExp as inclusive [first, last] ranges',
    unicode: process.versions.unicode || null,
    icu: process.versions.icu || null,
    node: process.version,
    classes: {
      punct: {
        regex: '/[\\p{P}\\p{S}]/u',
        ...extractRanges(/[\p{P}\p{S}]/u),
      },
      openQuote: {
        regex: '/[\\p{Pi}\\p{Ps}"\']/u',
        ...extractRanges(/[\p{Pi}\p{Ps}"']/u),
      },
      alnum: {
        regex: '/[\\p{L}\\p{N}]/u',
        ...extractRanges(/[\p{L}\p{N}]/u),
      },
      space: {
        regex: '/\\s/u',
        ...extractRanges(/\s/u),
      },
    },
  }
}

// ---------------------------------------------------------------------------------------------
// Bench document generators (~40-50 KB each, aiming for ~2-5 ms per call in JS).

const WORDS = [
  'compiler', 'parser', 'lexer', 'renderer', 'token', 'stream', 'document', 'section',
  'heading', 'paragraph', 'blockquote', 'table', 'alignment', 'checkbox', 'reference',
  'autolink', 'emphasis', 'strong', 'codespan', 'escape', 'entity', 'unicode', 'buffer',
  'pipeline', 'module', 'runtime', 'browser', 'engine', 'layout', 'syntax', 'grammar',
  'pattern', 'delimiter', 'flanking', 'nested', 'compact', 'ordered', 'unordered', 'loose',
  'tight', 'fenced', 'indented', 'metadata', 'schema', 'protocol', 'snapshot', 'release',
  'channel', 'cluster', 'worker', 'kernel', 'cache', 'index', 'cursor', 'offset', 'length',
  'prefix', 'suffix', 'branch', 'vector', 'matrix', 'profile', 'latency', 'memory',
  'café', 'naïve', 'résumé', 'Zürich', 'Straße', '東京',
]

const LANGS = ['rust', 'javascript', 'typescript', 'ocaml', 'scala', 'kotlin', 'dart', 'go', 'bash', 'json']
const DOMAINS = ['docs.example.org', 'api.service.net', 'status.cloud.io', 'spec.commonmark.org', 'pkg.registry.dev']
const PATHS = ['guide/getting-started', 'reference/options', 'v2/endpoints/parse', 'releases/notes', 'architecture/lexer']

function makeBenchParse(seed, sections) {
  const rnd = mulberry32(seed)
  const pick = (a) => a[Math.floor(rnd() * a.length)]
  const word = () => pick(WORDS)
  const phrase = (n) => {
    const out = []
    for (let i = 0; i < n; i++) out.push(word())
    return out.join(' ')
  }
  const parts = []
  const defs = []

  parts.push('# Technical Architecture & Parser Specification\n\n')
  parts.push(`This handbook describes *deterministic* **Markdown** compilation across **\`${sections}\`** modules, ~~legacy~~ modes, and [the specification][spec-root].\n\n`)
  defs.push('[spec-root]: https://spec.commonmark.org/0.31.2/ "CommonMark Specification"')

  for (let s = 1; s <= sections; s++) {
    const refId = `ref-${s}-doc`
    const dom = pick(DOMAINS)
    const pth = pick(PATHS)
    defs.push(`[${refId}]: https://${dom}/${pth}/sec-${s} "Section ${s} Reference"`)

    if (s % 4 === 0) {
      parts.push(`Section ${s}: ${word()} ${word()} Overview\n${'='.repeat(24)}\n\n`)
    } else if (s % 4 === 2) {
      parts.push(`Subsection ${s}: ${word()} ${word()} Details\n${'-'.repeat(24)}\n\n`)
    } else {
      parts.push(`## ${s}. ${word()} & \`${word()}_${s}\` Pipeline\n\n`)
    }

    // Rich inline paragraph with em/strong/del/code/inline-link/ref-link/autolink/entity/escape/html
    parts.push(
      `The *${word()}* **${word()}** module processes \`${word()}(${s})\` with `
      + `***${word()}*** precision, ~~${word()}~~ fallbacks, `
      + `[${word()} guide](https://${dom}/${pth}?section=${s}&mode=strict "Guide ${s}"), `
      + `and [${word()} notes][${refId}]. `
      + `Consult <https://${dom}/api/v${(s % 3) + 1}/${word()}> or `
      + `https://${dom}/status/${word()}_(v${s}). `
      + `Escaped \\*markers\\* and &copy; &#65;&#x42; stay intact inside <span class="badge">${word()}</span>.\n\n`,
    )

    // Blockquote (sometimes nested with lazy continuation)
    if (s % 2 === 1) {
      parts.push(
        `> **Note ${s}:** ${phrase(8)} with _${word()}_ and \`${word()}\`.\n`
        + `> ${phrase(6)} continuing lazily\n`
        + `still inside the quote for ${word()}.\n`
        + `>\n`
        + `> > Nested remark: ${phrase(6)} and ![**${word()}** *icon*](https://${dom}/img/${s}.png "${word()} icon").\n\n`,
      )
    }

    // Fenced code block (alternating backticks and tildes, occasionally indented)
    const lang = pick(LANGS)
    if (s % 3 === 0) {
      parts.push(
        `~~~${lang}\n`
        + `fn step_${s}(input: &str) -> usize {\n`
        + `    let tag = "<div data-id=\\"${s}\\">${word()} & ${word()}</div>";\n`
        + `    input.len() + tag.len()\n`
        + `}\n`
        + `~~~\n\n`,
      )
    } else {
      parts.push(
        `  \`\`\`${lang}\n`
        + `  // ${phrase(5)}\n`
        + `  export function run_${s}(cfg) {\n`
        + `    return cfg.${word()} && cfg.${word()} !== "<empty>";\n`
        + `  }\n`
        + `  \`\`\`\n\n`,
      )
    }

    // Nested list + GFM task list
    parts.push(
      `- [x] Verify **${word()}** handling for \`${word()}\` (${phrase(4)})\n`
      + `- [ ] Audit *${word()}* alignment and [${word()}][${refId}]\n`
      + `  1. First sub-step: ${phrase(6)}\n`
      + `  2. Second sub-step: ${phrase(5)} with \`${word()} | ${word()}\`\n`
      + `- [X] Confirm ~~${word()}~~ cleanup on <mailto:ops${s}@${dom}>\n\n`,
    )

    // GFM table with alignments, escaped pipes, inline code & formatting
    parts.push(
      `| Stage | Metric | Target | Notes |\n`
      + `| :--- | :---: | ---: | --- |\n`
      + `| **${word()}** | \`${word()}\` | ${s * 12} ms | ${phrase(4)} \\| *${word()}* |\n`
      + `| _${word()}_ | ~~${word()}~~ | ${s * 4 + 1} KB | [${word()}](https://${dom}/${word()}) &amp; ${word()} |\n`
      + `| ${word()} | \`${word()}_${s}\` | ${(s * 7) % 99}% | ${phrase(5)} |\n\n`,
    )

    // Occasional HTML block, hr, and indented code
    if (s % 5 === 0) {
      parts.push(
        `<div class="summary" data-section="${s}">\n`
        + `  <p>Summary block ${s}: ${phrase(5)}</p>\n`
        + `</div>\n\n`
        + `    indented_code_line_${s} = "${word()} < ${word()} & ${word()}"\n`
        + `    return indented_code_line_${s}\n\n`
        + `---\n\n`,
      )
    }
  }

  parts.push(defs.join('\n') + '\n')
  return parts.join('')
}

function makeBenchGfm(seed, entries) {
  const rnd = mulberry32(seed)
  const pick = (a) => a[Math.floor(rnd() * a.length)]
  const word = () => pick(WORDS)
  const phrase = (n) => {
    const out = []
    for (let i = 0; i < n; i++) out.push(word())
    return out.join(' ')
  }
  const parts = []
  parts.push('# Release Tracker & GFM Matrix\n\n')

  for (let e = 1; e <= entries; e++) {
    const dom = pick(DOMAINS)
    const pth = pick(PATHS)
    parts.push(`### Issue #${1000 + e}: ${word()} ${word()} (${word()})\n\n`)
    // Single-newline breaks + GFM autolinks + backpedal + strikethrough + inline code
    parts.push(
      `Reporter: dev${e}@${dom} (cc: mailto:team${e}@${dom}).\n`
      + `Spec URL: https://${dom}/${pth}/issue_${e}_(rev2)?a=1&b=2&amp;!\n`
      + `Mirror: www.${dom}/mirror/${word()}/${e}., backup ftp://${dom}/pub/${word()}.\n`
      + `Status: ~~${word()}~~ -> ~${word()}~ -> **${word()}** with \`${word()}_${e}\` and *${word()}*.\n\n`,
    )

    // Dense GFM checklist
    parts.push(
      `- [x] Reproduce on https://${dom}/case/${e} with **${word()}**\n`
      + `- [ ] Validate ~~${word()}~~ edge case (${phrase(5)})\n`
      + `- [X] Update \`${word()}\` docs at www.${dom}/docs/${e}\n\n`,
    )

    // Dense GFM table (4 rows)
    parts.push(
      `| Component | Owner | Build | Link | Status |\n`
      + `| :--- | :--- | :---: | :--- | ---: |\n`
      + `| \`${word()}\` | user${e}@${dom} | #${e}01 | https://${dom}/b/${e}01 | **pass** |\n`
      + `| *${word()}* | qa${e}@${dom} | #${e}02 | www.${dom}/b/${e}02 | ~~skip~~ |\n`
      + `| ${word()} \\| ${word()} | ops${e}@${dom} | #${e}03 | [${word()}](https://${dom}/r/${e}) | \`ok\` |\n`
      + `| _${word()}_ | rel${e}@${dom} | #${e}04 | https://${dom}/c/${e}_(v1) | ${e * 3} ms |\n\n`,
    )
  }

  return parts.join('')
}

// ---------------------------------------------------------------------------------------------
// Assemble corpus cases.

const cases = []
function add(id, note, input) {
  cases.push({ id, note, input })
}

// 1. Empty, whitespace, line endings, parseInline (4)
add('empty', 'empty string produces empty HTML', {
  op: 'parse',
  markdown: '',
})
add('whitespace-only', 'spaces, tabs, and blank lines only', {
  op: 'parse',
  markdown: '   \t \n\n  \t\n   ',
})
add('crlf-and-cr', 'CRLF and lone CR line endings normalized to LF in parse', {
  op: 'parse',
  markdown: '# Title\r\n\r\nLine 1\rLine 2\r\n\r\n- item 1\r\n- item 2\r',
})
add('parse-inline-basic', 'parseInline renders inline grammar without wrapping <p>', {
  op: 'parseInline',
  markdown: 'Hello **bold** *italic* `code` [link](https://example.com) \\*esc\\* &copy; &#65; <em>raw</em>',
})

// 2. Options matrix (mutant-killing!) (7)
const gfmProbeDoc = [
  'Visit https://example.com/path and www.example.org and user@example.com.',
  '',
  'Use ~one~ and ~~two~~ strikethrough.',
  '',
  '- [x] done task',
  '- [ ] open task',
  '',
  '| A | B |',
  '| :--- | ---: |',
  '| 1 | 2 |',
].join('\n')

add('opts-gfm-true', 'GFM enabled (default): tables, task lists, strikethrough, extended autolinks', {
  op: 'parse',
  markdown: gfmProbeDoc,
  options: { gfm: true, breaks: false, pedantic: false },
})
add('opts-gfm-false', 'GFM disabled: tables, task lists, strikethrough, extended autolinks all inactive', {
  op: 'parse',
  markdown: gfmProbeDoc,
  options: { gfm: false, breaks: false, pedantic: false },
})

const breaksProbeDoc = 'Line one\nLine two\n  Line three\n\nNext paragraph\nstill here'
add('opts-breaks-true', 'breaks: true with gfm: true turns single newlines into <br>', {
  op: 'parse',
  markdown: breaksProbeDoc,
  options: { gfm: true, breaks: true, pedantic: false },
})
add('opts-breaks-false', 'breaks: false keeps single newlines as soft breaks', {
  op: 'parse',
  markdown: breaksProbeDoc,
  options: { gfm: true, breaks: false, pedantic: false },
})
add('opts-gfm-false-breaks-true', 'breaks: true has no effect when gfm: false (matches marked Lexer rules selection)', {
  op: 'parse',
  markdown: breaksProbeDoc,
  options: { gfm: false, breaks: true, pedantic: false },
})

const pedanticProbeDoc = [
  '#NoSpaceHeading',
  '',
  '```js',
  'const x = 1;',
  '```',
  '',
  '- one',
  '* two',
  '+ three',
  '',
  'aa**"A"**bb and **foo:**bar and _foo:_bar',
  '',
  '[link](<https://example.com) and [ref] [id]',
  '',
  '[id]: https://example.org',
].join('\n')

add('opts-pedantic-true', 'pedantic: true activates John Gruber grammar (no fences, #NoSpace, merged bullets, openQuote/colon emphasis, loose links)', {
  op: 'parse',
  markdown: pedanticProbeDoc,
  options: { gfm: true, breaks: true, pedantic: true },
})
add('opts-pedantic-false', 'pedantic: false (default GFM) on the same pedantic probe document', {
  op: 'parse',
  markdown: pedanticProbeDoc,
  options: { gfm: true, breaks: false, pedantic: false },
})
const pedanticTaskProbeDoc = '- [x] done task\n- [ ] open task'
add('opts-pedantic-gfm-true-task', 'pedantic: true with gfm: true STILL enables GFM task list checkboxes (Tokenizer.ts checks options.gfm)', {
  op: 'parse',
  markdown: pedanticTaskProbeDoc,
  options: { gfm: true, breaks: false, pedantic: true },
})
add('opts-pedantic-gfm-false-task', 'pedantic: true with gfm: false disables GFM task list checkboxes', {
  op: 'parse',
  markdown: pedanticTaskProbeDoc,
  options: { gfm: false, breaks: false, pedantic: true },
})

// 3. Headings & thematic breaks (5)
add('headings-atx-all-levels', 'ATX headings h1-h6, 0-3 leading spaces, 4 spaces=code, closing # rules, empty heading, #nospace', {
  op: 'parse',
  markdown: [
    '# H1',
    ' ## H2 ##',
    '  ### H3 ###  ',
    '   #### H4\t####',
    '##### H5###',
    '###### H6 # not closed',
    '####### Not H7',
    '#',
    '# ###',
    '#no-space',
    '    # indented code',
  ].join('\n\n'),
})
add('headings-setext', 'Setext h1/h2, multiline, trailing spaces, precedence over hr, interrupted by blocks', {
  op: 'parse',
  markdown: [
    'Heading 1',
    '===   ',
    '',
    'Multi-line',
    'Heading 2',
    '   ---',
    '',
    '---',
    '',
    '> quoted',
    '---',
    '',
    '- item',
    '---',
    '',
    '    indented',
    '===',
  ].join('\n'),
})
add('headings-inline-content', 'ATX and setext headings with inline emphasis, codespan, link, escape, and entities', {
  op: 'parse',
  markdown: '# *Em* **Strong** `code` [link](https://example.com) \\# &amp; &#65;\n\nSub _heading_ `x`\n---',
})
add('headings-atx-pedantic', 'Pedantic ATX headings (#nospace, stripping # without space, indented # not heading, single-line setext, # /  #  paragraph interrupt)', {
  op: 'parse',
  markdown: '#NoSpace###\n\n # IndentedNotHeading\n\nLine 1\nLine 2\n===\n\na\n# \n\na\n # ',
  options: { pedantic: true },
})
add('hr-variants', 'Thematic breaks: ---, ***, ___, spaced, tabs, 3-space indent, 4-space code, interrupting paragraph', {
  op: 'parse',
  markdown: [
    '---',
    '***',
    '___',
    ' - - - - ',
    '  *\t*\t*',
    '   ____',
    '    ---',
    'Paragraph text',
    '***',
    'After hr',
  ].join('\n'),
})

// 4. Code blocks: indented & fenced (7)
add('code-indented-basic', 'Indented code blocks: 4 spaces, tab, 0-3 spaces+tab, internal blanks, trailing blanks trimmed, HTML escaped', {
  op: 'parse',
  markdown: '    line 1 <tag> & "q" \'s\'\n\tline 2\n  \tline 3\n\n    after blank\n\n\nnext para',
})
add('code-indented-no-interrupt', 'Indented code cannot interrupt paragraph or tight list text (6-space indent dedents to 4-space code inside tight list)', {
  op: 'parse',
  markdown: 'para line\n    indented line\n\n- tight item\n      indented code line\n\n---\n\n    code block\n\n\n',
})
add('code-indented-pedantic', 'Indented code in pedantic mode retains trailing blank lines', {
  op: 'parse',
  markdown: '    alpha\n\n\n\nbeta',
  options: { pedantic: true },
})
add('code-fenced-backticks', 'Backtick fences: 3/4/5 ticks, closing with extra ticks/tildes, empty, 1 vs 2 blank lines, unclosed at EOF', {
  op: 'parse',
  markdown: [
    '```',
    '```',
    '',
    '```',
    '',
    '```',
    '',
    '```',
    '',
    '',
    '```',
    '',
    '````',
    'inner ``` fence',
    '````~~~  ',
    '',
    '```js',
    'unclosed <eof> & line',
  ].join('\n'),
})
add('code-fenced-tildes', 'Tilde fences allow backticks in info string and do NOT apply indentCodeCompensation', {
  op: 'parse',
  markdown: [
    '~~~a`b c',
    'tilde body',
    '~~~~',
    '',
    '  ~~~',
    '  keep two spaces',
    '    keep four',
    '  ~~~',
  ].join('\n'),
})
add('code-fenced-indent-compensation', 'Backtick fences indented 1-3 spaces strip up to fence indent from body lines', {
  op: 'parse',
  markdown: [
    '   ```',
    '   exact three',
    '     five spaces',
    ' one space',
    'zero',
    '   ```',
  ].join('\n'),
})
add('code-fenced-info-and-fallback', 'Info string takes first non-space token, unescapes Unicode punct, escapes HTML; backtick in backtick-fence info falls back to paragraph', {
  op: 'parse',
  markdown: [
    '```c\\+\\+ extra tokens',
    'int main() {}',
    '```',
    '',
    '```js\\€',
    '1 + 1',
    '```',
    '',
    '```js&amp;"<x>',
    '2 + 2',
    '```',
    '',
    '```foo`bar',
    'not a fence',
  ].join('\n'),
})

// 5. Blockquotes (5)
add('blockquote-basic-nested', 'Blockquotes: single, nested, empty >, tab after >, multiple blocks inside', {
  op: 'parse',
  markdown: [
    '> # Quoted heading',
    '>',
    '> Paragraph one',
    '>\twith tab',
    '> > Nested quote',
    '> >',
    '> > - item in nested quote',
    '>',
    '> ---',
  ].join('\n'),
})
add('blockquote-lazy-continuation', 'Blockquote lazy continuation, blockquoteSetextReplace preventing setext (=== and ---), stopping after code/heading/hr/html', {
  op: 'parse',
  markdown: [
    '> a',
    'b',
    '===',
    '',
    '> lazy start',
    'lazy line',
    '---',
    '',
    '> # heading',
    'not lazy after heading',
    '',
    '>     code',
    'not lazy after code',
    '',
    '> ---',
    'not lazy after hr',
    '',
    '> <div>html</div>',
    'not lazy after html',
  ].join('\n'),
})
add('blockquote-nested-continuation', 'Nested blockquote lazy continuation and restated > marker', {
  op: 'parse',
  markdown: [
    '> > alpha',
    'beta',
    '> gamma',
    '',
    '> > one',
    'two',
    '> > three',
    '',
    '> > first',
    'second',
    'third',
  ].join('\n'),
})
add('blockquote-list-continuation', 'Blockquote containing a list with lazy continuation and following items', {
  op: 'parse',
  markdown: [
    '> - item one',
    'lazy line',
    '> - item two',
    '',
    '> 1. first',
    'continued',
  ].join('\n'),
})
add('blockquote-interrupts-paragraph', 'Blockquote interrupts paragraph; bare list marker inside blockquote is not lazily continued', {
  op: 'parse',
  markdown: 'para line\n> quoted\n\n> para\n- sibling item\n> more\n2. ordered sibling',
})

// 6. Lists & GFM Task Lists (9)
add('list-unordered-markers', 'Unordered lists split by marker (-, *, +) in CommonMark/GFM; hr breaks list', {
  op: 'parse',
  markdown: '- a\n- b\n* c\n* d\n+ e\n\n* x\n* * *\n* y',
})
add('list-ordered-start', 'Ordered lists: start=1, 0, 003, delimiter . vs ), 9-digit ok, 10-digit not a list', {
  op: 'parse',
  markdown: [
    '1. one',
    '2. two',
    '',
    '0. zero',
    '003. three',
    '',
    '5) paren five',
    '6) paren six',
    '7. dot seven',
    '',
    '123456789. nine digits',
    '',
    '1234567890. ten digits',
  ].join('\n'),
})
add('list-tight-vs-loose', 'Tight list vs loose (blank between items, blank inside item makes all loose, trailing blank after last stays tight)', {
  op: 'parse',
  markdown: [
    '- tight 1',
    '- tight 2',
    '',
    '',
    '---',
    '',
    '- loose 1',
    '',
    '- loose 2',
    '',
    '---',
    '',
    '- item a',
    '',
    '  second para in a',
    '- item b',
  ].join('\n'),
})
add('list-nested-and-indent', 'Nested ul/ol, >4 space indented code on first line, empty list items, endEarly double blank, tab expansion', {
  op: 'parse',
  markdown: [
    '- outer',
    '  - inner 1',
    '    1. deep 1',
    '    2. deep 2',
    '  - inner 2',
    '-     five spaces is code',
    '-',
    '- after empty',
    '-',
    '',
    '  outside after blank empty item',
    '',
    '-\twith tab',
    '\tcontinuation',
  ].join('\n'),
})
add('list-lazy-and-interrupts', 'List lazy continuation, interrupting blocks, and paragraph interruption (only 1. non-empty interrupts)', {
  op: 'parse',
  markdown: [
    'para before',
    '2. does not interrupt',
    '',
    'para before',
    '1. interrupts!',
    'lazy cont',
    '# heading stops item',
    '',
    '- item',
    '```',
    'fence stops item',
    '```',
    '- item 2',
    '> quote stops item',
  ].join('\n'),
})
add('list-task-tight', 'Tight GFM task lists: [ ], [x], [X], non-tasks, nested task lists', {
  op: 'parse',
  markdown: [
    '- [ ] unchecked',
    '- [x] lower checked',
    '- [X] upper checked',
    '  - [x] nested checked',
    '  - [ ] nested unchecked',
    '- [x]no space not task',
    '- [y] wrong char',
    '- [x]   ',
  ].join('\n'),
})
add('list-task-loose', 'Loose GFM task list puts checkbox inside <p>', {
  op: 'parse',
  markdown: '- [x] first loose task\n\n- [ ] second loose task\n\n  extra paragraph',
})
add('list-task-non-text-first', 'Task item whose first block is not text/paragraph (e.g. setext heading) clears task flag', {
  op: 'parse',
  markdown: '- [x] a\n===\n- [x] # heading inside? No, inline text\n- [x]   \n  > quote',
})
add('list-pedantic', 'Pedantic list merges -, *, + and applies listReplaceNesting', {
  op: 'parse',
  markdown: '- one\n* two\n+ three\n    - nested\n        deep',
  options: { pedantic: true },
})

// 7. GFM Tables (6)
add('table-alignments', 'GFM table with default, left, center, right alignments and optional outer pipes', {
  op: 'parse',
  markdown: [
    '| Def | Left | Center | Right |',
    '| --- | :--- | :---: | ---: |',
    '| a | b | c | d |',
    '',
    'No | Outer | Pipes',
    '--- | :--- | ---:',
    'x | y | z',
  ].join('\n'),
})
add('table-escaped-pipes-and-code', 'GFM table with escaped pipes \\|, \\\\|, \\\\\\|, code spans, and inline formatting', {
  op: 'parse',
  markdown: [
    '| Expr | Desc |',
    '| --- | --- |',
    '| a \\| b | escaped single pipe |',
    '| a \\\\| b | two backslashes splits cell |',
    '| a \\\\\\| b | three backslashes keeps pipe |',
    '| `x \\| y` | **bold** and *em* |',
  ].join('\n'),
})
add('table-header-only', 'GFM table with header and delimiter row only emits no <tbody>', {
  op: 'parse',
  markdown: '| Only | Header |\n| --- | --- |',
})
add('table-mismatched-rows', 'GFM table pads short body rows with empty <td> and truncates excess cells', {
  op: 'parse',
  markdown: [
    '| A | B | C |',
    '| --- | --- | --- |',
    '| 1 |',
    '| 1 | 2 | 3 | 4 | 5 |',
  ].join('\n'),
})
add('table-invalid-fallback', 'Mismatched header/align counts reject table (but still interrupt GFM paragraph!); delimiter without | or : becomes setext', {
  op: 'parse',
  markdown: [
    'para before',
    'a | b | c',
    '---|---',
    '',
    'a | b',
    '---',
    '1 | 2',
  ].join('\n'),
})
add('table-interrupted-by-blocks', 'GFM table body interrupted by heading, blockquote, fence, list, hr, html, indented code', {
  op: 'parse',
  markdown: [
    '| K | V |',
    '| --- | --- |',
    '| 1 | a |',
    '# Heading',
    '',
    '| K | V |',
    '| --- | --- |',
    '| 2 | b |',
    '> quote',
    '',
    '| K | V |',
    '| --- | --- |',
    '| 3 | c |',
    '- bullet',
    '',
    '| K | V |',
    '| --- | --- |',
    '| 4 | d |',
    '<div>html</div>',
  ].join('\n'),
})

// 8. HTML Blocks & Inline HTML (6)
add('html-blocks-types-1-to-5', 'CommonMark HTML block types 1 (script/pre/style/textarea), 2 (comment), 3 (PI), 4 (decl), 5 (CDATA)', {
  op: 'parse',
  markdown: [
    '<PRE>',
    '*not em*',
    '</PRE>',
    '',
    '<script type="text/javascript">',
    'if (a < b) {}',
    '</script>',
    '',
    '<style>.a { color: red; }</style>',
    '',
    '<textarea>raw & text</textarea>',
    '',
    '<!-- multi',
    'line comment -->',
    '',
    '<!-->',
    '',
    '<?xml version="1.0"?>',
    '',
    '<!DOCTYPE html>',
    '',
    '<![CDATA[<not tag> & ok]]>',
  ].join('\n'),
})
add('html-blocks-type-6-and-7', 'HTML block types 6 (standard tags) and 7 (custom complete open/close tags); paragraph interrupt rules and case-sensitivity', {
  op: 'parse',
  markdown: [
    '<div class="box">',
    '  *inside div*',
    '</div>',
    '',
    '<custom-widget data-x="1" disabled>',
    'type 7 block',
    '',
    'para line',
    '<custom-widget>',
    'type 7 does not interrupt para',
    '',
    'para line',
    '<div>type 6 lowercase interrupts</div>',
    '',
    'para line',
    '<DIV>uppercase does not interrupt in marked</DIV>',
    '',
    '</custom-end>',
    'after closing tag',
  ].join('\n'),
})
add('html-block-trailing-blanks', 'trimTrailingBlankLines on HTML blocks with 1 vs 2+ trailing blank lines', {
  op: 'parse',
  markdown: '<div>one</div>\n\n<div>two</div>\n\n\npara',
})
add('html-inline-tags-and-raw', 'Inline HTML tags: <span>, <a href> suppressing GFM url autolink, <pre>/<code>/<kbd>/<script> toggling inRawBlock, undef reflink & text merge inside raw', {
  op: 'parse',
  markdown: [
    'Text <span class="x" id=\'y\' data=z>inner *em*</span> end.',
    '',
    '<a href="https://example.com">https://not-nested.com</a> and https://outside.com',
    '',
    'Raw <pre>&#65; & < > *em*</pre> after &#65; & ok.',
    '',
    'Raw <code>&#66; &</code> and <kbd>&#67; &</kbd> end.',
    '',
    'x <pre>[foo] &</pre>',
  ].join('\n'),
})
add('html-inline-comments-pi-cdata', 'Inline comments, PI, declarations, CDATA, and unclosed <!-- not matching inline tag', {
  op: 'parse',
  markdown: 'a <!-- ok --> b <?pi?> c <!DECL x> d <![CDATA[raw]]> e <!-- unclosed',
})
add('html-pedantic', 'Pedantic block HTML vs inline HTML tags (including multiline <span> parsed as inline paragraph, not block HTML type 7)', {
  op: 'parse',
  markdown: '<div>\n*block*\n</div>\n\n<span>\n*inline*\n</span>\n\n<hr />',
  options: { pedantic: true },
})

// 9. Link Reference Definitions, Links, Images & Autolinks (10)
add('def-normalization-and-precedence', 'Link defs: Unicode case-folding (ẞ/ß/SS), whitespace collapsing, first-def wins, cannot interrupt paragraph or tight list text', {
  op: 'parse',
  markdown: [
    '[ẞ]: /first "First"',
    '[ss]: /second "Second"',
    '[  Foo   Bar\nBaz  ]: /multi',
    '',
    'Matches [SS], [ß], and [foo bar baz].',
    '',
    'Paragraph line',
    '[no-interrupt]: /nope',
    '',
    'Check [no-interrupt].',
    '',
    '- tight',
    '  [listdef]: /url',
    '',
    '[listdef]',
  ].join('\n'),
})
add('def-destinations-and-titles', 'Link defs: <angle> dest (including lazy backtracking across >), balanced vs unbalanced parens (own para), titles, invalid trailing text backtracking, Unicode punct unescape', {
  op: 'parse',
  markdown: [
    '[a]: <https://example.com/a b> "title \\"q\\" \\€"',
    '[b]: https://example.com/fn(1) \'multi',
    'line title\'',
    '[c]: https://example.com/paren (paren title)',
    '[d]: https://example.com/ok',
    '"not a title" trailing text',
    '',
    '[bad]: https://example.com/unbalanced(paren',
    '',
    '[bt1]: <foo>bar>',
    '[bt2]: <foo> bar> "t"',
    '',
    '[a] [b] [c] [d] [bad] [bt1] [bt2]',
  ].join('\n'),
})
add('links-inline-variants', 'Inline links: titles, empty, <url>, balanced parens, extra ) truncation, escaped brackets (incl. image codespan), cleanUrl encoding, invalid <url', {
  op: 'parse',
  markdown: [
    '[text](https://example.com/a%20b%25c "t1")',
    '[q](\'https://example.com/café [x]\' \'t2\')',
    '[p](https://example.com/fn(a(b)) (t3))',
    '[extra](https://example.com/x))',
    '[empty]()',
    '[angle](<https://example.com/a b>)',
    '[esc \\[bracket\\]](https://example.com/\\*star\\€)',
    '![`\\[x\\]`](/img.png)',
    '[bad-angle](<https://example.com/a\\>)',
    '[bad-angle2](<https://example.com/a)',
  ].join('\n\n'),
})
add('links-extra-paren-unicode', 'Inline link/image extra-) truncation computes linkLen in UTF-16 code units with leading whitespace before non-ASCII href', {
  op: 'parse',
  markdown: [
    '[x](  é))',
    '![x](  é))',
    '[x](  日本語))',
    '[x](    日本語))',
  ].join('\n\n'),
})
add('links-ref-full-collapsed-shortcut', 'Full, collapsed, shortcut reflinks, undefined ref literal [, nested brackets (2 levels)', {
  op: 'parse',
  markdown: [
    '[r]: https://example.com/r "Ref"',
    '',
    '[full text][r] and [r][] and [r] and [missing] and [a [b [c]]](https://example.com/nest) and ![img [a]](https://example.com/i.png)',
  ].join('\n'),
})
add('links-no-nesting-and-images', 'Links cannot contain links (inline or ref); image inside link and link inside image ARE allowed', {
  op: 'parse',
  markdown: [
    '[r1]: /r1',
    '[r2]: /r2',
    '',
    '[outer [inner](/in)](/out)',
    '',
    '[outer [inner][r1]][r2]',
    '',
    '[![alt](/img.png)](/link)',
    '',
    '![alt [inner](/in)](/img.png)',
  ].join('\n'),
})
add('links-label-inside-tag-or-code', 'isLabelEndInsideToken rejects ] inside inline HTML tag or autolink; codespan inside label; linkParenPossible no-paren path', {
  op: 'parse',
  markdown: [
    '[a <b c="](https://example.com)">',
    '',
    '[a <b c="]">](https://example.com)',
    '',
    '[a <https://example.com/]>](https://example.org)',
    '',
    '[a `b](x) c`](https://example.com/code)',
    '',
    '[a](b [c](d [e](f',
  ].join('\n'),
})
add('images-text-renderer', 'Image alt text rendered through TextRenderer (strong, em, codespan, del, link, image, html, br, escape, entities)', {
  op: 'parse',
  markdown: '![**bold** *em* `code` ~~del~~ [lnk](/u) ![sub](/s.png) <b>h</b> a  \nb \\* &amp; &#65; "q"](https://example.com/pic.png "My \\"Title\\"")',
})
add('links-pedantic', 'Pedantic links & defs: space before [ref], unclosed <href, pedanticHrefTitle, empty "" / () def titles rejected', {
  op: 'parse',
  markdown: '[ref]: https://example.com\n[eq]: /u ""\n[ep]: /u ()\n\n[a] [ref] and [b](https://example.com/u "title") and [c](<https://example.com/open) and [eq] [ep]',
  options: { pedantic: true },
})
add('autolinks-commonmark', 'CommonMark angle-bracket autolinks: URI schemes, email, escaping &, invalid autolinks', {
  op: 'parse',
  markdown: '<https://example.com/a?x=1&y=2> <mailto:a@b.com> <user+tag@sub.example.com> <http://bad space> <1bad:scheme> <bad@host_>',
})

// 10. GFM Extended Autolinks (3)
add('gfm-autolinks-urls', 'GFM extended autolinks: http/HTTPS/ftp/www/mailto/xmpp/bare email and emailProtocol prefix boundary', {
  op: 'parse',
  markdown: [
    'http://example.com/a HTTPS://EXAMPLE.ORG/B ftp://files.example.net/c',
    'www.example.com/docs user.name+1@example.co.uk',
    'mailto:user@example.com. xmpp:user@example.com/res.',
    'a:mailto:x@y.com vs amailto:x@y.com',
    'foo@bar.com-baz and foo@bar.com_baz',
  ].join('\n\n'),
})
add('gfm-autolinks-backpedal', 'GFM autolink _backpedal trailing punctuation, balanced/nested parens, trailing entities, inLink suppression', {
  op: 'parse',
  markdown: [
    'https://en.wikipedia.org/wiki/Foo_(bar))',
    'https://en.wikipedia.org/wiki/Foo_(bar(baz))',
    'https://example.com/?a=1&amp;',
    'https://example.com/a&b;c&amp;',
    'https://example.com/end?!.,:;*_\'"~',
    '[https://in-link.com](https://target.com)',
  ].join('\n\n'),
})
add('gfm-autolinks-disabled-in-normal', 'Bare URLs and emails are NOT autolinked when gfm: false', {
  op: 'parse',
  markdown: 'https://example.com www.example.com user@example.com mailto:a@b.com',
  options: { gfm: false },
})

// 11. Emphasis, Strong, Strikethrough & Masking (8)
add('em-strong-asterisk-flanking', 'Asterisk emphasis: em, strong, triple, intraword, rule 9/10 multiple-of-3, isolated midRun (**a*b*c), excess delims', {
  op: 'parse',
  markdown: [
    '*em* **strong** ***both***',
    'a*b*c a**b**c foo***bar***baz',
    'a**b*c**d',
    '**a*b*c',
    '*a*** ***a*',
    '****a**** *a *b* c* **a **b** c**',
  ].join('\n\n'),
})
add('em-strong-underscore-flanking', 'Underscore emphasis: intraword blocked, Unicode alnum blocked, isolated foo___bar___ prevChar underscore, punct flanking, orphan skip inside **/__', {
  op: 'parse',
  markdown: [
    '_em_ __strong__ ___both___',
    'foo_bar_baz foo_bar_ _foo_bar',
    'foo___bar___',
    '(_a_) a_!_b α_β_γ café_latte_ 1_2_3',
    '*__a*b__* and _**a_b**_',
  ].join('\n\n'),
})
add('em-strong-unicode-punctuation', 'Unicode \\p{P}/\\p{S} flanking and astral/emoji prevChar UTF-16 slice(-1) semantics', {
  op: 'parse',
  markdown: [
    '«*hello*» “_world_” —**dash**—',
    '€*!a!* vs 🚀*!a!*',
    '🚀_foo_ and 𝕳_foo_ and *🚀* and a**🚀**b',
    '€**a**€ and 🚀**a**🚀',
  ].join('\n\n'),
})
add('em-strong-masking', 'emStrong masking: escapes (incl. inside span), codespans with inner backticks, inline HTML, inline links, reflinks (shortcut vs collapsed vs nested)', {
  op: 'parse',
  markdown: [
    '[a*b]: /u1',
    '[inner]: /u2',
    '',
    '\\*a* and \\€*a* and *foo \\* bar*',
    '*a `*` b* and *a `` ` `` b*',
    '*a <b c="*"> d*',
    '*a [b*c](/u)*',
    '*foo [a*b]* vs *foo [a*b][]*',
    '*foo [x [inner] y*z][a*b]*',
  ].join('\n\n'),
})
add('em-strong-gfm-tilde', 'GFM ~ inside emphasis/strong (_punctuationGfmStrongEm)', {
  op: 'parse',
  markdown: '**~strike~** *~~a~~* ~~*a*~~ ~**a**~ a*~b~*c',
})
add('em-strong-pedantic', 'Pedantic emphasis: _openQuote (Pi/Ps/"/\') and colon/punct rules 3 & 6', {
  op: 'parse',
  markdown: 'aa**"A"**bb aa**(A)**bb «**a**» **foo:**bar _foo:_bar',
  options: { pedantic: true },
})
add('gfm-strikethrough', 'GFM strikethrough: ~single~, ~~double~~, ~~~triple~~~ rejected, flanking, nested, escaped, mismatched', {
  op: 'parse',
  markdown: '~one~ ~~two~~ ~~~three~~~ ~~a ~b~ c~~ \\~esc~ ~mis~~ (~punct~) a~~b~~c',
})
add('codespans-edge-cases', 'Codespans: multi-backtick, space stripping (all-space preserved!), newline->space, unclosed, HTML escaping', {
  op: 'parse',
  markdown: '`a` ` a ` `  ` ` a  ` `` ` `` ```` ``` ```` `line1\nline2` `<a & "b">` ``unclosed `',
})

// 12. Escapes, Hard Breaks, Entities, Unicode, parseInline options (5)
add('backslash-escapes', 'All 32 ASCII punctuation escapes, non-ASCII punct not escaped in text, HTML char escapes', {
  op: 'parse',
  markdown: '\\!\\"\\#\\$\\%\\&\\\'\\(\\)\\*\\+\\,\\-\\.\\/\\:\\;\\<\\=\\>\\?\\@\\[\\\\\\]\\^\\_\\`\\{\\|\\}\\~ \\€ \\— \\a',
})
add('hard-breaks', 'Hard line breaks: 2+ spaces + LF, backslash + LF, trailing spaces at end not <br>, leading spaces/tabs after br stripped', {
  op: 'parse',
  markdown: 'line 1  \n   \tline 2\\\n\tline 3  \n   \n\nend spaces  ',
})
add('html-entities-and-ncr', 'Numeric character references (decimal, hex, 0/surrogate/overflow->FFFD, &#38;, bare & adjacent to NCR), named & invalid entities, raw block', {
  op: 'parse',
  markdown: [
    '&#65; &#x41; &#X1F680; &#0; &#x0; &#55296; &#57343; &#9999999; &#x110000;',
    '&#38;amp; &&#65;; &copy; &amp; &nbsp; &foo_1;',
    '&foo-bar; &; &#12345678; &#x1234567; a & b < c > d "e" \'f\'',
    '<pre>&#65; & < > "q"</pre>',
  ].join('\n\n'),
})
add('unicode-cjk-emoji-rtl', 'CJK, combining marks, Greek, Cyrillic, Arabic, astral emoji, NBSP/BOM/EM-SPACE trimming', {
  op: 'parse',
  markdown: '# \uFEFF日本語テスト\u00A0\n\nCafé naïve résumé — Привет мир — مرحبا — 🚀😀𝕳\n\n\u2003Indented with em-space',
})
add('parse-inline-options', 'parseInline with gfm: true, breaks: true (autolinks, del, single-LF <br>, raw CR untouched)', {
  op: 'parseInline',
  markdown: 'Line 1\nLine 2 ~~del~~ https://example.com/a\r\nLine 3',
  options: { gfm: true, breaks: true, pedantic: false },
})
add('parse-inline-trailing-br-ws', 'parseInline trailing 2 spaces + LF + trailing spaces only must NOT emit <br> ((?!\\s*$) negative lookahead)', {
  op: 'parseInline',
  markdown: 'line  \n   ',
})

// 13. Regressions from the 2026-11-20 pre-merge review (31): native-scanner ports (Rust, Elm)
// must give regex `.` and the `m`-flag anchors JS's line terminators (\n, \r, U+2028, U+2029),
// count quantifier bounds and slices in UTF-16 code units, and backtrack where the regex does;
// the OCaml ports must build marked's exact rule sources and lex in constant stack.
const party = '\u{1F389}'
const P = { pedantic: true }

add('regress-dot-heading-u2028', 'ATX heading `(.*)(?:\\n+|$)` cannot cross U+2028 (JS `.` excludes \\r, U+2028, U+2029): a paragraph', {
  op: 'parse',
  markdown: '# a b',
})
add('regress-dot-heading-u2029', 'ATX heading text cannot cross U+2029: a paragraph', {
  op: 'parse',
  markdown: '# a b',
})
add('regress-dot-heading-cr', 'parse normalises the lone \\r to \\n before lexing: a heading, then a paragraph', {
  op: 'parse',
  markdown: '# a\rb',
})
add('regress-dot-pedantic-heading-u2028', 'pedantic heading `^(#{1,6})(.*)(?:\\n+|$)` cannot cross U+2028', {
  op: 'parse',
  markdown: '#a b',
  options: P,
})
add('regress-dot-lheading-u2029', 'setext heading text `(?:.|\\n(?!...))+?` cannot cross U+2029: a paragraph', {
  op: 'parse',
  markdown: 'a b\n===',
})
add('regress-dot-pedantic-lheading-lookahead', 'pedantic paragraph: its lheading lookahead stops at U+2028, so the lines stay one paragraph', {
  op: 'parse',
  markdown: 'a\nb c\n===\n',
  options: P,
})
add('regress-dot-table-header-u2028', 'GFM table header `([^\\n ].*)\\n` cannot cross U+2028: no table', {
  op: 'parse',
  markdown: 'a |b\n-|-\nc|d',
})
add('regress-dot-table-row-u2028', 'GFM table row `.*(?:\\n|$)`: a row holding U+2028 ends the table before it', {
  op: 'parse',
  markdown: 'a|b\n-|-\nc |d',
})
add('regress-dot-def-angle-u2028', 'def destination `<.*?>` cannot cross U+2028: no definition', {
  op: 'parse',
  markdown: '[a]: <b c>\n\n[a]',
})
add('regress-dot-pedantic-inline-link-cr', 'parseInline keeps \\r, and the pedantic link `\\((.*?)\\)` cannot cross it: literal text', {
  op: 'parseInline',
  markdown: '[a](b\rc)',
  options: P,
})
add('regress-dot-link-angle-escape-cr', 'parseInline link `<(?:\\\\.|[^\\n<>\\\\])+>`: the escape `\\\\.` cannot take \\r, so no link', {
  op: 'parseInline',
  markdown: '[a](<b\\\rc>)',
})
add('regress-mflag-code-indent', 'indented code: `codeRemoveIndent` has the `m` flag, so `^` also matches after U+2028', {
  op: 'parse',
  markdown: '    a     b',
})
add('regress-mflag-blockquote-marker', 'blockquote: `blockquoteSetextReplace2` (`m` flag) strips a `>` marker after U+2028', {
  op: 'parse',
  markdown: '> a > b',
})
add('regress-mflag-pedantic-space-line', 'pedantic preprocessing `^ +$` (`m` flag) empties the space run before U+2028', {
  op: 'parse',
  markdown: 'x\n   y',
  options: P,
})
add('regress-utf16-list-lazy-astral', 'list `line.slice(indent)` counts UTF-16 code units: an astral lazy line shifts the next indentation test', {
  op: 'parse',
  markdown: `* a\n${party}    o\na`,
})
add('regress-utf16-list-lazy-astral-tab', 'as above with a tab after the astral character', {
  op: 'parse',
  markdown: `- a\n${party}\tb\nc`,
})
add('regress-utf16-list-lazy-heading', '`x🎉` is 3 code units, so `slice(3)` leaves `# h`, a heading start that ends the item', {
  op: 'parse',
  markdown: `1. a\nx${party}# h\nc`,
})
add('regress-utf16-bounded-label', '`_boundedBlockLabel` `{1,999}` counts code units: a 1,202-unit label is not masked, so the `*` pair stays literal', {
  op: 'parse',
  markdown: `[${party.repeat(600)}*b]: /u\n\n*a [${party.repeat(600)}*b] c*`,
})
add('regress-utf16-bounded-label-edge', 'a 1,000-unit label (499 astral + `*b`) is one over the bound: not masked', {
  op: 'parse',
  markdown: `[${party.repeat(499)}*b]: /u\n\n*a [${party.repeat(499)}*b] c*`,
})
add('regress-lheading-table-pipe-dash', 'lheadingGfm table interrupt `\\|?(?:[:\\- ]*\\|)+`: `\\|?` backtracks, so a `|-` line interrupts the setext heading', {
  op: 'parse',
  markdown: 'a|b\n|-\nbar\n===',
})
add('regress-lheading-table-lone-pipe', 'a lone `|` line interrupts the setext heading', {
  op: 'parse',
  markdown: 'a\n|\nb\n===',
})
add('regress-lheading-table-lone-pipe-hr', 'a lone `|` line before `---` is a table interrupt: a paragraph and a thematic break, not an h2', {
  op: 'parse',
  markdown: '|\n---\n',
})
add('regress-pedantic-href-title-backtrack', 'pedanticHrefTitle `^([^\'"]*[^\\s])\\s+([\'"])(.*)\\2` backtracks: href `a\'`, title `t`', {
  op: 'parse',
  markdown: '[x](a\' "t")',
  options: P,
})
add('regress-pedantic-inline-href-title-backtrack', 'the same split through parseInline', {
  op: 'parseInline',
  markdown: '[x](a\' "t")',
  options: P,
})
add('regress-pedantic-href-title-mirror', 'the mirror case: href `a"`, title `t`', {
  op: 'parse',
  markdown: '[x](a" \'t\')',
  options: P,
})
add('regress-pedantic-def-empty-angle', 'pedantic def `<?([^\\s>]+)>?`: `<?` backtracks, the href is `<`, and the definition is consumed', {
  op: 'parse',
  markdown: '[a]: <>\n',
  options: P,
})
add('regress-pedantic-def-angle-title', '`<?` backtracks before a title: href `<`, title `t`', {
  op: 'parse',
  markdown: '[a]: < (t)\n\n[a]',
  options: P,
})
add('regress-nolink-empty-ref-paren', 'reflinkSearch `nolink(?!\\()`: `(?:\\[\\])?` backtracks before `(`, so `[*a]` is masked and `*x ... (*` pairs', {
  op: 'parse',
  markdown: '*x [*a][](* y\n\n[*a]: /u',
})
add('regress-html-begin-ul', 'a `<ul>` line (the last `_tag` entry) ends a list item at its indentation', {
  op: 'parse',
  markdown: '- a\n<ul>\n<li>x</li>\n</ul>\n',
})
add('regress-size-atx-headings', '5,000 ATX headings (20 KB, 5,000 block tokens): the block loop runs in constant stack', {
  op: 'parse',
  markdown: '# h\n'.repeat(5000),
})
add('regress-size-emphasis', '5,000 emphasis runs (20 KB, 10,000 inline tokens): the inline loop runs in constant stack', {
  op: 'parse',
  markdown: '_a_ '.repeat(5000),
})
add('regress-depth-list-100', '100 nested lists, the parity domain\'s nesting bound: one recursive blockTokens per level', {
  op: 'parse',
  markdown: `${'- '.repeat(100)}a\n`,
})
add('regress-depth-blockquote-100', '100 nested blockquotes, the parity domain\'s nesting bound: one recursive blockTokens per level', {
  op: 'parse',
  markdown: `${'>'.repeat(100)} a\n`,
})
// `levels` inline containers around `core`, cycling the delimiters whose content marked lexes and
// renders recursively: *em*, **strong**, ~~del~~, _em_ and __strong__.
function nestedInline(levels, core) {
  const delimiters = ['*', '**', '~~', '_', '__']
  let text = core
  for (let level = levels - 1; level >= 0; level--) {
    const d = delimiters[level % delimiters.length]
    text = `${d}a ${text} b${d}`
  }
  return text
}
add('regress-depth-inline-100', '100 nested inline containers, the parity domain\'s inline nesting bound: link text around em, strong and del, one recursive inlineTokens and parseInline per level', {
  op: 'parse',
  markdown: `[${nestedInline(99, 'x')}](https://example.com/u)\n`,
})
add('regress-depth-list-100-inline-100', '100 nested inline containers in the innermost of 100 nested lists: the block and inline bounds hold separately, and the renderer stacks the two recursions', {
  op: 'parse',
  markdown: `${'- '.repeat(100)}${nestedInline(100, 'x')}\n`,
})

// 14. Bench cases (2)
const benchParseMd = makeBenchParse(0x18112026, 32)
const benchGfmMd = makeBenchGfm(0x7f4a7c15, 44)

add('bench-parse', 'Mixed CommonMark+GFM technical handbook (~52 KB) under default options', {
  op: 'parse',
  markdown: benchParseMd,
  options: { gfm: true, breaks: false, pedantic: false },
})
add('bench-gfm', 'GFM-heavy issue tracker & tables document (~46 KB) under gfm: true, breaks: true', {
  op: 'parse',
  markdown: benchGfmMd,
  options: { gfm: true, breaks: true, pedantic: false },
})

// ---------------------------------------------------------------------------------------------
// Write unicode-classes.json and corpus.json.

const unicodeClasses = buildUnicodeClasses()
writeFileSync(resolve(here, 'unicode-classes.json'), `${JSON.stringify(unicodeClasses, null, 2)}\n`)

const corpus = {
  schema: 1,
  library: 'marked',
  cases,
}
const corpusJson = `${JSON.stringify(corpus, null, 2)}\n`
writeFileSync(resolve(here, 'corpus.json'), corpusJson)

console.log(
  `wrote ${cases.length} cases to corpus.json (${(corpusJson.length / 1024).toFixed(1)} KiB)`
  + `; bench-parse=${benchParseMd.length} chars, bench-gfm=${benchGfmMd.length} chars`
  + `; unicode-classes.json written`,
)

// ---------------------------------------------------------------------------------------------
// --verify: run reference.mjs on every case, check all invariants & mutant killers & bench-variant.

if (process.argv.includes('--verify')) {
  const { run } = await import('./reference.mjs')
  const { variant } = await import('./bench-variant.mjs')

  const assert = (cond, msg) => {
    if (!cond) throw new Error(`VERIFY FAILED: ${msg}`)
  }

  // Check unique ids and well-formed UTF-16 (no lone surrogates)
  const seenIds = new Set()
  const outputs = new Map()
  for (const c of cases) {
    assert(!seenIds.has(c.id), `duplicate case id ${c.id}`)
    seenIds.add(c.id)
    const md = c.input.markdown
    for (let i = 0; i < md.length; i++) {
      const code = md.charCodeAt(i)
      if (code >= 0xd800 && code <= 0xdbff) {
        const next = i + 1 < md.length ? md.charCodeAt(i + 1) : 0
        assert(next >= 0xdc00 && next <= 0xdfff, `lone high surrogate in case ${c.id}`)
        i++
      } else {
        assert(!(code >= 0xdc00 && code <= 0xdfff), `lone low surrogate in case ${c.id}`)
      }
    }
    const out = run(c.input)
    assert(typeof out === 'string', `case ${c.id} did not return string`)
    outputs.set(c.id, out)
  }

  // Mutant killers: every option toggle produces strictly distinct output
  assert(outputs.get('opts-gfm-true') !== outputs.get('opts-gfm-false'), 'gfm true vs false must differ')
  assert(outputs.get('opts-breaks-true') !== outputs.get('opts-breaks-false'), 'breaks true vs false must differ')
  assert(outputs.get('opts-pedantic-true') !== outputs.get('opts-pedantic-false'), 'pedantic true vs false must differ')
  assert(
    outputs.get('opts-gfm-false-breaks-true') === run({ op: 'parse', markdown: breaksProbeDoc, options: { gfm: false, breaks: false } }),
    'breaks: true must have no effect when gfm: false',
  )
  assert(
    outputs.get('opts-pedantic-true') === run({ op: 'parse', markdown: pedanticProbeDoc, options: { pedantic: true } }),
    'pedantic: true must override gfm & breaks',
  )
  assert(
    outputs.get('parse-inline-basic') !== run({ op: 'parse', markdown: cases.find((c) => c.id === 'parse-inline-basic').input.markdown }),
    'parseInline must differ from parse',
  )
  assert(outputs.get('opts-gfm-true').includes('<table>'), 'opts-gfm-true must emit <table>')
  assert(!outputs.get('opts-gfm-false').includes('<table>'), 'opts-gfm-false must not emit <table>')
  assert(outputs.get('opts-gfm-true').includes('<del>'), 'opts-gfm-true must emit <del>')
  assert(outputs.get('opts-gfm-true').includes('type="checkbox"'), 'opts-gfm-true must emit checkbox')
  assert(outputs.get('opts-pedantic-gfm-true-task').includes('type="checkbox"'), 'pedantic: true + gfm: true must STILL emit task checkbox')
  assert(!outputs.get('opts-pedantic-gfm-false-task').includes('type="checkbox"'), 'pedantic: true + gfm: false must NOT emit task checkbox')
  assert(outputs.get('opts-breaks-true').includes('<br>'), 'opts-breaks-true must emit <br>')
  assert(!outputs.get('opts-breaks-false').includes('<br>'), 'opts-breaks-false must not emit <br>')
  assert(!outputs.get('parse-inline-trailing-br-ws').includes('<br>'), 'parse-inline-trailing-br-ws must not emit <br>')
  assert(outputs.get('table-header-only').includes('<thead>') && !outputs.get('table-header-only').includes('<tbody>'), 'table-header-only must omit <tbody>')
  assert(outputs.get('list-task-loose').includes('<p><input '), 'list-task-loose must place checkbox inside <p>')
  assert(outputs.get('list-task-tight').includes('<li><input '), 'list-task-tight must place checkbox inside <li>')
  // Regression cases keep the behaviour they were added for
  assert(!outputs.get('regress-dot-heading-u2028').includes('<h1>'), 'U+2028 must stop the ATX heading regex')
  assert(outputs.get('regress-dot-heading-cr') === '<h1>a</h1>\n<p>b</p>\n', 'parse must normalise a lone \\r before lexing')
  assert(!outputs.get('regress-dot-lheading-u2029').includes('<h1>'), 'U+2029 must stop the setext heading regex')
  assert(outputs.get('regress-dot-table-row-u2028').includes('</table>\n<p>'), 'a U+2028 row must end the table')
  assert(!outputs.get('regress-dot-pedantic-inline-link-cr').includes('<a '), 'pedantic link must not cross \\r')
  assert(outputs.get('regress-utf16-list-lazy-astral').endsWith('</ul>\n<p>a</p>\n'), 'astral lazy line must end the item by UTF-16 indentation')
  assert(!outputs.get('regress-utf16-bounded-label').includes('<em>'), 'a label over 999 code units must not be masked')
  assert(!outputs.get('regress-utf16-bounded-label-edge').includes('<em>'), 'a 1,000-unit label must not be masked')
  assert(outputs.get('regress-lheading-table-lone-pipe-hr') === '<p>|</p>\n<hr>\n', 'a lone pipe must interrupt the setext heading')
  assert(outputs.get('regress-pedantic-href-title-backtrack').includes('title="t"'), 'pedanticHrefTitle must backtrack')
  assert(outputs.get('regress-pedantic-def-empty-angle') === '', 'pedantic `[a]: <>` must be a definition')
  assert(outputs.get('regress-nolink-empty-ref-paren').startsWith('<p><em>'), 'nolink must backtrack `[]` before `(`')
  assert(outputs.get('regress-html-begin-ul').startsWith('<ul>\n<li>a</li>\n</ul>\n<ul>'), 'a `<ul>` line must end the list item')
  assert(outputs.get('regress-size-atx-headings').split('<h1>').length === 5001, 'size case must render 5,000 headings')
  assert(outputs.get('regress-size-emphasis').split('<em>').length === 5001, 'size case must render 5,000 emphasis runs')
  assert(outputs.get('regress-depth-list-100').split('<ul>').length === 101, 'depth case must render 100 nested lists')
  assert(outputs.get('regress-depth-blockquote-100').split('<blockquote>').length === 101, 'depth case must render 100 nested blockquotes')
  const inlineLevels = (html) => ['<em>', '<strong>', '<del>', '<a '].reduce((sum, tag) => sum + html.split(tag).length - 1, 0)
  assert(inlineLevels(outputs.get('regress-depth-inline-100')) === 100, 'depth case must render 100 nested inline containers')
  assert(outputs.get('regress-depth-list-100-inline-100').split('<ul>').length === 101, 'depth case must render 100 nested lists')
  assert(inlineLevels(outputs.get('regress-depth-list-100-inline-100')) === 100, 'depth case must render 100 nested inline containers')
  assert(outputs.get('images-text-renderer').includes('alt="bold em code del lnk sub &lt;b&gt;h&lt;/b&gt; ab * &amp; A &quot;q&quot;"'), `unexpected TextRenderer alt: ${outputs.get('images-text-renderer')}`)

  // Verify bench-variant for k = 1..50 on both bench cases
  for (const benchId of ['bench-parse', 'bench-gfm']) {
    const baseCase = cases.find((c) => c.id === benchId)
    const baseOut = outputs.get(benchId)
    const inHashes = new Set([createHash('sha256').update(baseCase.input.markdown).digest('hex')])
    const outHashes = new Set([createHash('sha256').update(baseOut).digest('hex')])
    for (let k = 1; k <= 50; k++) {
      const v = variant(baseCase.input, k)
      assert(v.markdown.length === baseCase.input.markdown.length, `${benchId} k=${k} input length drifted`)
      inHashes.add(createHash('sha256').update(v.markdown).digest('hex'))
      const vOut = run(v)
      assert(vOut.length === baseOut.length, `${benchId} k=${k} output length drifted: ${vOut.length} vs ${baseOut.length}`)
      outHashes.add(createHash('sha256').update(vOut).digest('hex'))
      // Ensure no 256-char window remained identical to base
      const md = baseCase.input.markdown
      for (let off = 0; off + 256 <= md.length; off += 256) {
        assert(v.markdown.slice(off, off + 256) !== md.slice(off, off + 256), `${benchId} k=${k} unchanged 256-char window at ${off}`)
      }
    }
    assert(inHashes.size === 51, `${benchId}: expected 51 distinct inputs (base + 50 variants), got ${inHashes.size}`)
    assert(outHashes.size === 51, `${benchId}: expected 51 distinct outputs (base + 50 variants), got ${outHashes.size}`)

    // Quick timing check in JS
    for (let w = 0; w < 5; w++) run(baseCase.input)
    const times = []
    for (let r = 0; r < 20; r++) {
      const t0 = performance.now()
      run(baseCase.input)
      times.push(performance.now() - t0)
    }
    times.sort((a, b) => a - b)
    const p50 = times[Math.floor(times.length / 2)]
    console.log(`  verify ${benchId}: md=${baseCase.input.markdown.length}B html=${baseOut.length}B variants=50/50 exact-len p50=${p50.toFixed(2)}ms`)
  }

  console.log(`VERIFY PASSED: all ${cases.length} cases, option mutant-killers, and 50x bench variants OK.`)
}
