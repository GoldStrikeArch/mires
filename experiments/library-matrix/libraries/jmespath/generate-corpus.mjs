// Supply a checkout of jmespath/jmespath.test at the revision in CONTRACT.md.
import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
const dir = process.argv[2]
if (!dir) throw new Error('expected the path to the compliance checkout')
const cases = []
for (const file of readdirSync(join(dir, 'tests')).sort()) {
  let i = 0
  const groups = JSON.parse(readFileSync(join(dir,'tests',file)))
  for (const { given, cases: group } of groups) {
    const queries = []
    const expected = []
    for (const c of group) {
      if (c.bench) continue
      queries.push({op:'search',expression:c.expression},{op:'compile',expression:c.expression})
      expected.push(c.error ? {error:c.error} : {value:c.result})
    }
    if (queries.length) cases.push({id:file.replace('.json','')+'-'+(++i),input:{data:given,queries},expected})
  }
}
const boundaryData = JSON.parse('{"foo":["a","b"],"__proto__":7,"~__proto__":{"__proto__":[1]}}')
const boundaryQueries = ['@', 'to_string(@)', 'keys(@)', '"__proto__"', '"~__proto__"',
  'keys({__proto__:foo[0]})', 'keys(merge(@))',
  'foo[0:4294967296]', 'foo[-4294967296:]', 'foo[4294967296]', 'foo[-4294967296]',
  'foo[::-4294967296]', 'foo[::4294967296]', 'foo[4294967296:-4294967296:-1]']
cases.push({id:'property-and-subscript-boundaries',input:{data:boundaryData,
  queries:boundaryQueries.flatMap(expression=>['search','compile'].map(op=>({op,expression})))}})

// Pre-merge review (2026-11-20): the findings' failing inputs and the behaviours of 0.16.0 found
// while rewriting the ports. CONTRACT.md "Corpus" lists what each case exercises. JSON has no -0,
// so signed zeros come from literals.
const both = expressions => expressions.flatMap(expression => ['search','compile'].map(op => ({op, expression})))
const searches = expressions => expressions.map(expression => ({op:'search', expression}))
const review = (id, note, data, queries) => cases.push({id:'review-'+id, note, input:{data, queries}})
review('filter-two-pass', 'J1: every filter condition before any projection, so the first error is npm\'s',
  [1, '1', {a: 1}], searches(['[?abs(@) == `1`].nope(@)', '[?@ == `1`].abs(@)', '[?nope(@)]', '[?abs(@)].nope(@)',
    '[*].nope(@)', 'sort_by(@, &abs(@))', 'map(&nope(@), @)', 'map(&abs(@), `[]`)', '*.abs(@)', '[].abs(@)']))
review('filter-two-pass-mixed', 'J1, the review\'s input', [1, '1'], searches(['[?abs(@) == `1`].nope(@)']))
review('math-max-min', 'J2, B1: Math.max/Math.min with signed zeros, ±Infinity and NaN; J7: Math.abs(-0)', {},
  searches(['max(`[-0, 0]`)', 'min(`[0, -0]`)', 'max(`[0, -0]`)', 'min(`[-0, 0]`)', 'max(`[-0]`)', 'min(`[-0]`)',
    'max([`1`, avg(`[]`)])', 'min([`1`, avg(`[]`)])', 'max([avg(`[]`), `1`])', 'min([`1`, avg(`[]`), `0`])', 'max([avg(`[]`)])',
    'max([`1e400`, avg(`[]`)])', 'max([avg(`[]`), `1e400`])', 'min([`-1e400`, avg(`[]`)])', 'min([avg(`[]`), `-1e400`])',
    'max([`-1e400`, avg(`[]`)])', 'min([`1e400`, avg(`[]`)])',
    'max(`[1, 2]`)', 'abs(`-0`)', 'abs(`-1e400`)', 'ceil(`-0.5`)', 'floor(`-0.5`)', 'sum(`[-0]`)', 'sum(`[-0, -0]`)', 'avg(`[-0]`)',
    'max(`["a", "B", "b", "A", "é", "e"]`)', 'min(`["a", "B", "b", "A", "é", "e"]`)']))
