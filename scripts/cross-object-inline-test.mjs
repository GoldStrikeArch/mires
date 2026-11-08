import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { readArtifactManifest } from './artifact-manifest.mjs';

const [program, mode] = process.argv.slice(2);
readArtifactManifest(`${program}.artifacts.json`);
const api = await import(pathToFileURL(program));
for (const value of [0, 1, 7, 17, 0x7fffffff, 0xfffffffe, 0xffffffff]) {
  assert.equal(api.direct(value), (value + 7) >>> 0);
  assert.equal(api.pair(value), (value + ((value + 1) >>> 0)) >>> 0);
  assert.equal(api.indirect(value, false), api.direct(value));
  assert.equal(api.indirect(value, true), Math.imul(value, 3) >>> 0);
  assert.equal(api.unequal(value), value !== 7);
}
assert.equal(api.text(), 'provider constant');
const sentinel = {};
for (const value of [undefined, null, false, 0, '', 1n, Symbol(), sentinel, () => 0]) {
  assert.equal(api.host_null(value), value === null);
  function called(argument) { return { receiver: this, argument }; }
  Object.defineProperty(called, 'call', { get() { throw sentinel; } });
  assert.deepEqual(api.host_call(called, value, sentinel), { receiver: value, argument: sentinel });
}
assert.throws(api.host_call.bind(null, () => { throw sentinel; }, null, 1), error => error === sentinel);
const report = JSON.parse(await readFile(`${program}.lane.json`, 'utf8'));
assert.equal(report.fallback_count, 0);
assert.equal(report.witnesses.failed, 0);
assert.equal(report.inline.cross_object_applied > 0, mode !== 'off', JSON.stringify(report.inline));
const source = await readFile(program, 'utf8');
if (mode === 'on') {
  const unequal = source.split('function unequal(')[1].split('\nfunction ')[0];
  assert.match(unequal, /!==7/);
  assert.doesNotMatch(unequal.split('}')[0], /\(|\[/);
  const hostNull = source.split('function host_null(')[1].split('}')[0];
  assert.match(hostNull, /===null/);
  assert.doesNotMatch(hostNull, /\(/);
}
console.log(`cross-object inline: PASS (${mode}; overflow, constants, direct/indirect identity)`);
