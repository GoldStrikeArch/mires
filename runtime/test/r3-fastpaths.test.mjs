import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from 'acorn';
import { minimalShim } from '../../scripts/minimal-shim.mjs';

const shimSource = readFileSync(new URL('../shim.js', import.meta.url), 'utf8');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { ignoreBOM: true });

function scheduled(code = shimSource) {
  const jobs = [];
  const context = vm.createContext({
    TextEncoder,
    TextDecoder,
    Uint8Array,
    queueMicrotask: (job) => jobs.push(job),
  });
  vm.runInContext(code, context);
  return {
    rt: context.__rt,
    jobs,
    unwind: () => {
      while (jobs.length) jobs.shift()();
    },
  };
}

const view = (record) => Array.from(record.buf).slice(record.off, record.off + record.len);
const utf8 = (text) => Array.from(encoder.encode(text));

test('deep (>8) nested ASCII subslices survive 4-slot ring eviction without re-encoding', () => {
  const { rt, jobs, unwind } = scheduled();
  // > 64 bytes outer doc with > 10 nested slices simulating marked's markdown -> line -> inline -> span -> char.
  const doc =
    '## Section 00\n\n- **browser** compiler and [report](https://example.test/00)\n\nText with <escape> end!';
  const docRec = rt.str_bytes(doc);
  assert.equal(docRec.off, 0);
  assert.equal(docRec.len, doc.length);

  // Create 12 subslices of varying lengths (>4, spilling out of _sub_k0..3).
  const slices = [];
  let off = 0;
  for (let i = 0; i < 12; i++) {
    const len = 60 - i * 4;
    const sub = rt.bytes_str(docRec.buf, off, len);
    slices.push({ sub, off, len });
    off += 2;
  }

  // Every evicted outer slice (including slices[0..7] evicted from _sub_k0..3) still resolves
  // zero-copy to the parent buffer with exact `off > 0` (except slices[0] at 0) and `len`,
  // without re-encoding or adding entries to _str_memo.map.
  assert.equal(rt._str_memo.map.size, 1);
  assert.equal(rt._str_memo.bytes, doc.length);

  for (const { sub, off: wantOff, len: wantLen } of slices) {
    const r = rt.str_bytes(sub);
    assert.equal(r.buf, docRec.buf, `subslice "${sub}" should reuse doc buf`);
    assert.equal(r.off, wantOff);
    assert.equal(r.len, wantLen);
    assert.equal(rt.str_len(sub), wantLen);
    // Back-to-back calls hit 1-slot MRU with identical record identity.
    assert.equal(rt.str_bytes(sub), r);
    assert.equal(rt.str_len(sub), wantLen);
  }
  assert.equal(rt._str_memo.map.size, 1);

  // Nested subslice from an off > 0 subslice preserves cumulative offset.
  const mid = rt.str_bytes(slices[3].sub);
  assert.equal(mid.off, 6);
  const inner = rt.bytes_str(mid.buf, mid.off + 5, 22);
  const innerRec = rt.str_bytes(inner);
  assert.equal(innerRec.buf, docRec.buf);
  assert.equal(innerRec.off, 11);
  assert.equal(innerRec.len, 22);
  assert.equal(rt.str_len(inner), 22);

  // _str_memo_clear microtask clears MRU, 4-slot ring, and spill map.
  assert.equal(jobs.length, 1);
  unwind();
  assert.equal(rt._str_memo.map.size, 0);
  const afterReset = rt.str_bytes(slices[1].sub);
  assert.notEqual(afterReset.buf, docRec.buf);
  assert.equal(afterReset.off, 0);
  assert.deepEqual(view(afterReset), utf8(slices[1].sub));
});

