import { execFileSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { run } from './reference.mjs'

const cases = []
function add(id, input) { cases.push({ id, input }); return input }
function texts(id, op, before, after, options = {}) {
  return add(id, { op, before, after, options })
}
// Inputs adapted from jsdiff v9.0.0's test/diff/character.js and word.js.
const pairs = [
  ['', ''], ['', 'xx'], ['xx', ''], ['abcd', 'bcda'], ['abbaabaa', 'baabab'],
  ['Old Value.', 'New ValueMoreData.'], ['𝟘𝟙𝟚𝟛', '𝟘𝟙𝟚𝟜𝟝𝟞'],
  ['héllo WÖRLD', 'héllo wörld'], ["𠜎,𠜎. x", "𠜎! 𠜎. x"],
  ['Foo   Bar', 'Foo '], ['foo\nbar baz', 'foo baz'], ['Even    More    Value', 'Even Value'],
  ['Even Value', 'Even    More    Value'], ['New:Value:Test', 'New,Value,More,Data '],
  ['New Value End', 'New  ValueMoreData End'], ['\t Value', 'More  Value'],
  ['a\u3000b c', 'a\u3000c'], ['alpha\r\n\r\nomega', 'alpha\r\n\r\nend'],
]
for (const [i,[before,after]] of pairs.entries()) {
  for(const op of ['diffChars','diffWords']) {
    texts(op+'-'+i,op,before,after)
    if(i%2) texts(op+'-individual-'+i,op,before,after,{oneChangePerToken:true})
    if(i%3===1) texts(op+'-case-'+i,op,before,after,{ignoreCase:true})
  }
}
const linePairs = [
  ['',''], ['', 'x\n'], ['foo\n','foo'], ['\n','\n\n'], ['a\nb\n','b\na\n'],
  ['a\r\nB\r\nx','A\nb\nY'], [' a \n\n\tB\n','a\n\nb\n'],
]
for (const [i,pair] of linePairs.entries()) {
  for (const [j,opts] of [{},{ignoreCase:true},{stripTrailingCr:true},{newlineIsToken:true},
    {ignoreWhitespace:true,newlineIsToken:true},{ignoreNewlineAtEof:true},{oneChangePerToken:true}].entries())
    texts('lines-'+i+'-'+j,'diffLines',...pair,opts)
}
for (const [i,pair] of [
  [[],[]],[[0,false,null,'',1],[false,0,null,'',1]],[['x','x',3,1.5],['x',3,'x',true]],
].entries()) texts('arrays-'+i,'diffArrays',...pair)
for (const [i,pair] of [
  [{z:true,a:[1,2]},{a:[1,2],z:true}],[{nested:{key:'ok'}},{nested:{tail:2,key:'ok'}}],
  [[1,'a',false],[1,null,false,2]],[{10:3,2:1,k:5},{2:1,10:3,k:9}],
  ['{\n  "a": 2\n}\n','{\n  "a": 2,\n  "b": 3\n}\n'],
  [{𐀀:2,'':1,a:'x'},{a:'x',𐀀:3,'':1}],
].entries()) texts('json-'+i,'diffJson',...pair)
// Keep raw JS strings across the JSON/host boundary, including code units
// that cannot be represented as Unicode scalars.
texts('utf16-low-surrogate', 'diffChars', '', '\udc00')
const beforeUnits = '\ufeff\ud800a\udfff\ue800😀\ufffd\nlast\udbff'
const afterUnits = '\ufeff\udbffb\udc00\ue800😃\ufffd\nlast\ud800'
for (const op of ['diffChars', 'diffWords', 'diffLines', 'diffJson']) {
  texts('utf16-'+op, op, beforeUnits, afterUnits)
  texts('utf16-individual-'+op, op, '', afterUnits, {oneChangePerToken:true,ignoreCase:true})
}
texts('utf16-arrays', 'diffArrays', ['\udc00','\ufffd'], ['\ud800','\ufffd'])
texts('utf16-json-key', 'diffJson', {['\ud800']:1}, {['\udc00']:2})
const utfPatch = {before:beforeUnits, after:afterUnits, oldFileName:'before', newFileName:'after'}
const utfText = run(add('utf16-format', {op:'createTwoFilesPatch', ...utfPatch}))
const utfObject = run(add('utf16-structure', {op:'structuredPatch', ...utfPatch}))
add('utf16-parse', {op:'parsePatch',patch:utfText})
add('utf16-reverse', {op:'reversePatch',patch:utfObject})
add('utf16-apply', {op:'applyPatch',before:beforeUnits,patch:utfText})
for (const [i, name] of ['\ud800🦊', '\udc00.\ufeffX', '\ue800.a', '\ufeffα.x', 'Z\ufeff\ufeff.txt'].entries()) {
  const formatted = run(add('utf16-format-name-'+i, {...utfPatch, op:'createTwoFilesPatch',
    oldFileName:name, newFileName:name, oldHeader:'\ud800', newHeader:'\udc00'}))
  add('utf16-parse-name-'+i,{op:'parsePatch',patch:formatted})
}
add('utf16-quoted-raw', {op:'parsePatch',patch:'--- "literal\ud800\\357\\273\\277end"\n+++ new\n'})
const base = ['Row 1','Row 2','Row 3','Row 4','Row 5','Row 6','Row 7','Row 8','Row 9','Row 10','Row 11','Row 12',''].join('\n')
const gitCwd = fileURLToPath(new URL('../../../..',import.meta.url))
const revisions = ['90e08bb93f49ab6d2ddb944aad5f5984d654020b','16aafc881eccfb27b9bf16361c2618121d622ed6']
const history = revisions.map(rev => execFileSync('git',['show',rev+':Cargo.toml'],{cwd:gitCwd,encoding:'utf8'}))
const scenarios=[
  ['', '', 4], ['', 'added\n', 0], ['deleted\n', '', 0],
  ['first\nsecond','first\nlast',3],['x','x\n',0],['x\n','x',0],['a\nb\n','x\nb\n',2],
  ['a\r\nb\r\n','a\r\nc\r\n',0],[base,base.replace('Row 2','Edited 2').replace('Row 11','Edited 11'),2],
  [base,base.replace('Row 2','Edited 2').replace('Row 7','Edited 7'),2],[...history,4]
]
for(const [i,[before,after,context]] of scenarios.entries()) {
  const oldFileName=i===9?'héllo\t.txt':'file.txt',newFileName=i%2?'new.txt':oldFileName
  const params={before,after,oldFileName,newFileName,options:{context}}
  if(i%2){params.oldHeader='old rev';params.newHeader='new rev'}
  const structure=run(add('structure-'+i,{op:'structuredPatch',...params}))
  const patch=run(add('format-'+i,{op:'createTwoFilesPatch',...params}))
  add('parse-'+i,{op:'parsePatch',patch})
  add('apply-text-'+i,{op:'applyPatch',before,patch})
  add('apply-structure-'+i,{op:'applyPatch',before,patch:structure})
  add('reverse-'+i,{op:'reversePatch',patch:structure})
  if (before.length && after.length && before!==after) {
    add('apply-shifted-'+i,{op:'applyPatch',before:'another line\n'+before,patch})
    add('apply-mismatch-'+i,{op:'applyPatch',before:'unrelated source\n',patch})
  }
}
// jsdiff v9.0.0, test/patch/readme-rename-example.js.
const rename='diff --git a/old.txt b/new.txt\nsimilarity index 80%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2modified\n line3\n'
const copy='diff --git a/old b/new\nsimilarity index 100%\ncopy from old\ncopy to new\n'
for(const [i,patch] of [rename,copy,rename+copy,'@@ -1 +1 @@\n-foo\n+bar\n',
  'diff --git a/x b/x\nold mode 100644\nnew mode 100755\n',''].entries()){
  const parsed=run(add('sample-parse-'+i,{op:'parsePatch',patch}))
  add('sample-reverse-'+i,{op:'reversePatch',patch:parsed})
}
add('invalid-lines',{op:'parsePatch',patch:'@@ -1 +1 @@\n-b\n+c\n+extra\n'})
add('invalid-file',{op:'parsePatch',patch:'--- x.txt\n'})
add('invalid-multiple',{op:'applyPatch',before:'x',patch:rename+copy})
add('unknown-op',{op:'unknown'})

// Cases from the 2026-11-20 port review. parse.js matches its header lines
// with regexes: `.` stops at \r, U+2028 and U+2029, `\d` is ASCII, `\s` is
// the ECMAScript whitespace set, and the hunk range may sit anywhere in the
// `@@` line. apply.js tries the declared position first and then walks the
// distance iterator without clamping it.
const crlf = s => s.replace(/\n/g, '\r\n')
const gitParse = (id, extended) => add(id, {op:'parsePatch', patch:'diff --git a/x b/x\n'+extended+'\n'})
for (const [id, extended] of [
  ['rename-cr', 'rename from foo\rbar\nrename to baz'], ['rename-line-separators', 'rename from a\u2028b\nrename to c\u2029d'],
  ['copy-cr', 'copy from p\rq\ncopy to r'], ['mode-trailing-text', 'new file mode 100644x'],
  ['mode-non-ascii-digits', 'new file mode \u0661\u0662'], ['mode-cr', 'old mode 0755\rjunk\nnew mode 0644'],
  ['deleted-mode', 'deleted file mode 100644'], ['binary', 'Binary files a and b differ'],
  ['quoted-rename', 'rename from "q\\rx"\nrename to "q\\101\\102"'], ['bad-escape-rename', 'rename from "bad\\x"\nrename to ok'],
]) gitParse('git-ext-'+id, extended)
const gitMode = 'diff --git a/x b/x\nold mode 100644\nnew mode 100755\n'
const gitNew = 'diff --git a/n b/n\nnew file mode 100644\nindex 0000000..e69de29\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+hi\n'
const indexed = 'Index: f.txt\n===================================================================\n--- f.txt\told\n+++ f.txt\tnew\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n'
for (const [id, patch] of Object.entries({rename, mode:gitMode, created:gitNew, indexed})) {
  add('crlf-parse-'+id, {op:'parsePatch', patch:crlf(patch)})
  add('crlf-apply-'+id, {op:'applyPatch', before:crlf('line1\nline2\nline3\n'), patch:crlf(patch)})
}
add('crlf-apply-match', {op:'applyPatch', before:'a\r\nb\r\n', patch:crlf(indexed)})
add('crlf-apply-plain', {op:'applyPatch', before:'a\r\nb\r\n', patch:crlf('--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n')})
add('crlf-hunk-only', {op:'parsePatch', patch:'--- a\n+++ b\n@@ -1 +1 @@\r\n-a\r\n+b\r\n'})
for (const [i, source] of ['a\r\nb\r\n', 'a\nb\n', 'a\r\nb\n'].entries())
  add('crlf-lines-apply-'+i, {op:'applyPatch', before:source, patch:'--- a\n+++ b\n@@ -1,2 +1,2 @@\n a\r\n-b\r\n+c\r\n'})
for (const [id, patch] of [
  ['heading', '@@ -1,2 +1,2 @@ function foo() {\n a\n-b\n+c\n'], ['text-before-range', '@@ junk @@ -1 +1 @@\n-a\n+b\n'],
  ['unmatched', '@@ x @@\n-a\n+b\n'], ['double-space', '@@  -1 +1 @@\n-a\n+b\n'], ['tab', '@@\t-1 +1 @@\n-a\n+b\n'],
  ['leading-zeros-past-int32', '@@ -007,1 +0012345678901 @@\n-a\n+b\n'],
  ['past-2-53', '@@ -9007199254740993 +99999999999999999999999 @@\n-a\n+b\n'],
  ['overflow-to-infinity', '@@ -'+'9'.repeat(400)+',0 +1,0 @@\n'], ['zero-ranges', '@@ -0,0 +1 @@\n+x\n'],
  ['deletion-only', '@@ -1 +0,0 @@\n-x\n'], ['empty-ranges', '@@ -1,0 +1,0 @@\n'],
  ['missing-newline-markers', '@@ -1 +1 @@\n-a\n+b\n\\ No newline\n\\ again\n'], ['blank-context', '@@ -1,3 +1,3 @@\n a\n\n c\n'],
  ['short-body', '@@ -1,2 +1,2 @@\n a\n'], ['extra-body', '@@ -1 +1 @@\n-a\n+b\n c\n'],
  ['next-file', '@@ -1 +1 @@\n-a\n+b\n--- next\n+++ n\n@@ -1 +1 @@\n-c\n+d\n'],
  ['underline-separator', '@@ -1 +1 @@\n-a\n+b\n===================================================================\n@@ -2 +2 @@\n-c\n+d\n'],
]) add('hunk-header-'+id, {op:'parsePatch', patch:'--- a\n+++ b\n'+patch})
add('hunk-header-heading-apply', {op:'applyPatch', before:'a\nb\n', patch:'--- a\n+++ b\n@@ -1,2 +1,2 @@ function foo() {\n a\n-b\n+c\n'})
add('hunk-header-unmatched-apply', {op:'applyPatch', before:'q\n', patch:'@@ x @@\n+b\n'})
// The index is what follows /^(?:Index:|diff(?: -r \w+)+)\s+/, trimmed.
for (const [id, patch] of [
  ['hg-spaces', 'diff -r 9117c6561b0b -r 273ce12ad8f1 my file.txt\n--- a/my file.txt\n+++ b/my file.txt\n'],
  ['hg-trailing-space', 'diff -r 9117c6561b0b file.txt \n'], ['hg-word-break', 'diff -r abc -r def-x\n'],
  ['hg-nbsp', 'diff -r ab_c\u00a0f\n'], ['hg-single', 'diff -r abc file\n'], ['hg-file-headers', 'diff -r abc -r def file.txt\n--- a\n+++ b\n'],
  ['index-tab', 'Index:\tfoo\n--- a\n+++ b\n'], ['index-nbsp', 'Index:\u00a0foo bar  \n--- a\n+++ b\n'],
  ['index-bom', 'Index:\ufefffoo\n'], ['index-line-separator', 'Index:\u2028foo\n'], ['index-trailing', 'Index: file.txt  \n'],
  ['index-twice', 'Index: a\nIndex: b\n'], ['index-no-space', 'Index:x\n--- a\n+++ b\n'],
  ['file-header-nbsp', '---\u00a0a\tb\tc\tdd\n+++\u3000b \t  h  \n'], ['file-header-tab', '---\ta\n+++\tb\n'],
  ['file-header-cr', '---\r\n+++\r\n'], ['file-header-empty', '--- \n+++ \n'], ['file-header-tabs', '--- a\t\n+++ b\t\t\n'],
  ['file-header-quoted', '--- "a\\tb"\t hdr\n+++ "c\\400d"\n'], ['file-header-backslashes', '--- a\\\\\\\\b\n+++ c\\d\n'],
  ['file-header-unterminated', '--- "unterminated\n+++ y\n'], ['file-header-escapes', '--- "a\\"b"\n+++ "c\\\\"\n'],
  ['file-header-controls', '--- "\\a\\b\\f\\v\\n"\n+++ q\n'], ['file-header-short-octal', '--- "\\1"\n+++ y\n'],
  ['file-header-octal-then-digit', '--- "\\101\\10"\n+++ x\n'],
  ['git-two-headers', 'diff --git a/x b/x\ndiff --git a/y b/y\n'], ['git-quoted-old', 'diff --git "a/q\\tx" b/y\n'],
  ['git-quoted-new', 'diff --git a/simple "b/re\\nnamed"\n'], ['git-ambiguous', 'diff --git a/a b/c b/a b/c\n'],
  ['git-three-names', 'diff --git a/x b/y b/z\n'], ['git-unprefixed', 'diff --git x y\n'],
  ['git-unterminated-new', 'diff --git "a/x" "b/y\n'], ['git-quote-tail', 'diff --git "a/x"zz\n'],
  // The skipped separator is one UTF-16 unit, which can split an astral character.
  ['git-quote-astral-tail', 'diff --git "a/x"\ud83d\ude00b/y\n'], ['git-astral-before-quote', 'diff --git a/x\ud83d\ude00"b/y"\n'],
  ['garbage-before-header', 'garbage\n--- a\n'], ['new-header-only', '+++ only\n'], ['blank-lines', '\n\n\n'],
  // A second diff header ends the file's metadata before its file headers are checked.
  ['early-return-git', 'diff --git x y\nrename from q\ndiff --git a/z b/z\n'],
  ['early-return-index', 'Index: a\n--- a\nIndex: b\n+++ b\n'],
  // Errors: a malformed second file throws after a valid first one.
  ['error-in-second-file', '--- a\n+++ a\n@@ -1 +1 @@\n-x\n+y\n--- b\n+++ b\n@@ -1 +1 @@\n-x\n+y\n+z\n'],
]) add('header-'+id, {op:'parsePatch', patch})
// TextDecoder replaces each maximal ill-formed subpart of a run of octal escapes.
for (const [i, bytes] of ['\\340\\200', '\\342\\202', '\\355\\240\\200', '\\300\\200', '\\364\\220\\200\\200', '\\360\\237\\230',
  '\\377', '\\200\\200', '\\341\\200\\101', '\\360\\200\\200\\200', '\\357\\277\\276', '\\360\\237\\230\\200\\200', '\\302',
  '\\302\\101', '\\365\\200', '\\370\\210\\200\\200\\200', '\\357\\273\\277\\357\\273\\277z', '\\377\\376x'].entries())
  add('octal-utf8-'+i, {op:'parsePatch', patch:'--- "'+bytes+'z"\n+++ q\n'})
add('octal-many-runs', {op:'parsePatch', patch:'--- "'+'\\303\\251x'.repeat(3000)+'"\n+++ q\n'})
const rows = Array.from({length:40}, (_, i) => 'L'+i).join('\n')+'\n'
const context0 = run({op:'createTwoFilesPatch', before:'a\nb\nc\n', after:'a\nb\nc\nd\n', oldFileName:'f', newFileName:'f', options:{context:0}})
for (const [id, before, patch] of [
  ['context0-short-source', 'a\n', context0], ['context0-full-source', 'a\nb\nc\n', context0],
  ['past-eof', 'a\n', '@@ -5,0 +6 @@\n+x\n'], ['past-eof-two-lines', 'a\nb\n', '@@ -3,0 +4,2 @@\n+x\n+y\n'],
  ['past-eof-context', 'a\nb\nc\n', '@@ -9,2 +9,2 @@\n b\n-c\n+C\n'], ['line-zero', 'L1\n', '@@ -0 +1 @@\n+X\n'],
  ['out-of-order', rows, '@@ -30 +30 @@\n-L29\n+Z\n@@ -5 +5 @@\n-L4\n+Y\n'], ['in-order', rows, '@@ -5 +5 @@\n-L4\n+Y\n@@ -30 +30 @@\n-L29\n+Z\n'],
  ['shifted', rows, '@@ -1 +1 @@\n-L30\n+Z\n@@ -2 +2 @@\n-L33\n+W\n'],
  ['overlap', rows, '@@ -2,3 +2,3 @@\n L1\n-L2\n+A\n L3\n@@ -4,3 +4,3 @@\n L3\n-L4\n+B\n L5\n'],
  ['far-match', 'x\n'.repeat(30)+'a\nb\nc\n'+'y\n'.repeat(30), '@@ -2,3 +2,3 @@\n a\n-b\n+B\n c\n@@ -50,3 +50,3 @@\n y\n-y\n+Y\n y\n'],
  ['tie-after', 'k\nv\nk\nv\nk\nv\n', '@@ -3,2 +3,2 @@\n k\n-v\n+V\n'], ['tie-past-end', 'k\nv\nk\nv\nk\nv\n', '@@ -9,2 +9,2 @@\n k\n-v\n+V\n'],
  ['two-files', 'x', rename+copy],
]) add('apply-search-'+id, {op:'applyPatch', before, patch})
for (const [id, before, hunk] of [
  ['start-zero', 'L1\n', {oldStart:0, oldLines:0, newStart:1, newLines:1, lines:['+X']}],
  ['start-past-eof', 'L1\n', {oldStart:50, oldLines:1, newStart:50, newLines:1, lines:['-L1', '+X']}],
  ['unknown-operations', 'L1\nL2\n', {oldStart:1, oldLines:2, newStart:1, newLines:2, lines:['-L1', '?junk', '', '+X']}],
  ['empty-context-line', '\nL2\n', {oldStart:1, oldLines:2, newStart:1, newLines:2, lines:['', '-L2', '+X']}],
]) add('apply-structure-'+id, {op:'applyPatch', before, patch:{hunks:[hunk]}})
add('apply-structure-empty-list', {op:'applyPatch', before:'q\n', patch:[]})
add('apply-structure-list', {op:'applyPatch', before:'L1\nL2\n', patch:[{hunks:[{oldStart:1, oldLines:1, newStart:1, newLines:1, lines:['-L1', '+X']}]}]})
for (const [id, before, patch] of [
  ['eof-kept', 'a\nb', '@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n'],
  ['eof-mismatch', 'a\nb\n', '@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n'],
  ['eof-both', 'a\nb', '@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n'],
  ['eof-removed', 'a\nb\n', '@@ -1,2 +1,2 @@\n a\n-b\n+c\n\\ No newline at end of file\n'],
  ['eof-already-missing', 'a\nb', '@@ -1,2 +1,2 @@\n a\n-b\n+c\n\\ No newline at end of file\n'],
]) add('apply-'+id, {op:'applyPatch', before, patch})
add('reverse-missing-fields', {op:'reversePatch', patch:[{hunks:[{newStart:2, lines:['\\x', ' y', '--z', '++w'], extra:3}]}, {hunks:[], isDelete:true}]})
add('reverse-git-flags', {op:'reversePatch', patch:{oldFileName:'a/x', newFileName:'b/y', isGit:true, isCopy:true, isRename:true,
  isCreate:true, oldMode:'1', hunks:[{oldStart:1, oldLines:1, newStart:1, newLines:1, lines:['-a', '+b']}]}})
add('reverse-dev-null', {op:'reversePatch', patch:{hunks:[], extra:1, newHeader:'h', isGit:true, newFileName:'/dev/null', oldFileName:'c/x'}})
// JSON canonicalisation assigns keys to a plain object (a "__proto__" key is
// dropped), and the longest-token rule compares UTF-16 lengths.
add('json-proto-key', {op:'diffJson', before:JSON.parse('{"__proto__":{"x":1},"a":2,"b":{"__proto__":3}}'), after:{a:2}})
add('json-utf16-length', {op:'diffJson', before:'"\u212a"\n', after:'"k",\n', options:{ignoreCase:true}})
for (const w of ['\u001c', '\u0085', '\u180e', '\u1680', '\u2028', '\ufeff', '\u3000']) {
  const id = w.charCodeAt(0).toString(16)
  add('whitespace-'+id+'-words', {op:'diffWords', before:'a'+w+'b c', after:'a b c', options:{}})
  add('whitespace-'+id+'-lines', {op:'diffLines', before:'a'+w+'\nb\n', after:'a\nb\n', options:{ignoreWhitespace:true}})
  add('whitespace-'+id+'-file-header', {op:'parsePatch', patch:'--- a'+w+'\t'+w+'h'+w+'\n+++ b'+w+'\n'})
}
// Depth and width. The nesting bound is in CONTRACT.md.
const nested = (depth, wrap) => { let v = 0; for (let i = 0; i < depth; i++) v = wrap(v); return v }
add('json-depth-arrays', {op:'diffJson', before:nested(1000, v => [v]), after:nested(999, v => [v])})
add('json-depth-objects', {op:'diffJson', before:nested(1000, v => ({k:v})), after:{k:1}})
add('many-files', {op:'parsePatch', patch:Array.from({length:3000}, (_, i) => `--- f${i}\n+++ f${i}\n@@ -1 +1 @@\n-a\n+b\n`).join('')})
add('many-hunks', {op:'parsePatch', patch:'--- f\n+++ f\n'+Array.from({length:5000}, (_, i) => `@@ -${2*i+1} +${2*i+1} @@\n-a\n+b\n`).join('')})
add('long-hunk', {op:'parsePatch', patch:'--- f\n+++ f\n@@ -1,20000 +1,20000 @@\n'+' x\n'.repeat(20000)})
const manyLines = Array.from({length:6000}, (_, i) => 'line '+i+'\n').join('')
const manyEdited = manyLines.split('\n').map((l, i) => i % 9 === 0 && l ? l+'!' : l).join('\n')
add('many-hunks-apply', {op:'applyPatch', before:manyLines, patch:run({op:'createTwoFilesPatch', before:manyLines, after:manyEdited, oldFileName:'m', newFileName:'m', options:{context:1}})})
const before = Array.from({length:2000},(_,i)=>'const item'+i+' = '+(i*13)+';\n').join('')
const after = before.split('\n').map((line,i)=>line && i%20===3?line.replace(' = ',' = -'):line).join('\n')
for (const [id,op] of [['bench-lines','diffLines'],['bench-patch','createTwoFilesPatch']])
  add(id,{op,before,after,oldFileName:'program.js',newFileName:'program.js'})
writeFileSync(new URL('corpus.json',import.meta.url), JSON.stringify({schema:1,library:'diff',cases})+'\n')
