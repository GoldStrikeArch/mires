// Runtime shim for rustc-codegen-js.
//
// Plain script (NOT an ES module): it is concatenated ahead of the compiled
// output, so it only assigns to globalThis. The backend lowers every call to a
// foreign item to `__rt.<symbol_name>(args...)`, so the keys here are exactly
// the `extern "C"` symbol names declared by examples/mini_core.rs.
//
// This file is also the single source of the ES module form a program compiled
// with `-Cllvm-args=js-modules=esm` imports. That form is derived from this one
// by scripts/make-esm-shim.mjs, which evaluates this script and appends an
// export clause naming the members it installed — so a member added here is
// exported by the next run, and there is nothing to keep in step by hand. Keep
// installing members on `globalThis.__rt`; do not add an `export` to this file,
// which would stop every page that loads it with a plain `<script src>`.

globalThis.__rt = {
  js_log_i32: (x) => console.log(x),
  // i64/u64/i128 arrive as BigInt; `toString` prints them bare, without the
  // trailing `n` that `console.log(1n)` would add.
  js_log_i64: (x) => console.log(x.toString()),
  js_log_f64: (x) => console.log(x),
  js_log_bool: (x) => console.log(x),
  js_log_str: (s) => console.log(s),
  js_abort: (msg) => {
    throw new Error("rust abort: " + msg);
  },
};

// ---------------------------------------------------------------------------
// Numerics (S1-M2). Append below, do not reorder.
//
// Helpers the backend emits calls to for operations that have no short inline
// JavaScript spelling. They are not foreign items, so they live outside the
// object literal above; the naming rule is the same, `__rt.<name>(...)`.
// ---------------------------------------------------------------------------

// A float to integer cast, which Rust defines as saturating: an input past the
// destination's range clamps to its end, and NaN becomes zero. `min` and `max`
// are the destination type's bounds, passed as literals by the backend.
//
// The clamp comes before the truncation, so `Infinity` and huge finite values
// both land on `max` instead of reaching the bitwise operators as NaN.
globalThis.__rt.f2i = (x, min, max) => {
  if (Number.isNaN(x)) return 0;
  if (x <= min) return min;
  if (x >= max) return max;
  return Math.trunc(x);
};

// The same cast for a BigInt destination: `bits` wide, `signed` or not. The
// bounds are computed here rather than passed in, because they do not fit a
// JS number and so cannot be literals in the argument list.
globalThis.__rt.f2i_big = (x, bits, signed) => {
  if (Number.isNaN(x)) return 0n;
  const width = BigInt(bits);
  const min = signed ? -(1n << (width - 1n)) : 0n;
  const max = signed ? (1n << (width - 1n)) - 1n : (1n << width) - 1n;
  // Compare in floats: `BigInt(x)` throws on a non-integral or infinite `x`.
  if (x <= Number(min)) return min;
  if (x >= Number(max)) return max;
  return BigInt(Math.trunc(x));
};

// ---------------------------------------------------------------------------
// Fat pointers (S1-M4). Append below, do not reorder.
//
// `&[T]`/`&mut [T]` are `{ buf, off, len }` and `&dyn Trait` is `{ ptr, meta }`,
// both built inline by the backend; nothing here is needed for them. What is
// needed is a write through a reference to an aggregate.
// ---------------------------------------------------------------------------

// `*target = value` where the pointee is an aggregate: every alias has to see
// the new contents, so the object is overwritten in place rather than rebound.
//
// `Object.assign` alone is wrong for an enum: assigning a variant with fewer
// fields over one with more leaves the old payload keys beside the new `TAG`,
// and a later read of the *new* variant's field would find a stale value. So
// the old keys go first. An array (a tuple, an array, a closure environment)
// truncates instead, since `delete` cannot remove its `length`.
globalThis.__rt.overwrite = (target, value) => {
  if (Array.isArray(target)) {
    target.length = 0;
  } else {
    for (const key of Object.keys(target)) delete target[key];
  }
  return Object.assign(target, value);
};

// ---------------------------------------------------------------------------
// Pointer model (stage 1.5). Append below, do not reorder.
//
// A pointer is a slot: `{ buf, off }`, a container plus a key, with `off`
// counted in units of the pointee type. Reading through one is `p.buf[p.off]`
// and writing is an assignment to it, both emitted inline by the backend. What
// needs a helper here is everything that has to look at a pointer's identity,
// at its address, or at a run of elements at once. CONTRACT.md, section
// "Pointers", is the specification.
//
// A helper that throws is refusing, not unfinished: every refusal names the
// operation and what about the pointers it was handed has no meaning here.
// ---------------------------------------------------------------------------

// The slot a raw pointer to a bare aggregate points into. A reference to an
// aggregate is the object itself, so a raw pointer taken from one has no slot
// to name until this makes it one. The map keeps a single slot per object, so
// two raw pointers taken from the same object are the same record and
// `ptr::eq` on them is true; `buf[0]` is the object itself, so a write through
// the pointer reaches every other alias.
globalThis.__rt._boxes = new WeakMap();
// The buffers this made, so that `_put` can tell the one place that stands for a
// local from an element of a real buffer. See `_put`.
globalThis.__rt._boxed = new WeakSet();
globalThis.__rt.box = (value) => {
  let slot = globalThis.__rt._boxes.get(value);
  if (slot === undefined) {
    slot = { buf: [value], off: 0 };
    globalThis.__rt._boxes.set(value, slot);
    globalThis.__rt._boxed.add(slot.buf);
  }
  return slot;
};

// A record with a `buf` key whose value is undefined: what the *strict* offset
// builds when it is handed an address rather than a slot, which is Rust's
// undefined behaviour (`ptr::add` requires a pointer into an allocation) and
// this model's one silent-wrongness risk. The three operations that would
// otherwise answer about `undefined` and `NaN` -- an address, an equality, an
// ordering -- say so instead. The wrapping offset (`__rt.offset`) is the form
// Rust does allow on an address, and it never builds one of these.
//
// The test is the *key*, not the value: a slot always has a buffer, so a `buf`
// property that is there and undefined can only have come from `p.buf` on
// something that had none.
globalThis.__rt._named = (p, op) => {
  if (p !== null && typeof p === "object" && p.buf === undefined && "buf" in p) {
    throw new Error(
      "rustc_codegen_mires: `" + op + "` on a pointer that was offset from an address with no " +
        "provenance",
    );
  }
};