test('multi-byte UTF-8, lone surrogates, mutable Vec<u8> non-aliasing, and _utf8 ASCII threshold up to 65536', () => {
  const { rt, unwind } = scheduled();

  // Multi-byte UTF-8 and lone surrogates never enter _ascii_src, so slicing them does not alias.
  const utf8Text = 'Grüße 世界 😀 ' + 'abcdefghij'.repeat(2);
  const utf8Rec = rt.str_bytes(utf8Text);
  assert.equal(utf8Rec.len, utf8(utf8Text).length);
  const asciiTail = rt.bytes_str(utf8Rec.buf, utf8Rec.len - 20, 20);
  assert.equal(asciiTail, 'abcdefghij'.repeat(2));
  const freshTail = rt.str_bytes(asciiTail);
  assert.notEqual(freshTail.buf, utf8Rec.buf);
  assert.equal(freshTail.off, 0);

  const lone = '\ud800' + '0123456789abcdef' + '\udfff';
  const loneRec = rt.str_bytes(lone);
  assert.deepEqual(view(loneRec), utf8(lone));
  assert.equal(rt.str_len(lone), utf8(lone).length);
  assert.equal(rt.bytes_str(loneRec.buf, loneRec.off, loneRec.len), decoder.decode(encoder.encode(lone)));
  const loneMid = rt.bytes_str(loneRec.buf, 3, 16);
  assert.equal(loneMid, '0123456789abcdef');
  assert.notEqual(rt.str_bytes(loneMid).buf, loneRec.buf);

  // Mutable buffer (`Vec<u8>` / `str_bytes_mut` / `convert_str_block`) never aliases memoized views.
  const src = 'mutable-isolation-check-0123456789';
  const shared = rt.str_bytes(src);
  const mut = rt.str_bytes_mut(src);
  const owned = rt.convert_str_block(src);
  assert.notEqual(mut.buf, shared.buf);
  assert.notEqual(owned.buf, shared.buf);
  mut.buf[0] = 0x58;
  owned.buf[1] = 0x59;
  assert.equal(rt.bytes_str(mut.buf, mut.off, mut.len), 'X' + src.slice(1));
  assert.equal(rt.bytes_str(owned.buf, owned.off, owned.len), 'mY' + src.slice(2));
  assert.equal(rt.bytes_str(shared.buf, shared.off, shared.len), src);
  assert.deepEqual(view(rt.str_bytes(src)), utf8(src));

  // _utf8 ASCII threshold: 49, 7678 (marked size), 61424 (fflate size), 65536, 65537,
  // plus non-ASCII late in a <=65536 string falling back cleanly to TextEncoder.
  for (const len of [48, 49, 4096, 4097, 7678, 61424, 65536, 65537]) {
    const ascii = 'a'.repeat(len - 1) + 'z';
    const got = rt._utf8(ascii);
    assert.deepEqual(Array.from(got), utf8(ascii), `ASCII len=${len}`);
  }
  const lateNonAscii = 'a'.repeat(65000) + 'é😀\ud800';
  assert.deepEqual(Array.from(rt._utf8(lateNonAscii)), utf8(lateNonAscii));
  assert.equal(rt.str_len(lateNonAscii), utf8(lateNonAscii).length);
  unwind();
});

test('back-to-back str_bytes / str_len on short literals and empty string preserve exact semantics', () => {
  const { rt, unwind } = scheduled();
  assert.equal(rt.str_len(''), 0);
  const empty1 = rt.str_bytes('');
  const empty2 = rt.str_bytes('');
  assert.equal(empty1, empty2);
  assert.equal(empty1.off, 0);
  assert.equal(empty1.len, 0);
  assert.equal(rt.str_len(''), 0);

  for (const lit of ['**', '- ', '```', 'browser']) {
    const r1 = rt.str_bytes(lit);
    const r2 = rt.str_bytes(lit);
    assert.equal(r1, r2);
    assert.equal(rt.str_len(lit), lit.length);
    assert.equal(rt.str_len(lit), lit.length);
  }
  unwind();
});