review('undefined-results', 'J3: type() and other functions of an empty max_by/min_by (undefined)', {},
  searches(['type(max_by(`[]`, &a))', 'type(min_by(`[]`, &a))', 'max_by(`["abc"]`, &@)', 'type(max_by(`["abc"]`, &@))',
    'max_by(`[[]]`, &avg(@))', 'min_by(`[[], [1]]`, &avg(@))', 'not_null(max_by(`[]`, &a))', 'to_string(max_by(`[]`,&a))',
    'to_array(max_by(`[]`,&a))', 'max_by(`[]`,&a) | [a]', 'max_by(`[]`,&a) | {a: a}', '!max_by(`[]`,&a)', 'max_by(`[]`,&a) || `1`',
    'max_by(`[]`,&a) && `1`', 'max_by(`[]`,&a) == `null`', 'max_by(`[]`,&a) < `1`', '{a: max_by(`[]`, &x), b: `1`}',
    'max_by(`[1, "2", 3]`, &@)', 'min_by(`["5","10","abc","7"]`, &@)', 'max_by(`[{"k":-0},{"k":0}]`, &k)']))
const whiteSpace = [' ', '\t', '\n', '\v', '\f', '\r', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ',
  '　', '﻿', '\u001c', '\u001f', '\u0085', '᠎', '​']
review('literal-trim-left', 'J4: a backtick literal loses JS trimLeft\'s whitespace (WhiteSpace and LineTerminator), only that', {},
  searches(whiteSpace.flatMap(w => ['true', '1', '[1]', 'abc'].map(body => '`' + w + body + '`'))))
review('wide-2000-records', 'J6, the review\'s input: a projection over 2,000 records', {items: Array.from({length: 2000}, (_, i) => ({id: i}))},
  searches(['items[*].id']))
review('expref-tag', 'Objects tagged jmespathType "Expref" are exprefs to type checks and objects to strictDeepEqual and isFalse',
  {ex: {jmespathType: 'Expref'}, ey: {jmespathType: 'Expref'}, ez: {a: [1], jmespathType: 'Expref'}, ew: {jmespathType: 'Expref', a: [1]}},
  searches(['ex == `{"jmespathType":"Expref"}`', 'ex == ey', 'ez == ew', 'type(ex)', 'length(ex)', 'keys(ex)', 'merge(ex)', '!ex', 'ex.*', 'ez.a',
    'to_string(ex)', 'contains([ex], ey)', 'contains([ex], ex)', 'type(&a)', 'to_string(&a)', '&a', '[&a, &b.c]', 'map(&a, `[{"a":1}]`)']))
review('slice-numbers', 'Slices and indices in JS numbers: parseInt tokens that are NaN ("-") or past int32 and 2^53',
  [1, 2, 3], searches(['[0:3:-]', '[3:0:-]', '[2:0:-]', '[-:]', '[0:-:1]', '[-:-:-]', '[:-:-1]', '[3:0:-] | [0]', '[3:0:-][0]',
    '[1:3:2147483648]', '[-1::-2147483648]', '[::99999999999999999999]', '[::-99999999999999999999]', '[0:1:' + '9'.repeat(400) + ']',
    '[::' + '9'.repeat(400) + ']', '[::-' + '9'.repeat(400) + ']', '[' + '9'.repeat(400) + ']', '[-' + '9'.repeat(400) + ']',
    '[9007199254740993]', '[-9007199254740993:]', '[:4294967297]', '[2147483648:]', '[-2147483649:]', '[0:5:2147483648]',
    '[12345678901234567890123]', '[--]', '[1-1]', '[-0]', '[-0:]', '[::-0]', '[007]']))
review('slice-numbers-ast', 'The ASTs of those tokens', {}, both(['[0:3:-]', '[-:]', '[9007199254740993]', '[--]', '[1-1]', '[-0]',
  '[' + '9'.repeat(30) + ']', '[' + '9'.repeat(400) + ']', '[::-0]']))
review('projection-binding-power', 'bindingPower has no Literal or Colon entry, so a projection followed by a literal is a syntax error',
  {a: [1, 2]}, both(['[a[*] `1`]', '(a[*] `1`)', 'not_null(a[*] `1`)', 'abs(a[*] `1`)', '[a[*] :]', 'a[*] `1`', '[a.* `1`]',
    '[a[] `1`]', '[a[?@] `1`]', "[a[*] 'x']", '[a[*]] `1`', '{x: a[*] `1`}', 'a[*] | `1`']))
review('deep-equal-undefined', 'strictDeepEqual compares own keys of the first against the second\'s values, so undefined equals missing', {},
  searches(['{a: max_by(`[]`, &x)} == `{}`', '`{}` == {a: max_by(`[]`, &x)}', '{a: max_by(`[]`, &x)} == {b: max_by(`[]`, &x)}',
    '{a: max_by(`[]`, &x)} != `{}`', '[max_by(`[]`, &x)] == [max_by(`[]`, &y)]', '`{"a":1,"b":2}` == `{"b":2,"a":1}`', '`[1]` == `{"0":1}`']))