// ---------------------------------------------------------------------------
// The dangling pointer
// ---------------------------------------------------------------------------
//
// `NonNull::dangling()` is an alignment held as a bare number, which is what
// `addr` reserves every address below 4096 for. That worked for as long as such
// a pointer was only ever compared against null -- and it is not. A collection
// that has never allocated holds one as its data pointer, so `Vec::new()`
// followed by `for x in &v` builds a fat pointer out of it (`{ buf: p.buf,
// off: p.off, len: 0 }` over a number) and gets `{ buf: undefined, off:
// undefined }`. `slice::Iter` then offsets that by the length, producing the
// poisoned `{ buf: undefined, off: NaN }` record `_named` refuses -- so
// iterating an empty `Vec` threw, on a program that is perfectly sound Rust.
//
// The fix is to give the dangling pointer a buffer of its own the moment it is
// cast to elements, rather than to teach every consumer about numbers. It is a
// FROZEN EMPTY ARRAY, one per alignment, so:
//
//   * two dangling pointers of the same alignment compare EQUAL (same buffer,
//     same offset), which is what makes `iter.ptr === iter.end` terminate the
//     loop on the first test;
//   * the strict offset builds `{ buf: <that array>, off: 0 + n }` rather than
//     a poisoned record, so nothing downstream has to know;
//   * `_named` keeps its job. The poisoned shape can still be built -- by
//     offsetting a real `without_provenance` address, which is the case Rust
//     genuinely forbids -- and that is now the ONLY way to build one, which is
//     what makes the refusal mean something.
//
// A dangling pointer is not dereferenceable, and the empty array is why: there
// is no element at any index. A program that reads one is already undefined
// behaviour in Rust, and reads `undefined` here rather than throwing. That is
// the one thing this trades away, and it is traded for `for x in v` over a
// `Vec` that has never allocated, which is ordinary code.
//
// `0` is not converted: that is `null()`, and `is_null()` is `addr(p) === 0`.
globalThis.__rt._dangling_bufs = new Map();
// buffer -> the alignment it is the dangling pointer for, so `addr` can answer
// with the alignment the model promises rather than a synthetic base.
globalThis.__rt._dangling_of = new WeakMap();

globalThis.__rt._dangling = (align) => {
  let buf = globalThis.__rt._dangling_bufs.get(align);
  if (buf === undefined) {
    buf = Object.freeze([]);
    globalThis.__rt._dangling_bufs.set(align, buf);
    globalThis.__rt._dangling_of.set(buf, align);
  }
  return { buf, off: 0 };
};

// `p as usize`: the synthetic address of a pointer, where `size` is the size of
// the pointee in bytes. Every buffer is given a base of `4096 * n` the first
// time it is asked about, kept in a WeakMap so that it never moves, and an
// element at offset `off` sits at `base + off * size`. A scaled slot's offset
// is already in bytes and is added as it is; a number is an address already and
// comes back unchanged.
//
// The four properties this scheme exists for: a base is never zero, so
// `is_null()` is false for every real pointer and true only for `null()`; a
// base is at least 4096, so `NonNull::dangling()` (an alignment, always
// smaller) never collides with one; an address is a multiple of the element
// size, so alignment predicates answer correctly; and the low bits are
// therefore determined by the element size, which is what `fmt::Arguments`
// tests when it looks at `bits & 1`.
globalThis.__rt._addrs = new WeakMap();
globalThis.__rt._addrNext = 0;
globalThis.__rt.addr = (p, size) => {
  if (typeof p === "number") return p;
  if (p === null || p === undefined) return 0;
  globalThis.__rt._named(p, "addr");
  const key = typeof p.buf === "object" && p.buf !== null ? p.buf : p;
  // A dangling pointer keeps the alignment it was born as, which is the
  // guarantee this whole scheme is built on: it is below 4096, so it never
  // collides with a base. The buffer is recognised rather than a field on the
  // record, because the strict offset and `__rt.offset` both rebuild the record
  // and only the buffer survives both.
  const align = globalThis.__rt._dangling_of.get(key);
  if (align !== undefined) {
    const at = typeof p.off === "number" ? p.off : 0;
    return align + (p.sc !== undefined ? at : at * (p.es === undefined ? size : p.es));
  }
  let base = globalThis.__rt._addrs.get(key);
  if (base === undefined) {
    base = 4096 * ++globalThis.__rt._addrNext;
    globalThis.__rt._addrs.set(key, base);
  }
  // A slot keyed by a property name (a field of an aggregate) has no address of
  // its own, and reports the address of the object it is a field of.
  const off = typeof p.off === "number" ? p.off : 0;
  if (p.sc !== undefined) return base + off;
  // `es` is the element size a `*const ()` remembers (see `erase`): the caller
  // that reaches here for an erased pointer knows only that the pointee is zero
  // sized, and the offset is still counted in the elements it was erased from.
  return base + off * (p.es === undefined ? size : p.es);
};

// offset_from(a, b): how many elements `a` is past `b`, for an *aggregate*
// pointee. The thin case is `a.off - b.off` and the backend emits it inline;
// this exists for the one shape that subtraction cannot answer.
//
// A reference to an aggregate is the object itself, so a raw pointer made from
// one that arrived as a value -- a parameter, the result of a call -- is a
// `__rt.box`: a buffer of one holding the object, which knows the object but
// not where it lives. When the two records name different buffers, exactly one
// of them can be that box, and the place it is asking about is where its object
// sits in the *other* buffer. Identity finds it: an aggregate is one JavaScript
// object, and it is in a buffer at most once.
globalThis.__rt.offset_from = (a, b) => {
  if (typeof a === "number" || typeof b === "number") return a - b;
  if (a === null || b === null) return 0;
  if (a.buf === b.buf) return a.off - b.off;
  const home = (p, other) => {
    if (!globalThis.__rt._boxed.has(p.buf) || !Array.isArray(other.buf)) return -1;
    return other.buf.indexOf(p.buf[p.off]);
  };
  const inb = home(a, b);
  if (inb >= 0) return inb - b.off;
  const ina = home(b, a);
  if (ina >= 0) return a.off - ina;
  throw new Error(
    "rustc_codegen_mires: `offset_from` between pointers into two different buffers",
  );
};

// `ptr::eq`: whether two pointers name the same place. `===` answers first, so
// two addresses, and two references to one object, cost nothing; anything that
// is not an object can only be equal that way. `len` is deliberately not
// compared, which makes comparing a fat pointer with the thin pointer to its
// start free.
//
// A pointer that is an object is one of two things, and only one of them is a
// slot. A reference to an aggregate IS that aggregate, and two of those are
// equal exactly when `===` said so -- which is why a `buf` has to be there
// before the fields are compared. Without that test two unrelated records
// compare `undefined` against `undefined` three times and every pair of them is
// equal: `Rc::ptr_eq` on two separate allocations answered true, because
// `ptr::addr_eq` casts both to `*const ()` and a cast to a zero sized pointee is
// the reference form of the source.
globalThis.__rt.ptr_eq = (a, b) => {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
  globalThis.__rt._named(a, "ptr_eq");
  globalThis.__rt._named(b, "ptr_eq");
  if (!("buf" in a) || !("buf" in b)) return false;
  return a.buf === b.buf && a.off === b.off && a.sc === b.sc;
};

// `<`, `<=`, `>`, `>=` and `Ord` on two pointers, as -1, 0 or 1. Two slots into
// one buffer compare by offset, which is exact. Anything else falls back to the
// synthetic addresses, which orders by buffer first and by offset within it.
globalThis.__rt.ptr_cmp = (a, b, size) => {
  if (a !== null && b !== null && typeof a === "object" && typeof b === "object"
      && a.buf === b.buf) {
    globalThis.__rt._named(a, "ptr_cmp");
    return a.off < b.off ? -1 : a.off > b.off ? 1 : 0;
  }
  const x = globalThis.__rt.addr(a, size);
  const y = globalThis.__rt.addr(b, size);
  return x < y ? -1 : x > y ? 1 : 0;
};

