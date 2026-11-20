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
const before = Array.from({length:2000},(_,i)=>'const item'+i+' = '+(i*13)+';\n').join('')
const after = before.split('\n').map((line,i)=>line && i%20===3?line.replace(' = ',' = -'):line).join('\n')
for (const [id,op] of [['bench-lines','diffLines'],['bench-patch','createTwoFilesPatch']])
  add(id,{op,before,after,oldFileName:'program.js',newFileName:'program.js'})
writeFileSync(new URL('corpus.json',import.meta.url), JSON.stringify({schema:1,library:'diff',cases})+'\n')
