// Source statistics for tools/sizes.mjs: code lines, embedded-JS lines, eval references and a
// module-level-state lint, per language. Approximate by design (a small lexer, not a parser): it
// knows each language's comments and string literals, so markers inside comments or strings are
// not counted, and an embedded-JS construct's extent is its balanced bracket group or string
// literal, wherever the lines break. Review aids, not proofs.

export const SYNTAX = {
  rust: { line: '//', block: ['/*', '*/'], nest: true, strings: ['"'] },
  rescript: { line: '//', block: ['/*', '*/'], strings: ['"', '`'] },
  melange: { block: ['(*', '*)'], nest: true, strings: ['"'], quoted: true },
  jsoo: { block: ['(*', '*)'], nest: true, strings: ['"'], quoted: true },
  scalajs: { line: '//', block: ['/*', '*/'], strings: ['"""', '"'] },
  dart: { line: '//', block: ['/*', '*/'], nest: true, strings: ['"""', "'''", '"', "'"] },
  kotlin: { line: '//', block: ['/*', '*/'], nest: true, strings: ['"""', '"'] },
  elm: { line: '--', block: ['{-', '-}'], nest: true, strings: ['"""', '"'] },
  gopherjs: { line: '//', block: ['/*', '*/'], strings: ['"', '`'] },
  js: { line: '//', block: ['/*', '*/'], strings: ['"', "'", '`'] },
}