// offset(p, count, size): the wrapping offset, `ptr::wrapping_add` and the
// `arith_offset` intrinsic, which is the one offset Rust allows on a pointer
// with no provenance -- so it is the one that has to tell the forms apart. A
// number moves by `count * size` bytes and stays a number; a slot gets a new
// record, because a slot record is immutable; a scaled slot keeps its `sc`, or
// it could never be cast back.
//
// The strict offset (`ptr::add`) is emitted inline by the backend instead: it
// requires the pointer to point into an allocation, so an address cannot reach
// it in a program that is not already undefined behaviour, and keeping it a
// record literal is what lets `p.buf[p.off + 1]` fold out of it.
globalThis.__rt.offset = (p, count, size) => {
  if (typeof p === "number") return p + count * size;
  if (p.sc === undefined) return { buf: p.buf, off: p.off + count };
  return { buf: p.buf, off: p.off + count, sc: p.sc };
};

// The element range a run of `count` units at `p` names: the buffer, the index
// of the first element, and how many there are. `op` names the caller, so that
// every refusal below says which operation refused.
//
// For an ordinary slot the count is already in elements. For a scaled slot --
// the byte view an `as *const u8` cast produces -- both the offset and the
// count are in bytes, and only a whole number of elements can be named: an
// offset or a length that is not a multiple of the element size describes bytes
// that this model does not have, and throwing is the honest answer.
//
// The bounds check is not a Rust bounds check (a run past the end of an
// allocation is undefined behaviour, so a program has no right to one); it is
// how a *byte* count over a buffer that does not hold bytes -- the shape a
// missing scale cast would produce -- fails loudly rather than comparing
// `undefined` with `undefined` and reporting equality.
globalThis.__rt._run = (p, count, op) => {
  if (p === null || typeof p !== "object" || p.buf === undefined) {
    throw new Error("rustc_codegen_mires: `" + op + "` through a pointer that names no buffer");
  }
  let off = p.off;
  let n = count;
  if (p.sc !== undefined) {
    if (off % p.sc !== 0 || n % p.sc !== 0) {
      throw new Error(
        "rustc_codegen_mires: `" + op + "` over " + n + " bytes at byte offset " + off +
          " of " + p.sc + "-byte elements",
      );
    }
    off /= p.sc;
    n /= p.sc;
  }
  if (Array.isArray(p.buf) && off + n > p.buf.length) {
    throw new Error(
      "rustc_codegen_mires: `" + op + "` over " + n + " elements at " + off +
        " reaches past the end of a buffer of " + p.buf.length,
    );
  }
  return { buf: p.buf, off: off, n: n };
};

// The scales of two pointers one operation reads together. They have to agree:
// the bytes of a buffer of one element size do not line up with the bytes of a
// buffer of another, and no answer about them would mean anything.
globalThis.__rt._scales = (a, b, op) => {
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    throw new Error("rustc_codegen_mires: `" + op + "` on a pointer that names no buffer");
  }
  if (a.sc !== b.sc) {
    throw new Error(
      "rustc_codegen_mires: `" + op + "` between a pointer of scale " + a.sc +
        " and one of scale " + b.sc,
    );
  }
};

// copy(dst, src, count): moves `count` elements, correct when the two ranges
// overlap, which is `ptr::copy` and `slice::copy_within`. `clone` is present
// when an element is an aggregate: a byte for byte copy produces an independent
// value, and assigning a JavaScript object would produce an alias.
//
// Inside one buffer with nothing to clone, `copyWithin` is the whole operation
// and handles the overlap itself. Otherwise the loop runs away from the
// overlap: upwards when the destination is below the source, downwards when it
// is above.
globalThis.__rt.copy = (dst, src, count, clone) => {
  globalThis.__rt._scales(dst, src, "copy");
  const d = globalThis.__rt._run(dst, count, "copy");
  const s = globalThis.__rt._run(src, count, "copy");
  if (clone === undefined && d.buf === s.buf && Array.isArray(d.buf)) {
    d.buf.copyWithin(d.off, s.off, s.off + s.n);
    return;
  }
  const each = clone === undefined ? (v) => v : clone;
  const put = globalThis.__rt._put;
  if (d.buf === s.buf && d.off > s.off) {
    for (let i = s.n - 1; i >= 0; i--) put(d.buf, d.off + i, each(s.buf[s.off + i]));
  } else {
    for (let i = 0; i < s.n; i++) put(d.buf, d.off + i, each(s.buf[s.off + i]));
  }
};

// The one element store the copies share, and the one place where storing is
// not an assignment.
//
// A `__rt.box` buffer is not a buffer: it is the single place that stands for a
// local whose address was taken, and the local still names the *object* that was
// in it. Storing a new object there would leave the local looking at the old
// one, so the object is overwritten in place instead -- which is the deref
// table's "object" row, and what makes `ptr::swap(&mut local, ..)` and
// `ptr::write(p, ..)` reach the local.
//
// An element of a real buffer is rebound, because there the place is what
// pointers name: a slot reads `buf[off]`, so it sees the new object, while an
// aggregate *read* out of the buffer was copied on the way out (`read_via_copy`)
// and is nobody's alias.
globalThis.__rt._put = (buf, off, value) => {
  const old = buf[off];
  if (old !== null && typeof old === "object" && value !== null && typeof value === "object") {
    globalThis.__rt.overwrite(old, value);
    return;
  }
  buf[off] = value;
  // An aggregate stored into a **heap block** remembers where it lives, so that
  // `__rt.box` hands back that place rather than inventing a buffer of one.
  //
  // `Box<dyn Trait>` is why. Its data half is the *reference* to the concrete
  // value, which for an aggregate is the object itself, so the slot naming the
  // allocation is not in the fat pointer any more -- and dropping the box has to
  // find it again to free the block. Only a heap block is registered: for every
  // other buffer, a raw pointer made from a bare object is not expected to name
  // the place the object happens to sit in.
  if (
    value !== null && typeof value === "object" &&
    globalThis.__rt._blocks.has(buf) && !globalThis.__rt._boxes.has(value)
  ) {
    globalThis.__rt._boxes.set(value, { buf, off });
  }
};

// copy_nonoverlapping(dst, src, count): the same move, without the overlap, so
// the direction never matters.
globalThis.__rt.copy_nonoverlapping = (dst, src, count, clone) => {
  globalThis.__rt._scales(dst, src, "copy_nonoverlapping");
  const d = globalThis.__rt._run(dst, count, "copy_nonoverlapping");
  const s = globalThis.__rt._run(src, count, "copy_nonoverlapping");
  const each = clone === undefined ? (v) => v : clone;
  for (let i = 0; i < s.n; i++) globalThis.__rt._put(d.buf, d.off + i, each(s.buf[s.off + i]));
};

// write_bytes(dst, byte, count): sets `count` elements to the value that byte
// pattern denotes, which is only a value this model can name for a byte-sized
// or all-zero pointee.
//
// `zero` is the pointee's zero and is absent for a one byte pointee, where the
// pattern *is* the value (the backend has already read it with the pointee's
// signedness). With it, only a zero pattern has a name: `0x01` repeated over a
// four byte integer is `16843009`, which is arithmetic on a representation this
// model does not have.
globalThis.__rt.write_bytes = (dst, byte, count, zero) => {
  let value = byte;
  if (zero !== undefined) {
    if (byte !== 0) {
      throw new Error(
        "rustc_codegen_mires: `write_bytes` of the byte " + byte +
          " over a pointee that is wider than a byte (only zero has a value there)",
      );
    }
    value = zero;
  } else if (dst !== null && typeof dst === "object" && dst.sc !== undefined && byte !== 0) {
    throw new Error(
      "rustc_codegen_mires: `write_bytes` of the byte " + byte + " over the byte view of " +
        dst.sc + "-byte elements",
    );
  }
  const d = globalThis.__rt._run(dst, count, "write_bytes");
  if (Array.isArray(d.buf)) {
    d.buf.fill(value, d.off, d.off + d.n);
    return;
  }
  for (let i = 0; i < d.n; i++) d.buf[d.off + i] = value;
};

