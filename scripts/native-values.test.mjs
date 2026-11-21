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
  // An unaligned word is the bytes at its offset (MIRES-018), in the view of its own phase.
  const odd = rt.unscale_scalar({ buf: bytes, off: 1 }, 4, () => 0, "u32");
  assert.equal(odd.buf[odd.off], 0xff89abcd);
  assert.notEqual(odd.buf, words.buf);
  assert.equal(rt.unscale_scalar({ buf: bytes, off: 1 }, 4, () => 0, "u32").buf, odd.buf);
  assert.deepEqual(rt.scale(odd, 4), { buf: bytes, off: 1 });
  assert.equal(rt.addr(odd, 4), rt.addr(pointer, 1) + 1);
  odd.buf[odd.off] = 0x01020304;
  assert.deepEqual(bytes.slice(0, 6), [0xef, 0x04, 0x03, 0x02, 0x01, 0xff]);
  assert.equal(words.buf[words.off], 0x020304ef);
  const half = rt.unscale_scalar({ buf: bytes, off: 3 }, 2, () => 0, "u16");
  assert.equal(half.buf[half.off], 0x0102);
  assert.equal(half.buf[half.off + 1], 0xffff);
  assert.throws(
    () => rt.unscale_scalar({ buf: bytes, off: 1 }, 2, () => 0, "u32"),
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

test("native string mutation preserves byte capacity and the exact byte length", () => {
  const value = { text: "", cap: 32, len: 0 };

  rt.native_str_push(value, 0x61);
  rt.native_str_push(value, 0x1f600);
  rt.native_str_push(value, 0x20ac);
  assert.deepEqual(value, { text: "a\u{1f600}\u20ac", cap: 32, len: 8 });

  assert.equal(rt.native_str_pop(value), 0x20ac);
  assert.equal(value.len, 5);
  assert.equal(rt.native_str_pop(value), 0x1f600);
  assert.equal(value.len, 1);
  assert.equal(rt.native_str_pop(value), 0x61);
  assert.equal(rt.native_str_pop(value), -1);
  assert.deepEqual(value, { text: "", cap: 32, len: 0 });

  value.text = "a\u00e9\u20ac\u{1f600}z";
  value.len = 11;
  rt.native_str_truncate(value, 10);
  assert.deepEqual(value, { text: "a\u00e9\u20ac\u{1f600}", cap: 32, len: 10 });
  rt.native_str_truncate(value, 6);
  assert.deepEqual(value, { text: "a\u00e9\u20ac", cap: 32, len: 6 });
  rt.native_str_truncate(value, 100);
  rt.native_str_truncate(value, 6);
  assert.deepEqual(value, { text: "a\u00e9\u20ac", cap: 32, len: 6 });
  assert.throws(() => rt.native_str_truncate(value, 2), RangeError);
  assert.throws(() => rt.native_str_truncate(value, -1), RangeError);
  assert.deepEqual(value, { text: "a\u00e9\u20ac", cap: 32, len: 6 });
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
    assert.deepEqual(actual, { text: expected, cap: expected.length, len: expected.length });
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

test("wider scalar byte views alias little-endian bytes and non-scalar storage refuses", () => {
  const words = [0x0102, 0x0304];
  const ptr = { buf: words, off: 0 };
  const bytes = rt.scale_scalar(ptr, 2, "u16");
  assert.deepEqual(Array.prototype.slice.call(bytes.buf, 0, 4), [0x02, 0x01, 0x04, 0x03]);
  assert.equal(rt.scale_scalar(ptr, 2, "u16").buf, bytes.buf);
  assert.equal(rt.addr(bytes, 1), rt.addr(ptr, 2));
  assert.equal(rt.addr({ buf: bytes.buf, off: 2 }, 1), rt.addr({ buf: words, off: 1 }, 2));
  assert.equal(rt.ptr_eq(bytes, ptr), true);
  assert.equal(rt.ptr_eq({ buf: bytes.buf, off: 2 }, { buf: words, off: 1 }), true);
  assert.equal(rt.ptr_eq({ buf: bytes.buf, off: 1 }, { buf: words, off: 1 }), false);

  // Live two-way mutation (u16 and signed i16).
  bytes.buf[1] = 0xfe;
  assert.equal(words[0], 0xfe02);
  words[1] = 0xabcd;
  assert.deepEqual([bytes.buf[2], bytes.buf[3]], [0xcd, 0xab]);

  const signed16 = [0];
  const sb = rt.scale_scalar({ buf: signed16, off: 0 }, 2, "i16");
  sb.buf[0] = 0xff;
  sb.buf[1] = 0xff;
  assert.equal(signed16[0], -1);

  // Signed i8 byte view over u16 storage.
  const i8bytes = rt.scale_scalar({ buf: [0xfe02], off: 0 }, 2, "u16", true);
  assert.deepEqual([i8bytes.buf[0], i8bytes.buf[1]], [2, -2]);

  // Round-trip back to original buffer via unscale_scalar and thin.
  const back = rt.unscale_scalar({ buf: bytes.buf, off: 2 }, 2, () => 0, "u16");
  assert.equal(back.buf, words);
  assert.equal(back.off, 1);
  const thinBack = rt.thin({ buf: bytes.buf, off: 2 }, 2);
  assert.equal(thinBack.buf, words);
  assert.equal(thinBack.off, 1);
  assert.throws(() => rt.thin({ buf: bytes.buf, off: 1 }, 2), /not a multiple of 2/);

  // BigInt (i64) and float (f64) read/write.
  const big = [0n];
  const bb = rt.scale_scalar({ buf: big, off: 0 }, 8, "i64");
  bb.buf[7] = 0x80;
  assert.equal(big[0], -9223372036854775808n);
  assert.equal(bb.buf[7], 0x80);

  const flt = [1.5];
  const fb = rt.scale_scalar({ buf: flt, off: 0 }, 8, "f64");
  fb.buf[7] |= 0x80;
  assert.equal(flt[0], -1.5);

  // Heap block retyped via unscale_scalar records kind for GVN-folded raw pointers.
  const allocPtr = rt.alloc(4, 2);
  const typed = rt.unscale_scalar(allocPtr, 2, () => 0, "u16");
  typed.buf[0] = 0x0102;
  typed.buf[1] = 0x0304;
  // Direct GVN-folded byte reads/writes, offset slice, copy, write_bytes, realloc.
  assert.deepEqual([allocPtr.buf[0], allocPtr.buf[1], allocPtr.buf[2], allocPtr.buf[3]], [0x02, 0x01, 0x04, 0x03]);
  const gvnSlice = rt.slice_from_parts(allocPtr, 4, 1);
  assert.deepEqual(Array.prototype.slice.call(gvnSlice.buf, 0, 4), [0x02, 0x01, 0x04, 0x03]);
  const gvnOffSlice = rt.slice_from_parts({ buf: allocPtr.buf, off: 1 }, 2, 1);
  assert.deepEqual([gvnOffSlice.buf[gvnOffSlice.off], gvnOffSlice.buf[gvnOffSlice.off + 1]], [0x01, 0x04]);
  assert.equal(rt.ptr_eq({ buf: allocPtr.buf, off: 2 }, { buf: typed.buf, off: 1 }), true);
  assert.equal(rt.ptr_eq({ buf: allocPtr.buf, off: 1 }, { buf: typed.buf, off: 1 }), false);
  const grown = rt.realloc(allocPtr, 4, 2, 8);
  assert.equal(grown.buf, allocPtr.buf);
  typed.buf[2] = 0x0506;
  assert.deepEqual([grown.buf[4], grown.buf[5]], [0x06, 0x05]);
  rt.write_bytes(grown, 0x5a, 4);
  assert.deepEqual([typed.buf[0], typed.buf[1]], [0x5a5a, 0x5a5a]);
  const sv = rt.convert_block_sv(rt.scale_scalar(typed, 2, "u16"), 2);
  assert.equal(sv.buf, typed.buf);
  assert.equal(sv.off, 0);
  assert.equal(sv.len, 2);
  rt.dealloc(rt.scale_scalar(typed, 2, "u16"), 8, 2);

  // Erased pointer round-trip through thin(p, 1, signed?).
  const erWords = [0xfe02, 0x7f01];
  const erased = rt.erase({ buf: erWords, off: 0 }, 2, "u16");
  const thinU8 = rt.thin(erased, 1);
  const thinI8 = rt.thin(erased, 1, true);
  assert.deepEqual([thinU8.buf[0], thinU8.buf[1]], [2, 254]);
  assert.deepEqual([thinI8.buf[0], thinI8.buf[1]], [2, -2]);

  // Layered _scalar_view over _byte_view preserves addr() and ptr_eq().
  const layeredWords = [0x0102, 0x0304, 0x0506, 0x0708];
  const lp = { buf: layeredWords, off: 0 };
  const lbp = rt.scale_scalar(lp, 2, "u16");
  const unal = rt.unscale_scalar({ buf: lbp.buf, off: 1 }, 2, () => 0, "u16");
  assert.equal(unal.buf[unal.off], 0x0401);
  assert.equal(rt.addr(unal, 2), rt.addr(lp, 2) + 1);
  assert.equal(rt.ptr_eq(unal, { buf: lbp.buf, off: 1 }), true);
  const cross32 = rt.unscale_scalar(lbp, 4, () => 0, "u32");
  assert.equal(rt.addr(cross32, 4), rt.addr(lp, 2));
  assert.equal(rt.ptr_eq(cross32, lp), true);
  const signedView = rt.unscale_scalar(lbp, 2, () => 0, "i16");
  assert.equal(rt.addr(signedView, 2), rt.addr(lp, 2));

  // Out-of-bounds byte slice / index on scalar storage fails closed (e.g. nested [[u16; 2]; 2]).
  const inner = [0x0102, 0x0304];
  const innerBv = rt.scale_scalar({ buf: inner, off: 0 }, 2, "u16");
  assert.throws(
    () => rt.slice_from_parts(innerBv, 8, 1),
    (err) => rt.is_abort(err) && /exceeds 4-byte/.test(err.message),
  );
  assert.throws(
    () => innerBv.buf[4],
    (err) => rt.is_abort(err) && /out of bounds/.test(err.message),
  );
  assert.throws(
    () => {
      innerBv.buf[4] = 1;
    },
    (err) => rt.is_abort(err) && /out of bounds/.test(err.message),
  );

  // Prefix scalar cast preserves aliasing byte view, addr, ptr_eq, and unproject round-trip.
  const wrap = { a: 0x11223344, b: 0x55667788 };
  const pw = rt.box(wrap);
  const pa = rt.prefix(pw, "a", 8);
  const pb = rt.scale_scalar(pa, 4, "u32");
  assert.deepEqual([pb.buf[0], pb.buf[1], pb.buf[2], pb.buf[3]], [0x44, 0x33, 0x22, 0x11]);
  assert.equal(rt.addr(pb, 1), rt.addr(pw, 8));
  assert.equal(rt.addr({ buf: pb.buf, off: 3 }, 1), rt.addr(pw, 8) + 3);
  assert.equal(rt.ptr_eq(pb, pw), true);
  assert.throws(
    () => pb.buf[4],
    (err) => rt.is_abort(err) && /out of bounds/.test(err.message),
  );
  assert.equal(rt.unproject(rt.unscale_scalar(pb, 4, () => 0, "u32"), "a"), pw);

  // Non-scalar aggregate storage fails closed loudly on both slice and single-byte deref.
  const pairs = [{ a: 1, b: 2 }];
  const scaledPair = rt.scale({ buf: pairs, off: 0 }, 8);
  assert.throws(
    () => scaledPair.buf[0],
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => {
      scaledPair.buf[0] = 1;
    },
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.equal(rt.unscale(scaledPair, 8).buf, pairs);
  assert.throws(
    () => rt.slice_from_parts(scaledPair, 8, 1),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  const pairHeap = rt.alloc(8, 4);
  const pairTyped = rt.unscale(pairHeap, 8, () => ({ a: 0, b: 0 }));
  const rawPairSlice = rt.slice_from_parts(pairHeap, 8, 1);
  assert.throws(
    () => rt.byte_slice_ref(rawPairSlice),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rawPairSlice.buf[0],
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => {
      rawPairSlice.buf[0] = 1;
    },
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.equal(rt.unscale(rawPairSlice, 8, () => ({ a: 0, b: 0 })).buf, pairTyped.buf);
  rt.dealloc(rawPairSlice, 8, 4);
});
