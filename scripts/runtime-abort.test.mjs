import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from 'acorn';
import { minimalShim } from './minimal-shim.mjs';

const source = readFileSync(new URL('../runtime/shim.js', import.meta.url), 'utf8');
const direct = ['js_abort', 'abort_error', 'is_abort', 'prefix', 'native_str_truncate',
  'native_vec_set_capacity', 'native_vec_set_len', 'keyed_key_type'];
const extracted = minimalShim([], source, { direct });
const errorOf = (f) => {
  try { f(); } catch (error) { return error; }
  assert.fail('expected a throw');
};

function verify(rt) {
  const abort = errorOf(() => rt.js_abort('panic'));
  assert.equal(abort.message, 'rust abort: panic');
  assert.equal(abort.name, 'Error');
  assert.equal(rt.is_abort(abort), true);
  assert.deepEqual(Reflect.ownKeys(abort).sort(), ['message', 'stack']);
  assert.equal(Object.getPrototypeOf(abort).constructor.name, 'Error');
  const proxy = Proxy.revocable({}, {}); proxy.revoke();
  const foreign = vm.runInNewContext('new Error("foreign")');
  for (const value of [undefined, null, false, 0, -0, NaN, 'error', Symbol('error'), {},
    new Error('rust abort: panic'), new RangeError('range'), proxy.proxy, foreign]) {
    assert.equal(rt.is_abort(value), false);
    let conversions = 0;
    const thrown = errorOf(() => rt.js_abort({[Symbol.toPrimitive]() { conversions++; throw value; }}));
    assert.ok(Object.is(thrown, value));
    assert.equal(conversions, 1);
    assert.equal(rt.is_abort(thrown), false);
  }
  const observed = new Proxy({}, {get() { assert.fail('classification read a host property'); }});
  assert.equal(rt.is_abort(observed), false);
  let conversions = 0;
  const converted = errorOf(() => rt.js_abort({[Symbol.toPrimitive](hint) {
    assert.equal(hint, 'default'); conversions++; return 'converted';
  }}));
  assert.equal(conversions, 1);
  assert.equal(converted.message, 'rust abort: converted');
  assert.equal(rt.is_abort(converted), true);
  const trap = new Error('trap');
  const descriptors = Object.getOwnPropertyDescriptors(trap);
  assert.equal(rt.abort_error(trap), trap);
  assert.deepEqual(Object.getOwnPropertyDescriptors(trap), descriptors);
  assert.equal(rt.is_abort(trap), true);
  for (const run of [() => rt.prefix(null, 0, 0),
    () => rt.native_str_truncate({text: '\u03b1'}, 1),
    () => rt.native_vec_set_capacity({len: 2}, 1),
    () => rt.native_vec_set_len({cap: 1}, 2)]) {
    assert.equal(rt.is_abort(errorOf(run)), true);
  }
  assert.equal(rt.is_abort(errorOf(() => rt.keyed_key_type({}))), false);
  return abort;
}

for (const [name, text] of [['full', source], ['extracted', extracted.source]]) {
  test(`${name}: abort identity, host values, conversions and explicit runtime traps`, () => {
    parse(text, {ecmaVersion: 2018});
    const context = vm.createContext({ TextEncoder, TextDecoder });
    vm.runInContext(text, context);
    verify(context.__rt);
  });
  test(`${name}: separately instantiated local runtimes share abort identities`, () => {
    const context = vm.createContext({ TextEncoder, TextDecoder });
    const local = text.replaceAll('globalThis.__rt', '__rt');
    vm.runInContext(`globalThis.make = () => { let __rt; ${local}; return __rt; };`, context);
    const first = context.make(), second = context.make();
    const abort = verify(first);
    assert.notEqual(first, second);
    assert.equal(second.is_abort(abort), true);
    assert.equal(first.is_abort(errorOf(() => second.js_abort('other'))), true);
  });
  test(`${name}: runtime reload in a fresh lexical scope keeps prior abort identities`, () => {
    const context = vm.createContext({ TextEncoder, TextDecoder });
    vm.runInContext(`{ ${text} }`, context);
    const abort = verify(context.__rt);
    vm.runInContext(`{ ${text} }`, context);
    assert.equal(context.__rt.is_abort(abort), true);
  });
}

test('extraction retains abort dependencies and omits them for an unrelated helper', () => {
  for (const name of ['_abort_errors', 'abort_error', 'is_abort']) assert.ok(extracted.names.includes(name));
  const log = minimalShim([], source, {direct: ['js_log_i32']});
  assert.deepEqual(log.names, ['js_log_i32']);
});

test('unregistered-abort mutation rejects', () => {
  const mutant = source.replace('throw __rt.abort_error(new Error("rust abort: " + msg));',
    'throw new Error("rust abort: " + msg);');
  assert.notEqual(mutant, source);
  const context = vm.createContext({ TextEncoder, TextDecoder }); vm.runInContext(mutant, context);
  assert.throws(() => verify(context.__rt), assert.AssertionError);
});

test('separate ESM runtime instances share the same abort registry', async () => {
  const module = minimalShim([], source, {direct, module: true}).source
    .replaceAll('globalThis.__rt', '__rt');
  const load = (name) => import('data:text/javascript;base64,' +
    Buffer.from(`let __rt; ${module}\n// ${name}`).toString('base64'));
  const first = await load('first'), second = await load('second');
  assert.notEqual(first.js_abort, second.js_abort);
  const abort = verify(first);
  assert.equal(second.is_abort(abort), true);
  assert.equal(first.is_abort(errorOf(() => second.js_abort('other'))), true);
});