// compare_bytes(a, b, count): the lexicographic comparison of `count` bytes, as
// a negative number, zero or a positive one. Unscaled byte buffers compare
// exactly -- the byte count is the element count, so the difference of two
// elements is the difference of two bytes, which is what `memcmp` returns and
// what the bytewise `PartialEq` of `str` and `[u8]` reads. Two scaled slots of
// equal `sc` compare element by element over `count / sc`, which is what makes
// `[i32] == [i32]` work; the sign of the answer is then the order of two whole
// elements rather than of their first differing byte, which no caller of this
// intrinsic looks at.
globalThis.__rt.compare_bytes = (a, b, count) => {
  globalThis.__rt._scales(a, b, "compare_bytes");
  const x = globalThis.__rt._run(a, count, "compare_bytes");
  const y = globalThis.__rt._run(b, count, "compare_bytes");
  const scaled = a.sc !== undefined;
  for (let i = 0; i < x.n; i++) {
    const left = x.buf[x.off + i];
    const right = y.buf[y.off + i];
    if (left !== right) {
      if (scaled) return left < right ? -1 : 1;
      return left - right;
    }
  }
  return 0;
};

// read_array(p, n): the `n` elements at `p` as a JavaScript array, which is
// what a read through a window slot (`*const [E; N]`) produces.
//
// A window slot spans `n` consecutive elements of its buffer, so the array is
// the window copied out; a plain slot names one place, which for an array
// pointee already holds the whole array. Copying is right for both: `*p` on an
// array pointee is a read of the value, and the projections that write through
// the pointer never come here -- they index the buffer directly.
globalThis.__rt.read_array = (p, n) => {
  if (p === null || typeof p !== "object") {
    throw new Error("rustc_codegen_mires: cannot read through this pointer");
  }
  if (p.w === undefined) return p.buf[p.off];
  return Array.prototype.slice.call(p.buf, p.off, p.off + n);
};

// array_ref(p, n): the array object a `&[E; N]` or `&mut [E; N]` is, where the
// pointer it is taken from may name the array or may name its elements.
//
// A reference to an aggregate is the aggregate's own JavaScript object, so this
// has to hand back something that IS an array and that ALIASES what the pointer
// names. Which of those two the record is cannot be read off the type, so it is
// asked here, the same way `unwindow` asks it:
//
//   * a plain slot holds the whole array as its one element, so that element is
//     the answer. A place a write is about to put an array in -- a fresh heap
//     block, which is what `Box::new([1, 2, 3])` allocates -- has nothing there
//     yet, and handing back what is there is still right: the write reaches the
//     place through the pointer, not through this.
//   * a window slot (`as *const [E; N]` over a longer buffer) and a slice record
//     that a `try_into` left its `len` on both name `n` consecutive elements of
//     somebody else's buffer. There is no array object there to hand back, so one
//     is made whose `n` elements are accessors onto those places: it is a real
//     array (`Array.isArray`, `length`, indexing, spreading), and a write through
//     it is seen by every alias of the buffer, which a copy would not be.
globalThis.__rt.array_ref = (p, n) => {
  if (p === null || typeof p !== "object") {
    throw new Error("rustc_codegen_mires: this pointer does not name an array");
  }
  if (p.w === undefined) {
    const held = p.buf === undefined ? undefined : p.buf[p.off];
    if (held === undefined || (Array.isArray(held) && held.length === n)) {
      return held;
    }
  }
  return globalThis.__rt._element_view(p.buf, p.off, n);
};

// The element views built so far, keyed by buffer and then by offset and length,
// so that two references to the same `n` elements are the same object and pointer
// identity holds across them.
globalThis.__rt._views = new WeakMap();

// _element_view(buf, off, n): an array of `n` accessors onto `buf[off .. off + n]`.
globalThis.__rt._element_view = (buf, off, n) => {
  if (buf === null || typeof buf !== "object") {
    throw new Error("rustc_codegen_mires: this pointer names no buffer to take an array out of");
  }
  // The buffer already is that array, which is the common case: a window over the
  // whole of what it was cast from.
  if (off === 0 && buf.length === n) return buf;

  let views = globalThis.__rt._views.get(buf);
  if (views === undefined) {
    views = new Map();
    globalThis.__rt._views.set(buf, views);
  }
  const key = off + ":" + n;
  const found = views.get(key);
  if (found !== undefined) return found;

  const view = new Array(n);
  for (let i = 0; i < n; i++) {
    Object.defineProperty(view, i, {
      get: () => buf[off + i],
      set: (value) => {
        buf[off + i] = value;
      },
      enumerable: true,
      configurable: true,
    });
  }
  views.set(key, view);
  return view;
};

// write_array(p, values): stores `values` element by element at `p`, the write
// half of read_array. Element by element, never a rebind: the elements live in
// somebody else's buffer and every alias of it has to see the new contents.
globalThis.__rt.write_array = (p, values) => {
  if (p === null || typeof p !== "object") {
    throw new Error("rustc_codegen_mires: cannot write through this pointer");
  }
  if (p.w === undefined) {
    // `_put` rather than `overwrite`: the one place a plain slot names may hold
    // no array yet, which is what a fresh heap block is (`Box::new([1, 2, 3])`).
    globalThis.__rt._put(p.buf, p.off, values);
    return;
  }
  for (let i = 0; i < values.length; i++) p.buf[p.off + i] = values[i];
};

// unref(p): the raw pointer a reference denotes, where the type does not say
// which form that reference has.
//
// The data half of a `&dyn Trait` is the reference to a concrete value whose type
// is gone: an object for an aggregate, a slot for everything else. A cast of the
// trait object to a thin pointer has to produce the raw pointer either way, so
// the record is asked. An object goes through `box`, which for one that lives in
// a heap block hands back the place it lives in -- that is what lets a
// `Box<dyn Trait>` be freed.
globalThis.__rt.unref = (p) => {
  if (p === null || typeof p !== "object") return p;
  if ("buf" in p || "ptr" in p) return p;
  return globalThis.__rt.box(p);
};

// unwindow_slice(p, len): the fat slice a `&[E; N] -> &[E]` coercion of a *raw*
// pointer produces.
//
// Where the elements live is a property of the record, not of the type, so it is
// asked here for the same reason `unwindow` asks it: a plain slot holds the whole
// array as its one element, a window slot already spans them, and a `Box<[E; N]>`
// is a heap block keyed to `E`, which is the second shape.
globalThis.__rt.unwindow_slice = (p, len) => {
  const q = globalThis.__rt.unwindow(p);
  return { buf: q.buf, off: q.off, len };
};

// chunk_slice(p, len, width): `from_raw_parts` where the element of the slice
// is itself an array -- `as_chunks::<width>()`, and the tail of `align_to`.
//
// Whether that is expressible is a property of the buffer, not of the type, so
// it is asked here. A plain slot into a buffer whose elements really are those
// arrays rebuilds into a slice over them, unchanged. A window slot spans `width`
// consecutive elements of a flat buffer, and a plain slot over bare elements
// holds no arrays at all: neither buffer has a place holding a chunk, so there
// is nothing for the slice to name and throwing is the honest answer.
globalThis.__rt.chunk_slice = (p, len, width) => {
  if (p === null || typeof p !== "object" || p.buf === undefined) {
    throw new Error("rustc_codegen_mires: `as_chunks` through a pointer that names no buffer");
  }
  if (p.w === undefined && (len === 0 || Array.isArray(p.buf[p.off]))) {
    return { buf: p.buf, off: p.off, len };
  }
  throw new Error(
    "rustc_codegen_mires: this buffer holds no [E; " + width +
      "] elements, so it cannot be read at chunk granularity",
  );
};

