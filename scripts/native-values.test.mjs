import assert from "node:assert/strict";
import test from "node:test";
import { types } from "node:util";
import { readFileSync } from "node:fs";
import vm from "node:vm";

import "../runtime/shim.js";
import { minimalShim } from "./minimal-shim.mjs";

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

test("host dec2flt (parse_f64 and parse_f32) matches Rust grammar and double-rounding rules", () => {
  for (const bad of ["", " ", "+", "-", ".", "1e", "1e+", "0x10", "1_0", " 1.0", "1.0 ", "infin", "nana"]) {
    assert.equal(rt.parse_f64(bad), undefined);
    assert.equal(rt.parse_f32(bad), undefined);
  }
  assert.equal(Object.is(rt.parse_f64("-0.0"), -0), true);
  assert.equal(Object.is(rt.parse_f32("-0.0"), -0), true);
  assert.equal(rt.parse_f64("+Infinity"), Infinity);
  assert.equal(rt.parse_f64("-inf"), -Infinity);
  assert.equal(Number.isNaN(rt.parse_f64("NaN")), true);
  assert.equal(Number.isNaN(rt.parse_f64("-nan")), true);
  const dv = new DataView(new ArrayBuffer(8));
  dv.setFloat64(0, rt.parse_f64("-nan"), false);
  assert.equal(dv.getUint32(0, false) >>> 31, 1);

  // Known f32 double-rounding traps
  const f32Bits = (v) => {
    dv.setFloat32(0, v, false);
    return dv.getUint32(0, false);
  };
  assert.equal(f32Bits(rt.parse_f32("7.038531e-26")), 0x15ae43fd);
  assert.equal(f32Bits(rt.parse_f32("1.000000059604644775390625")), 0x3f800000);
  assert.equal(f32Bits(rt.parse_f32("1.00000005960464477539062500000000000000000000001")), 0x3f800001);
  assert.equal(f32Bits(rt.parse_f32("1.000000178813934326171875")), 0x3f800002);
  assert.equal(f32Bits(rt.parse_f32("1.00000017881393432617187499999999999999999999999")), 0x3f800001);
  assert.equal(f32Bits(rt.parse_f32("3.40282356779733661637539395458142568447999999e38")), 0x7f7fffff);
  assert.equal(f32Bits(rt.parse_f32("3.40282356779733661637539395458142568448000001e38")), 0x7f800000);
  assert.equal(f32Bits(rt.parse_f32("7.006492321624085354618647916449580656401309709382578858785341419448955413429303007433190941810607910156249e-46")), 0x00000000);
  assert.equal(f32Bits(rt.parse_f32("7.006492321624085354618647916449580656401309709382578858785341419448955413429303007433190941810607910156251e-46")), 0x00000001);
});

