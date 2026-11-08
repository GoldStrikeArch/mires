import assert from "node:assert/strict";
import test from "node:test";

import "../runtime/shim.js";

const rt = globalThis.__rt;

test("empty slices preserve the identity and address of their dangling pointer", () => {
  const first = rt.slice_from_parts(1, 0);
  const second = rt.slice_from_parts(1, 0);
  assert.equal(first.len, 0);
  assert.equal(rt.ptr_eq(first, second), true);
  assert.equal(rt.addr(first, 1), 1);
});

test("heap byte slices retain bytes, partial tails, and aliases after growth", () => {
  let pointer = rt.alloc(33, 1);
  pointer.buf.fill(0x61);
  const bytes = rt.byte_slice_ref(rt.slice_from_parts(pointer, 33));
  const words = rt.unscale_scalar(bytes, 4, () => 0, "u32");
  assert.equal(words.buf[0], 0x61616161);
  assert.equal(bytes.buf.length, 33);
  assert.equal(bytes.buf[32], 0x61);
  words.buf[1] = 0x12345678;
  assert.deepEqual(bytes.buf.slice(4, 8), [0x78, 0x56, 0x34, 0x12]);
  assert.equal(rt.addr(words, 4), rt.addr(bytes, 1));
  assert.equal(rt.addr({ buf: words.buf, off: 1 }, 4), rt.addr({ buf: bytes.buf, off: 4 }, 1));
  assert.throws(() => rt.unscale(bytes, 4, () => ({})), /already holds 1-byte elements/);

  pointer = rt.realloc(pointer, 33, 1, 58);
  pointer.buf.fill(0x62, 33);
  const grown = rt.unscale_scalar(rt.byte_slice_ref(rt.slice_from_parts(pointer, 58)), 4, () => 0, "u32");
  assert.equal(grown.buf, words.buf);
  assert.equal(grown.buf[13], 0x62626262);
  assert.equal(pointer.buf[57], 0x62);
  rt.dealloc(pointer, 58, 1);
  assert.throws(() => rt.unscale_scalar(pointer, 4, () => 0, "u32"), /freed/);

  const fresh = rt.alloc_zeroed(12, 4);
  rt.slice_from_parts(fresh, 12);
  const integers = rt.unscale_scalar(fresh, 4, () => 0, "u32");
  assert.equal(integers.buf.length, 3);
  assert.deepEqual(integers.buf, [0, 0, 0]);
  rt.dealloc(integers, 12, 4);
});

test("zero-offset aggregate projection is reversible but standalone values are not", () => {
  const leaf = { len: 0 };
  const internal = { data: leaf, edges: [] };
  const allocation = [internal];
  const projected = rt.prefix({ buf: allocation, off: 0 }, "data", 24);
  const restored = rt.unproject(projected, "data");

  assert.equal(restored.buf[restored.off], internal);
  assert.throws(
    () => rt.unproject({ buf: [leaf], off: 0 }, "data"),
    /does not name the required zero-offset aggregate field/,
  );
});

test("scalar byte views alias storage and aggregate byte punning still refuses", () => {
  const bytes = [0x78, 0x56, 0x34, 0x12, 0xff, 0xff, 0xff, 0xff];
  const pointer = { buf: bytes, off: 0 };
  const words = rt.unscale_scalar(pointer, 4, () => 0, "u32");

  assert.equal(words.buf[words.off], 0x12345678);
  assert.equal(words.buf[words.off + 1], 0xffffffff);
  words.buf[words.off] = 0x89abcdef;
  assert.deepEqual(bytes.slice(0, 4), [0xef, 0xcd, 0xab, 0x89]);
  assert.deepEqual(rt.scale(words, 4), pointer);
  assert.equal(rt.addr(words, 4), rt.addr(pointer, 1));
  assert.equal(rt.unscale_scalar(pointer, 4, () => 0, "u32").buf, words.buf);

  const signed = rt.unscale_scalar(pointer, 4, () => 0, "i32");
  assert.equal(signed.buf[signed.off], -1985229329);
  assert.throws(
    () => rt.unscale(pointer, 4, () => ({ value: 0 }), null),
    /is not a byte view of 4-byte elements/,
  );
  assert.throws(
    () => rt.unscale_scalar({ buf: bytes, off: 1 }, 4, () => 0, "u32"),
    /cannot form a u32 scalar view/,
  );
});