// erase(p, size): `p as *const ()`, where `size` is the size of the pointee
// being erased.
//
// The value is the pointer itself, unchanged in everything a dereference or a
// comparison reads -- that is what lets `core::fmt` hand the erased pointer
// straight to a formatter expecting a `&T` (CONTRACT.md, the casts table). What
// it gains is `es`, the element size the type no longer says: `<*const T>::addr`
// is written `transmute(self.cast::<()>())`, so without a record of it the
// address of `&v[1]` would come out one past the address of `&v[0]` instead of
// four, and every alignment predicate built on it would lie.
//
// A record that already counts in bytes (`sc`) knows its own scale, and an
// address is a number with no offset to scale; both come back unchanged.
globalThis.__rt.erase = (p, size) => {
  if (p === null || typeof p !== "object") return p;
  if (p.sc !== undefined || p.es !== undefined || p.buf === undefined) return p;
  return { buf: p.buf, off: p.off, es: size };
};

// scale(p, size): the byte view of a pointer to `size`-wide elements, which is
// what `as *const u8` produces -- `{ buf, off * size, sc: size }`, a record that
// is not dereferenceable and exists to be offset in bytes and cast back.
//
// An *address* comes back unchanged, and that is the whole reason this is a
// helper rather than the record literal it used to be: `ptr::null::<i32>()` is
// the number `0`, `is_null` is `self.cast::<u8>().addr() == 0`, and a record
// built out of `(0).buf` names nothing at all.
globalThis.__rt.scale = (p, size) => {
  if (typeof p === "number") return p;
  if (p === null || p === undefined) return 0;
  return { buf: p.buf, off: p.off * size, sc: size };
};

// unscale(p, size, mk): turns a byte-granular pointer into a slot over
// `size`-wide elements. Two very different pointers reach it.
//
// A **scaled slot** is a byte offset view an `as *const u8` cast made over a
// buffer that already holds `sc`-byte elements, and unscaling it is re-keying
// the offset. It answers only at the scale the view was made with, and only for
// an offset that lands on an element -- which is what makes an aligned
// `byte_add` honest rather than silently wrong. A pointer into a buffer that is
// bytes all the way down has no `sc` and no element holding the `size` bytes
// the cast asks for: that is byte punning, which this model cannot express.
//
// A **heap block** is the other, and it is where a `Vec` is born. The block
// comes back from `alloc` byte granular because a `Layout` is bytes, and the
// very next thing every allocation does is cast it to the type it was made for.
// So the first such cast *retypes* the block: its length becomes the element
// count the byte capacity buys, and its element size is recorded. That is sound
// because the bytes are uninitialized -- there is nothing in them to lose --
// and it is the only moment at which it is, so a block that has already been
// retyped may only be asked for the size it already has.
//
// `mk` is how a *zeroed* block is filled once retyped: only the compiler knows
// what a zero element of the target type looks like, so the cast hands over a
// factory for one (`alloc_support::zero_factory`), or `null` when it cannot
// spell one.
globalThis.__rt.unscale = (p, size, mk) => {
  // The way back for an address, which [`scale`] let through unchanged --
  // except for the one address that is not an address. See `_dangling`.
  if (typeof p === "number") {
    return p > 0 && p < 4096 ? globalThis.__rt._dangling(p) : p;
  }
  if (p === null || typeof p !== "object") {
    throw new Error(
      "rustc_codegen_mires: this pointer is not a byte view of " + size +
        "-byte elements, so it cannot be cast back to one",
    );
  }
  if (p.sc !== undefined) {
    if (p.sc !== size) {
      throw new Error(
        "rustc_codegen_mires: this pointer is a byte view of " + p.sc +
          "-byte elements, not of " + size + "-byte ones",
      );
    }
    if (p.off % size !== 0) {
      throw new Error(
        "rustc_codegen_mires: byte offset " + p.off + " is not a multiple of " +
          size + ", so it does not name an element",
      );
    }
    return { buf: p.buf, off: p.off / size };
  }
  const block = globalThis.__rt._heap_block(p.buf);
  if (block !== undefined) return globalThis.__rt._retype(p, block, size, mk);
  throw new Error(
    "rustc_codegen_mires: this pointer is not a byte view of " + size +
      "-byte elements, so it cannot be cast back to one",
  );
};

// thin(p, size): the pointer `from_raw_parts` answers with when the metadata is
// `()`, which is to say no rebuild at all -- a thin pointer *is* its data
// pointer, over `size`-wide elements.
//
// All of the work is in the erasure that data pointer arrived through.
// `from_raw_parts` takes a `*const impl Thin`, and `with_metadata_of` -- how
// `byte_offset`, `byte_add` and `byte_sub` are written -- hands it
// `self as *const ()`. So the value can be the scaled byte slot that
// `cast::<u8>()` produced, which has to be keyed back into elements, or a plain
// slot that is already one, or the reference form of an aggregate, or an address.
// Only the record knows which, so it is asked rather than guessed from the type,
// the same way `unwindow` is.
globalThis.__rt.thin = (p, size) => {
  if (p === null || typeof p !== "object") return p;
  // A byte view: key it back, which throws unless the offset lands on an element.
  // No zero factory: a byte view is never a fresh block, so no retype can happen
  // here and there is nothing to fill.
  if (p.sc !== undefined) return globalThis.__rt.unscale(p, size, null);
  // Already a slot -- including the provenance-less record `_named` reports on,
  // which stays as it is so that the operation that cannot answer says so.
  if ("buf" in p) return p;
  // A bare aggregate: the reference form, whose raw pointer is its box.
  return globalThis.__rt.box(p);
};

// unwindow(p): a pointer to `[E; N]`, as a pointer to its first element.
//
// The two forms a pointer to an array can have are told apart here, because
// only the record knows which one it is. A window slot -- what an
// `as *const [E; N]` cast produces -- already spans elements of the buffer, so
// dropping `w` is the whole of it. A plain slot holds the array as its one
// element, so the pointer has to step into it. Both come back naming the same
// first element, which is what the cast owes.
globalThis.__rt.unwindow = (p) => {
  if (p === null || typeof p !== "object") {
    throw new Error("rustc_codegen_mires: this pointer does not name an array");
  }
  // A bare array is the *reference* form of `&[E; N]` being reinterpreted as a thin
  // pointer (`Arguments::new`'s `&[u8; N] -> NonNull<u8>` transmute): the pointer to
  // its first element is the array itself with index zero.
  if (Array.isArray(p)) return { buf: p, off: 0 };
  if (p.w === undefined) return { buf: p.buf[p.off], off: 0 };
  return { buf: p.buf, off: p.off };
};

// The `str` hybrid. A `&str` is a JavaScript string, and its byte view is
// produced on demand by the three helpers below: the encoder and the decoder
// are built once, because constructing one per call is the expensive part.
globalThis.__rt._encoder = new TextEncoder();
globalThis.__rt._decoder = new TextDecoder();