test('ptr_eq fast path preserves poisoned provenance refusal, prefix projections, null/ZST/dangling, and equality', () => {
  const { rt } = scheduled();
  const buf = [10, 20, 30];
  const p0a = { buf, off: 0 };
  const p0b = { buf, off: 0 };
  const p1 = { buf, off: 1 };
  const p0s = { buf, off: 0, sc: 4 };
  const other = [10, 20, 30];

  // Fast-path equality and inequality.
  assert.equal(rt.ptr_eq(p0a, p0a), true);
  assert.equal(rt.ptr_eq(p0a, p0b), true);
  assert.equal(rt.ptr_eq(p0a, p1), false);
  assert.equal(rt.ptr_eq(p0a, p0s), false);
  assert.equal(rt.ptr_eq({ buf: other, off: 0 }, p0a), false);

  // Null, undefined (ZST), primitive numbers, and bare aggregates without `buf` key.
  assert.equal(rt.ptr_eq(null, null), true);
  assert.equal(rt.ptr_eq(null, p0a), false);
  assert.equal(rt.ptr_eq(p0a, null), false);
  assert.equal(rt.ptr_eq(undefined, undefined), true);
  assert.equal(rt.ptr_eq(undefined, p0a), false);
  assert.equal(rt.ptr_eq(4, 4), true);
  assert.equal(rt.ptr_eq(4, 8), false);
  assert.equal(rt.ptr_eq({ x: 1 }, { x: 1 }), false);

  // Dangling pointers of the same alignment compare equal; different alignments do not.
  const d4a = rt._dangling(4);
  const d4b = rt._dangling(4);
  const d8 = rt._dangling(8);
  assert.equal(rt.ptr_eq(d4a, d4b), true);
  assert.equal(rt.ptr_eq(d4a, d8), false);

  // Prefix / `pr` projections compare by underlying containing pointer.
  const aggBuf = [{ head: 1, tail: 2 }];
  const aggPtr = { buf: aggBuf, off: 0 };
  const proj = rt.prefix(aggPtr, 'head', 8);
  assert.equal(rt.ptr_eq(proj, aggPtr), true);
  assert.equal(rt.ptr_eq(aggPtr, proj), true);
  assert.equal(rt.ptr_eq(proj, rt.prefix({ buf: aggBuf, off: 0 }, 'head', 8)), true);
  assert.equal(rt.ptr_eq(proj, { buf: aggBuf, off: 1 }), false);

  // Poisoned provenance-less pointer (`buf: undefined` with `"buf" in p`) must still abort.
  const poisoned = { buf: undefined, off: NaN };
  assert.throws(() => rt.ptr_eq(poisoned, p0a), /no provenance/);
  assert.throws(() => rt.ptr_eq(p0a, poisoned), /no provenance/);
  assert.throws(() => rt.ptr_eq(poisoned, { buf: undefined, off: 0 }), /no provenance/);
});