test("host float fixed/f32-shortest and integer hex/dec formatters match Rust", () => {
  assert.equal(rt.fmt_float_fixed(0.5, 0, false), "0");
  assert.equal(rt.fmt_float_fixed(1.5, 0, false), "2");
  assert.equal(rt.fmt_float_fixed(2.5, 0, false), "2");
  assert.equal(rt.fmt_float_fixed(3.5, 0, false), "4");
  assert.equal(rt.fmt_float_fixed(-0.0, 3, false), "-0.000");
  assert.equal(rt.fmt_float_fixed(NaN, 2, true), "NaN");
  assert.equal(rt.fmt_float_fixed(Infinity, 2, true), "+inf");
  assert.equal(rt.fmt_float_fixed(1.005, 2, false), "1.00");
  assert.equal(rt.fmt_float_fixed(1e25, 0, false), "10000000000000000905969664");
  assert.equal(rt.native_f32_to_string(Math.fround(0.1)).text, "0.1");
  assert.equal(rt.native_f32_to_string(Math.fround(1 / 3)).text, "0.33333334");
  assert.equal(rt.fmt_int_hex(-1, 8, false, 0, false, false), "ff");
  assert.equal(rt.fmt_int_hex(-1, 32, false, 0, false, false), "ffffffff");
  assert.equal(rt.fmt_int_hex(10, 8, true, 6, false, true), "   0xA");
  assert.equal(rt.fmt_int_hex(10, 8, false, 6, true, true), "0x000a");
  assert.equal(rt.fmt_int_hex(BigInt("-1"), 64, false, 0, false, false), "ffffffffffffffff");
  assert.equal(rt.fmt_int_dec(-42, 5, true, false), "-0042");
  assert.equal(rt.fmt_int_dec(42, 5, true, true), "+0042");
  assert.equal(rt.fmt_int_dec(0, 0, false, true), "+0");
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

  // Heap block retyped via unscale_scalar records kind for GVN-folded raw pointers without eager Proxy.
  const allocPtr = rt.alloc(4, 2);
  assert.equal(types.isProxy(allocPtr.buf), false);
  const typed = rt.unscale_scalar(allocPtr, 2, () => 0, "u16");
  assert.equal(types.isProxy(allocPtr.buf), false);
  assert.equal(types.isProxy(typed.buf), false);
  assert.notEqual(allocPtr.buf, typed.buf);
  assert.equal(allocPtr.buf.length, 0);
  typed.buf[0] = 0x0102;
  typed.buf[1] = 0x0304;
  // Direct GVN-folded byte reads/writes, offset slice, copy, write_bytes, realloc.
  assert.deepEqual(
    [0, 1, 2, 3].map((k) => rt.read_byte({ buf: allocPtr.buf, off: k })),
    [0x02, 0x01, 0x04, 0x03],
  );
  const gvnSlice = rt.byte_slice_ref(rt.slice_from_parts(allocPtr, 4, 1));
  assert.deepEqual(Array.prototype.slice.call(gvnSlice.buf, 0, 4), [0x02, 0x01, 0x04, 0x03]);
  const gvnOffSlice = rt.byte_slice_ref(rt.slice_from_parts({ buf: allocPtr.buf, off: 1 }, 2, 1));
  assert.deepEqual([gvnOffSlice.buf[gvnOffSlice.off], gvnOffSlice.buf[gvnOffSlice.off + 1]], [0x01, 0x04]);
  assert.equal(rt.ptr_eq({ buf: allocPtr.buf, off: 2 }, { buf: typed.buf, off: 1 }), true);
  assert.equal(rt.ptr_eq({ buf: allocPtr.buf, off: 1 }, { buf: typed.buf, off: 1 }), false);
  const grown = rt.realloc(allocPtr, 4, 2, 8);
  assert.equal(grown.buf, allocPtr.buf);
  assert.equal(types.isProxy(grown.buf), false);
  typed.buf[2] = 0x0506;
  assert.deepEqual(
    [rt.read_byte({ buf: grown.buf, off: 4 }), rt.read_byte({ buf: grown.buf, off: 5 })],
    [0x06, 0x05],
  );
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

  // Non-scalar aggregate storage fails closed loudly on both slice and single-byte deref without Proxy.
  const pairs = [{ a: 1, b: 2 }];
  const scaledPair = rt.scale({ buf: pairs, off: 0 }, 8);
  assert.equal(types.isProxy(scaledPair.buf), false);
  assert.throws(
    () => rt.read_byte(scaledPair),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.write_byte(scaledPair, 1),
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
  assert.equal(types.isProxy(rawPairSlice.buf), false);
  assert.throws(
    () => rt.byte_slice_ref(rawPairSlice),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.read_byte(rawPairSlice),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.write_byte(rawPairSlice, 1),
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
  const s1u16 = rt.byte_slice_ref(rt.slice_from_parts({ buf: a1u16.buf, off: 2 }, 4, 1));
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

  // 6. ptr_eq on empty slice built from scaled non-scalar pointer preserves sc and equality.
  const nsBuf = [{ a: 1, b: 2 }, { a: 3, b: 4 }];
  const nsScaled = rt.scale({ buf: nsBuf, off: 1 }, 8);
  const nsSlice = rt.slice_from_parts(nsScaled, 0, 1);
  assert.equal(rt.ptr_eq(nsScaled, nsSlice), true);
  assert.equal(rt.ptr_eq(nsScaled, { buf: nsBuf, off: 1 }), true);

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

test("slice_ptr_cmp and fat_ptr_cmp order by data address first and tie-break by metadata", () => {
  // 1. Dangling ZST slices with equal address and unequal length (`slice_order.rs`).
  const zstShort = rt.slice_from_parts(1, 1);
  const zstLong = rt.slice_from_parts(1, 2);
  assert.equal(rt.slice_ptr_eq(zstShort, zstLong), false);
  assert.equal(rt.slice_ptr_cmp(zstShort, zstLong, 0), -1);
  assert.equal(rt.slice_ptr_cmp(zstLong, zstShort, 0), 1);
  assert.equal(rt.slice_ptr_cmp(zstShort, rt.slice_from_parts(1, 1), 0), 0);

  // 2. Non-ZST slices: equal addr + unequal len, unequal addr + reversed len.
  const arr = [10, 20, 30, 40];
  const s01 = { buf: arr, off: 0, len: 1 };
  const s02 = { buf: arr, off: 0, len: 2 };
  const s11 = { buf: arr, off: 1, len: 1 };
  const s03 = { buf: arr, off: 0, len: 3 };
  assert.equal(rt.slice_ptr_cmp(s01, s02, 4), -1);
  assert.equal(rt.slice_ptr_cmp(s02, s01, 4), 1);
  assert.equal(rt.slice_ptr_cmp(s11, s03, 4), 1);
  assert.equal(rt.slice_ptr_cmp(s03, s11, 4), -1);
  // Bare-address slice matching s11's synthetic address tie-breaks by length using elem size 4.
  const s11Addr = rt.addr(s11, 4);
  const s11BareLong = rt.slice_from_parts(s11Addr, 5);
  assert.equal(rt.slice_ptr_cmp(s11, s11BareLong, 4), -1);
  assert.equal(rt.slice_ptr_cmp(s11BareLong, s11, 4), 1);

  // 3. `*const str` / `*mut str` byte slices (elem size 1).
  const bytes = new Uint8Array([104, 101, 108, 108, 111]);
  const strShort = { buf: bytes, off: 0, len: 2 };
  const strLong = { buf: bytes, off: 0, len: 4 };
  const strTail = { buf: bytes, off: 1, len: 1 };
  assert.equal(rt.slice_ptr_cmp(strShort, strLong, 1), -1);
  assert.equal(rt.slice_ptr_cmp(strLong, strShort, 1), 1);
  assert.equal(rt.slice_ptr_cmp(strTail, strLong, 1), 1);
  assert.equal(rt.slice_ptr_cmp(strLong, strTail, 1), -1);

  // 4. `*const dyn Trait`: equal data ptr with same & distinct vtables, antisymmetry & total order.
  const objA = { x: 1 };
  const objB = { x: 2 };
  const vtable1 = [() => {}, 4, 4, () => 1];
  const vtable2 = [() => {}, 4, 4, () => 2];
  const dynList = [
    { ptr: objA, meta: vtable1 },
    { ptr: objA, meta: vtable2 },
    { ptr: objB, meta: vtable1 },
    { ptr: objB, meta: vtable2 },
    { ptr: undefined, meta: vtable1 },
    { ptr: undefined, meta: vtable2 },
  ];
  for (let i = 0; i < dynList.length; i++) {
    assert.equal(rt.fat_ptr_cmp(dynList[i], dynList[i], 1), 0);
    for (let j = 0; j < dynList.length; j++) {
      const ab = rt.fat_ptr_cmp(dynList[i], dynList[j], 1);
      const ba = rt.fat_ptr_cmp(dynList[j], dynList[i], 1);
      const eq = rt.fat_ptr_eq(dynList[i], dynList[j]);
      assert.equal(ab, ba === 0 ? 0 : -ba);
      assert.equal(eq, ab === 0);
      assert.equal(ab === 0, i === j);
    }
  }

  // 5. Unsized struct tail `{ ptr, meta: len }`.
  const slot = { buf: [{ hdr: 1 }], off: 0 };
  const tail1 = { ptr: slot, meta: 1 };
  const tail3 = { ptr: slot, meta: 3 };
  assert.equal(rt.fat_ptr_cmp(tail1, tail3, 1), -1);
  assert.equal(rt.fat_ptr_cmp(tail3, tail1, 1), 1);
  assert.equal(rt.fat_ptr_cmp(tail1, { ptr: slot, meta: 1 }, 1), 0);
});

test("ordinary Vec/Box allocs stay Proxy-free and minimal-shim excludes byte-view machinery", () => {
  // 1. Runtime: alloc, alloc_zeroed, unscale, unscale_scalar, realloc, slice_from_parts,
  //    convert_nv_block, and dealloc never allocate a Proxy.
  for (const [size, align, es, kind] of [
    [16, 4, 4, "u32"],
    [32, 8, 8, null],
    [8, 1, 2, "u16"],
  ]) {
    const a = rt.alloc(size, align);
    assert.equal(types.isProxy(a.buf), false);
    const addrBefore = rt.addr(a, 1);
    const sRaw = rt.slice_from_parts(a, size, 1);
    assert.equal(types.isProxy(sRaw.buf), false);
    const p = kind ? rt.unscale_scalar(a, es, () => 0, kind) : rt.unscale(a, es, () => ({ x: 0 }));
    assert.equal(types.isProxy(p.buf), false);
    assert.equal(rt.addr(p, es), addrBefore);
    assert.equal(rt.addr(a, 1), addrBefore);
    assert.equal(rt.ptr_eq(a, p), true);
    const g = rt.realloc(a, size, align, size * 2);
    assert.equal(types.isProxy(g.buf), false);
    const gRaw = rt.slice_from_parts(g, size * 2, 1);
    assert.equal(types.isProxy(gRaw.buf), false);
    assert.equal(rt._byte_view_of.has(a.buf), false);
    assert.equal(rt._byte_view_of.has(p.buf), false);
    rt.dealloc(g, size * 2, align);
    assert.throws(() => rt.read_byte(a), (err) => rt.is_abort(err) && /freed/.test(err.message));
  }
  const nv = rt.convert_nv_block({ buf: [1, 2, 3], len: 3, cap: 4 }, 4, "u32");
  assert.equal(types.isProxy(nv.buf), false);
  const nvTyped = rt.unscale_scalar(nv, 4, () => 0, "u32");
  assert.equal(types.isProxy(nvTyped.buf), false);
  assert.deepEqual(nvTyped.buf.slice(0, 3), [1, 2, 3]);
  rt.dealloc(nv, 16, 4);

  // 2. Static minimal-shim extraction: plain Vec<u8>/Vec<u32>/Vec<Struct> programs exclude
  //    all byte-view helpers and constant-fold `$rt__bv = null` in module mode.
  const shimSource = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const plainDirect = [
    "alloc", "alloc_zeroed", "realloc", "dealloc", "unscale", "unscale_scalar",
    "slice_from_parts", "byte_slice_ref", "copy", "copy_nonoverlapping",
    "write_bytes", "compare_bytes", "ptr_eq", "ptr_is_null", "addr", "scale",
  ];
  const plain = minimalShim([], shimSource, { module: true, direct: plainDirect });
  for (const forbidden of [
    "_bv_enable", "_byte_view", "_byte_load", "_byte_store",
    "_byte_views", "_prefix_byte_views", "_byte_view_of",
    "_prep_copy", "_unwrap_view", "_alloc_byte_proxy", "_non_scalar_byte_view",
  ]) {
    assert.equal(plain.names.includes(forbidden), false, `plain shim must not include ${forbidden}`);
  }
  assert.match(plain.source, /const \$rt__bv = null;/);

  // Programs that reach wide byte views pull `_bv_enable` and the lazy byte-view helpers.
  for (const trigger of ["scale_scalar", "byte_sign", "read_byte", "write_byte"]) {
    const withBv = minimalShim([], shimSource, { module: true, direct: ["alloc", trigger] });
    assert.equal(withBv.names.includes("_bv_enable"), true, `${trigger} must pull _bv_enable`);
    assert.equal(withBv.names.includes("_byte_view"), true, `${trigger} must pull _byte_view`);
    assert.match(withBv.source, /let \$rt__bv = null;/);
  }
});

test("minimal-shim ptr_eq/addr (_bv === null), scaled realloc non-scalar abort, and post-retype byte_slice_ref", () => {
  const shimSource = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");

  // 1. Minimal shim with _bv === null: ptr_eq and addr unwrap _boxes and _scalar_view_of,
  //    and wide heap blocks retype in-place (zero-allocation unscale, single array) without
  //    pulling _bv_enable or _byte_view.
  const minDirect = [
    "alloc", "realloc", "dealloc", "scale", "unscale", "unscale_scalar", "box",
    "ptr_eq", "addr", "slice_from_parts", "byte_slice_ref", "compare_bytes",
    "convert_nv_block", "slice_sort", "is_abort",
  ];
  const min = minimalShim([], shimSource, { direct: minDirect });
  assert.equal(min.names.includes("_bv_enable"), false, "minimal shim must exclude _bv_enable");
  assert.equal(min.names.includes("_byte_view"), false, "minimal shim must exclude _byte_view");
  const ctx = { console };
  vm.runInNewContext(min.source, ctx);
  const mrt = ctx.__rt;
  assert.equal(mrt._bv, null);

  // _boxes: raw slot vs bare object reference (Box<Pair> -> Box<dyn Trait>)
  const pair = { a: 1, b: 2 };
  const boxed = mrt.box(pair);
  assert.equal(mrt.ptr_eq(boxed, pair), true);
  assert.equal(mrt.ptr_eq(pair, boxed), true);
  assert.equal(mrt.addr(boxed, 8), mrt.addr(pair, 8));

  // _scalar_view_of: [u8]::align_to::<u32>() scalar view vs underlying byte buffer
  const bytes = [1, 2, 3, 4, 5, 6, 7, 8];
  const sv = mrt.unscale_scalar({ buf: bytes, off: 0 }, 4, () => 0, "u32");
  assert.equal(mrt.ptr_eq({ buf: bytes, off: 0 }, sv), true);
  assert.equal(mrt.ptr_eq({ buf: bytes, off: 4 }, { buf: sv.buf, off: 1 }), true);
  assert.equal(mrt.ptr_eq({ buf: bytes, off: 1 }, sv), false);

  // In-place retype when _bv === null: zero-allocation unscale + _bytes compare_bytes + realloc + dealloc
  const raw = mrt.alloc(16, 4);
  const typed = mrt.unscale_scalar(raw, 4, () => 0, "u32");
  assert.equal(typed, raw, "in-place retype when _bv === null must return the same slot");
  assert.equal(mrt.unscale(raw, 4), raw, "subsequent unscale when _bv === null must not allocate");
  assert.equal(typed.buf.length, 4);
  typed.buf[0] = 10; typed.buf[1] = 20; typed.buf[2] = 30; typed.buf[3] = 40;
  assert.equal(mrt.ptr_eq(raw, typed), true);
  assert.equal(mrt.ptr_eq({ buf: typed.buf, off: 4, sc: 4 }, { buf: typed.buf, off: 1 }), true);
  assert.equal(mrt.ptr_eq({ buf: typed.buf, off: 2, sc: 4 }, typed), false);
  // GVN-folded RawVecInner pointer (`raw`, no `.sc`) compared against a scaled slice of u32s
  const scaledOther = mrt.scale({ buf: [10, 20, 30, 40], off: 0 }, 4);
  assert.equal(mrt.compare_bytes(raw, scaledOther, 16), 0);
  const scaledDiff = mrt.scale({ buf: [10, 20, 99, 40], off: 0 }, 4);
  assert.notEqual(mrt.compare_bytes(raw, scaledDiff, 16), 0);
  // Scaled realloc under _bv === null keeps single in-place array
  const grownMin = mrt.realloc(mrt.scale(typed, 4), 16, 4, 32);
  assert.equal(grownMin.buf, typed.buf);
  assert.equal(grownMin.buf.length, 8);
  assert.equal(mrt.unscale(grownMin, 4), grownMin);
  mrt.dealloc(grownMin, 32, 4);
  assert.throws(() => mrt.unscale(grownMin, 4), (err) => mrt.is_abort(err));
  // convert_nv_block under _bv === null skips raw/typed split
  const nvPtr = mrt.convert_nv_block({ buf: [7, 8, 9], len: 3, cap: 4 }, 4, "u32");
  assert.equal(mrt.unscale(nvPtr, 4), nvPtr);
  assert.equal(nvPtr.buf[2], 9);
  mrt.dealloc(nvPtr, 16, 4);
  // slice_sort fast paths: len=2, len=16, len=40 (stability + panic safety)
  const s2 = [9, 2];
  mrt.slice_sort(s2, 0, 2, null, (_c, a, b) => a.buf[a.off] < b.buf[b.off], false);
  assert.deepEqual(s2, [2, 9]);
  const s40 = Array.from({ length: 40 }, (_, i) => ({ k: (40 - i) % 7, id: i }));
  mrt.slice_sort(s40, 0, 40, null, (_c, a, b) => a.k < b.k, true);
  for (let i = 1; i < 40; i++) {
    assert.ok(s40[i - 1].k < s40[i].k || (s40[i - 1].k === s40[i].k && s40[i - 1].id < s40[i].id));
  }
  const sThrow = [3, 1, 2];
  assert.throws(() => mrt.slice_sort(sThrow, 0, 3, null, () => { throw new Error("boom"); }, false));
  assert.deepEqual(sThrow, [3, 1, 2]);

  // 1b. Full shim (_bv !== null): two-array raw vs typed block ptr_eq
  const fullRaw = rt.alloc(16, 4);
  const fullTyped = rt.unscale_scalar(fullRaw, 4, () => 0, "u32");
  assert.notEqual(fullRaw.buf, fullTyped.buf);
  assert.equal(rt.ptr_eq(fullRaw, fullTyped), true);
  assert.equal(rt.ptr_eq({ buf: fullRaw.buf, off: 4 }, { buf: fullTyped.buf, off: 1 }), true);
  assert.equal(rt.ptr_eq({ buf: fullRaw.buf, off: 2 }, fullTyped), false);
  rt.dealloc(fullRaw, 16, 4);

  // 2. Full shim: realloc on a scaled non-scalar pointer MUST preserve `.sc` so reading/slicing
  //    or writing bytes through the grown pointer still aborts fail-closed.
  const nsRaw = rt.alloc(8, 4);
  const nsTyped = rt.unscale(nsRaw, 8, () => ({ a: 0, b: 0 }));
  nsTyped.buf[0] = { a: 11, b: 22 };
  const scaled = rt.scale(nsTyped, 8);
  assert.equal(scaled.sc, 8);
  const grown = rt.realloc(scaled, 8, 4, 16);
  assert.equal(rt.ptr_eq(grown, nsTyped), true);
  assert.equal(types.isProxy(rt.slice_from_parts(grown, 16, 1).buf), false);
  assert.throws(
    () => rt.read_byte(grown),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.byte_slice_ref(rt.slice_from_parts(grown, 8, 1)),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  assert.throws(
    () => rt.write_bytes(grown, 0xaa, 4),
    (err) => rt.is_abort(err) && /non-scalar/.test(err.message),
  );
  rt.dealloc(grown, 16, 4);

  // 3. Full shim: slice_from_parts on post-retype scalar raw stays Proxy-free, while
  //    byte_slice_ref materializes the aliasing byte view (and aborts on non-scalar raw).
  const scRaw = rt.alloc(8, 4);
  const scTyped = rt.unscale_scalar(scRaw, 4, () => 0, "u32");
  scTyped.buf[0] = 0x11223344;
  const rawSlice = rt.slice_from_parts(scRaw, 4, 1);
  assert.equal(types.isProxy(rawSlice.buf), false);
  const refSlice = rt.byte_slice_ref(rawSlice);
  assert.equal(refSlice.buf[refSlice.off], 0x44);
  assert.equal(refSlice.buf[refSlice.off + 1], 0x33);
  refSlice.buf[refSlice.off + 1] = 0x99;
  assert.equal(scTyped.buf[0], 0x11229944);
  rt.dealloc(scRaw, 8, 4);
});

test("slice_sort is spec-stable, updates _boxes on heap blocks, preserves buf on throw, and handles _bsub_* propagation + bstr case mapping", () => {
  // 1. Stability & subslice offset on direct elements
  const direct = [99, 3, 1, 2, 1, 0, -5];
  rt.slice_sort(direct, 1, 5, undefined, (_ctx, a, b) => a.buf[a.off] < b.buf[b.off], false);
  assert.deepEqual(direct, [99, 0, 1, 1, 2, 3, -5]);

  // 2. Stability & _boxes tracking on indirect heap block objects
  const raw = rt.alloc(32, 8);
  const typed = rt.unscale(raw, 8, () => ({ k: 0, id: 0 }));
  const objs = [
    { k: 2, id: 1 },
    { k: 1, id: 2 },
    { k: 2, id: 3 },
    { k: 0, id: 4 },
  ];
  for (let i = 0; i < 4; i++) {
    typed.buf[i] = objs[i];
    rt._boxes.set(objs[i], { buf: typed.buf, off: i });
  }
  rt.slice_sort(typed.buf, 0, 4, undefined, (_ctx, a, b) => a.k < b.k, true);
  assert.deepEqual(typed.buf.map((o) => o.id), [4, 2, 1, 3]);
  for (let i = 0; i < 4; i++) {
    const slot = rt._boxes.get(typed.buf[i]);
    assert.equal(slot.buf, typed.buf);
    assert.equal(slot.off, i);
  }
  rt.dealloc(raw, 32, 8);

  // 3. Throwing comparator leaves buf completely untouched with interior mutations preserved
  const items = [{ k: 3, seen: 0 }, { k: 1, seen: 0 }, { k: 2, seen: 0 }];
  let calls = 0;
  assert.throws(() => {
    rt.slice_sort(
      items,
      0,
      3,
      undefined,
      (_ctx, a, b) => {
        a.seen += 1;
        b.seen += 1;
        calls += 1;
        if (calls === 2) throw new Error("cmp-throw");
        return a.k < b.k;
      },
      true,
    );
  }, /cmp-throw/);
  assert.deepEqual(items.map((x) => x.k), [3, 1, 2]);
  assert.equal(items.reduce((acc, x) => acc + x.seen, 0), 4);

  // 4. _bsub_* parent-slice propagation in bstr_get / bstr_get_from / bstr_get_to
  const base = "abcdefghij".repeat(20); // 200 bytes > 64
  const baseView = rt.bstr_bytes(base);
  const tail1 = rt.bstr_get_from(base, 17);
  const view1 = rt.bstr_bytes(tail1);
  assert.equal(view1.buf, baseView.buf);
  assert.equal(view1.off, 17);
  assert.equal(view1.len, 183);
  const mid = rt.bstr_get(tail1, 10, 120);
  const view2 = rt.bstr_bytes(mid);
  assert.equal(view2.buf, baseView.buf);
  assert.equal(view2.off, 27);
  assert.equal(view2.len, 110);
  const head = rt.bstr_get_to(mid, 90);
  const view3 = rt.bstr_bytes(head);
  assert.equal(view3.buf, baseView.buf);
  assert.equal(view3.off, 27);
  assert.equal(view3.len, 90);

  // 5. minimal-shim tree-shaking: bstr_get alone does NOT pull _bstr_memo
  const shimSrc = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const getOnly = minimalShim([], shimSrc, { direct: ["bstr_get_from", "bstr_get", "bstr_get_to"] });
  assert.equal(getOnly.source.includes("_bstr_memo"), false);
  assert.equal(getOnly.source.includes("_bstr_sub_propagate"), false);

  // 6. bstr_to_lowercase / bstr_to_uppercase (ASCII fast path + Greek Final Sigma)
  assert.equal(rt.bstr_to_lowercase("Hello WORLD 123"), "hello world 123");
  assert.equal(rt.bstr_to_uppercase("Hello world 123"), "HELLO WORLD 123");
  assert.equal(rt.bstr_out(rt.bstr_to_lowercase(rt.bstr_in("ΟΣ ΟΣΑ Σ"))), "ος οσα σ");
  assert.equal(rt.bstr_out(rt.bstr_to_uppercase(rt.bstr_in("tschüß"))), "TSCHÜSS");
});

test("TypedArray/NativeVec helpers, _element_view_of elision, ptr_eq same-buf fast path, and 2-slot _blk_get", async () => {
  // 1. minimal-shim elides _element_view_of when _element_view is unreachable
  const shimSrc = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const minNoEv = minimalShim([], shimSrc, {
    module: true,
    direct: ["alloc", "dealloc", "ptr_eq", "addr", "offset_from", "overwrite", "unscale_scalar"],
  });
  assert.equal(minNoEv.source.includes("const $rt__element_view_of = null;"), true);
  assert.equal(minNoEv.source.includes("_tv_ctors"), false);
  const mrt = await import(`data:text/javascript;base64,${Buffer.from(minNoEv.source).toString("base64")}`);
  const a = mrt.alloc(16, 4);
  const b = mrt.alloc(16, 4);
  mrt.unscale_scalar(a, 4, () => 0, "u32");
  mrt.unscale_scalar(b, 4, () => 0, "u32");
  assert.equal(mrt.ptr_eq({ buf: a.buf, off: 1 }, { buf: a.buf, off: 1 }), true);
  assert.equal(mrt.ptr_eq({ buf: a.buf, off: 1 }, { buf: a.buf, off: 2 }), false);
  assert.equal(mrt.ptr_eq({ buf: a.buf, off: 0 }, { buf: b.buf, off: 0 }), false);
  // Alternating lookups hit 2-slot _blk_get cache without corruption
  assert.equal(mrt.offset_from({ buf: a.buf, off: 3 }, { buf: a.buf, off: 1 }, 4), 2);
  assert.equal(mrt.offset_from({ buf: b.buf, off: 2 }, { buf: b.buf, off: 0 }, 4), 2);
  mrt.dealloc(a, 16, 4);
  mrt.dealloc(b, 16, 4);

  // 2. Unified growable TypedArray & Array NativeVec helpers (kinds 0..=7)
  const v = rt.tv_new(4); // u32
  assert.equal(v.buf instanceof Uint32Array, true);
  assert.throws(() => rt.nv_cap(101, 100, 4), (e) => e instanceof RangeError && rt.is_abort(e));
  assert.throws(() => rt.nv_cap(101, 100), (e) => e instanceof RangeError && rt.is_abort(e));
  rt.nv_push(v, 10, 100);
  assert.equal(v.cap, 4); // RawVec min_non_zero_cap for 4-byte u32
  rt.nv_push(v, 25, 100);
  assert.equal(v.len, 2);
  assert.equal(v.buf[1], 25);
  rt.nv_insert(v, 1, 15, 100);
  rt.nv_insert(v, 2, 19, 100);
  assert.deepEqual(Array.from(v.buf.subarray(0, v.len)), [10, 15, 19, 25]);
  assert.throws(() => rt.nv_insert(v, 5, 1, 100), (e) => e instanceof RangeError && rt.is_abort(e));
  // Panic-handler routing preserves caller location and registered abort identity
  let panicSeen = null;
  const fakeLoc = { filename: { buf: [97, 46, 114, 115], off: 0, len: 4 }, line: 42, col: 7 };
  const fakePanic = (pi) => { panicSeen = pi; throw rt.abort_error(new Error("rust panic")); };
  assert.throws(
    () => rt.nv_insert(v, 9, 1, 100, 4, (m, l) => rt._nv_pinfo(m, fakePanic, l), fakeLoc),
    (e) => e.message === "rust panic" && rt.is_abort(e),
  );
  assert.equal(panicSeen.location, fakeLoc);
  // RawVec amortized growth formula: u32 min 4, u8 min 8, cap 9 -> 18, exact stays exact
  const vu32 = rt.tv_new(4);
  rt.nv_reserve(vu32, 1, 100, false, 4);
  assert.equal(vu32.cap, 4);
  const vu8 = rt.tv_new(0);
  rt.nv_reserve(vu8, 1, 100, false, 8);
  assert.equal(vu8.cap, 8);
  const v9 = rt.nv_cap(9, 100, 4);
  rt.nv_reserve(v9, 10, 100, false, 4);
  assert.equal(v9.cap, 18);
  const vLarge = rt.native_vec_new(0);
  rt.nv_reserve(vLarge, 1, 100, false, 1);
  assert.equal(vLarge.cap, 1);
  rt.nv_reserve(v, 10, 100, true);
  assert.equal(v.cap, 14);
  rt.nv_resize(v, 5, 7, 100);
  assert.deepEqual(Array.from(v.buf.subarray(0, v.len)), [10, 15, 19, 25, 7]);
  assert.equal(rt.nv_remove(v, 1), 15);
  assert.equal(rt.nv_swap_remove(v, 0), 10);
  assert.deepEqual(Array.from(v.buf.subarray(0, v.len)), [7, 19, 25]);
  rt.nv_shrink(v);
  assert.equal(v.cap, 3);
  assert.throws(() => rt.nv_remove(v, 3), RangeError);
  assert.throws(() => rt.nv_swap_remove(v, 3), RangeError);
  assert.throws(() => rt.nv_push(v, 1, 3), RangeError);
  assert.equal(v.len, 3);

  // -0.0 preservation in Float64Array / Float32Array nv_from_elem
  const fz = rt.nv_from_elem(-0.0, 4, 100, 7);
  assert.equal(fz.buf instanceof Float64Array, true);
  assert.equal(Object.is(fz.buf[0], -0.0), true);
  assert.equal(Object.is(fz.buf[3], -0.0), true);

  // nv_extend, nv_extend_slice (same TypedArray, plain Array, Proxy-wrapped TypedArray), nv_clone, nv_from_slice
  const dst = rt.nv_from_elem(1, 2, 100, 0);
  const src = rt.nv_from_elem(2, 3, 100, 0);
  rt.nv_extend(dst, src, 100);
  assert.deepEqual(Array.from(dst.buf.subarray(0, dst.len)), [1, 1, 2, 2, 2]);
  assert.equal(src.len, 0);
  assert.equal(src.cap, 0);
  rt.nv_extend_slice(dst, { buf: [9, 8], off: 0, len: 2 }, 100);
  const proxied = new Proxy(new Uint8Array([4, 5, 6]), {});
  rt.nv_extend_slice(dst, { buf: proxied, off: 1, len: 2 }, 100);
  assert.deepEqual(Array.from(dst.buf.subarray(0, dst.len)), [1, 1, 2, 2, 2, 9, 8, 5, 6]);
  const cloned = rt.nv_clone(dst);
  assert.equal(cloned.buf instanceof Uint8Array, true);
  cloned.buf[0] = 99;
  assert.equal(dst.buf[0], 1);
  const fromEmpty = rt.nv_from_slice({ buf: 1, off: 0, len: 0 }, 0);
  assert.equal(fromEmpty.len, 0);
  assert.equal(fromEmpty.buf instanceof Uint8Array, true);
  const fromProxy = rt.nv_from_slice({ buf: proxied, off: 0, len: 3 }, 0);
  assert.deepEqual(Array.from(fromProxy.buf), [4, 5, 6]);
  // nv_from_box: plain slot, window slot, bare array, and empty dangling box; zero unwindow/_element_view_of deps
  const boxPlain = rt.nv_from_box({ buf: [[10, 20, 30]], off: 0 }, 3, 4);
  assert.equal(boxPlain.buf instanceof Uint32Array, true);
  assert.deepEqual(Array.from(boxPlain.buf), [10, 20, 30]);
  const boxWin = rt.nv_from_box({ buf: [99, 1, 2, 3, 99], off: 1, w: 3 }, 3, 0);
  assert.equal(boxWin.buf instanceof Uint8Array, true);
  assert.deepEqual(Array.from(boxWin.buf), [1, 2, 3]);
  const boxBare = rt.nv_from_box(["x", "y"], 2);
  assert.deepEqual(boxBare, { buf: ["x", "y"], len: 2, cap: 2 });
  const boxEmpty = rt.nv_from_box(4, 0, 4);
  assert.equal(boxEmpty.buf instanceof Uint32Array, true);
  assert.equal(boxEmpty.len, 0);
  const boxMin = minimalShim([], shimSrc, { module: true, direct: ["nv_from_box"] });
  assert.equal(boxMin.names.includes("unwindow"), false);
  assert.equal(boxMin.names.includes("_element_view_of"), false);

  // 3. Plain-Array NativeVec helpers (nv_*) + dangling empty slice safety
  const nv = rt.native_vec_new(0);
  rt.nv_push(nv, "a", 100);
  const obj = { x: 1 };
  rt.nv_push(nv, obj, 100);
  obj.x = 42;
  assert.equal(nv.buf[1].x, 42);
  rt.nv_reserve(nv, 10, 100, false);
  assert.equal(nv.cap >= 12, true);
  rt.nv_shrink(nv);
  assert.equal(nv.cap, 2);
  const nvEmpty = rt.nv_from_slice({ buf: 4, off: 0, len: 0 });
  assert.deepEqual(nvEmpty, { buf: [], len: 0, cap: 0 });
  const rng = rt.nv_range(nv, 0, 2);
  assert.equal(rng.len, 2);
  assert.throws(() => rt.nv_range(nv, 2, 1), RangeError);
  assert.throws(() => rt.nv_range(nv, 0, 3), RangeError);
  assert.throws(() => rt.nv_idx(nv, 2, false), RangeError);

  // 4. TypedArray <-> baseline block adapters + offset_from/copy on TypedArray buffers
  const blkPtr = rt.convert_nv_block(dst, 1);
  assert.equal(blkPtr.buf[0], 1);
  assert.equal(blkPtr.buf[8], 6);
  const backTv = rt.convert_block_nv(blkPtr.buf, blkPtr.off, dst.len, dst.cap, 0);
  assert.equal(backTv.buf instanceof Uint8Array, true);
  assert.deepEqual(Array.from(backTv.buf.subarray(0, backTv.len)), [1, 1, 2, 2, 2, 9, 8, 5, 6]);
  rt.dealloc(blkPtr, dst.cap, 1);
  // Float NaN payload preservation across convert_nv_block (f32 and f64, repeated for V8 IC)
  const f32Buf = new Float32Array(2);
  new Uint32Array(f32Buf.buffer)[0] = 0x7fc00123;
  f32Buf[1] = 1.5;
  const f64Buf = new Float64Array(2);
  new BigUint64Array(f64Buf.buffer)[0] = 0x7ff8000000000123n;
  f64Buf[1] = 2.5;
  for (let iter = 0; iter < 500; iter += 1) {
    const p32 = rt.convert_nv_block({ buf: f32Buf, len: 2, cap: 4 }, 4, "f32");
    const u32 = rt.unscale_scalar(p32, 4, () => 0, "f32");
    assert.equal(new Uint32Array(Float32Array.of(u32.buf[u32.off]).buffer)[0], 0x7fc00123);
    rt.dealloc(p32, 16, 4);
    const p64 = rt.convert_nv_block({ buf: f64Buf, len: 2, cap: 4 }, 8, "f64");
    const u64 = rt.unscale_scalar(p64, 8, () => 0, "f64");
    assert.equal(new BigUint64Array(Float64Array.of(u64.buf[u64.off]).buffer)[0], 0x7ff8000000000123n);
    rt.dealloc(p64, 32, 8);
  }
  assert.equal(rt.offset_from({ buf: dst.buf, off: 5 }, { buf: dst.buf, off: 2 }, 1), 3);
  const dst2 = new Uint8Array(4);
  rt.copy({ buf: dst2, off: 0 }, { buf: dst.buf, off: 5 }, 4);
  assert.deepEqual(Array.from(dst2), [9, 8, 5, 6]);
  rt.copy({ buf: dst2, off: 1 }, { buf: dst2, off: 0 }, 3);
  assert.deepEqual(Array.from(dst2), [9, 9, 8, 5]);
  rt.write_bytes({ buf: dst2, off: 1 }, 0xab, 2);
  assert.deepEqual(Array.from(dst2), [9, 0xab, 0xab, 5]);

  // 5. Wave 4 runtime helpers: map_values, _nv_fail/_nv_pinfo, u8array_into_bytes fast path
  const m = new Map([["a", [10]], ["b", [20]]]);
  assert.deepEqual(rt.map_values(m), { buf: [[10], [20]], off: 0, len: 2 });
  let seenPinfo = null;
  assert.throws(
    () => rt._nv_fail("oob", (msg, loc) => rt._nv_pinfo(msg, (info) => { seenPinfo = info; }, loc), { filename: { buf: [], off: 0, len: 0 }, line: 7, col: 3 }),
    (err) => err instanceof RangeError && rt.is_abort(err),
  );
  assert.equal(seenPinfo.location.line, 7);
  assert.equal(seenPinfo.location.col, 3);
  assert.deepEqual(seenPinfo.message.template.buf, [111, 111, 98]);
  assert.throws(() => rt._nv_fail("cap"), (err) => err instanceof RangeError && rt.is_abort(err));
  const sub = new Uint8Array([9, 1, 2, 3, 9]).subarray(1, 4);
  const taDst = new Uint8Array([0, 0, 0, 0, 0]);
  rt.u8array_into_bytes(sub, { buf: taDst, off: 1, len: 3 });
  assert.deepEqual(Array.from(taDst), [0, 1, 2, 3, 0]);
  const arrDst = [0, 0, 0, 0, 0];
  rt.u8array_into_bytes(sub, { buf: arrDst, off: 1, len: 3 });
  assert.deepEqual(arrDst, [0, 1, 2, 3, 0]);
});

test("Phase 1B monomorphic TypedArray helpers, packed zeroed arrays, 32-elem copy fast path, and minimal-shim tree-shaking", async () => {
  const shimSrc = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const kinds = [
    ["u8", Uint8Array, 0, 8, 2147483647],
    ["i8", Int8Array, 1, 8, 2147483647],
    ["u16", Uint16Array, 2, 4, 1073741823],
    ["i16", Int16Array, 3, 4, 1073741823],
    ["u32", Uint32Array, 4, 4, 536870911],
    ["i32", Int32Array, 5, 4, 536870911],
    ["f32", Float32Array, 6, 4, 536870911],
    ["f64", Float64Array, 7, 4, 268435455],
  ];

  // 1. All 8 monomorphic TypedArray constructors, zeroed, grow, and reserve helpers
  for (const [suffix, Ctor, k, minCap, maxElems] of kinds) {
    const emptyCap = rt[`nv_with_cap_${suffix}`](0, maxElems);
    assert.equal(emptyCap.buf, rt._nv_et(k));
    assert.equal(emptyCap.len, 0);

    const withCap = rt[`nv_with_cap_${suffix}`](6, maxElems);
    assert.equal(withCap.buf instanceof Ctor, true);
    assert.equal(withCap.buf.length, 6);
    assert.equal(withCap.len, 0);
    assert.throws(
      () => rt[`nv_with_cap_${suffix}`](maxElems + 1, maxElems),
      (e) => rt.is_abort(e) && /capacity overflow/.test(e.message),
    );

    const emptyZ = rt[`nv_zeroed_${suffix}`](0, maxElems);
    assert.equal(emptyZ.buf, rt._nv_et(k));
    assert.equal(emptyZ.len, 0);

    const zeroed = rt[`nv_zeroed_${suffix}`](5, maxElems);
    assert.equal(zeroed.buf instanceof Ctor, true);
    assert.equal(zeroed.buf.length, 5);
    assert.equal(zeroed.len, 5);
    assert.deepEqual(Array.from(zeroed.buf), [0, 0, 0, 0, 0]);
    assert.throws(
      () => rt[`nv_zeroed_${suffix}`](maxElems + 1, maxElems),
      (e) => rt.is_abort(e) && /capacity overflow/.test(e.message),
    );

    // Grow from empty -> minCap -> 2 * minCap
    const vg = { buf: rt._nv_et(k), len: 0 };
    rt[`nv_grow_${suffix}`](vg, minCap, maxElems);
    assert.equal(vg.buf instanceof Ctor, true);
    assert.equal(vg.buf.length, minCap);
    for (let i = 0; i < minCap; i++) vg.buf[i] = i + 1;
    rt[`nv_grow_${suffix}`](vg, minCap, maxElems);
    assert.equal(vg.buf.length, minCap * 2);
    for (let i = 0; i < minCap; i++) assert.equal(vg.buf[i], i + 1);
    assert.throws(
      () => rt[`nv_grow_${suffix}`](vg, minCap, vg.buf.length),
      (e) => rt.is_abort(e) && /capacity overflow/.test(e.message),
    );

    // Reserve amortized + exact + no-op when already large enough
    const vr = { buf: rt._nv_et(k), len: 0 };
    rt[`nv_reserve_${suffix}`](vr, 0, 1, minCap, false, maxElems);
    assert.equal(vr.buf instanceof Ctor, true);
    assert.equal(vr.buf.length, minCap);
    vr.buf[0] = 7;
    const prevBuf = vr.buf;
    rt[`nv_reserve_${suffix}`](vr, 1, minCap - 1, minCap, false, maxElems);
    assert.equal(vr.buf, prevBuf);
    rt[`nv_reserve_${suffix}`](vr, minCap, 3, minCap, true, maxElems);
    assert.equal(vr.buf.length, minCap + 3);
    assert.equal(vr.buf[0], 7);
    assert.throws(
      () => rt[`nv_reserve_${suffix}`](vr, maxElems, 1, minCap, false, maxElems),
      (e) => rt.is_abort(e) && /capacity overflow/.test(e.message),
    );
  }

  // 2. Dense packed zeroed array helpers (both non-packed `_nv_ea` on 0 and packed `_p` fresh `[]`)
  const za0 = rt.nv_zeroed_arr(0, 8, 100, 42);
  assert.equal(za0.buf, rt._nv_ea(8));
  const za = rt.nv_zeroed_arr(4, 8, 100, 42);
  assert.deepEqual(za, { buf: [42, 42, 42, 42], len: 4 });
  for (let i = 0; i < 4; i++) assert.equal(i in za.buf, true);

  const zap0 = rt.nv_zeroed_arr_p(0, 100, 42);
  assert.equal(Object.isFrozen(zap0.buf), false);
  zap0.buf.push(99);
  assert.deepEqual(zap0.buf, [99]);
  const zap = rt.nv_zeroed_arr_p(3, 100, 7);
  assert.deepEqual(zap, { buf: [7, 7, 7], len: 3 });

  const zafp = rt.nv_zeroed_arr_fn_p(2, 100, () => ({ v: 1 }));
  assert.equal(zafp.len, 2);
  assert.notEqual(zafp.buf[0], zafp.buf[1]);
  const zafp0 = rt.nv_zeroed_arr_fn_p(0, 100, () => ({ v: 1 }));
  assert.equal(Object.isFrozen(zafp0.buf), false);

  const zaftp = rt.nv_zeroed_arr_fn_tracked_p(2, 100, () => ({ v: 2 }));
  assert.equal(rt._boxes.get(zaftp.buf[0]).buf, zaftp.buf);
  assert.equal(rt._boxes.get(zaftp.buf[1]).off, 1);

  // 3. copy & copy_nonoverlapping across <= 32 loop and > 32 subarray thresholds
  for (const n of [1, 8, 20, 32, 33, 64]) {
    const src = new Uint8Array(n + 4);
    for (let i = 0; i < src.length; i++) src[i] = (i * 13 + 5) & 255;
    const d1 = new Uint8Array(n + 4);
    const d2 = new Uint8Array(n + 4);
    rt.copy_nonoverlapping({ buf: d1, off: 2 }, { buf: src, off: 1 }, n);
    rt.copy({ buf: d2, off: 2 }, { buf: src, off: 1 }, n);
    assert.deepEqual(Array.from(d1.subarray(2, 2 + n)), Array.from(src.subarray(1, 1 + n)));
    assert.deepEqual(Array.from(d2.subarray(2, 2 + n)), Array.from(src.subarray(1, 1 + n)));
  }

  // 4. minimal-shim tree-shaking isolates each monomorphic TypedArray helper cleanly
  const growU8Only = minimalShim(["__rt.nv_grow_u8(v, 8, 2147483647);"], shimSrc, { module: true });
  assert.deepEqual(growU8Only.names.includes("nv_grow_u8"), true);
  assert.deepEqual(growU8Only.names.includes("nv_grow_u32"), false);
  assert.deepEqual(growU8Only.names.includes("nv_grow_f64"), false);
  assert.deepEqual(growU8Only.names.includes("_nv_ctors"), false);
  assert.deepEqual(growU8Only.names.includes("_nv_et"), false);

  const packedZeroOnly = minimalShim(["__rt.nv_zeroed_arr_p(n, 100, 0);"], shimSrc, { module: true });
  assert.deepEqual(packedZeroOnly.names.includes("nv_zeroed_arr_p"), true);
  assert.deepEqual(packedZeroOnly.names.includes("_nv_ea"), false);
  assert.deepEqual(packedZeroOnly.names.includes("_dangling_of"), false);

  const mod = await import(
    `data:text/javascript;base64,${Buffer.from(growU8Only.source).toString("base64")}`
  );
  const mv = { buf: new Uint8Array(0), len: 0 };
  mod.nv_grow_u8(mv, 8, 2147483647);
  assert.equal(mv.buf instanceof Uint8Array, true);
  assert.equal(mv.buf.length, 8);
});

test("branchless _bstr_is_ascii and _bstr_is_ascii_range agree across all boundary lengths and code units", () => {
  const lengths = [0, 1, 2, 3, 4, 23, 24, 25, 64, 127, 128, 129, 256];
  const nonAscii = ["\x80", "\u0100", "\uD800", "\uFFFF"];
  const padL = "\u00e9\uD800";
  const padR = "\uFFFF\x80";

  for (const n of lengths) {
    // Pure ASCII including 0x00 and 0x7f boundaries
    const ascii =
      n === 0
        ? ""
        : n === 1
          ? "\x7f"
          : "\x00" + "a".repeat(n - 2) + "\x7f";
    assert.equal(rt._bstr_is_ascii(ascii), true, `ascii len=${n}`);
    assert.equal(rt._bstr_is_ascii_range(ascii, 0, n), true, `range full ascii len=${n}`);
    assert.equal(
      rt._bstr_is_ascii_range(padL + ascii + padR, padL.length, n),
      true,
      `range padded ascii len=${n}`,
    );

    if (n === 0) continue;
    const positions = new Set([0, n >> 1, n - 1]);
    for (const pos of positions) {
      for (const ch of nonAscii) {
        const s = "a".repeat(pos) + ch + "b".repeat(n - 1 - pos);
        assert.equal(s.length, n);
        assert.equal(
          rt._bstr_is_ascii(s),
          false,
          `non-ascii len=${n} pos=${pos} code=${ch.charCodeAt(0)}`,
        );
        assert.equal(
          rt._bstr_is_ascii_range(s, 0, n),
          false,
          `range full non-ascii len=${n} pos=${pos} code=${ch.charCodeAt(0)}`,
        );
        assert.equal(
          rt._bstr_is_ascii_range(padL + s + padR, padL.length, n),
          false,
          `range padded non-ascii len=${n} pos=${pos} code=${ch.charCodeAt(0)}`,
        );
      }
    }
  }

  // Standalone minimal-shim extraction of `_bstr_is_ascii_range` must keep `_bstr_wide` intact.
  const shimSrc = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const rangeOnly = minimalShim(["__rt._bstr_is_ascii_range(s, 1, 200);"], shimSrc, { direct: ["_bstr_is_ascii_range"] });
  const ctx = {};
  vm.runInNewContext(rangeOnly.source, ctx);
  assert.equal(ctx.__rt._bstr_is_ascii_range("\x80" + "a".repeat(200) + "\x80", 1, 200), true);
  assert.equal(ctx.__rt._bstr_is_ascii_range("a".repeat(100) + "\u0100" + "a".repeat(100), 1, 200), false);
});

test("slice_sort direct-value sort preserves stability, panic safety, and _boxes tracking", () => {
  // 1. Stability on both small (insertion) and large (>16 merge) indirect slices with non-zero off.
  for (const len of [0, 1, 2, 5, 16, 17, 45]) {
    const pad = { k: -999, id: -1 };
    const items = Array.from({ length: len }, (_, i) => ({ k: (len - i) % 5, id: i }));
    const buf = [pad, ...items, pad];
    rt.slice_sort(buf, 1, len, null, (_, a, b) => a.k < b.k, true);
    assert.equal(buf[0], pad);
    assert.equal(buf[len + 1], pad);
    for (let i = 1; i < len; i++) {
      const prev = buf[i];
      const cur = buf[i + 1];
      assert.ok(prev.k < cur.k || (prev.k === cur.k && prev.id < cur.id), `unstable at len=${len} i=${i}`);
    }
  }

  // 2. Direct scalar slices (isIndirect = false) with subslice offset and already-sorted fast path.
  const nums = [99, 5, 1, 4, 2, 3, 88];
  rt.slice_sort(nums, 1, 5, null, (_, a, b) => a.buf[a.off] < b.buf[b.off], false);
  assert.deepEqual(nums, [99, 1, 2, 3, 4, 5, 88]);
  // Already-sorted re-run leaves buffer untouched.
  rt.slice_sort(nums, 1, 5, null, (_, a, b) => a.buf[a.off] < b.buf[b.off], false);
  assert.deepEqual(nums, [99, 1, 2, 3, 4, 5, 88]);

  // 3. Exception safety: comparator throwing midway must not partially mutate `buf`.
  const orig = [
    { k: 3, id: 0 },
    { k: 1, id: 1 },
    { k: 2, id: 2 },
    { k: 0, id: 3 },
  ];
  const snap = orig.slice();
  let calls = 0;
  assert.throws(() => {
    rt.slice_sort(
      orig,
      0,
      orig.length,
      null,
      (_, a, b) => {
        if (++calls === 3) throw new Error("boom");
        return a.k < b.k;
      },
      true,
    );
  }, /boom/);
  assert.deepEqual(orig, snap);

  // 4. Heap-allocated indirect slice updates `_boxes` after permutation.
  const cloneObj = (o) => ({ k: o.k, id: o.id });
  const raw = rt.alloc(32, 4);
  const typed = rt.unscale(raw, 8, () => ({ k: 0, id: 0 }), cloneObj);
  const objs = [
    { k: 30, id: 0 },
    { k: 10, id: 1 },
    { k: 20, id: 2 },
    { k: 10, id: 3 },
  ];
  for (let i = 0; i < 4; i++) rt._put(typed.buf, i, objs[i], true);
  rt.slice_sort(typed.buf, 0, 4, null, (_, a, b) => a.k < b.k, true);
  assert.deepEqual(
    typed.buf.slice(0, 4).map((o) => [o.k, o.id]),
    [
      [10, 1],
      [10, 3],
      [20, 2],
      [30, 0],
    ],
  );
  for (let i = 0; i < 4; i++) {
    assert.equal(rt.ptr_eq(typed.buf[i], { buf: typed.buf, off: i }), true);
  }
  rt.dealloc(raw, 32, 4);
});