// str_bytes(s): the UTF-8 bytes of a JavaScript string as `{ buf, off, len }`,
// which is `str::as_bytes` and `str::as_ptr`.
//
// `buf` is a plain JavaScript array, not the `Uint8Array` the encoder returns,
// because that is what every other `[u8]` in the emitted program is: a slice
// built from a Rust array literal is a plain array, and the two have to be the
// same kind of thing for `__rt.copy`, `__rt.overwrite` and a bare `p.buf[p.off]`
// write to work on both.
//
// A four entry memo ring keyed by the string returns the *same* record for a
// repeated call, so `s.as_bytes().as_ptr() == s.as_bytes().as_ptr()` holds in
// the shapes a program actually writes and a byte slice compared with itself
// takes the fast path. Correctness never depends on it: the ring is four
// entries, so a fifth distinct string evicts the first, and two records over
// equal bytes are as good as one everywhere but pointer identity — which
// CONTRACT.md documents as not guaranteed across independent calls.
globalThis.__rt._strBytes = [];
globalThis.__rt._strBytesNext = 0;
globalThis.__rt.str_bytes = (s) => {
  const memo = globalThis.__rt._strBytes;
  for (let i = 0; i < memo.length; i++) {
    if (memo[i].s === s) return memo[i].r;
  }
  const bytes = Array.prototype.slice.call(globalThis.__rt._encoder.encode(s));
  const record = { buf: bytes, off: 0, len: bytes.length };
  memo[globalThis.__rt._strBytesNext] = { s: s, r: record };
  globalThis.__rt._strBytesNext = (globalThis.__rt._strBytesNext + 1) % 4;
  return record;
};

// str_len(s): the UTF-8 byte length of a JavaScript string, which is the
// metadata of a `&str`.
//
// The fast path is the common one: a string whose every code unit is below 0x80
// is one byte per unit, so its length is the answer and nothing is encoded. A
// scan is still O(n), but it allocates nothing, and it is exact for the
// surrogate pairs an astral character arrives as.
globalThis.__rt.str_len = (s) => {
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) > 0x7f) return globalThis.__rt._encoder.encode(s).length;
  }
  return s.length;
};

// bytes_str(buf, off, len): decodes the `len` UTF-8 bytes at `buf[off]` back
// into a JavaScript string, which is `str::from_utf8_unchecked` and a raw
// pointer whose pointee is `str`.
//
// The bytes are a plain array (see str_bytes), which the decoder does not take,
// so they are copied into a `Uint8Array` first; a typed array that arrived from
// somewhere else is sliced directly.
//
// The ASCII fast path is what a `String` built out of `alloc` actually hits:
// every byte below 0x80 is its own code point, so the string is assembled
// directly and neither the copy nor the decoder runs. `String.fromCharCode`
// takes a bounded run of arguments, so the loop chunks; above the chunk size,
// or at the first byte that is not ASCII, the decoder takes over.
globalThis.__rt._ASCII_CHUNK = 4096;
globalThis.__rt.bytes_str = (buf, off, len) => {
  // A zero length `str` reads nothing, and the buffer it names may not exist at
  // all: `let none: [u8; 0] = []` is a zero sized place, whose JavaScript value
  // is `undefined`.
  if (len === 0) return "";
  if (len <= globalThis.__rt._ASCII_CHUNK) {
    let ascii = true;
    for (let i = 0; i < len; i++) {
      if (buf[off + i] > 0x7f) {
        ascii = false;
        break;
      }
    }
    if (ascii) {
      return String.fromCharCode.apply(
        null,
        Array.prototype.slice.call(buf, off, off + len),
      );
    }
  }
  const bytes = buf instanceof Uint8Array
    ? buf.subarray(off, off + len)
    : Uint8Array.from(Array.prototype.slice.call(buf, off, off + len));
  return globalThis.__rt._decoder.decode(bytes);
};

// ---------------------------------------------------------------------------
// The heap
// ---------------------------------------------------------------------------
//
// A heap block is a JavaScript array, and everything else known about it lives
// in a side table keyed by that array: its byte capacity, the size of the
// elements it currently holds, whether it has been retyped yet, and whether it
// was handed out zeroed. The array is the allocation, so a pointer into it is
// an ordinary `{ buf, off }` slot and every operation the pointer model already
// has -- offsets, writes, slices -- works over it unchanged.
//
// A block is born **byte granular**, because a `Layout` is a size in bytes and
// nothing at the point of allocation says what will be put there. It is retyped
// exactly once, by `unscale`, at the first `*mut u8` to `*mut T` cast while it
// is still fresh: `RawVec::allocate_in` allocates and immediately casts, and so
// does `Box::new`. See `CONTRACT.md`'s "Allocation" section.
//
// The table is a `WeakMap`, so a block nothing points at is collected with its
// entry; `_freed` is a `WeakSet` of blocks that were deallocated, kept only so
// that a use after free is a refusal naming itself rather than a read of an
// emptied array.
globalThis.__rt._blocks = new WeakMap();
globalThis.__rt._freed = new WeakSet();

// _heap_block(buf): the block record for a buffer, or undefined if it is not
// one. A freed block is a refusal rather than a miss: the mistake is worth
// naming.
globalThis.__rt._heap_block = (buf) => {
  if (globalThis.__rt._freed.has(buf)) {
    throw new Error(
      "rustc_codegen_mires: this pointer names a heap block that has been freed",
    );
  }
  return globalThis.__rt._blocks.get(buf);
};

// _retype(p, block, size, mk): the heap arm of `unscale`.
//
// Asking for the size the block already holds is the common case and is free --
// `RawVecInner` stores its buffer as a `*mut u8` and casts it back on every
// single access, so this runs once per element read. Asking for a *different*
// size is the retype, and it is allowed only while the block is fresh, only
// when the byte capacity is a whole number of elements, and only from an offset
// that lands on one.
globalThis.__rt._retype = (p, block, size, mk) => {
  const same = block.es === size;
  // A block already read at this element size, with nothing left to do to it.
  // The common case by far: `RawVecInner` stores its buffer as a `*mut u8` and
  // casts it back on every single access.
  if (same && !(block.fresh && block.zeroed)) return p;
  if (!same) {
    if (!block.fresh) {
      throw new Error(
        "rustc_codegen_mires: this heap block already holds " + block.es +
          "-byte elements, so it cannot be read as " + size + "-byte ones",
      );
    }
    if (block.bytes % size !== 0) {
      throw new Error(
        "rustc_codegen_mires: a heap block of " + block.bytes +
          " bytes is not a whole number of " + size + "-byte elements",
      );
    }
  }
  const byte_off = p.off * block.es;
  if (byte_off % size !== 0) {
    throw new Error(
      "rustc_codegen_mires: byte offset " + byte_off + " is not a multiple of " +
        size + ", so it does not name an element",
    );
  }
  const length = block.bytes / size;
  if (!same) {
    p.buf.length = length;
    block.es = size;
  }
  block.fresh = false;
  // A zeroed block holds zero *bytes*, and a zero element is not always the
  // number 0: it is `false` for a `bool`, `{ TAG: "None" }` for a niched
  // `None`, `{ x: 0, y: 0 }` for a struct. Only the compiler knows which, so the cast
  // hands over a factory. This runs even when the element size did not change,
  // which is what `vec![false; n]` needs -- a `bool` is one byte too.
  if (block.zeroed) {
    if (mk === null || mk === undefined) {
      throw new Error(
        "rustc_codegen_mires: a zeroed heap block cannot be read as " + size +
          "-byte elements, because this backend cannot spell a zero one",
      );
    }
    for (let i = 0; i < length; i++) p.buf[i] = mk();
  }
  return same ? p : { buf: p.buf, off: byte_off / size };
};