test('copy_nonoverlapping, copy, and compare_bytes fast paths preserve bounds, scaled, and _boxed invariants', () => {
  const { rt } = scheduled();

  // Fast-path byte copy and compare_bytes.
  const src = [1, 2, 3, 4, 5, 6];
  const dst = [0, 0, 0, 0, 0, 0];
  rt.copy_nonoverlapping({ buf: dst, off: 1 }, { buf: src, off: 2 }, 4);
  assert.deepEqual(dst, [0, 3, 4, 5, 6, 0]);
  assert.equal(rt.compare_bytes({ buf: dst, off: 1 }, { buf: src, off: 2 }, 4), 0);
  assert.ok(rt.compare_bytes({ buf: dst, off: 1 }, { buf: src, off: 0 }, 4) > 0);
  assert.ok(rt.compare_bytes({ buf: src, off: 0 }, { buf: dst, off: 1 }, 4) < 0);

  // Self-overlapping copy fast path.
  const ov = [10, 20, 30, 40, 50];
  rt.copy({ buf: ov, off: 1 }, { buf: ov, off: 0 }, 4);
  assert.deepEqual(ov, [10, 10, 20, 30, 40]);

  // Bounds-past-end error refusal.
  assert.throws(
    () => rt.copy_nonoverlapping({ buf: dst, off: 4 }, { buf: src, off: 0 }, 3),
    /reaches past the end/,
  );
  assert.throws(
    () => rt.copy_nonoverlapping({ buf: dst, off: 0 }, { buf: src, off: 5 }, 2),
    /reaches past the end/,
  );
  assert.throws(
    () => rt.copy({ buf: dst, off: 5 }, { buf: src, off: 0 }, 2),
    /reaches past the end/,
  );
  assert.throws(
    () => rt.compare_bytes({ buf: dst, off: 0 }, { buf: src, off: 4 }, 3),
    /reaches past the end/,
  );
  assert.throws(
    () => rt.compare_bytes({ buf: dst, off: 4 }, { buf: src, off: 0 }, 3),
    /reaches past the end/,
  );

  // Scaled pointer refusal and scaled comparison semantics.
  assert.throws(
    () => rt.copy_nonoverlapping({ buf: dst, off: 0, sc: 4 }, { buf: src, off: 0 }, 4),
    /between a pointer of scale 4 and one of scale undefined/,
  );
  assert.throws(
    () => rt.compare_bytes({ buf: dst, off: 0, sc: 4 }, { buf: src, off: 0 }, 4),
    /between a pointer of scale 4 and one of scale undefined/,
  );
  assert.throws(
    () => rt.compare_bytes({ buf: [1, 2], off: 2, sc: 4 }, { buf: [1, 2], off: 0, sc: 4 }, 4),
    /byte offset 2 of 4-byte elements/,
  );
  // Scaled valid compare_bytes compares element-wise (-1/0/1).
  assert.equal(rt.compare_bytes({ buf: [1, 100], off: 0, sc: 4 }, { buf: [1, 200], off: 0, sc: 4 }, 8), -1);

  // _boxed local-slot overwrite preservation (single-object move into __rt.box overwrites in place).
  const local = { TAG: 0, x: 1, y: 2 };
  const boxedPtr = rt.box(local);
  rt.copy_nonoverlapping(boxedPtr, { buf: [{ TAG: 1, x: 9 }], off: 0 }, 1);
  assert.equal(boxedPtr.buf[0], local);
  assert.deepEqual({ ...local }, { TAG: 1, x: 9 });
  assert.equal('y' in local, false);

  rt.copy(boxedPtr, { buf: [{ TAG: 2, y: 7 }], off: 0 }, 1);
  assert.equal(boxedPtr.buf[0], local);
  assert.deepEqual({ ...local }, { TAG: 2, y: 7 });
  assert.equal('x' in local, false);

  // Non-boxed single-object move rebinds instead of overwriting.
  const pOld = { x: 1 };
  const pNew = { x: 2 };
  const slots = [pOld];
  rt.copy_nonoverlapping({ buf: slots, off: 0 }, { buf: [pNew], off: 0 }, 1);
  assert.equal(slots[0], pNew);
  assert.deepEqual(pOld, { x: 1 });
});

test('minimal-shim extraction preserves all fast paths in classic and ESM module modes', () => {
  const direct = [
    'str_bytes',
    'str_len',
    'bytes_str',
    'ptr_eq',
    'copy',
    'copy_nonoverlapping',
    'compare_bytes',
    'box',
    'prefix',
    '_dangling',
  ];
  for (const module of [false, true]) {
    const extracted = minimalShim([], shimSource, { direct, module });
    parse(extracted.source, { ecmaVersion: 2018, sourceType: module ? 'module' : 'script' });
    if (!module) {
      const { rt, unwind } = scheduled(extracted.source);
      const rec = rt.str_bytes('0123456789abcdefghij'.repeat(2));
      const s0 = rt.bytes_str(rec.buf, 1, 20);
      for (let i = 0; i < 6; i++) rt.bytes_str(rec.buf, 2 + i, 18);
      assert.equal(rt.str_bytes(s0).buf, rec.buf);
      assert.equal(rt.str_bytes(s0).off, 1);
      assert.equal(rt.str_len(s0), 20);
      assert.equal(rt.ptr_eq({ buf: rec.buf, off: 1 }, { buf: rec.buf, off: 1 }), true);
      const out = [0, 0, 0];
      rt.copy_nonoverlapping({ buf: out, off: 0 }, { buf: [7, 8, 9], off: 0 }, 3);
      assert.equal(rt.compare_bytes({ buf: out, off: 0 }, { buf: [7, 8, 9], off: 0 }, 3), 0);
      unwind();
    }
  }
});