test("native string indices use UTF-8 bytes and UTF-16 boundaries", () => {
  const text = "a\u00e9\u20ac\u{1f600}z";
  const boundaries = [
    [0, 0],
    [1, 1],
    [3, 2],
    [6, 3],
    [10, 5],
    [11, 6],
  ];

  assert.equal(rt.str_len(text), 11);
  assert.equal(rt.str_len(""), 0);
  assert.equal(rt.native_str_utf16_index("", 0), 0);
  assert.equal(rt.native_str_byte_index("", 0), 0);
  assert.equal(rt.native_str_is_char_boundary("", 0), true);
  assert.equal(rt.native_str_is_char_boundary("", 1), false);
  for (const [byteIndex, utf16Index] of boundaries) {
    assert.equal(rt.native_str_utf16_index(text, byteIndex), utf16Index);
    assert.equal(rt.native_str_byte_index(text, utf16Index), byteIndex);
    assert.equal(rt.native_str_is_char_boundary(text, byteIndex), true);
  }

  for (const byteIndex of [2, 4, 5, 7, 8, 9, 12, -1, 1.5]) {
    assert.equal(rt.native_str_utf16_index(text, byteIndex), -1);
    assert.equal(rt.native_str_is_char_boundary(text, byteIndex), false);
  }
  for (const utf16Index of [4, 7, -1, 1.5]) {
    assert.equal(rt.native_str_byte_index(text, utf16Index), -1);
  }
});

test("native string mutation preserves byte capacity", () => {
  const value = { text: "", cap: 32 };

  rt.native_str_push(value, 0x61);
  rt.native_str_push(value, 0x1f600);
  rt.native_str_push(value, 0x20ac);
  assert.deepEqual(value, { text: "a\u{1f600}\u20ac", cap: 32 });

  assert.equal(rt.native_str_pop(value), 0x20ac);
  assert.equal(rt.native_str_pop(value), 0x1f600);
  assert.equal(rt.native_str_pop(value), 0x61);
  assert.equal(rt.native_str_pop(value), -1);
  assert.equal(value.cap, 32);

  value.text = "a\u00e9\u20ac\u{1f600}z";
  rt.native_str_truncate(value, 10);
  assert.equal(value.text, "a\u00e9\u20ac\u{1f600}");
  rt.native_str_truncate(value, 6);
  assert.equal(value.text, "a\u00e9\u20ac");
  rt.native_str_truncate(value, 100);
  assert.equal(value.text, "a\u00e9\u20ac");
  assert.throws(() => rt.native_str_truncate(value, 2), RangeError);
  assert.equal(value.text, "a\u00e9\u20ac");
  assert.equal(value.cap, 32);
});

test("native f64 strings match Rust default Display", () => {
  const cases = [
    [NaN, "NaN"],
    [Infinity, "inf"],
    [-Infinity, "-inf"],
    [0, "0"],
    [-0, "-0"],
    [1 / 3, "0.3333333333333333"],
    [7.5e-11, "0.000000000075"],
    [1.9971e20, "199710000000000000000"],
    [1e23, "100000000000000000000000"],
    [2 ** 64, "18446744073709552000"],
    [1.00000762939453125, "1.0000076293945313"],
    [Number.MIN_VALUE, "0." + "0".repeat(323) + "5"],
    [Number.MAX_VALUE, "17976931348623157" + "0".repeat(292)],
  ];

  for (const [value, expected] of cases) {
    const actual = rt.native_f64_to_string(value);
    assert.deepEqual(actual, { text: expected, cap: expected.length });
  }
});

test("native vectors retain capacity and clear removed slots", () => {
  const value = rt.native_vec_new(4);
  assert.equal(value.buf.length, 4);
  assert.deepEqual({ len: value.len, cap: value.cap }, { len: 0, cap: 4 });

  const first = { id: 1 };
  const second = { id: 2 };
  value.buf[0] = first;
  value.buf[1] = second;
  rt.native_vec_set_len(value, 2);
  assert.deepEqual(value.buf.slice(0, value.len), [first, second]);

  rt.native_vec_set_len(value, 1);
  assert.equal(value.len, 1);
  assert.equal(value.buf[0], first);
  assert.equal(value.buf[1], undefined);
  assert.equal(value.buf.length, 4);

  rt.native_vec_set_capacity(value, 8);
  assert.deepEqual({ len: value.len, cap: value.cap }, { len: 1, cap: 8 });
  assert.equal(value.buf.length, 8);
  value.buf[7] = second;
  rt.native_vec_set_capacity(value, 2);
  assert.deepEqual({ len: value.len, cap: value.cap }, { len: 1, cap: 2 });
  assert.equal(value.buf.length, 2);

  assert.throws(() => rt.native_vec_set_len(value, 3), RangeError);
  assert.throws(() => rt.native_vec_set_capacity(value, 0), RangeError);
  assert.deepEqual({ len: value.len, cap: value.cap }, { len: 1, cap: 2 });

  rt.native_vec_set_capacity(value, 8);
  assert.equal(value.buf[7], undefined);
});

test("exact typed vectors keep physical allocation and logical capacity separate", () => {
  const value = { buf: new Uint8Array(3), len: 0, cap: 1 };
  rt.typed_vec_push(value, 257);
  assert.deepEqual({ values: [...value.buf], len: value.len, cap: value.cap }, {
    values: [1, 0, 0], len: 1, cap: 1,
  });
  rt.typed_vec_push(value, 258);
  rt.typed_vec_push(value, 259);
  assert.deepEqual({ values: [...value.buf], len: value.len, cap: value.cap }, {
    values: [1, 2, 3], len: 3, cap: 3,
  });
});