// retype(p, size, mk): the fill half of `unscale`, for a cast that needs no
// re-keying because the element size did not change.
//
// The identity for every pointer that is not a **fresh zeroed heap block**. That
// block is the one case a same-size cast still has to act on: a block is born
// byte granular, so a one byte element is read at the size the block already
// has, and the zero bytes `alloc_zeroed` left are not the zero of every one byte
// type. `vec![false; n]` is the whole of it.
globalThis.__rt.retype = (p, size, mk) => {
  if (p === null || typeof p !== "object" || p.sc !== undefined) return p;
  const block = globalThis.__rt._heap_block(p.buf);
  if (block === undefined || !block.fresh || !block.zeroed) return p;
  return globalThis.__rt._retype(p, block, size, mk);
};

// retype_rc(p, header_mk, tail_es, len): the block RESHAPE, which is what an
// `Rc<[T]>` or an `Arc<str>` allocates.
//
// Such a block is a header plus a run of elements -- `RcInner<[T]>` is
// `{ strong, weak, value: [T] }` -- and a JavaScript array holds one element
// type. So the block is laid out as
//
//   [ <the header record>, t0, t1, ..., t(len-1) ]
//
// element 0 being the header and the tail starting at index 1, and the side
// table remembers where the tail starts as `hdr`. Every `{ buf, off }` the
// pointer model already builds then works on the tail unchanged: it is an
// ordinary slice at offset 1.
//
// The result is the fat pointer a reference to a struct with an unsized tail is,
// `{ ptr, meta }` (CONTRACT.md, "A struct with an unsized tail"), whose `ptr` is
// the slot naming the block's start.
//
// TWO ARRIVAL STATES, because `alloc` reaches this from two directions:
//
//   * `allocate_for_slice` casts the fresh `*mut u8` to a `*mut T` FIRST, so the
//     block has already been uniformly retyped to `tail_es` and `p` is a slot
//     (or a fat slice) at offset 0;
//   * `allocate_for_ptr_in` goes through `with_metadata_of`, so the block is
//     still byte granular and `p` is the `*mut u8` it was allocated as.
//
// Both are accepted, and both are a reshape of a block nothing has written yet.
// That is the invariant, and it is CHECKED rather than assumed: the retype-once
// rule becomes "at most one uniform retype, plus at most one struct reshape
// while provably unwritten", and "provably unwritten" here means every element
// really is `undefined`.
globalThis.__rt.retype_rc = (p, header_mk, tail_es, len) => {
  const buf = p === null || typeof p !== "object" ? null : p.buf;
  if (buf === null || buf === undefined) {
    throw new Error(
      "rustc_codegen_mires: a pointer with no buffer cannot be reshaped into a " +
        "header plus an unsized tail",
    );
  }
  const block = globalThis.__rt._heap_block(buf);
  if (block === undefined) {
    throw new Error(
      "rustc_codegen_mires: this pointer does not name a live heap block, so it " +
        "cannot be reshaped into a header plus an unsized tail",
    );
  }
  // Already reshaped. `RcInner` is cast back out of its `*mut u8` more than
  // once on the way through `Rc::from_raw`-shaped code, and reshaping a block
  // twice would throw its header away.
  if (block.hdr !== undefined) {
    if (block.es !== tail_es) {
      throw new Error(
        "rustc_codegen_mires: this heap block holds a tail of " + block.es +
          "-byte elements, so it cannot be read as one of " + tail_es +
          "-byte ones",
      );
    }
    return { ptr: { buf, off: 0 }, meta: len };
  }
  if (header_mk === null || header_mk === undefined) {
    throw new Error(
      "rustc_codegen_mires: this backend cannot spell the header of a struct with " +
        "an unsized tail, so its block cannot be reshaped",
    );
  }
  for (let i = 0; i < buf.length; i++) {
    if (buf[i] !== undefined) {
      throw new Error(
        "rustc_codegen_mires: this heap block has already been written, so it " +
          "cannot be reshaped into a header plus an unsized tail",
      );
    }
  }
  buf.length = 1 + len;
  buf[0] = header_mk();
  block.es = tail_es;
  block.hdr = 1;
  block.fresh = false;
  return { ptr: { buf, off: 0 }, meta: len };
};

// _register(buf, bytes, zeroed): a new block, and the pointer to its start.
globalThis.__rt._register = (buf, bytes, zeroed) => {
  globalThis.__rt._blocks.set(buf, { bytes, es: 1, fresh: true, zeroed });
  return { buf, off: 0 };
};

// alloc(size, align): `__rust_alloc`. The alignment is accepted and ignored --
// there are no addresses to align in a value model, and the guarantees the
// pointer model makes about `addr` are documented in CONTRACT.md.
//
// A zero sized allocation never reaches here: `Layout` of a ZST is handled in
// `alloc` itself, which hands back a dangling pointer.
globalThis.__rt.alloc = (size, align) =>
  globalThis.__rt._register(new Array(size), size, false);

// alloc_zeroed(size, align): `__rust_alloc_zeroed`. The bytes really are zero,
// which is what `vec![0; n]` and `vec![false; n]` read back; the block also
// remembers that it was zeroed, so a later retype refills it at the new element
// size rather than leaving bytes where elements belong.
globalThis.__rt.alloc_zeroed = (size, align) =>
  globalThis.__rt._register(new Array(size).fill(0), size, true);

// dealloc(ptr, size, align): `__rust_dealloc`. The block is unregistered and
// poisoned: emptying the array makes every stale pointer into it read
// `undefined` rather than the value it used to hold, and the `_freed` set turns
// a later cast through that pointer into a refusal that says so.
globalThis.__rt.dealloc = (p, size, align) => {
  const buf = p === null || typeof p !== "object" ? null : p.buf;
  if (buf === null || !globalThis.__rt._blocks.has(buf)) {
    throw new Error(
      "rustc_codegen_mires: this pointer does not name a live heap block, so it " +
        "cannot be deallocated",
    );
  }
  globalThis.__rt._blocks.delete(buf);
  globalThis.__rt._freed.add(buf);
  buf.length = 0;
};

// realloc(ptr, size, align, new_size): `__rust_realloc`, in place.
//
// The block keeps its identity, which is the whole point: a JavaScript array
// grows and shrinks without moving, so every pointer already derived from it
// stays valid and there is nothing to copy. The new length is in elements at
// whatever size the block currently holds, so a `Vec<i32>` that has already
// been retyped grows by whole `i32`s.
globalThis.__rt.realloc = (p, size, align, new_size) => {
  const buf = p === null || typeof p !== "object" ? null : p.buf;
  const block = buf === null ? undefined : globalThis.__rt._heap_block(buf);
  if (block === undefined) {
    throw new Error(
      "rustc_codegen_mires: this pointer does not name a live heap block, so it " +
        "cannot be reallocated",
    );
  }
  if (new_size % block.es !== 0) {
    throw new Error(
      "rustc_codegen_mires: a heap block of " + new_size +
        " bytes is not a whole number of " + block.es + "-byte elements",
    );
  }
  block.bytes = new_size;
  buf.length = new_size / block.es;
  return { buf, off: p.off };
};

