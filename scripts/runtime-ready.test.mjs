import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {minimalShim} from './minimal-shim.mjs';

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8');
const extracted = minimalShim(['__rt.host_future_ready(future, poll, drop)'], source);

for (const [label, code] of [['full', source], ['extracted', extracted.source]]) {
  test(`${label}: ready driver preserves poll, drop, adoption and exception order`, async () => {
    const context = vm.createContext({queueMicrotask() { assert.fail('a ready driver must not schedule'); }});
    vm.runInContext(code, context);
    const run = context.__rt.host_future_ready;
    for (const failure of ['none', 'poll', 'drop', 'both']) {
      const trace = [], pollError = {}, dropError = {}, value = {};
      let retained;
      const promise = run(value, (state, waker) => {
        assert.equal(state.future, value);
        assert.equal(typeof waker.vtable.drop, 'function');
        retained = state;
        trace.push('poll');
        if (failure === 'poll' || failure === 'both') throw pollError;
        return {TAG:'Ready', get _0() { trace.push('output'); return {TAG:'Ok', _0:value}; }};
      }, state => {
        assert.equal(state.future, value);
        trace.push('drop');
        if (failure === 'drop' || failure === 'both') throw dropError;
      });
      assert.equal(retained.future, undefined);
      assert.deepEqual(trace, failure === 'poll' || failure === 'both' ? ['poll','drop'] : ['poll','output','drop']);
      if (failure === 'none') assert.equal(await promise, value);
      else await assert.rejects(promise, error => error === (failure === 'poll' ? pollError : dropError));
    }
    const trace = [], token = {};
    const thenable = {get then() { trace.push('get then'); return resolve => { trace.push('call then'); resolve(token); }; }};
    const promise = run({}, () => ({TAG:'Ready', _0:{TAG:'Ok', _0:thenable}}), () => trace.push('drop'));
    assert.deepEqual(trace, ['drop','get then']);
    assert.equal(await promise, token);
    assert.deepEqual(trace, ['drop','get then','call then']);
    const existing = Promise.resolve(token);
    assert.notEqual(run({}, () => ({TAG:'Ready', _0:{TAG:'Ok', _0:existing}})), existing);
    const rejected = {};
    await assert.rejects(run({}, () => ({TAG:'Ready', _0:{TAG:'Err', _0:rejected}})), error => error === rejected);
  });
}
