import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from 'acorn';
import { minimalShim } from './minimal-shim.mjs';

const source = readFileSync(new URL('../runtime/shim.js', import.meta.url), 'utf8');

function runtime(code = source, globals = { TextEncoder, TextDecoder, Uint8Array }) {
  const context = vm.createContext(globals);
  vm.runInContext(code, context);
  return context.__rt;
}

test('minimalShim closes over _blocks cache helpers and parses as ES2018', () => {
  for (const direct of [
    ['alloc', 'dealloc', 'realloc', 'unscale', '_put', 'copy', 'copy_nonoverlapping', 'overwrite', 'addr'],
    ['str_bytes', 'str_len', 'bytes_str'],
  ]) {
    const extracted = minimalShim([], source, { direct });
    parse(extracted.source, { ecmaVersion: 2018 });
    const rt = runtime(extracted.source);
    assert.ok(rt);
  }
});

test('heap block MRU cache preserves alloc, unscale, realloc, dealloc and use-after-free refusal', () => {
  const rt = runtime();
  const a = rt.alloc(16, 4);
  const b = rt.alloc(32, 4);
  // JSArray backing must remain pure with zero extra own properties or symbols.
  assert.deepEqual(Object.getOwnPropertySymbols(a.$b), []);
  assert.deepEqual(Object.getOwnPropertySymbols(b.$b), []);
  assert.equal(Object.keys(a.$b).length, 0);

  const ua = rt.unscale(a, 4);
  assert.equal(rt._heap_block(ua.$b).es, 4);
  assert.equal(rt._heap_block(ua.$b).bytes, 16);
  const ub = rt.unscale(b, 8);
  assert.equal(rt._heap_block(ub.$b).es, 8);
  // Cached re-lookup of previous block still works.
  assert.equal(rt._heap_block(ua.$b).es, 4);

  const rb = rt.realloc(ub, 32, 4, 64);
  assert.equal(rt._heap_block(rb.$b).bytes, 64);
  assert.equal(rb.$b.length, 8);

  // Dealloc must evict the MRU cache slot so use-after-free still throws.
  rt.dealloc(ua, 16, 4);
  assert.equal(rt._blocks.has(ua.$b), false);
  assert.throws(() => rt._heap_block(ua.$b), /has been freed/);
  assert.throws(() => rt.dealloc(ua, 16, 4), /cannot be deallocated/);
});

test('copy and copy_nonoverlapping hoist heap check and track _boxes only on heap blocks', () => {
  const rt = runtime();
  const heap = rt.alloc(16, 4);
  const typedHeap = rt.unscale(heap, 4);
  const stackBuf = [undefined, undefined, undefined, undefined];
  const src = [{ x: 1 }, { x: 2 }, { x: 3 }, { x: 4 }];
  const clone = (o) => ({ x: o.x });

  rt.copy_nonoverlapping(typedHeap, { $b: src, $o: 0 }, 4, clone);
  assert.deepEqual([...typedHeap.$b.map((e) => e.x)], [1, 2, 3, 4]);
  assert.equal(rt._boxes.has(typedHeap.$b[0]), true);
  assert.equal(rt._boxes.has(typedHeap.$b[3]), true);

  rt.copy_nonoverlapping({ $b: stackBuf, $o: 0 }, { $b: src, $o: 0 }, 4, clone);
  assert.deepEqual([...stackBuf.map((e) => e.x)], [1, 2, 3, 4]);
  assert.equal(rt._boxes.has(stackBuf[0]), false);

  // Overlapping copy inside heap block with hoisted dIsHeap.
  rt.copy({ $b: typedHeap.$b, $o: 1 }, { $b: typedHeap.$b, $o: 0 }, 3, clone);
  assert.deepEqual([...typedHeap.$b.map((e) => e.x)], [1, 1, 2, 3]);
});

test('overwrite preserves identity, updates structs/arrays, and keeps TAG-object own keys exact', () => {
  const rt = runtime();
  const s = { x: 1, y: 2 };
  rt.overwrite(s, { x: 10, y: 20 });
  assert.deepEqual(s, { x: 10, y: 20 });

  const arr = [1, 2, 3];
  rt.overwrite(arr, [4, 5]);
  assert.deepEqual(arr, [4, 5]);

  const e = { TAG: 'Some', _0: 7, _1: 8 };
  rt.overwrite(e, { TAG: 'None' });
  assert.deepEqual({ ...e }, { TAG: 'None' });
  assert.equal('_0' in e, false);
  rt.overwrite(e, { TAG: 'Some', _0: 9, _1: 10 });
  assert.deepEqual({ ...e }, { TAG: 'Some', _0: 9, _1: 10 });
});

test('standalone jsc CLI runs runtime/shim.js UTF-8 and heap ops without TextEncoder/TextDecoder', (t) => {
  const jsc = '/System/Library/Frameworks/JavaScriptCore.framework/Versions/Current/Helpers/jsc';
  if (!existsSync(jsc)) {
    t.skip('jsc binary not present');
    return;
  }
  const script = `
    const rt = globalThis.__rt;
    const eq = (a, b, m) => { if (a !== b) throw new Error(m + ': ' + a + ' !== ' + b); };
    eq(typeof TextEncoder, 'undefined', 'no TextEncoder');
    eq(typeof TextDecoder, 'undefined', 'no TextDecoder');
    for (const s of ['', '\\ufeff[]', 'Grüße 世界 😀', 'a'.repeat(128), 'é'.repeat(5000), '\\ud800x']) {
      const r = rt.str_bytes(s);
      eq(rt.str_len(s), r.$l, 'str_len');
      const want = s.replace(/\\ud800/g, '\\ufffd');
      eq(rt.bytes_str(r.$b, r.$o, r.$l), want, 'roundtrip');
    }
    const p = rt.alloc(16, 4);
    const u = rt.unscale(p, 4);
    u.$b[0] = 99;
    eq(rt._heap_block(u.$b).es, 4, 'es');
    rt.dealloc(u, 16, 4);
    print('ok');
  `;
  const res = spawnSync(jsc, ['runtime/shim.js', '-e', script], { encoding: 'utf8' });
  assert.equal(res.status, 0, res.stderr);
  assert.match(res.stdout, /^ok/m);
});

test('shim hot-path microbenchmarks (printed, not asserted)', () => {
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array });
  vm.runInContext(source, context);
  const rows = vm.runInContext(`(() => {
    const rt = globalThis.__rt;
    const now = () => Date.now();
    const p = rt.unscale(rt.alloc(64, 4), 4);
    const s = { x: 1, y: 2, z: 3 };
    const s32 = 'application/json; charset=utf-8!';
    const b32 = rt.str_bytes(s32).$b;
    const bench = (fn, iters) => {
      for (let i = 0; i < 5000; i++) fn(i);
      const t0 = now();
      for (let i = 0; i < iters; i++) fn(i);
      return (now() - t0) + ' ms (' + iters + ' iters)';
    };
    return [
      { op: '_heap_block (cached)', timing: bench(() => rt._heap_block(p.$b), 1000000) },
      { op: 'overwrite struct', timing: bench((i) => rt.overwrite(s, { x: i, y: i + 1, z: i + 2 }), 200000) },
      { op: '_utf8 32B ASCII', timing: bench(() => rt._utf8(s32), 100000) },
      { op: 'bytes_str 32B ASCII', timing: bench(() => rt.bytes_str(b32, 0, 32), 100000) },
    ];
  })()`, context);
  console.log('# shim hot-path timing (in-realm)');
  console.table(rows);
});