review('merge-for-in', 'merge type-checks its first argument only, and for-in walks strings and arrays by index', {},
  searches(["merge(`{}`, 'ab')", 'merge(`{}`, `[5,6]`)', 'merge(`{}`, `1`)', 'merge(`{}`, &a)', 'merge(`{"a":1}`, `null`)',
    'merge(`{"a":1}`, `true`, `{"b":2}`)', 'merge(`{"b":1}`, `{"2":2,"a":3,"1":4}`)']))
review('sort-by-engine', 'sort_by sorts with the engine\'s Array.prototype.sort: with NaN keys its comparator is inconsistent, and '
  + 'npm\'s own result differs between V8 and SpiderMonkey', {xs: Array.from({length: 40}, (_, i) => (i % 3 ? [(i * 7) % 11] : []))},
  searches(['sort_by(`[[], [1], [], [3]]`, &avg(@))', 'sort_by(`[[2], [], [1], [], [3], [0], []]`, &avg(@))',
    'sort_by(`[[5], [], [4], [3], [], [2], [1], [], [0], [9], [], [8]]`, &avg(@))', 'sort_by(xs, &avg(@))', 'sort_by(`[3, 1, 2]`, &@)',
    'sort_by(`["b", "a", "B"]`, &@)', 'sort_by(`[1, "a"]`, &@)', 'sort_by(`[null, 1]`, &@)', 'sort_by(`[1, null]`, &@)',
    'sort_by(`[[1], 2]`, &@[::0])', 'sort_by(`[2, [1]]`, &@[::0])', 'sort(`[10, 9, 1, -1, 1e21, -0, 0, 0.5]`)', 'sort(`["😀", "\\uffff", "a", "B"]`)']))
review('contains-identity', 'contains on arrays is indexOf: === finds only the same array or object; on strings it is String(x)',
  {mix: [1, '1', null, true, [1], {a: 1}]},
  searches(['contains(mix, mix[5])', 'contains(mix, mix[4])', 'contains(mix, `{"a":1}`)', 'contains(mix, `[1]`)', 'contains(mix, `"1"`)',
    'contains(mix, `null`)', 'map(&`{}`, `[1,2]`) | contains(@, @[0])', 'map(&{a: @}, `[1]`) | contains(@, @[0])', 'contains(`[{}]`, `{}`)',
    'contains([`{}`], `{}`)', 'contains(`[-0]`, `0`)', 'contains([avg(`[]`)], avg(`[]`))', "contains('a1c', `1`)", "contains('anull', `null`)",
    "contains('a1,2', `[1,2]`)", "contains('x[object Object]', `{}`)", "contains('xundefined', max_by(`[]`, &a))"]))
review('string-functions', 'starts_with is lastIndexOf(prefix) === 0; ends_with is indexOf from length - suffix length', {s: 'aa'},
  searches(["starts_with('a', '')", "starts_with('', '')", "starts_with('abab', 'ab')", "starts_with(s, 'a')", "ends_with('a', 'abc')",
    "ends_with('', '')", "ends_with('aXa', 'a')", 'length(`"😀"`)', 'reverse(`"a😀b"`)', 'to_number(`" 0x1F "`)', 'to_number(`"\\u00a01"`)',
    'to_number(`"0b101"`)', 'to_number(`"-0x10"`)', 'to_number(`"1_0"`)']))
review('numbers-past-2-53', 'Hexadecimal, octal and binary strings past 2^53 round once, to the nearest double', {},
  searches(['to_number(`"0x20000000000001"`)', 'to_number(`"0x200000000000011"`)', 'to_number(`"0x1fffffffffffffff"`)',
    'to_number(`"0b' + '1'.repeat(60) + '"`)', 'to_number(`"0o7777777777777777777"`)', 'to_number(`"0x' + 'f'.repeat(300) + '"`)',
    'to_number(`"9007199254740993"`)', '`"0x200000000000011"` > `144115188075855888`', '`"0x200000000000011"` < `144115188075855889`',
    '`1e400`', 'to_string(`[1e21, 1e-7, -0, 0.1, 5e-324, 123456789012345680000]`)']))
review('lexer', 'Lexer corners: unterminated raw strings and literals, a lone =, escapes, the first-only replacement', {},
  both(["'abc", "'ab\\", '`foo', '`', '``', '`"a`', '"a', '"a\\"', 'a = b', 'a =', '=', '!', '<', 'a\rb', "'\\''", "'a\\'b\\'c'",
    '`a\\`b\\`c`', '`-`', '`-1`', '`01`', '`1 2`', '`tru`', '`true `', '`[1,]`', '`{"a":1,"a":2}`', '`{"1":1,"a":2,"0":3}`']))
