import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from 'acorn';
import { minimalShim } from './minimal-shim.mjs';

const source = readFileSync(new URL('../runtime/shim.js', import.meta.url), 'utf8');
const direct = ['alloc', 'retype_rc', 'tail_ref', 'tail_raw', 'tail_slice', 'tail_slice_raw', 'addr', 'ptr_eq', 'dealloc', 'copy', 'copy_nonoverlapping', 'write_bytes', 'compare_bytes', 'unscale', 'alloc_zeroed', 'slice_ptr_eq', 'fat_ptr_eq', 'slice_ptr_cmp', 'fat_ptr_cmp'];
const extracted = minimalShim([], source, { direct }).source;
function verify(rt) {
  // Zero-sized accesses may use aligned dangling pointers and consume no storage.
  const untouched = [41];
  rt.copy({$b: untouched, $o: 0}, 4, 0);
  rt.copy_nonoverlapping(8, {$b: untouched, $o: 0}, 0);
  rt.write_bytes(4, 255, 0, {value: 0});
  assert.equal(rt.compare_bytes(4, 8, 0), 0);
  assert.deepEqual(untouched, [41]);
  assert.throws(() => rt.copy(4, 8, 1), /pointer that names no buffer/);
  assert.throws(() => rt.copy_nonoverlapping(4, 8, 1), /pointer that names no buffer/);
  for (const len of [0, 1, 2, 3]) {
    // Casting a byte allocation does not require all its bytes to be full elements.
    const bytes = 8 + 12 * len;
    const allocated = rt.alloc(bytes, 4);
    const typed = rt.unscale(allocated, 12, () => ({a: 0, b: 0, c: 0}));
    assert.equal(typed.$b.length, Math.floor(bytes / 12));
    assert.equal(rt._blocks.get(typed.$b).bytes, bytes);
    assert.ok(typed.$b.every(value => value === undefined));
    const raw = rt.retype_rc({...typed, $l: len}, () => ({strong: 1, weak: 1}), 12, len);
    assert.equal(raw.$p.$b.length, 1 + len);
    assert.equal(raw.$m, len);
  }
  assert.throws(() => rt.unscale(rt.alloc_zeroed(20, 4), 12, () => ({})), /whole number/);
  const data = [1, 2];
  const small = {$b: data, $o: 0, $l: 1}, large = {...small, $l: 2};
  assert.equal(rt.slice_ptr_eq(small, {...small}), true);
  assert.equal(rt.slice_ptr_eq(small, large), false);
  assert.equal(rt.slice_ptr_cmp(small, {...small}, 4), 0);
  assert.equal(rt.slice_ptr_cmp(small, large, 4), -1);
  assert.equal(rt.slice_ptr_cmp(large, small, 4), 1);
  const fat = {$p: {$b: data, $o: 0}, $m: 1};
  assert.equal(rt.fat_ptr_eq(fat, {$p: {...fat.$p}, $m: 1}), true);
  assert.equal(rt.fat_ptr_eq(fat, {...fat, $m: 2}), false);
  assert.equal(rt.fat_ptr_eq(fat, {$p: {$b: data, $o: 1}, $m: 1}), false);
  assert.equal(rt.fat_ptr_cmp(fat, {$p: {...fat.$p}, $m: 1}, 1), 0);
  assert.equal(rt.fat_ptr_cmp(fat, {...fat, $m: 2}, 1), -1);
  assert.equal(rt.fat_ptr_cmp({...fat, $m: 2}, fat, 1), 1);
  assert.equal(rt.fat_ptr_cmp(fat, {$p: {$b: data, $o: 1}, $m: 0}, 1), -1);
  for (const len of [0, 1, 4, 17]) {
    for (const size of [1, 4, 8]) {
      const allocated = rt.alloc(8 + len * size, 4);
      const raw = rt.retype_rc(allocated, () => ({strong: 1, weak: 1}), size, len);
      const reference = rt.tail_ref(raw);
      assert.equal(reference.$p, allocated.$b[0]);
      reference.$p.strong = 2;
      assert.equal(raw.$p.$b[raw.$p.$o].strong, 2);
      const tail = rt.tail_slice(reference.$p, 'value', len);
      const rawTail = rt.tail_slice_raw(raw, 'value');
      assert.equal(rawTail.$b, tail.$b);
      assert.equal(rawTail.$o, tail.$o);
      assert.equal(rawTail.$l, tail.$l);
      assert.equal(tail.$b, allocated.$b);
      assert.equal(tail.$o, 1);
      assert.equal(tail.$l, len);
      for (let i = 0; i < len; i++) tail.$b[tail.$o + i] = i + 10;
      const again = rt.tail_ref(raw);
      const alias = rt.tail_slice(again.$p, 'value', len);
      for (let i = 0; i < len; i++) assert.equal(alias.$b[alias.$o + i], i + 10);
      const roundTrip = rt.tail_raw(reference);
      assert.equal(roundTrip.$p, raw.$p);
      assert.equal(roundTrip.$m, len);
      assert.equal(rt.ptr_eq(roundTrip.$p, raw.$p), true);
      assert.equal(rt.addr(roundTrip.$p, size), rt.addr(raw.$p, size));
      rt.dealloc(roundTrip.$p, 8 + len * size, 4);
      assert.equal(rt._blocks.has(allocated.$b), false);
    }
    // Coercing a sized stack or heap record keeps its actual tail field.
    for (const heap of [false, true]) {
      const record = {header: 9, data: Array.from({length: len}, (_, i) => i)};
      const slot = heap ? rt.alloc(4 + len * 4, 4) : {$b: [null, record], $o: 1};
      if (heap) slot.$b[0] = record;
      const raw = {$p: slot, $m: len};
      const rawTail = rt.tail_slice_raw(raw, 'data');
      assert.equal(rawTail.$b, record.data);
      assert.equal(rawTail.$o, 0);
      assert.equal(rawTail.$l, len);
      assert.equal(rt._boxes.has(record), false);
      const reference = rt.tail_ref(raw);
      const tail = rt.tail_slice(reference.$p, 'data', len);
      assert.equal(tail.$b, record.data);
      assert.equal(tail.$o, 0);
      assert.equal(rt.tail_raw(reference).$p, slot);
      if (len) {
        tail.$b[0] = 55;
        assert.equal(record.data[0], 55);
      }
    }
  }
}
for (const [name, text] of [['full', source], ['extracted', extracted]]) {
  test(`${name}: unsized references preserve header, tail and allocation identity`, () => {
    parse(text, {ecmaVersion: 2018});
    const context = vm.createContext({TextEncoder, TextDecoder});
    vm.runInContext(text, context);
    verify(context.__rt);
  });
}
test('the raw/reference identity mismatch is detected', () => {
  const context = vm.createContext({TextEncoder, TextDecoder});
  vm.runInContext(source, context);
  context.__rt.tail_ref = p => p;
  assert.throws(() => verify(context.__rt), assert.AssertionError);
});

test('wide-pointer equality and ordering checks detect dropped metadata', () => {
  for (const helper of ['slice_ptr_eq', 'fat_ptr_eq']) {
    const context = vm.createContext({TextEncoder, TextDecoder});
    vm.runInContext(source, context);
    context.__rt[helper] = () => true;
    assert.throws(() => verify(context.__rt), assert.AssertionError);
  }
  for (const helper of ['slice_ptr_cmp', 'fat_ptr_cmp']) {
    const context = vm.createContext({TextEncoder, TextDecoder});
    vm.runInContext(source, context);
    context.__rt[helper] = () => 0;
    assert.throws(() => verify(context.__rt), assert.AssertionError);
  }
});
