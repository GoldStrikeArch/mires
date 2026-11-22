import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from 'acorn';
import { minimalShim } from './minimal-shim.mjs';

const source = readFileSync(new URL('../runtime/shim.js', import.meta.url), 'utf8');
const direct = ['alloc', 'retype_rc', 'tail_ref', 'tail_raw', 'tail_slice', 'addr', 'ptr_eq', 'dealloc', 'copy', 'copy_nonoverlapping', 'write_bytes', 'compare_bytes', 'unscale', 'alloc_zeroed', 'slice_ptr_eq', 'fat_ptr_eq', 'slice_ptr_cmp', 'fat_ptr_cmp'];
const extracted = minimalShim([], source, { direct }).source;
function verify(rt) {
  // Zero-sized accesses may use aligned dangling pointers and consume no storage.
  const untouched = [41];
  rt.copy({buf: untouched, off: 0}, 4, 0);
  rt.copy_nonoverlapping(8, {buf: untouched, off: 0}, 0);
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
    assert.equal(typed.buf.length, Math.floor(bytes / 12));
    assert.equal(rt._blocks.get(typed.buf).bytes, bytes);
    assert.ok(typed.buf.every(value => value === undefined));
    const raw = rt.retype_rc({...typed, len}, () => ({strong: 1, weak: 1}), 12, len);
    assert.equal(raw.ptr.buf.length, 1 + len);
    assert.equal(raw.meta, len);
  }
  assert.throws(() => rt.unscale(rt.alloc_zeroed(20, 4), 12, () => ({})), /whole number/);
  const data = [1, 2];
  const small = {buf: data, off: 0, len: 1}, large = {...small, len: 2};
  assert.equal(rt.slice_ptr_eq(small, {...small}), true);
  assert.equal(rt.slice_ptr_eq(small, large), false);
  assert.equal(rt.slice_ptr_cmp(small, {...small}, 4), 0);
  assert.equal(rt.slice_ptr_cmp(small, large, 4), -1);
  assert.equal(rt.slice_ptr_cmp(large, small, 4), 1);
  const fat = {ptr: {buf: data, off: 0}, meta: 1};
  assert.equal(rt.fat_ptr_eq(fat, {ptr: {...fat.ptr}, meta: 1}), true);
  assert.equal(rt.fat_ptr_eq(fat, {...fat, meta: 2}), false);
  assert.equal(rt.fat_ptr_eq(fat, {ptr: {buf: data, off: 1}, meta: 1}), false);
  assert.equal(rt.fat_ptr_cmp(fat, {ptr: {...fat.ptr}, meta: 1}, 1), 0);
  assert.equal(rt.fat_ptr_cmp(fat, {...fat, meta: 2}, 1), -1);
  assert.equal(rt.fat_ptr_cmp({...fat, meta: 2}, fat, 1), 1);
  assert.equal(rt.fat_ptr_cmp(fat, {ptr: {buf: data, off: 1}, meta: 0}, 1), -1);
  for (const len of [0, 1, 4, 17]) {
    for (const size of [1, 4, 8]) {
      const allocated = rt.alloc(8 + len * size, 4);
      const raw = rt.retype_rc(allocated, () => ({strong: 1, weak: 1}), size, len);
      const reference = rt.tail_ref(raw);
      assert.equal(reference.ptr, allocated.buf[0]);
      reference.ptr.strong = 2;
      assert.equal(raw.ptr.buf[raw.ptr.off].strong, 2);
      const tail = rt.tail_slice(reference.ptr, 'value', len);
      assert.equal(tail.buf, allocated.buf);
      assert.equal(tail.off, 1);
      assert.equal(tail.len, len);
      for (let i = 0; i < len; i++) tail.buf[tail.off + i] = i + 10;
      const again = rt.tail_ref(raw);
      const alias = rt.tail_slice(again.ptr, 'value', len);
      for (let i = 0; i < len; i++) assert.equal(alias.buf[alias.off + i], i + 10);
      const roundTrip = rt.tail_raw(reference);
      assert.equal(roundTrip.ptr, raw.ptr);
      assert.equal(roundTrip.meta, len);
      assert.equal(rt.ptr_eq(roundTrip.ptr, raw.ptr), true);
      assert.equal(rt.addr(roundTrip.ptr, size), rt.addr(raw.ptr, size));
      rt.dealloc(roundTrip.ptr, 8 + len * size, 4);
      assert.equal(rt._blocks.has(allocated.buf), false);
    }
    // Coercing a sized stack or heap record keeps its actual tail field.
    for (const heap of [false, true]) {
      const record = {header: 9, data: Array.from({length: len}, (_, i) => i)};
      const slot = heap ? rt.alloc(4 + len * 4, 4) : {buf: [null, record], off: 1};
      if (heap) slot.buf[0] = record;
      const raw = {ptr: slot, meta: len};
      const reference = rt.tail_ref(raw);
      const tail = rt.tail_slice(reference.ptr, 'data', len);
      assert.equal(tail.buf, record.data);
      assert.equal(tail.off, 0);
      assert.equal(rt.tail_raw(reference).ptr, slot);
      if (len) {
        tail.buf[0] = 55;
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
