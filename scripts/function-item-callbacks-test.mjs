import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { readArtifactManifest } from './artifact-manifest.mjs';

const program = resolve(process.argv[2]);
readArtifactManifest(`${program}.artifacts.json`);
const api = await import(pathToFileURL(program));
const sentinel = Object.freeze({ sentinel: true });
const trace = [];
globalThis.callbackFailure = sentinel;
globalThis.callbackProbe = (...args) => { trace.push(args); return args[1]; };
let checks = 0;
for (const [name, arity, args, expected, label] of [
  ['external_zero', 0, [], 7, 'zero'],
  ['external_one', 1, [sentinel], sentinel, 'one'],
  ['external_two', 2, [sentinel, 23], sentinel, 'two'],
]) {
  const callback = api[name]();
  assert.equal(callback.length, arity);
  assert.equal(trace.length, 0);
  assert.equal(callback(...args), expected);
  assert.deepEqual(trace.splice(0), [label === 'zero' ? [label, 7, undefined] : [label, args[0], args[1]]]);
  checks += 4;
}
assert.throws(() => api.external_throwing()(), error => error === sentinel);
assert.equal(api.caught_external(), sentinel);
assert.equal(api.successful_external(), 7);
assert.deepEqual(trace.splice(0), [['zero', 7, undefined]]);
assert.equal(api.generic_external()(), 17);
assert.equal(api.local_item()(sentinel), sentinel);
assert.equal(api.captured(sentinel)(), sentinel);
checks += 7;
if (program.includes('/jir-')) {
  const report = JSON.parse(await readFile(`${program}.lane.json`, 'utf8'));
  assert.equal(report.fallback_count, 0);
}
await writeFile(`${program}.test.json`, JSON.stringify({ passed: true, checks }) + '\n');
console.log(`function item callbacks: ${checks} checks passed`);