// Inline-JS constructs (the code they carry is counted by lines) and eval references (counted by
// occurrence), matched on code with comments blanked and outside string literals.
export const PATTERNS = {
  rust: {
    embeddedJs: [/js!\s*(?=[{([])/g],
    evalReferences: [/#\[js\([^\]]*=\s*"(?:eval|Function|globalThis\.eval)"/g],
  },
  rescript: {
    embeddedJs: [/%%?raw\s*(?=\()/g, /%%?ffi\s*(?=\()/g],
    evalReferences: [/external\s+\w+\s*:[^=]*=\s*"(?:eval|Function)"/g],
  },
  melange: {
    embeddedJs: [/\[%%?(?:mel\.)?raw\b/g],
    evalReferences: [/external\s+\w+\s*:[^=]*=\s*"(?:eval|Function)"/g],
  },
  jsoo: {
    embeddedJs: [/Js\.Unsafe\.(?:js_expr|pure_js_expr|eval_string)\b/g, /\[%js\b/g],
    evalReferences: [/Js\.Unsafe\.(?:variable|global)\b[^;\n]*\beval\b/g, /Js\.Unsafe\.eval_string\b/g, /Js\.Unsafe\.variable\s*"(?:eval|Function)"/g],
  },
  scalajs: {
    embeddedJs: [/\bjs\.eval\s*(?=\()/g, /\bnew\s+js\.Function\s*(?=\()/g],
    evalReferences: [/\bjs\.Dynamic\.global\.(?:eval|Function)\b/g, /\bjs\.eval\b/g],
  },
  dart: {
    embeddedJs: [],
    evalReferences: [/callMethod\s*\(\s*['"](?:eval|Function)['"]/g, /\[\s*['"](?:eval|Function)['"]\s*\]/g, /@JS\(\s*['"](?:eval|Function)['"]\s*\)/g],
  },
  kotlin: {
    embeddedJs: [/\bjs\s*\(\s*(?=")/g],
    evalReferences: [/\beval\s*\(/g],
  },
  elm: { embeddedJs: [], evalReferences: [] },
  gopherjs: {
    embeddedJs: [],
    evalReferences: [/\bjs\.Global\.(?:Call|Get)\s*\(\s*"(?:eval|Function)"/g],
  },
  js: { embeddedJs: [], evalReferences: [/\beval\s*\(/g, /\bnew\s+Function\s*\(/g, /\bimport\s*\(/g, /\bimport\.meta\b/g] },
}

// Code loaded from bytes or from a URL the page builds, which the module shape gate cannot see
// (WebAssembly from inline bytes is not guarded at run time either; data: and blob: modules are
// refused by the Node resolve hook and the page CSP): matched on the raw text of every source,
// string literals included since interop bindings name the API in a string, and counted as eval
// references.
const RAW_LOADS = [
  { kind: 'wasm', pattern: /\bWebAssembly\s*\.\s*(?:instantiateStreaming|instantiate|compileStreaming|compile|Module|Instance)\b/g },
  { kind: 'data-or-blob-url', pattern: /['"`](?:data|blob):/g },
  { kind: 'object-url', pattern: /\bURL\s*\.\s*createObjectURL\b/g },
]

// Module-level mutable state: a cache or counter that survives between calls (README "Port
// convention": no cross-call memoisation the reference does not do). Matched line by line on code
// with comments and strings blanked.
export const MODULE_STATE = {
  rust: [/^(?:pub(?:\([^)]*\))?\s+)?static\s+mut\b/, /^(?:pub(?:\([^)]*\))?\s+)?static\s+\w+\s*:\s*[^=]*(?:Cell|Mutex|RwLock|Atomic|Once|Lazy)/, /^\s*thread_local!/],
  rescript: [/^let\s+\w+(?:\s*:\s*[^=]+)?\s*=\s*(?:ref\(|Map\.make|Dict\.make|Set\.make|WeakMap\.make|Belt\.(?:MutableMap|HashMap|MutableSet)|Hashtbl\.create|\[\s*\])/],
  melange: [/^let\s+\w+(?:\s*:\s*[^=]+)?\s*=\s*(?:ref\b|Hashtbl\.create|Js\.Map\.make|Js\.Dict\.empty|Array\.make|Buffer\.create|Queue\.create|Stack\.create|\[\|\s*\|\])/],
  jsoo: [/^let\s+\w+(?:\s*:\s*[^=]+)?\s*=\s*(?:ref\b|Hashtbl\.create|Array\.make|Buffer\.create|Queue\.create|Stack\.create|\[\|\s*\|\])/],
  scalajs: [/^\s{0,2}(?:private\s+|protected\s+)?var\s+\w+/, /^\s{0,2}(?:private\s+|protected\s+)?val\s+\w+[^=]*=\s*(?:mutable\.|scala\.collection\.mutable|collection\.mutable|new\s+java\.util|js\.Dictionary|js\.Map|new\s+js\.Map)/],
  dart: [/^(?:late\s+)?(?:var|final)\s+\w+\s*=/, /^(?:late\s+)?(?:final\s+)?(?:Map|Set|List|HashMap|LinkedHashMap)\s*<[^>]*>\s+\w+\s*=/],
  kotlin: [/^(?:private\s+|internal\s+)?var\s+\w+/, /^(?:private\s+|internal\s+)?val\s+\w+[^=]*=\s*(?:mutableMapOf|mutableListOf|mutableSetOf|HashMap|LinkedHashMap|ArrayList|HashSet|LinkedHashSet)\b/],
  elm: [],
  gopherjs: [/^var\s+(?:\w+|\()/],
  js: [/^(?:let|var)\s+\w+/, /^const\s+\w+\s*=\s*(?:new\s+(?:Map|Set|WeakMap|WeakSet)\b|\[|\{)/],
}

// Comments blanked (newlines kept) and a per-character mask of string-literal contents.
export function lex(text, syntax) {
  const out = text.split('')
  const inString = new Uint8Array(text.length)
  let index = 0
  const blank = (from, to) => {
    for (let at = from; at < to; at++) if (out[at] !== '\n') out[at] = ' '
  }
  while (index < text.length) {
    if (syntax.line && text.startsWith(syntax.line, index)) {
      const end = text.indexOf('\n', index)
      const stop = end === -1 ? text.length : end
      blank(index, stop)
      index = stop
      continue
    }
    if (syntax.block && text.startsWith(syntax.block[0], index)) {
      let depth = 0
      let at = index
      while (at < text.length) {
        if (text.startsWith(syntax.block[0], at) && (depth === 0 || syntax.nest)) {
          depth++
          at += syntax.block[0].length
        } else if (text.startsWith(syntax.block[1], at)) {
          depth--
          at += syntax.block[1].length
          if (depth === 0) break
        } else {
          at++
        }
      }
      blank(index, at)
      index = at
      continue
    }
    if (syntax.quoted && text[index] === '{') {
      const opener = /^\{([a-z_]*)\|/.exec(text.slice(index, index + 32))
      if (opener) {
        const closer = `|${opener[1]}}`
        const end = text.indexOf(closer, index + opener[0].length)
        const stop = end === -1 ? text.length : end + closer.length
        for (let at = index; at < stop; at++) inString[at] = 1
        index = stop
        continue
      }
    }
    const quote = syntax.strings.find((candidate) => text.startsWith(candidate, index))
    if (quote) {
      let at = index + quote.length
      while (at < text.length && !text.startsWith(quote, at)) at += quote.length === 1 && text[at] === '\\' ? 2 : 1
      const stop = Math.min(text.length, at + quote.length)
      for (let mark = index; mark < stop; mark++) inString[mark] = 1
      index = stop
      continue
    }
    index++
  }
  return { code: out.join(''), inString }
}

export function codeLines(text, syntax) {
  const { code } = lex(text, syntax)
  const lines = text.split('\n')
  const blanked = code.split('\n')
  return { nonBlank: lines.filter((line) => line.trim()).length, code: blanked.filter((line) => line.trim()).length }
}

// The end of the construct that starts at `from`: a balanced bracket group (strings skipped), or
// a string literal, or the end of the line.
function constructEnd(code, inString, from) {
  let index = from
  while (index < code.length && (code[index] === ' ' || code[index] === '\t')) index++
  const pairs = { '(': ')', '[': ']', '{': '}' }
  if (pairs[code[index]] && !inString[index]) {
    const stack = [pairs[code[index]]]
    let at = index + 1
    while (at < code.length && stack.length) {
      if (!inString[at]) {
        if (pairs[code[at]]) stack.push(pairs[code[at]])
        else if (code[at] === stack.at(-1)) stack.pop()
      }
      at++
    }
    return at
  }
  if (inString[index]) {
    let at = index
    while (at < code.length && inString[at]) at++
    return at
  }
  const end = code.indexOf('\n', from)
  return end === -1 ? code.length : end
}

export function embeddedJs(text, language) {
  const syntax = SYNTAX[language]
  const patterns = PATTERNS[language] ?? { embeddedJs: [], evalReferences: [] }
  const { code, inString } = lex(text, syntax)
  const lineOf = (at) => code.slice(0, at).split('\n').length
  // One left-to-right pass over all markers, so overlapping spellings (%%raw and %raw) count once.
  const matches = []
  for (const pattern of patterns.embeddedJs) {
    for (const match of code.matchAll(new RegExp(pattern.source, 'g'))) if (!inString[match.index]) matches.push({ index: match.index, end: match.index + match[0].length, marker: match[0].trim() })
  }
  matches.sort((left, right) => left.index - right.index || right.end - left.end)
  const constructs = []
  let covered = -1
  for (const match of matches) {
    if (match.index < covered) continue
    const end = constructEnd(code, inString, match.end)
    covered = end
    const lines = lineOf(end) - lineOf(match.index) + 1
    constructs.push({ marker: match.marker, line: lineOf(match.index), lines, from: match.index, to: end })
  }
  const evalReferences = []
  // The JS an embedded construct carries is JS: its eval, Function, import() and import.meta count.
  for (const construct of constructs) {
    const body = text.slice(construct.from, construct.to)
    for (const pattern of PATTERNS.js.evalReferences) {
      for (const match of body.matchAll(new RegExp(pattern.source, 'g'))) evalReferences.push({ line: lineOf(construct.from + match.index), text: match[0].slice(0, 80), inEmbeddedJs: true })
    }
    delete construct.from
    delete construct.to
  }
  for (const pattern of patterns.evalReferences) {
    for (const match of code.matchAll(new RegExp(pattern.source, 'g'))) if (!inString[match.index]) evalReferences.push({ line: lineOf(match.index), text: match[0].slice(0, 80) })
  }
  for (const { kind, pattern } of RAW_LOADS) {
    for (const match of text.matchAll(new RegExp(pattern.source, 'g'))) {
      // Inside a comment it is prose, not a load.
      if (code.slice(match.index, match.index + match[0].length).trim() || inString[match.index]) evalReferences.push({ line: lineOf(match.index), text: match[0], kind })
    }
  }
  return { lines: constructs.reduce((sum, item) => sum + item.lines, 0), constructs, evalReferences }
}

export function moduleState(text, language) {
  const syntax = SYNTAX[language]
  const { code, inString } = lex(text, syntax)
  const plain = code.split('').map((char, at) => (inString[at] && char !== '\n' ? ' ' : char)).join('')
  const findings = []
  const original = text.split('\n')
  plain.split('\n').forEach((line, index) => {
    if ((MODULE_STATE[language] ?? []).some((pattern) => pattern.test(line))) findings.push({ line: index + 1, text: original[index].trim().slice(0, 120) })
  })
  return findings
}