// alloc_error(size, align): `__rust_alloc_error_handler`. Diverging, and there
// is nothing better to do: a JavaScript array does not run out before the
// engine does.
globalThis.__rt.alloc_error = (size, align) => {
  throw new Error(
    "rustc_codegen_mires: memory allocation of " + size + " bytes failed",
  );
};

// ---------------------------------------------------------------------------
// The string sink
// ---------------------------------------------------------------------------
//
// Building a `String` a character at a time through the heap works, but it
// spends an array element per byte and a UTF-8 encode per push. A sink is the
// direct route: a host-side string builder a `fmt::Write` implementation can
// push into, and one string at the end.
//
// The handle is an index rather than the builder itself, so the whole surface
// is `usize`-shaped and an `extern "C"` declaration can name it.
globalThis.__rt._sinks = [];

// sb_new() -> handle
globalThis.__rt.sb_new = () => {
  globalThis.__rt._sinks.push("");
  return globalThis.__rt._sinks.length - 1;
};

// sb_push(h, s): appends a `&str`, which is already a JavaScript string.
globalThis.__rt.sb_push = (h, s) => {
  globalThis.__rt._sinks[h] += s;
};

// sb_push_char(h, c): appends a `char`, which is a code point number.
globalThis.__rt.sb_push_char = (h, c) => {
  globalThis.__rt._sinks[h] += String.fromCodePoint(c);
};

// sb_take(h) -> string: the finished string, and the handle is spent.
globalThis.__rt.sb_take = (h) => {
  const text = globalThis.__rt._sinks[h];
  globalThis.__rt._sinks[h] = "";
  return text;
};

// ---------------------------------------------------------------------------
// Host collections
// ---------------------------------------------------------------------------
//
// `mires_js::collections::{HashMap, HashSet}` are one host `Map` each, and
// these are the whole of what they reach for. `hashbrown` cannot be compiled by
// this backend -- it scans sixteen control bytes as one wide integer and holds
// two element granularities in one allocation, and a heap block here is a
// JavaScript array of one element type -- so the table is the engine's.
//
// A map value is NOT the Rust value: it is the `Option<V>` object holding it,
// built on the Rust side. So nothing below knows what a Rust value looks like,
// `map_get` of a present key is always an object, and `insert`/`remove` move the
// old value out with `Option::replace`/`Option::take`. See CONTRACT.md, "Host
// collections".
//
// A key is a JavaScript value the `Map` compares with SameValueZero: a number, a
// BigInt, a boolean or a string. The backend refuses anything else at the call
// site, so nothing here validates.

// map_new() -> Map: the map IS the value; nothing frees it but the collector.
globalThis.__rt.map_new = () => new Map();

// map_len(m) -> number
globalThis.__rt.map_len = (m) => m.size;

// map_clear(m)
globalThis.__rt.map_clear = (m) => {
  m.clear();
};

// map_has(m, k) -> boolean
globalThis.__rt.map_has = (m, k) => m.has(k);

// map_set(m, k, v)
globalThis.__rt.map_set = (m, k, v) => {
  m.set(k, v);
};

// map_get(m, k) -> the stored `Option<V>` object. Only ever called for a key the
// caller has already tested with `map_has`.
globalThis.__rt.map_get = (m, k) => m.get(k);

// map_del(m, k)
globalThis.__rt.map_del = (m, k) => {
  m.delete(k);
};

// map_keys(m) -> `&[K]`: a snapshot of the keys, as a fat slice over a fresh
// array. Fresh because it is a snapshot: the map may change afterwards and the
// array must not notice.
globalThis.__rt.map_keys = (m) => {
  const keys = Array.from(m.keys());
  return { buf: keys, off: 0, len: keys.length };
};

// ---------------------------------------------------------------------------
// Keyed rows
// ---------------------------------------------------------------------------
//
// The row cache behind `view_abi::push_keyed`. A view's `for` is an eager Rust
// loop, so every row is built on every render; what a key buys is that a row
// whose key was seen before contributes the NODE it contributed last time. So
// reordering a list moves the existing nodes instead of replacing them, and
// whatever DOM state they carry (focus, scroll, a playing video) survives.
//
// One cache per loop, identified by the `site` the emitter numbers the
// `push_keyed` call with, so two loops never share rows. The cache is keyed by
// the key's text, which is what `view_abi::push_keyed` documents.
//
// The freshly built row of a cached key is DISCARDED, which is the cost of
// keying an eager loop and is documented as a delta against the runtime's own
// `_$mapArray`. The cache is not pruned: a key that stops appearing keeps its
// node alive until the program ends. Both are why this is called pragmatic.
globalThis.__rt._keyed = new Map();

// keyed_row(site, key, node) -> node: the row this key contributed last time,
// or `node`, which becomes that row.
globalThis.__rt.keyed_row = (site, key, node) => {
  let rows = globalThis.__rt._keyed.get(site);
  if (rows === undefined) {
    rows = new Map();
    globalThis.__rt._keyed.set(site, rows);
  }
  const seen = rows.get(key);
  if (seen !== undefined) {
    return seen;
  }
  rows.set(key, node);
  return node;
};

// ---------------------------------------------------------------------------
// The executor (view-abi's `microtask` and `on_settled`)
// ---------------------------------------------------------------------------
//
// The whole JavaScript half of running a Rust `Future` inside an island. The
// poll loop, the task table and the owner policy are Rust, in `view-async`,
// because a loop on this side would have to call a monomorphized
// `Future::poll` through a `&mut F` and the value model hands out neither.
// What is left is a scheduler and a promise bridge, and both are three lines.

// microtask(f): runs `f` after the current call stack unwinds and before the
// browser paints.
//
// `queueMicrotask` rather than a timer: the pinned runtime's own deferral is a
// microtask (`runHydrationEvents`), and a task woken during an update pass
// should resume in the same frame that woke it.
globalThis.__rt.microtask = (f) => {
  queueMicrotask(f);
};

// settled(value, f): calls `f(x, ok)` when `value` settles.
//
// The thenable test is `typeof v.then === "function"` and NOT the runtime's own
// `"then" in v`. Upstream's is a property test, so a thenable whose `then` is
// inherited or is not callable passes it and then throws where it is called;
// this one refuses to call what is not callable. The difference is recorded in
// CONTRACT-DOM 14.9 and this is the safer half of it, which matters more here
// because the value comes from a declared interface rather than from a fetcher
// the runtime wrote.
//
// A non-thenable settles IMMEDIATELY and successfully, so awaiting a plain
// value works and costs one microtask. It is not called synchronously: a future
// that resolved without suspending would otherwise run inside its own first
// poll, which is the one place an executor must not run user code.
//
// The rejection arm delivers the reason as a value with `ok === false` rather
// than letting it stay a rejection. The pinned runtime has no asynchronous
// error path at all -- `catchError` and `ErrorBoundary` are synchronous
// try/catch -- so a rejection reaches nothing and becomes a host-level
// unhandled rejection (CONTRACT-DOM 14.4). Handing it to `f` puts it back
// inside the poll, where Rust sees an `Err`.
globalThis.__rt.settled = (value, f) => {
  if (value !== null && value !== undefined && typeof value.then === "function") {
    value.then(
      (x) => f(x, true),
      (e) => f(e, false)
    );
    return;
  }
  queueMicrotask(() => f(value, true));
};
