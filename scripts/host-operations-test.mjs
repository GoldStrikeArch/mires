import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
import { readArtifactManifest } from './artifact-manifest.mjs';

const program = path.resolve(process.argv[2]);
readArtifactManifest(`${program}.artifacts.json`);
const api = await import(pathToFileURL(program));
let inputs, trace, fail = -1;
const sentinel = Object.freeze({});
const define = Object.defineProperty, descriptor = Object.getOwnPropertyDescriptor;
const byteConstructor = descriptor(globalThis, 'Uint8Array');
const getPrototypeOf = descriptor(Object, 'getPrototypeOf');
let stopAt = '';
function effect(name) { trace.push(name); if (stopAt === name) throw sentinel; }
const native = { get prototype() { effect('prototype'); return sentinel; } };
define(globalThis, 'Uint8Array', { configurable: true, get() { effect('global'); return native; } });
define(Object, 'getPrototypeOf', { configurable: true, get() {
  effect('lookup');
  return function(value) { effect('call'); assert.equal(this, Object); assert.equal(value, sentinel); return sentinel; };
} });
try {
  for (stopAt of ['', 'global', 'prototype', 'lookup', 'call']) {
    trace = [];
    if (stopAt) assert.throws(api.global_argument, error => error === sentinel);
    else assert.equal(api.global_argument(), sentinel);
    const order = ['global', 'prototype', 'lookup', 'call'];
    assert.deepEqual(trace, stopAt ? order.slice(0, order.indexOf(stopAt) + 1) : order);
  }
  for (stopAt of ['', 'global']) {
    trace = [];
    if (stopAt) assert.throws(api.discard_global, error => error === sentinel);
    else api.discard_global();
    assert.deepEqual(trace, ['global']);
  }
} finally { define(globalThis, 'Uint8Array', byteConstructor); define(Object, 'getPrototypeOf', getPrototypeOf); }
const fields = {
  get left() { effect('left'); return 11; },
  get right() { effect('right'); return 17; }
};
globalThis.hostRecord = (left, right) => { effect('record'); return [left, right]; };
for (stopAt of ['', 'left', 'right', 'record']) {
  trace = [];
  if (stopAt) assert.throws(() => api.ordered_fields(fields), error => error === sentinel);
  else assert.deepEqual(api.ordered_fields(fields), [11, 17]);
  const order = ['left', 'right', 'record'];
  assert.deepEqual(trace, stopAt ? order.slice(0, order.indexOf(stopAt) + 1) : order);
}
for (stopAt of ['', 'left']) {
  trace = [];
  if (stopAt) assert.throws(() => api.discard_field(fields), error => error === sentinel);
  else api.discard_field(fields);
  assert.deepEqual(trace, ['left']);
}
stopAt = '';
globalThis.hostRead = index => {
  trace.push(index);
  if (index === fail) throw sentinel;
  return inputs[index];
};
const cases = [undefined, null, false, true, 0, -0, NaN, Infinity, '', 'text', 1n, Symbol(), {}, [], () => 0];
for (const value of cases) {
  inputs = [value]; trace = [];
  const object = value !== null && typeof value === 'object';
  assert.equal(api.object(), object);
  assert.deepEqual(trace, [0]);
  assert.equal(api.indirect(value, true), object);
  assert.equal(api.indirect(value, false), typeof value === 'function');
  const expected = [value === undefined, value === null, !!value, typeof value === 'boolean',
    typeof value === 'number', typeof value === 'string', typeof value === 'function', object, value === value]
    .reduce((bits, result, index) => bits | (Number(result) << index), 0);
  assert.equal(api.predicates(value), expected);
}
function callable(...args) { return { receiver: this, args }; }
Object.defineProperty(callable, 'call', { get() { throw sentinel; } });
for (const receiver of [undefined, null, 42, 'text', sentinel]) {
  inputs = [callable, receiver, sentinel, 7]; trace = [];
  const result = api.call();
  assert.equal(result.receiver, receiver);
  assert.deepEqual(result.args, [sentinel, 7]);
  assert.deepEqual(trace, [0, 1, 2, 3]);
  assert.deepEqual(api.dynamic(callable, receiver, { $b: [0, sentinel, 7, 0], $o: 1, $l: 2 }), result);
}
function Constructor(...args) { this.args = args; this.target = new.target; }
inputs = [Constructor, sentinel, 8]; trace = [];
const created = api.create();
assert.ok(created instanceof Constructor);
assert.equal(created.target, Constructor);
assert.deepEqual(created.args, [sentinel, 8]);
assert.deepEqual(trace, [0, 1, 2]);
for (const [run, values] of [[api.object, [{}]], [api.call, [callable, null, 1, 2]], [api.create, [Constructor, 1, 2]]]) {
  inputs = values;
  for (fail = 0; fail < values.length; fail++) {
    trace = [];
    assert.throws(run, error => error === sentinel);
    assert.deepEqual(trace, Array.from({ length: fail + 1 }, (_, index) => index));
  }
}
fail = -1;
for (const reason of [undefined, null, false, 0, '', sentinel]) {
  inputs = [function () { throw reason; }, null, 1, 2]; trace = [];
  assert.throws(api.call, error => error === reason);
  assert.deepEqual(trace, [0, 1, 2, 3]);
}
inputs = [0, null, 1, 2]; trace = [];
assert.throws(api.call, TypeError);
assert.deepEqual(trace, [0, 1, 2, 3]);
inputs = [() => 0, 1, 2]; trace = [];
assert.throws(api.create, TypeError);
assert.deepEqual(trace, [0, 1, 2]);
const target = {
  get method() {
    trace.push('method');
    return function (...args) { trace.push('called'); return { receiver: this, args }; };
  }
};
globalThis.hostConstructors = {
  get Constructor() { trace.push('constructor'); return Constructor; }
};
inputs = [sentinel]; trace = [];
assert.deepEqual(api.native_create().args, [sentinel]);
assert.deepEqual(trace, [0, 'constructor']);
inputs = [target, sentinel]; trace = [];
assert.deepEqual(api.member(), { receiver: target, args: [sentinel] });
assert.deepEqual(trace, [0, 1, 'method', 'called']);
inputs = [target, sentinel, 3]; trace = [];
assert.deepEqual(api.retrieved(), { receiver: sentinel, args: [3] });
assert.deepEqual(trace, [0, 'method', 1, 2, 'called']);
const key = { [Symbol.toPrimitive]() { trace.push('key'); return 'value'; } };
const proxy = new Proxy({}, {
  get(_target, name, receiver) {
    assert.equal(name, 'value'); assert.equal(receiver, proxy);
    trace.push('get'); return sentinel;
  },
  set(_target, name, value, receiver) {
    assert.equal(name, 'value'); assert.equal(value, sentinel); assert.equal(receiver, proxy);
    trace.push('set'); return false;
  }
});
inputs = [proxy, key]; trace = [];
assert.equal(api.get(), sentinel);
assert.deepEqual(trace, [0, 1, 'key', 'get']);
inputs = [proxy, key, sentinel]; trace = [];
assert.equal(api.set(), false);
assert.deepEqual(trace, [0, 1, 2, 'key', 'set']);
for (const primitive of [undefined, null, false, 1, 'text', 1n, Symbol()]) {
  inputs = [primitive, key, sentinel]; trace = [];
  assert.throws(api.get, TypeError);
  assert.deepEqual(trace, [0, 1]);
  trace = [];
  assert.throws(api.set, TypeError);
  assert.deepEqual(trace, [0, 1, 2]);
}
for (const [name, run, values] of [
  ['apply', api.call, [callable, sentinel, 1, 2]],
  ['construct', api.create, [Constructor, sentinel, 3]],
  ['apply', api.empty_call, [callable, sentinel]],
  ['construct', api.empty_create, [Constructor]],
  ['get', api.get, [proxy, 'value']],
  ['set', api.set, [proxy, 'value', sentinel]],
]) {
  const descriptor = Object.getOwnPropertyDescriptor(Reflect, name);
  Object.defineProperty(Reflect, name, {
    configurable: true, get() { trace.push(name); return descriptor.value; }
  });
  inputs = values; trace = [];
  try { run(); } finally { Object.defineProperty(Reflect, name, descriptor); }
  assert.deepEqual(trace.slice(0, values.length + 1), [...values.map((_, index) => index), name]);
}
for (const [name, run, values] of [
  ['apply', api.empty_call, [callable, sentinel]],
  ['construct', api.empty_create, [Constructor]],
]) {
  const original = Reflect[name];
  const retained = [];
  Reflect[name] = (...args) => {
    const pack = args.at(-1);
    assert.deepEqual(pack, []);
    assert.ok(!retained.includes(pack));
    retained.push(pack);
    const result = original(...args);
    pack.push(sentinel);
    return result;
  };
  inputs = values;
  try {
    for (let repeat = 0; repeat < 2; repeat++) {
      trace = [];
      const result = run();
      assert.deepEqual(result.args, []);
      if (name === 'apply') assert.equal(result.receiver, sentinel);
      else assert.equal(result.target, Constructor);
      assert.deepEqual(trace, values.map((_, index) => index));
    }
  } finally { Reflect[name] = original; }
}
const source = await readFile(program, 'utf8');
if (program.endsWith('-report.mjs')) {
  const off = await readFile(program.replace(/-report\.mjs$/, '-off.mjs'), 'utf8');
  const body = source => source.replace(/\/\/# sourceMappingURL=[^\n]*\n?$/, '');
  assert.equal(body(source), body(off), 'report mode changed emitted host code');
}
assert.match(source, /Reflect\.apply/);
assert.match(source, /Reflect\.construct/);
const map = JSON.parse(await readFile(`${program}.map`, 'utf8'));
assert.ok(map.sources.some((name, index) => name.endsWith('examples/host_operations.rs') &&
  map.sourcesContent[index]?.includes('invoke(read(0), read(1), &[read(2), read(3)])')));
if (path.basename(program).startsWith('jir-')) {
  const report = JSON.parse(await readFile(`${program}.lane.json`, 'utf8'));
  for (const name of ['call', 'create', 'empty_call', 'empty_create']) {
    const item = report.items.find(item => item.name === name);
    const optimized = path.basename(program) === 'jir-on.mjs';
    assert.equal(item.host_args.final_emitted, optimized ? 1 : 0);
    assert.equal(item.host_args.would_lower, path.basename(program) === 'jir-off.mjs' ? 0 : 1);
    if (optimized) {
      const body = source.split(`function ${name}()`)[1].split('\nfunction ')[0];
      assert.doesNotMatch(body, /\.slice\(|\.\$b|\.\$o|\.\$l/);
    }
  }
  const dynamic = report.items.find(item => item.name === 'dynamic');
  assert.equal(dynamic.host_args.final_emitted, 0);
  if (path.basename(program) !== 'jir-off.mjs') assert.ok(Object.keys(dynamic.host_args.declines).length);
}
console.log(`host operations: PASS (${path.basename(program)}; ordered inputs, receiver, types, indirect calls, throw identity)`);