review('parser', 'Parser corners: undefined nodes, calls on non-identifiers, optional commas, @ arguments', {a: [1, 2, {b: 1, c: 2}, [3]], b: 'x'},
  both(['()', '(())', '()()', '(a)(b)', 'a.b(c)', '(a == b)(c)', '@(foo)', 'a[0](b)', '"a"(b)', 'a.', 'a.1', 'a.(b)', '(a.)', '[a.]',
    'abs(@.a)', '(@.a)', 'length(@ | a)', 'not_null(@, @)', '{a: b c: d}', '{a: b, }', '[a b]', '[a,]', 'a *', 'a !b', 'a {b: c}', '&()',
    '[:::]', '[::::]', '[0:1:2:3]', '[1 2]', '[:1 2]', '[*, a]', '[a, *]']))
// The nesting bound (CONTRACT "Parity domain"): every construct at 500 levels, expressions and data.
let nestedObject = 1, nestedArray = 1, nestedArrayCopy = 1
for (let i = 0; i < 500; i++) { nestedObject = {a: nestedObject}; nestedArray = [nestedArray]; nestedArrayCopy = [nestedArrayCopy] }
const nest = (n, piece) => Array(n).fill(piece).join('')
review('nesting-500', 'Every construct nested 500 levels: the bound in CONTRACT.md', {o: nestedObject, x: nestedArray, y: nestedArrayCopy},
  [...both(['o' + nest(499, '.a'), nest(500, '(') + 'a' + nest(500, ')'), nest(500, '!') + 'a', Array(500).fill('a').join(' || '),
    Array(500).fill('a').join(' && '), Array(500).fill('@').join(' | '), nest(500, '[') + 'a' + nest(500, ']'), nest(500, '{a:') + 'a' + nest(500, '}'),
    'x' + nest(500, '[0]'), 'x' + nest(500, '[]'), 'x' + nest(500, '[*]'), 'x' + nest(500, '[?@]'), 'o' + nest(500, '.*'), 'x' + nest(500, '[::]'),
    nest(500, '&') + '@', nest(250, 'map(&') + '@' + nest(250, ', `[1]`)'), nest(250, 'max_by(`[1]`, &') + '@' + nest(250, ')'),
    nest(250, 'sort_by(`[1]`, &') + '@' + nest(250, ')'), '`' + nest(500, '[') + nest(500, ']') + '`',
    nest(500, 'abs(') + '`1`' + nest(500, ')'), Array(500).fill('a').join(' == ')]),
  ...searches(['length(x)', 'x == y', 'o == o.a', 'to_string(x)', 'x', '[x]', 'x[]', '!o', 'x < `1`', 'keys(o)'])])
// Wide input: 20,000 records through projections, filters, sorts, slices and functions.
const records = Array.from({length: 20000}, (_, i) => ({id: i, name: 'n' + ((i * 7919) % 20000), k: (i * 104729) % 20000}))
review('wide-20000-records', 'Projections, filters, sorts, slices and functions over 20,000 records', {items: records},
  searches(['items[*].id', 'items[?k < `10000`].id', 'sort_by(items, &name)[0:20]', 'sort_by(items, &k)[*].id', 'map(&k, items)',
    'items[::-1][0:10]', 'items[1:-1:7][*].k', 'max_by(items, &k).id', 'min_by(items, &name).id', 'length(items[].name)', 'max(items[*].k)',
    'min(items[*].k)', 'sum(items[*].k)', 'sort(items[*].name)[0:5]', 'contains(items, items[19999])', 'items[*].{a: id}[-3:]',
    'to_string(items[0:2])', 'keys(items[0])', 'length(items[?contains(name, `"99"`)])']))
const data = {items: Array.from({length:1024},(_,i)=>({
  rank: (i*517)%1024, name: 'record-' + i, detail: ('entry '+i+' 日本語 | ').repeat(8), group: i%17
}))}
const queries = Array.from({length:100},(_,i)=>({op:'search',expression:i%2 ?
  'sum(items[?group == `'+(i%17)+'`].rank)' : 'items['+(i*17%1024)+'].name'}))
cases.push({id:'bench-search',input:{data,queries}})
writeFileSync(new URL('corpus.json',import.meta.url), JSON.stringify({schema:1,library:'jmespath',cases})+'\n')
