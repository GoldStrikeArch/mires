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

test("retyped alloc byte arithmetic, compiler-typed non-scalar copy/abort, and prefix polish (MIRES-008)", () => {
  // 1. Non-scalar retyped allocation: aligned unscale, ptr_eq/addr with bare heap aggregate,
  //    compiler-typed clone on copy/copy_nonoverlapping (owned deep clone vs &T direct rebind),
  //    and fail-closed misaligned/partial copy & write_bytes.
  const clonePair = ($e) => ({ a: $e.a, b: $e.b });
  const a = rt.alloc(32, 4);
  const p = rt.unscale(a, 8, () => ({ a: 0, b: 0 }), clonePair);
  rt._put(p.buf, 0, { a: 10, b: 11 }, true);
  rt._put(p.buf, 1, { a: 20, b: 21 }, true);
  rt._put(p.buf, 2, { a: 30, b: 31 }, true);
  const a1 = { buf: a.buf, off: 8 };
  const p1 = rt.unscale(a1, 8, () => ({ a: 0, b: 0 }), clonePair);
  assert.equal(p1.buf, p.buf);
  assert.equal(p1.off, 1);
  assert.equal(rt.ptr_eq(a1, p.buf[1]), true);
  assert.equal(rt.addr(p.buf[1], 1), rt.addr(a1, 1));
  assert.throws(
    () => rt.unscale({ buf: a.buf, off: 4 }, 8, () => ({ a: 0, b: 0 }), clonePair),
    (err) => rt.is_abort(err) && /not a multiple of 8/.test(err.message),
  );

  // Aligned whole-element byte copy clones owned aggregates independently (including fresh dst
  // retype for MIR-GVN-folded Vec::with_capacity) and aborts on misaligned/partial copies.
  const b = rt.alloc(32, 4);
  rt.copy_nonoverlapping({ buf: b.buf, off: 16 }, a1, 8);
  const pb = rt.unscale(b, 8, () => ({ a: 0, b: 0 }), clonePair);
  assert.deepEqual(pb.buf[2], { a: 20, b: 21 });
  pb.buf[2].a = 999;
  assert.equal(p.buf[1].a, 20);
  rt.copy({ buf: a.buf, off: 16 }, a1, 16);
  p.buf[2].a = 888;
  assert.equal(p.buf[1].a, 20);
  assert.throws(
    () => rt.copy_nonoverlapping({ buf: b.buf, off: 4 }, a1, 8),
    (err) => rt.is_abort(err),
  );
  assert.throws(
    () => rt.copy_nonoverlapping({ buf: b.buf, off: 0 }, a1, 4),
    (err) => rt.is_abort(err),
  );

  // Non-indirect (&Pair / &u32) byte copies (cl omitted) rebind slots directly without
  // mutating borrowed referents or poisoning _boxes.
  const localP = { a: 42, b: 43 };
  const dummyP = { a: 7, b: 8 };
  const rSrc = rt.alloc(8, 4);
  const rSrcP = rt.unscale(rSrc, 4, null);
  rSrcP.buf[0] = localP;
  rSrcP.buf[1] = localP;
  const rDst = rt.alloc(8, 4);
  rt.copy_nonoverlapping(rDst, rSrc, 8);
  const rDstP = rt.unscale(rDst, 4, null);
  assert.equal(rDstP.buf[0], localP);
  assert.equal(rt.ptr_eq(localP, rDst), false);
  const stackRefs = [dummyP, dummyP];
  rt.copy_nonoverlapping(rt.scale({ buf: stackRefs, off: 0 }, 4), rSrc, 8);
  assert.equal(stackRefs[0], localP);
  assert.equal(dummyP.a, 7);
  rt.dealloc(rSrc, 8, 4);
  rt.dealloc(rDst, 8, 4);

  // write_array(..., true) and convert_nv_block(..., clonePair) register owned aggregates in _boxes,
  // while non-indirect write_array (e.g. Vec<&Pair>) never registers borrowed referents.
  const wa = rt.alloc(16, 4);
  const wp = rt.unscale(wa, 8, () => ({ a: 0, b: 0 }), clonePair);
  rt.write_array({ buf: wp.buf, off: 0, w: 2 }, [{ a: 1, b: 2 }, { a: 3, b: 4 }], true);
  assert.equal(rt.ptr_eq({ buf: wa.buf, off: 8 }, wp.buf[1]), true);
  assert.equal(rt.addr(wp.buf[1], 1), rt.addr({ buf: wa.buf, off: 8 }, 1));
  const borrowed = { a: 42, b: 43 };
  const ra = rt.alloc(8, 4);
  const rp = rt.unscale(ra, 4, () => 0);
  rt.write_array({ buf: rp.buf, off: 0, w: 2 }, [borrowed, borrowed]);
  assert.equal(rt.ptr_eq(borrowed, rp), false);
  const nvBlock = rt.convert_nv_block({ buf: [{ a: 5, b: 6 }, { a: 7, b: 8 }], len: 2, cap: 2 }, 8, clonePair);
  const nvTyped = rt.unscale(nvBlock, 8, () => ({ a: 0, b: 0 }), clonePair);
  assert.equal(rt.ptr_eq({ buf: nvBlock.buf, off: 8 }, nvTyped.buf[1]), true);
  rt.dealloc(wa, 16, 4);
  rt.dealloc(ra, 8, 4);
  rt.dealloc(nvBlock, 16, 4);

  // write_bytes(0) fails closed on both raw non-scalar alloc proxy and scaled non-scalar pointer.
  assert.throws(
    () => rt.write_bytes(a1, 0, 8),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.write_bytes({ buf: rt.scale(p, 8, clonePair).buf, off: 8, sc: 8 }, 0, 8),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  rt.dealloc(a, 32, 4);
  rt.dealloc(b, 32, 4);

  // 2. Non-fresh scalar retype guard fires at both offset 0 and non-zero offsets on fromAlloc.
  const sa = rt.alloc(8, 2);
  const sp = rt.unscale_scalar(sa, 2, () => 0, "u16");
  sp.buf[0] = 11;
  sp.buf[1] = 22;
  assert.equal(rt.unscale_scalar({ buf: sa.buf, off: 2 }, 2, () => 0, "u16").off, 1);
  assert.throws(
    () => rt.unscale_scalar({ buf: sa.buf, off: 0 }, 4, () => 0, "u32"),
    (err) => rt.is_abort(err) && /already holds 2-byte/.test(err.message),
  );
  assert.throws(
    () => rt.unscale_scalar({ buf: sa.buf, off: 2 }, 4, () => 0, "u32"),
    (err) => rt.is_abort(err) && /already holds 2-byte/.test(err.message),
  );
  rt.dealloc(sa, 8, 2);

  // 3. align == 1 retyped allocations: byte offsets, ptr_eq on erased pointers, and thin offset-0 guard.
  const a1u16 = rt.alloc(8, 1);
  const p1u16 = rt.unscale_scalar(a1u16, 2, () => 0, "u16");
  p1u16.buf[0] = 11;
  p1u16.buf[1] = 22;
  p1u16.buf[2] = 33;
  assert.equal(rt.unscale_scalar({ buf: a1u16.buf, off: 2 }, 2, () => 0, "u16").off, 1);
  assert.equal(rt.ptr_eq({ buf: a1u16.buf, off: 2 }, rt.erase({ buf: p1u16.buf, off: 1 }, 2, "u16")), true);
  assert.throws(
    () => rt.thin({ buf: a1u16.buf, off: 0 }, 4),
    (err) => rt.is_abort(err) && /already holds 2-byte/.test(err.message),
  );
  const s1u16 = rt.slice_from_parts({ buf: a1u16.buf, off: 2 }, 4, 1);
  assert.deepEqual([s1u16.buf[2], s1u16.buf[3], s1u16.buf[4], s1u16.buf[5]], [22, 0, 33, 0]);
  rt.write_bytes({ buf: a1u16.buf, off: 2 }, 0xab, 2);
  assert.equal(p1u16.buf[1], 0xabab);
  rt.dealloc(a1u16, 8, 1);

  // 4. Nested prefix retains outermost stride in _unwrap_view & ptr_cmp.
  const arr = [{ inner: { x: 1, y: 2 }, z: 3n }, { inner: { x: 0x11223344, y: 5 }, z: 9n }];
  const po = { buf: arr, off: 1 };
  const pi = rt.prefix(po, "inner", 16);
  const px = rt.prefix(pi, "x", 8);
  const pbx = rt.scale_scalar(px, 4, "u32");
  assert.equal(rt.addr(pbx, 1), rt.addr(po, 16));
  assert.equal(rt.addr(pbx, 1), rt.addr(px, 4));
  assert.equal(rt.ptr_eq(pbx, px), true);
  assert.equal(rt.ptr_cmp(rt.prefix(rt.prefix({ buf: arr, off: 0 }, "inner", 16), "x", 8), px, 4), -1);

  // 5. Prefix byte view cache does not mutate direct field byte view entry.
  const w = { a: 0xdeadbeef, b: 0x12345678 };
  const pf = rt.scale_scalar({ buf: w, off: "a" }, 4, "u32");
  const addrBefore = rt.addr(pf, 1);
  const pp = rt.scale_scalar(rt.prefix(rt.box(w), "a", 8), 4, "u32");
  assert.equal(rt.addr(pf, 1), addrBefore);
  assert.equal(rt.addr(pp, 1), rt.addr(rt.box(w), 8));
  assert.notEqual(pf.buf, pp.buf);

  // 6. ptr_eq on shared _non_scalar_byte_view (sc === undefined vs sc === 8).
  const nsBuf = [{ a: 1, b: 2 }, { a: 3, b: 4 }];
  const nsScaled = rt.scale({ buf: nsBuf, off: 1 }, 8);
  const nsNoSc = { buf: nsScaled.buf, off: 8 };
  assert.equal(rt.ptr_eq(nsScaled, nsNoSc), true);

  // 7. ptr_eq + thin roundtrip for wider scalars (stack, heap align > 1, heap align == 1).
  const stk32 = [10, 20, 30, 40];
  const sp1 = { buf: stk32, off: 1 };
  const sp1Rt = rt.thin(rt.erase(sp1, 4, "u32"), 4);
  assert.equal(sp1Rt.es, undefined);
  assert.equal(rt.ptr_eq(sp1, sp1Rt), true);
  assert.equal(rt.ptr_eq(sp1, rt.erase(sp1, 4, "u32")), true);
  const hp16 = rt.alloc(8, 2);
  const hp16T = rt.unscale_scalar(hp16, 2, () => 0, "u16");
  const hp16One = { buf: hp16T.buf, off: 1 };
  assert.equal(rt.ptr_eq(hp16One, rt.thin(rt.erase(hp16One, 2, "u16"), 2)), true);
  assert.equal(rt.ptr_eq(hp16One, rt.erase(hp16One, 2, "u16")), true);
  rt.dealloc(hp16, 8, 2);
  const hp16A1 = rt.alloc(8, 1);
  const hp16A1T = rt.unscale_scalar(hp16A1, 2, () => 0, "u16");
  const hp16A1One = { buf: hp16A1T.buf, off: 1 };
  assert.equal(rt.ptr_eq(hp16A1One, rt.thin(rt.erase(hp16A1One, 2, "u16"), 2)), true);
  rt.dealloc(hp16A1, 8, 1);

  // 8. Prefix byte-copy fails closed both directly (pv present) and after .add(0) (pv dropped).
  const prefScaled = rt.scale(rt.prefix({ buf: arr, off: 0 }, "inner", 16), 8, clonePair);
  const prefDropPv = { buf: prefScaled.buf, off: 0, sc: prefScaled.sc };
  const dstInner = rt.scale({ buf: [{ x: 0, y: 0 }], off: 0 }, 8, clonePair);
  assert.throws(
    () => rt.copy_nonoverlapping(dstInner, prefScaled, 8),
    (err) => rt.is_abort(err) && /prefix/.test(err.message),
  );
  assert.throws(
    () => rt.copy_nonoverlapping(dstInner, prefDropPv, 8),
    (err) => rt.is_abort(err) && /prefix/.test(err.message),
  );

  // 9. _uninit_blocks source (GVN-folded unscale_uninit) copies cleanly into non-scalar byte view.
  const rawMu = new Array(16);
  const pMu = rt.unscale_uninit({ buf: rawMu, off: 0 }, 8, () => ({ a: 0, b: 0 }), clonePair);
  pMu.buf[0] = { a: 31, b: 32 };
  const dstMuArr = [undefined, undefined];
  rt.copy_nonoverlapping(rt.scale({ buf: dstMuArr, off: 0 }, 8, clonePair), { buf: rawMu, off: 0 }, 16);
  dstMuArr[0].a = 666;
  assert.equal(pMu.buf[0].a, 31);
  assert.equal(dstMuArr[1], undefined);
});

test("provenance-free slice addresses preserve addr, ptr_is_null, and ptr_eq", () => {
  const dangling = rt.slice_from_parts(4, 0);
  assert.equal(rt.addr(dangling, 4), 4);
  assert.equal(rt.ptr_is_null(dangling, 4), false);
  assert.equal(rt.ptr_eq(dangling, 4), true);
  assert.equal(rt.ptr_eq(4, dangling), true);
  assert.equal(rt.ptr_eq(dangling, 8), false);

  const custom = rt.slice_from_parts(4096, 5);
  assert.deepEqual(custom, { buf: 4096, off: 0, len: 5 });
  assert.equal(rt.addr(custom, 4), 4096);
  assert.equal(rt.addr(rt.offset(custom, 2, 4), 4), 4104);
  assert.equal(rt.ptr_is_null(custom, 4), false);
  assert.equal(rt.ptr_eq(custom, 4096), true);
  assert.equal(rt.ptr_eq(4096, custom), true);
  assert.equal(rt.ptr_eq(custom, 8192), false);

  const nul = rt.slice_from_parts(0, 0);
  assert.deepEqual(nul, { buf: 0, off: 0, len: 0 });
  assert.equal(rt.addr(nul, 4), 0);
  assert.equal(rt.ptr_is_null(nul, 4), true);
  assert.equal(rt.ptr_eq(nul, 0), true);
  assert.equal(rt.ptr_eq(0, nul), true);

  // `unwindow_slice` on bare dangling/custom addresses (`Box::<[T]>::default()` unsizing).
  const unwDangling = rt.unwindow_slice(4, 0);
  assert.equal(unwDangling.len, 0);
  assert.equal(rt.addr(unwDangling, 4), 4);
  assert.equal(rt.ptr_eq(unwDangling, dangling), true);
  assert.equal(rt.ptr_eq(unwDangling, 4), true);
  const unwCustom = rt.unwindow_slice(4096, 0);
  assert.deepEqual(unwCustom, { buf: 4096, off: 0, len: 0 });

  // `scale_scalar` / `scale` / `thin` / `offset` / `unscale` / `slice_from_parts` on bare-address
  // and dangling slices return bare numeric addresses and agree across `ptr_eq`, `ptr_cmp`, `addr`.
  const scaledCustom = rt.scale_scalar(custom, 4, "u32", false);
  assert.equal(scaledCustom, 4096);
  assert.equal(rt.addr(scaledCustom, 1), 4096);
  assert.equal(rt.ptr_is_null(scaledCustom, 1), false);
  assert.equal(rt.ptr_eq(scaledCustom, custom), true);
  assert.deepEqual(rt.slice_from_parts(scaledCustom, 5, 1), { buf: 4096, off: 0, len: 5 });
  assert.equal(rt.unscale(scaledCustom, 4, null), 4096);
  assert.equal(rt.unscale_scalar(scaledCustom, 4, null, "u32"), 4096);
  assert.equal(rt.unscale_scalar(custom, 4, null, "u32"), 4096);
  const scaledNull = rt.scale_scalar(nul, 4, "u32", false);
  assert.equal(scaledNull, 0);
  assert.equal(rt.addr(scaledNull, 1), 0);
  assert.equal(rt.ptr_is_null(scaledNull, 1), true);
  assert.equal(rt.scale(custom, 8), 4096);
  assert.equal(rt.scale(nul, 8), 0);
  assert.equal(rt.thin(rt.erase(custom, 4, "u32"), 1, false), 4096);
  assert.equal(rt.thin(rt.erase(custom, 8), 1, false), 4096);
  assert.deepEqual(rt.slice_from_parts(rt.erase(custom, 4, "u32"), 0, 1), {
    buf: 4096,
    off: 0,
    len: 0,
  });

  // Dangling byte views (`scale_scalar` / `scale` over `_dangling`) compare equal to bare numbers
  // and bare ZST slice objects `{ buf: 4, off: 0, len: 0 }`.
  const danglingU8 = rt.scale_scalar(dangling, 4, "u32", false);
  assert.equal(rt.addr(danglingU8, 1), 4);
  assert.equal(rt.ptr_eq(danglingU8, 4), true);
  assert.equal(rt.ptr_eq(4, danglingU8), true);
  assert.equal(rt.ptr_eq(danglingU8, dangling), true);
  assert.equal(rt.ptr_eq({ buf: 4, off: 0, len: 0 }, dangling), true);
  assert.equal(rt.ptr_eq({ buf: 4, off: 0, len: 0 }, danglingU8), true);
  assert.equal(rt.ptr_eq(rt.scale(dangling, 8), 4), true);

  // Offset bare-address slices preserve exact 32-bit addresses, `ptr_eq`, and `ptr_cmp` ordering.
  const customOff2 = rt.offset(custom, 2, 4);
  assert.equal(customOff2, 4104);
  assert.equal(rt.ptr_eq(customOff2, 4104), true);
  assert.equal(rt.ptr_eq(customOff2, rt.slice_from_parts(4104, 3)), true);
  assert.equal(rt.ptr_cmp(rt.offset(custom, 1, 4), rt.offset(scaledCustom, 2, 1), 1), 1);
  assert.equal(rt.ptr_cmp({ buf: 4096, off: 1, es: 4 }, { buf: 4096, off: 2, sc: 4 }, 1), 1);
  const huge = rt.slice_from_parts(0xfffffff0, 0);
  assert.equal(rt.addr(huge, 1), 0xfffffff0);
  assert.equal(rt.ptr_cmp(huge, 4096, 1), 1);
  const hugeOff16 = rt.offset(huge, 16, 1);
  assert.equal(hugeOff16, 0);
  assert.equal(rt.addr(hugeOff16, 1), 0);
  assert.equal(rt.ptr_is_null(hugeOff16, 1), true);

  // Poisoned offset-from-address records still throw in ptr_eq even when compared to a number.
  assert.throws(
    () => rt.ptr_eq({ buf: undefined, off: NaN }, 4),
    (err) => rt.is_abort(err),
  );
  assert.throws(
    () => rt.ptr_eq(4, { buf: undefined, off: NaN }),
    (err) => rt.is_abort(err),
  );
});
