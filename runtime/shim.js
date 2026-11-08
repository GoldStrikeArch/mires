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
  // trailing `n` that logging a BigInt would add.
  js_log_i64: (x) => console.log(x.toString()),
  js_log_f64: (x) => console.log(x),
  js_log_bool: (x) => console.log(x),
  js_log_str: (s) => console.log(s),
  js_abort: (msg) => {
    throw __rt.abort_error(new Error("rust abort: " + msg));
  },
};

// Compiler aborts retain ordinary Error identity and shape. Keep their identities
// outside the error itself so checking an arbitrary host value never reads it.
// Independently extracted runtimes in one realm use the same weak registry.
globalThis.__rt._abort_errors = () => {
  const key = Symbol.for("rustc_codegen_js.abort-errors.v1");
  return globalThis[key] || (globalThis[key] = new WeakSet());
};

globalThis.__rt.abort_error = (error) => {
  __rt._abort_errors().add(error);
  return error;
};

globalThis.__rt.is_abort = (error) => __rt._abort_errors().has(error);

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
  // `Math.trunc(-0.5)` is `-0`, and an integer lane must never hold JavaScript's
  // negative zero (`String(-0)` is "-0"); adding zero normalizes it to `+0`.
  return Math.trunc(x) + 0;
};

// The same cast for a BigInt destination: `bits` wide, `signed` or not. The
// bounds are computed here rather than passed in, because they do not fit a
// JS number and so cannot be literals in the argument list.
globalThis.__rt.f2i_big = (x, bits, signed) => {
  if (Number.isNaN(x)) return BigInt("0");
  const width = BigInt(bits);
  const min = signed ? -(BigInt("1") << (width - BigInt("1"))) : BigInt("0");
  const max = signed
    ? (BigInt("1") << (width - BigInt("1"))) - BigInt("1")
    : (BigInt("1") << width) - BigInt("1");
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

// A pointer projected into a non-zero-sized field at layout offset zero. The
// slot keeps the original containing pointer and its element size because the
// two pointers have the same address even though their JavaScript places and
// pointee sizes differ.
globalThis.__rt.prefix = (p, key, size) => {
  if (p === null || typeof p !== "object" || !("buf" in p)) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: a zero-offset aggregate field was projected from a pointer without a slot",
    ));
  }
  return { buf: p.buf[p.off], off: key, pr: p, ps: size };
};

// The inverse of `prefix`. Checking the retained field key keeps a standalone
// value from being treated as a larger containing aggregate.
globalThis.__rt.unproject = (p, key) => {
  if (
    p === null || typeof p !== "object" || p.sc !== undefined || p.w !== undefined ||
    p.off !== key || p.pr === undefined
  ) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer does not name the required zero-offset aggregate field",
    ));
  }
  return p.pr;
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
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: `" + op + "` on a pointer that was offset from an address with no " +
        "provenance",
    ));
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

// Raw slice metadata can describe an allocator's uninitialized byte block.
globalThis.__rt.slice_from_parts = (p, len) => {
  if (typeof p === "number" && p > 0 && p < 4096) p = globalThis.__rt._dangling(p);
  return { buf: p.buf, off: p.off, len };
};

// A u8/i8 slice reference establishes initialized byte storage. Word reads must
// alias these bytes. Raw slices and MaybeUninit slices do not establish this fact.
globalThis.__rt.byte_slice_ref = (p) => {
  const block = globalThis.__rt._heap_block(p.buf);
  if (block !== undefined && block.es === 1) block.fresh = false;
  return p;
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
  if (p.pr !== undefined) return globalThis.__rt.addr(p.pr, p.ps);
  globalThis.__rt._named(p, "addr");
  const scalar = globalThis.__rt._scalar_view_of.get(p.buf);
  const key = scalar === undefined
    ? (typeof p.buf === "object" && p.buf !== null ? p.buf : p)
    : scalar.buf;
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
  let off = typeof p.off === "number" ? p.off : 0;
  if (scalar !== undefined) off *= scalar.size;
  if (p.sc !== undefined || scalar !== undefined) return base + off;
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
  throw __rt.abort_error(new Error(
    "rustc_codegen_js: `offset_from` between pointers into two different buffers",
  ));
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
  const left = a.pr === undefined ? a : a.pr;
  const right = b.pr === undefined ? b : b.pr;
  return left.buf === right.buf && left.off === right.off && a.sc === b.sc;
};

// Metadata participates in equality of two pointers to the same unsized type.
globalThis.__rt.slice_ptr_eq = (a, b) =>
  a.len === b.len && globalThis.__rt.ptr_eq(a, b);
globalThis.__rt.fat_ptr_eq = (a, b) =>
  a.meta === b.meta && globalThis.__rt.ptr_eq(a.ptr, b.ptr);

// `<`, `<=`, `>`, `>=` and `Ord` on two pointers, as -1, 0 or 1. Two slots into
// one buffer compare by offset, which is exact. Anything else falls back to the
// synthetic addresses, which orders by buffer first and by offset within it.
globalThis.__rt.ptr_cmp = (a, b, size) => {
  if (a !== null && typeof a === "object" && a.pr !== undefined) a = a.pr;
  if (b !== null && typeof b === "object" && b.pr !== undefined) b = b.pr;
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
  if (p.pr !== undefined) {
    if (count === 0) return p;
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: pointer arithmetic cannot leave a zero-offset aggregate field",
    ));
  }
  if (p.sc === undefined) return { buf: p.buf, off: p.off + count };
  const moved = { buf: p.buf, off: p.off + count, sc: p.sc };
  if (p.pv !== undefined) {
    moved.pv = p.pv;
    moved.po = p.po;
  }
  return moved;
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
    throw __rt.abort_error(new Error("rustc_codegen_js: `" + op + "` through a pointer that names no buffer"));
  }
  let off = p.off;
  let n = count;
  if (p.sc !== undefined) {
    if (off % p.sc !== 0 || n % p.sc !== 0) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: `" + op + "` over " + n + " bytes at byte offset " + off +
          " of " + p.sc + "-byte elements",
      ));
    }
    off /= p.sc;
    n /= p.sc;
  }
  if (Array.isArray(p.buf) && off + n > p.buf.length) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: `" + op + "` over " + n + " elements at " + off +
        " reaches past the end of a buffer of " + p.buf.length,
    ));
  }
  return { buf: p.buf, off: off, n: n };
};

// The scales of two pointers one operation reads together. They have to agree:
// the bytes of a buffer of one element size do not line up with the bytes of a
// buffer of another, and no answer about them would mean anything.
globalThis.__rt._scales = (a, b, op) => {
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    throw __rt.abort_error(new Error("rustc_codegen_js: `" + op + "` on a pointer that names no buffer"));
  }
  if (a.sc !== b.sc) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: `" + op + "` between a pointer of scale " + a.sc +
        " and one of scale " + b.sc,
    ));
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
  if (count === 0) return;
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
  if (count === 0) return;
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
  if (count === 0) return;
  let value = byte;
  if (zero !== undefined) {
    if (byte !== 0) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: `write_bytes` of the byte " + byte +
          " over a pointee that is wider than a byte (only zero has a value there)",
      ));
    }
    value = zero;
  } else if (dst !== null && typeof dst === "object" && dst.sc !== undefined && byte !== 0) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: `write_bytes` of the byte " + byte + " over the byte view of " +
        dst.sc + "-byte elements",
    ));
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
  if (count === 0) return 0;
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
    throw __rt.abort_error(new Error("rustc_codegen_js: cannot read through this pointer"));
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
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer does not name an array"));
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
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer names no buffer to take an array out of"));
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
    throw __rt.abort_error(new Error("rustc_codegen_js: cannot write through this pointer"));
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
    throw __rt.abort_error(new Error("rustc_codegen_js: `as_chunks` through a pointer that names no buffer"));
  }
  if (p.w === undefined && (len === 0 || Array.isArray(p.buf[p.off]))) {
    return { buf: p.buf, off: p.off, len };
  }
  throw __rt.abort_error(new Error(
    "rustc_codegen_js: this buffer holds no [E; " + width +
      "] elements, so it cannot be read at chunk granularity",
  ));
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
  if (p.pr !== undefined) {
    if (typeof p.pr.off !== "number") {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a nested zero-offset aggregate field has no byte view",
      ));
    }
    const off = p.pr.off * p.ps;
    return { buf: p.pr.buf, off, sc: size, pv: p, po: off };
  }
  const scalar = globalThis.__rt._scalar_view_of.get(p.buf);
  if (scalar !== undefined) {
    if (scalar.size !== size) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a " + scalar.size + "-byte scalar view cannot be scaled as " +
          size + "-byte elements",
      ));
    }
    return { buf: scalar.buf, off: p.off * size };
  }
  return { buf: p.buf, off: p.off * size, sc: size };
};

// Aliasing scalar views over buffers whose elements really are bytes. Numeric properties on the
// view are accessors, so the compiler's ordinary `p.buf[p.off]` load/store spelling continues to
// work while every read and write observes the original byte buffer. Views are cached per source
// buffer and scalar kind, preserving pointer identity for repeated casts.
globalThis.__rt._scalar_views = new WeakMap();
globalThis.__rt._scalar_view_of = new WeakMap();

globalThis.__rt._scalar_parts = (kind) => {
  if (typeof kind !== "string" || !/^[uif](16|32|64|128)$/.test(kind)) return null;
  const bits = Number(kind.slice(1));
  if (kind[0] === "f" && bits !== 32 && bits !== 64) return null;
  return { kind: kind[0], bits, size: bits / 8 };
};

globalThis.__rt._scalar_load = (buf, off, parts) => {
  if (parts.kind === "f") {
    const bytes = new ArrayBuffer(parts.size);
    const view = new DataView(bytes);
    for (let i = 0; i < parts.size; i++) view.setUint8(i, buf[off + i] & 255);
    return parts.bits === 32 ? view.getFloat32(0, true) : view.getFloat64(0, true);
  }
  if (parts.bits <= 32) {
    let raw = 0;
    for (let i = 0; i < parts.size; i++) raw += (buf[off + i] & 255) * 2 ** (8 * i);
    if (parts.kind === "i" && raw >= 2 ** (parts.bits - 1)) raw -= 2 ** parts.bits;
    return raw;
  }
  let raw = BigInt("0");
  for (let i = 0; i < parts.size; i++) {
    raw |= BigInt(buf[off + i] & 255) << BigInt(8 * i);
  }
  return parts.kind === "i" ? BigInt.asIntN(parts.bits, raw) : raw;
};

globalThis.__rt._scalar_store = (buf, off, parts, value) => {
  if (parts.kind === "f") {
    const bytes = new ArrayBuffer(parts.size);
    const view = new DataView(bytes);
    if (parts.bits === 32) view.setFloat32(0, value, true);
    else view.setFloat64(0, value, true);
    for (let i = 0; i < parts.size; i++) buf[off + i] = view.getUint8(i);
    return;
  }
  if (parts.bits <= 32) {
    let raw = ((value % 2 ** parts.bits) + 2 ** parts.bits) % 2 ** parts.bits;
    for (let i = 0; i < parts.size; i++) {
      buf[off + i] = Math.floor(raw / 2 ** (8 * i)) & 255;
    }
    return;
  }
  let raw = BigInt.asUintN(parts.bits, BigInt(value));
  for (let i = 0; i < parts.size; i++) {
    buf[off + i] = Number((raw >> BigInt(8 * i)) & BigInt("255"));
  }
};

globalThis.__rt._scalar_view = (p, size, kind) => {
  const parts = globalThis.__rt._scalar_parts(kind);
  if (parts === null || parts.size !== size || p.off % size !== 0) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: byte offset " + p.off + " cannot form a " + kind + " scalar view",
    ));
  }
  if (p.buf === null || typeof p.buf !== "object" || typeof p.buf.length !== "number") {
    throw __rt.abort_error(new Error("rustc_codegen_js: a scalar byte view requires contiguous byte storage"));
  }
  let views = globalThis.__rt._scalar_views.get(p.buf);
  if (views === undefined) {
    views = new Map();
    globalThis.__rt._scalar_views.set(p.buf, views);
  }
  let view = views.get(kind);
  if (view === undefined) {
    view = [];
    views.set(kind, view);
    globalThis.__rt._scalar_view_of.set(view, { buf: p.buf, size, kind });
  }
  // Reallocation preserves the byte buffer's identity and can add new words.
  for (let i = view.length; i < Math.floor(p.buf.length / size); i++) {
    Object.defineProperty(view, i, {
      get: () => globalThis.__rt._scalar_load(p.buf, i * size, parts),
      set: (value) => globalThis.__rt._scalar_store(p.buf, i * size, parts, value),
      enumerable: true,
      configurable: false,
    });
  }
  return { buf: view, off: p.off / size };
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
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer is not a byte view of " + size +
        "-byte elements, so it cannot be cast back to one",
    ));
  }
  if (p.sc !== undefined) {
    if (p.sc !== size) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: this pointer is a byte view of " + p.sc +
          "-byte elements, not of " + size + "-byte ones",
      ));
    }
    if (p.off % size !== 0) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: byte offset " + p.off + " is not a multiple of " +
          size + ", so it does not name an element",
      ));
    }
    if (p.pv !== undefined) {
      if (p.off !== p.po) {
        throw __rt.abort_error(new Error(
          "rustc_codegen_js: a byte offset moved outside its zero-offset aggregate field",
        ));
      }
      return p.pv;
    }
    return { buf: p.buf, off: p.off / size };
  }
  const block = globalThis.__rt._heap_block(p.buf);
  if (block !== undefined) return globalThis.__rt._retype(p, block, size, mk);
  throw __rt.abort_error(new Error(
    "rustc_codegen_js: this pointer is not a byte view of " + size +
      "-byte elements, so it cannot be cast back to one",
  ));
};

// The scalar-only extension of `unscale`. Keeping this as a separate runtime member means an
// ordinary allocation cast does not pull the byte-decoding/accessor implementation into a
// minimal shim. Fresh heap blocks retain allocation retyping; initialized byte storage gets
// an aliasing view, including when its capacity is not a multiple of the word size.
globalThis.__rt.unscale_scalar = (p, size, mk, kind) => {
  if (p !== null && typeof p === "object" && p.sc === undefined) {
    const block = globalThis.__rt._heap_block(p.buf);
    if (block === undefined || (block.es === 1 && !block.fresh)) {
      return globalThis.__rt._scalar_view(p, size, kind);
    }
  }
  return globalThis.__rt.unscale(p, size, mk);
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
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer does not name an array"));
  }
  // A bare array is the *reference* form of `&[E; N]` being reinterpreted as a thin
  // pointer (`Arguments::new`'s `&[u8; N] -> NonNull<u8>` transmute): the pointer to
  // its first element is the array itself with index zero.
  if (Array.isArray(p)) return { buf: p, off: 0 };
  if (p.w === undefined) return { buf: p.buf[p.off], off: 0 };
  return { buf: p.buf, off: p.off };
};

// The `str` hybrid. A `&str` is a JavaScript string, and its byte view is
// produced on demand by the three helpers below. The wrappers construct their
// native codec on first use, so importing a package never needs browser codecs.
globalThis.__rt._encoder = {
  value: null,
  encode(value) {
    return (this.value || (this.value = new TextEncoder())).encode(value);
  },
};
globalThis.__rt._decoder = {
  value: null,
  decode(value) {
    return (this.value || (this.value = new TextDecoder("utf-8", { ignoreBOM: true }))).decode(value);
  },
};

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
// Native String and Vec primitives
// ---------------------------------------------------------------------------
//
// Whole-program representation analysis may prove that an ordinary Rust
// `String` or `Vec<T>` can stay in a native host representation for its whole
// lifetime. These helpers implement only the operations JavaScript cannot
// spell inline without duplicating exact UTF-8 or slot-clearing logic. Rust
// still computes capacity growth and runs element drops before shrinking.

globalThis.__rt._utf8_width = (point) => {
  if (point <= 0x7f) return 1;
  if (point <= 0x7ff) return 2;
  if (point <= 0xffff) return 3;
  return 4;
};

// Maps a UTF-8 byte boundary to the corresponding JavaScript UTF-16 index.
// A byte inside a code point, a negative/non-integral index, or an index past
// the end has no equivalent and returns -1.
globalThis.__rt.native_str_utf16_index = (text, byteIndex) => {
  if (!Number.isInteger(byteIndex) || byteIndex < 0) return -1;
  let bytes = 0;
  for (let utf16 = 0; utf16 < text.length;) {
    if (bytes === byteIndex) return utf16;
    const point = text.codePointAt(utf16);
    const next = bytes + globalThis.__rt._utf8_width(point);
    if (byteIndex < next) return -1;
    bytes = next;
    utf16 += point > 0xffff ? 2 : 1;
  }
  return bytes === byteIndex ? text.length : -1;
};

// The inverse map. An index between the two code units of a surrogate pair is
// not a Rust char boundary and returns -1.
globalThis.__rt.native_str_byte_index = (text, utf16Index) => {
  if (!Number.isInteger(utf16Index) || utf16Index < 0) return -1;
  let bytes = 0;
  for (let utf16 = 0; utf16 < text.length;) {
    if (utf16 === utf16Index) return bytes;
    const point = text.codePointAt(utf16);
    const units = point > 0xffff ? 2 : 1;
    if (utf16Index < utf16 + units) return -1;
    bytes += globalThis.__rt._utf8_width(point);
    utf16 += units;
  }
  return utf16Index === text.length ? bytes : -1;
};

globalThis.__rt.native_str_is_char_boundary = (text, byteIndex) =>
  globalThis.__rt.native_str_utf16_index(text, byteIndex) !== -1;

// Rust's default f64 Display uses the shortest round-tripping decimal in fixed
// notation. JavaScript supplies the same shortest digits, but uses exponent
// notation at its own thresholds and resolves an exact decimal midpoint toward
// an even significand. Rust's Dragon fallback resolves that midpoint upward.
// The BigInt check below changes only that one ambiguous case before rendering
// the common shortest digits in fixed notation.
globalThis.__rt.native_f64_to_string = (value) => {
  let text;
  if (Number.isNaN(value)) text = "NaN";
  else if (value === Infinity) text = "inf";
  else if (value === -Infinity) text = "-inf";
  else if (Object.is(value, -0)) text = "-0";
  else {
    const negative = value < 0;
    const magnitude = negative ? -value : value;
    const rendered = String(magnitude).toLowerCase();
    const exponentAt = rendered.indexOf("e");
    const mantissa = exponentAt < 0 ? rendered : rendered.slice(0, exponentAt);
    const exponent = exponentAt < 0 ? 0 : Number(rendered.slice(exponentAt + 1));
    const point = mantissa.indexOf(".");
    const fractionDigits = point < 0 ? 0 : mantissa.length - point - 1;
    let digits = point < 0 ? mantissa : mantissa.slice(0, point) + mantissa.slice(point + 1);
    let power = exponent - fractionDigits;
    while (digits.length > 1 && digits.endsWith("0")) {
      digits = digits.slice(0, -1);
      power += 1;
    }

    let significand = BigInt(digits);
    if ((significand & BigInt("1")) === BigInt("0") && magnitude !== 0) {
      const bits = new DataView(new ArrayBuffer(8));
      bits.setFloat64(0, magnitude, false);
      const high = bits.getUint32(0, false);
      const low = bits.getUint32(4, false);
      const exponentBits = (high >>> 20) & 0x7ff;
      const fraction = (BigInt(high & 0xfffff) << BigInt("32")) | BigInt(low);
      let binaryNumerator;
      let binaryDenominator;
      if (exponentBits === 0) {
        binaryNumerator = fraction;
        binaryDenominator = BigInt("1") << BigInt("1074");
      } else {
        const mantissaBits = (BigInt("1") << BigInt("52")) | fraction;
        const binaryPower = exponentBits - 1075;
        if (binaryPower >= 0) {
          binaryNumerator = mantissaBits << BigInt(binaryPower);
          binaryDenominator = BigInt("1");
        } else {
          binaryNumerator = mantissaBits;
          binaryDenominator = BigInt("1") << BigInt(-binaryPower);
        }
      }

      const decimalScale = BigInt("10") ** BigInt(Math.abs(power));
      const decimalNumerator = power >= 0 ? significand * decimalScale : significand;
      const decimalDenominator = power >= 0 ? BigInt("1") : decimalScale;
      const midpointNumerator = power >= 0
        ? (BigInt("2") * significand + BigInt("1")) * decimalScale
        : BigInt("2") * significand + BigInt("1");
      const midpointDenominator = power >= 0 ? BigInt("2") : BigInt("2") * decimalScale;
      const exactMidpoint =
        binaryNumerator * midpointDenominator ===
        midpointNumerator * binaryDenominator;
      const belowValue =
        decimalNumerator * binaryDenominator <
        binaryNumerator * decimalDenominator;
      if (exactMidpoint && belowValue) significand += BigInt("1");
    }

    digits = significand.toString();
    const decimalAt = digits.length + power;
    if (decimalAt <= 0) text = "0." + "0".repeat(-decimalAt) + digits;
    else if (decimalAt >= digits.length) text = digits + "0".repeat(decimalAt - digits.length);
    else text = digits.slice(0, decimalAt) + "." + digits.slice(decimalAt);
    if (negative) text = "-" + text;
  }
  return { text: text, cap: text.length };
};

// A mutable native String is `{ text, cap }`, where `cap` is measured in UTF-8
// bytes. Its reserve path has already established enough capacity, so pushing
// a Rust `char` changes only the text.
globalThis.__rt.native_str_push = (value, point) => {
  value.text += String.fromCodePoint(point);
};

// Returns -1 for `None`; every Rust `char` is otherwise a non-negative Unicode
// scalar value. Capacity is retained, as it is by `String::pop`.
globalThis.__rt.native_str_pop = (value) => {
  const text = value.text;
  if (text.length === 0) return -1;
  let at = text.length - 1;
  const tail = text.charCodeAt(at);
  if (tail >= 0xdc00 && tail <= 0xdfff && at > 0) {
    const lead = text.charCodeAt(at - 1);
    if (lead >= 0xd800 && lead <= 0xdbff) at -= 1;
  }
  const point = text.codePointAt(at);
  value.text = text.slice(0, at);
  return point;
};

// Rust makes an index at or past the current byte length a no-op and rejects an
// in-range index that is not a char boundary. Capacity is retained.
globalThis.__rt.native_str_truncate = (value, byteIndex) => {
  const text = value.text;
  const utf16 = globalThis.__rt.native_str_utf16_index(text, byteIndex);
  if (utf16 >= 0) {
    if (utf16 < text.length) value.text = text.slice(0, utf16);
    return;
  }
  if (byteIndex > globalThis.__rt.str_len(text)) return;
  throw __rt.abort_error(new RangeError("native String truncate index is not a UTF-8 boundary"));
};

globalThis.__rt.native_vec_new = (capacity) => ({
  buf: new Array(capacity),
  len: 0,
  cap: capacity,
});

// Phase 12D's exact numeric backing store has already allocated the proven maximum length.
// Growth therefore changes only Rust's logical capacity before writing the next POD element.
globalThis.__rt.typed_vec_push = (value, element) => {
  const length = value.len;
  if (length === value.cap) value.cap = length + 1;
  value.buf[length] = element;
  value.len = length + 1;
};

// The compiled Rust reserve/shrink path computes `capacity`; this helper only
// applies it without changing buffer identity. Reducing `length` releases all
// unused slots beyond the new capacity.
globalThis.__rt.native_vec_set_capacity = (value, capacity) => {
  if (capacity < value.len) {
    throw __rt.abort_error(new RangeError("native Vec capacity cannot be smaller than its length"));
  }
  value.buf.length = capacity;
  value.cap = capacity;
};

// Drops run in compiled Rust before this call. Filling the removed range with
// `undefined` releases its host references while retaining the allocation and
// its explicit capacity.
globalThis.__rt.native_vec_set_len = (value, length) => {
  if (length > value.cap) {
    throw __rt.abort_error(new RangeError("native Vec length cannot exceed its capacity"));
  }
  if (length < value.len) value.buf.fill(undefined, length, value.len);
  value.len = length;
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
// Stack arrays reached through `MaybeUninit<u8>` need the same one-time scale
// bookkeeping without becoming heap blocks. In particular, `_put` must not
// attach heap ownership metadata to values stored in sort scratch space.
globalThis.__rt._uninit_blocks = new WeakMap();

// _heap_block(buf): the block record for a buffer, or undefined if it is not
// one. A freed block is a refusal rather than a miss: the mistake is worth
// naming.
globalThis.__rt._heap_block = (buf) => {
  if (globalThis.__rt._freed.has(buf)) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer names a heap block that has been freed",
    ));
  }
  return globalThis.__rt._blocks.get(buf);
};

// _retype(p, block, size, mk): the heap arm of `unscale`.
//
// Asking for the size the block already holds is the common case and is free --
// `RawVecInner` stores its buffer as a `*mut u8` and casts it back on every
// single access, so this runs once per element read. Asking for a *different*
// size is the retype, and it is allowed only while the block is fresh, only
// from an offset that lands on an element. Uninitialized trailing bytes remain
// outside the complete-element view; the allocation retains its full byte size.
// Zeroed partial elements stay unsupported because their byte values need storage.
globalThis.__rt._retype = (p, block, size, mk) => {
  const same = block.es === size;
  // A block already read at this element size, with nothing left to do to it.
  // The common case by far: `RawVecInner` stores its buffer as a `*mut u8` and
  // casts it back on every single access.
  if (same && !(block.fresh && block.zeroed)) return p;
  if (!same) {
    if (!block.fresh) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: this heap block already holds " + block.es +
          "-byte elements, so it cannot be read as " + size + "-byte ones",
      ));
    }
    if (block.zeroed && block.bytes % size !== 0) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a heap block of " + block.bytes +
          " bytes is not a whole number of " + size + "-byte elements",
      ));
    }
  }
  const byte_off = p.off * block.es;
  if (byte_off % size !== 0) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: byte offset " + byte_off + " is not a multiple of " +
        size + ", so it does not name an element",
    ));
  }
  const length = Math.floor(block.bytes / size);
  if (!same) {
    p.buf.length = length;
    block.es = size;
    // `Box::new_uninit` allocates one aggregate and initializes its fields
    // through a raw pointer before the whole value exists. Give that one-slot
    // allocation an object shell; ordinary multi-element Vec capacity remains
    // sparse until elements are written.
    if (!block.zeroed && block.bytes === size && typeof mk === "function") {
      const shell = mk();
      if (shell !== null && typeof shell === "object") {
        globalThis.__rt._put(p.buf, 0, shell);
      }
    }
  }
  block.fresh = false;
  // A zeroed block holds zero *bytes*, and a zero element is not always the
  // number 0: it is `false` for a `bool`, `{ TAG: "None" }` for a niched
  // `None`, `{ x: 0, y: 0 }` for a struct. Only the compiler knows which, so the cast
  // hands over a factory. This runs even when the element size did not change,
  // which is what `vec![false; n]` needs -- a `bool` is one byte too.
  if (block.zeroed) {
    if (mk === null || mk === undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a zeroed heap block cannot be read as " + size +
          "-byte elements, because this backend cannot spell a zero one",
      ));
    }
    for (let i = 0; i < length; i++) p.buf[i] = mk();
  }
  return same ? p : { buf: p.buf, off: byte_off / size };
};

// unscale_uninit(p, size, mk): the `MaybeUninit<u8>` storage form of
// `unscale`. Rust also uses aligned stack byte arrays as scratch storage and
// casts them to `MaybeUninit<T>`. The union at the source proves that no value
// is observed through the old element shape, so the array may acquire an
// element scale exactly once.
globalThis.__rt.unscale_uninit = (p, size, mk) => {
  if (typeof p === "number" || p === null || typeof p !== "object" || p.sc !== undefined) {
    return globalThis.__rt.unscale(p, size, mk);
  }
  let block = globalThis.__rt._heap_block(p.buf);
  if (block === undefined) {
    if (!Array.isArray(p.buf)) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: uninitialized storage does not have an array backing buffer",
      ));
    }
    block = globalThis.__rt._uninit_blocks.get(p.buf);
    if (block === undefined) {
      block = {
        bytes: p.buf.length,
        es: 1,
        fresh: true,
        zeroed: false,
      };
      globalThis.__rt._uninit_blocks.set(p.buf, block);
    }
  }
  return globalThis.__rt._retype(p, block, size, mk);
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
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: a pointer with no buffer cannot be reshaped into a " +
        "header plus an unsized tail",
    ));
  }
  const block = globalThis.__rt._heap_block(buf);
  if (block === undefined) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer does not name a live heap block, so it " +
        "cannot be reshaped into a header plus an unsized tail",
    ));
  }
  // Already reshaped. `RcInner` is cast back out of its `*mut u8` more than
  // once on the way through `Rc::from_raw`-shaped code, and reshaping a block
  // twice would throw its header away.
  if (block.hdr !== undefined) {
    if (block.es !== tail_es) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: this heap block holds a tail of " + block.es +
          "-byte elements, so it cannot be read as one of " + tail_es +
          "-byte ones",
      ));
    }
    return { ptr: { buf, off: 0 }, meta: len };
  }
  if (header_mk === null || header_mk === undefined) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this backend cannot spell the header of a struct with " +
        "an unsized tail, so its block cannot be reshaped",
    ));
  }
  for (let i = 0; i < buf.length; i++) {
    if (buf[i] !== undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: this heap block has already been written, so it " +
          "cannot be reshaped into a header plus an unsized tail",
      ));
    }
  }
  buf.length = 1 + len;
  buf[0] = header_mk();
  block.es = tail_es;
  block.hdr = 1;
  block.fresh = false;
  return { ptr: { buf, off: 0 }, meta: len };
};

// Unsized structs retain their metadata while crossing the raw/reference ABI.
// Remember the exact slot so a reborrow returns to the same allocation.
globalThis.__rt.tail_ref = (p) => {
  const record = p.ptr.buf[p.ptr.off];
  globalThis.__rt._boxes.set(record, p.ptr);
  return { ptr: record, meta: p.meta };
};
globalThis.__rt.tail_raw = (p) => ({
  ptr: globalThis.__rt.box(p.ptr), meta: p.meta,
});

// A sized-to-unsized coercion leaves the tail in a record field. An allocated
// unsized struct keeps it after the header. Both return the original storage.
globalThis.__rt.tail_slice = (record, key, len) => {
  const slot = globalThis.__rt._boxes.get(record);
  const block = slot === undefined ? undefined : globalThis.__rt._heap_block(slot.buf);
  if (block !== undefined && block.hdr !== undefined) {
    return { buf: slot.buf, off: slot.off + block.hdr, len };
  }
  return { buf: record[key], off: 0, len };
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
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer does not name a live heap block, so it " +
        "cannot be deallocated",
    ));
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
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer does not name a live heap block, so it " +
        "cannot be reallocated",
    ));
  }
  if (new_size % block.es !== 0) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: a heap block of " + new_size +
        " bytes is not a whole number of " + block.es + "-byte elements",
    ));
  }
  block.bytes = new_size;
  buf.length = new_size / block.es;
  return { buf, off: p.off };
};

// alloc_error(size, align): `__rust_alloc_error_handler`. Diverging, and there
// is nothing better to do: a JavaScript array does not run out before the
// engine does.
globalThis.__rt.alloc_error = (size, align) => {
  throw __rt.abort_error(new Error(
    "rustc_codegen_js: memory allocation of " + size + " bytes failed",
  ));
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
// Transactional keyed lists (K1-K5)
// ---------------------------------------------------------------------------
//
// The legacy eager cache below remains byte-for-byte in charge of existing
// `push_keyed` output. These helpers are the separate lane for the replacement:
// one instance-scoped state, a validate-before-create transaction, and one owner
// per row. The versioned keyed-list marker selects this lane in JIR output.

globalThis.__rt._keyed_metrics = {
  collectionScans: 0,
  createdRows: 0,
  disposals: 0,
  domMoves: 0,
  effects: 0,
  handlers: 0,
  hydrationFallbacks: 0,
  hydratedRows: 0,
  liveRowRecords: 0,
  owners: 0,
  removedRows: 0,
  rangeReplacements: 0,
  retainedRows: 0,
  rowFactories: 0,
  selectionEffects: 0,
  transactionStates: 0,
};

// keyed_metric(name, amount): the benchmark-only hook for operations whose
// creation happens in the DOM/reactive runtime rather than in this shim.
globalThis.__rt.keyed_metric = (name, amount = 1) => {
  const metrics = globalThis.__rt._keyed_metrics;
  if (!Object.prototype.hasOwnProperty.call(metrics, name)) {
    throw new Error(`unknown keyed metric: ${name}`);
  }
  if (!Number.isSafeInteger(amount) || amount < 0) {
    throw new Error(`invalid keyed metric increment: ${amount}`);
  }
  metrics[name] += amount;
};

globalThis.__rt.keyed_metrics_snapshot = () => ({
  ...globalThis.__rt._keyed_metrics,
});

globalThis.__rt._keyed_states = new Set();
globalThis.__rt._keyed_dom_values = new WeakMap();

// Reset event counters without lying about rows that are already live.
globalThis.__rt.keyed_metrics_reset = () => {
  const metrics = globalThis.__rt._keyed_metrics;
  for (const name of Object.keys(metrics)) metrics[name] = 0;
  if (globalThis.__rt._keyed instanceof Map) {
    for (const rows of globalThis.__rt._keyed.values()) {
      metrics.liveRowRecords += rows.size;
    }
  }
  for (const state of globalThis.__rt._keyed_states) {
    if (!state.disposed) {
      metrics.liveRowRecords += state.rows.size;
      metrics.transactionStates += 1;
    }
  }
};

// keyed_state(hooks) -> one list-instance owner. Optional hooks are the host
// reactive runtime's batch, createRoot, createSignal, and onCleanup equivalents.
globalThis.__rt.keyed_state = (hooks) => {
  if (hooks !== undefined && (hooks === null || typeof hooks !== "object")) {
    throw new TypeError("keyed runtime hooks must be an object");
  }
  for (const name of ["batch", "cleanup", "root", "signal", "untrack"]) {
    if (hooks && hooks[name] !== undefined && typeof hooks[name] !== "function") {
      throw new TypeError(`keyed runtime hook ${name} must be a function`);
    }
  }
  const state = {
    batch: hooks && hooks.batch,
    disposePending: false,
    disposed: false,
    hooks,
    keyType: null,
    order: [],
    pending: null,
    rows: new Map(),
    running: false,
  };
  globalThis.__rt._keyed_states.add(state);
  globalThis.__rt.keyed_metric("transactionStates");
  if (hooks && hooks.cleanup !== undefined) {
    try {
      hooks.cleanup(() => {
        if (state.disposed) return;
        if (state.dom) globalThis.__rt.keyed_dom_dispose(state);
        else globalThis.__rt.keyed_dispose(state);
      });
    } catch (error) {
      if (globalThis.__rt._keyed_states.delete(state)) {
        globalThis.__rt._keyed_metrics.transactionStates -= 1;
      }
      throw error;
    }
  }
  return state;
};

const keyedSlot = (value, hooks) => {
  if (!hooks || hooks.signal === undefined) return { value };
  const signal = hooks.signal(value);
  if (!Array.isArray(signal) || typeof signal[0] !== "function" || typeof signal[1] !== "function") {
    throw new TypeError("a keyed signal hook must return a getter/setter pair");
  }
  return {
    get value() {
      return signal[0]();
    },
    set value(next) {
      signal[1](next);
    },
  };
};

// A stable callable proxy over a row signal. Calling it returns the live value,
// property reads forward to that value, and primitive coercion reads it too.
// Reference-typed Rust row arguments therefore keep their ordinary emitted
// field/deref spelling, while nested effects and handlers retain this proxy
// instead of the first item's value.
const keyedLive = (slot) => {
  const read = () => slot.value;
  const primitive = (hint) => {
    const value = read();
    if (value === null || (typeof value !== "object" && typeof value !== "function")) {
      return value;
    }
    const convert = value[Symbol.toPrimitive];
    if (typeof convert === "function") return convert.call(value, hint);
    const converted = hint === "string" ? String(value) : value.valueOf();
    return converted !== value ? converted : String(value);
  };
  return new Proxy(read, {
    get(target, property, receiver) {
      if (property === Symbol.toPrimitive) return primitive;
      if (property === "valueOf") return () => primitive("number");
      if (property === "toString") return () => String(primitive("string"));
      const value = read();
      if (value !== null && value !== undefined && property in Object(value)) {
        const member = value[property];
        return typeof member === "function" ? member.bind(value) : member;
      }
      return Reflect.get(target, property, receiver);
    },
  });
};

// keyed_owner(factory, item, index, key) -> a row record with live item/index
// slots and an exact, idempotent disposal boundary. `effect` and `handler` are
// instrumentation hooks: the DOM integration registers their actual cleanup
// functions here when it creates them.
globalThis.__rt.keyed_owner = (factory, item, index, key, hooks) => {
  const cleanups = [];
  const itemSlot = keyedSlot(item, hooks);
  const indexSlot = keyedSlot(index, hooks);
  let disposed = false;
  const owner = {
    effect: (cleanup) => {
      globalThis.__rt.keyed_metric("effects");
      if (typeof cleanup === "function") cleanups.push(cleanup);
    },
    get index() {
      return indexSlot.value;
    },
    get item() {
      return itemSlot.value;
    },
    liveIndex: keyedLive(indexSlot),
    liveItem: keyedLive(itemSlot),
    handler: (cleanup) => {
      globalThis.__rt.keyed_metric("handlers");
      if (typeof cleanup === "function") cleanups.push(cleanup);
    },
    key,
    onCleanup: (cleanup) => {
      if (typeof cleanup !== "function") {
        throw new TypeError("a keyed row cleanup must be a function");
      }
      cleanups.push(cleanup);
    },
  };
  globalThis.__rt.keyed_metric("owners");

  const dispose = () => {
    if (disposed) return;
    disposed = true;
    globalThis.__rt.keyed_metric("disposals");
    let firstError = null;
    for (let at = cleanups.length - 1; at >= 0; --at) {
      try {
        cleanups[at]();
      } catch (error) {
        if (firstError === null) firstError = { error };
      }
    }
    if (firstError !== null) throw firstError.error;
  };

  try {
    const build = () => factory(owner);
    const run = hooks && hooks.untrack !== undefined
      ? () => hooks.untrack(build)
      : build;
    let value;
    if (hooks && hooks.root !== undefined) {
      value = hooks.root(rootDispose => {
        if (typeof rootDispose !== "function") {
          throw new TypeError("a keyed root hook must provide a disposer");
        }
        cleanups.push(rootDispose);
        return run();
      });
    } else {
      value = run();
    }
    return {
      dispose,
      index: indexSlot,
      item: itemSlot,
      key,
      liveIndex: owner.liveIndex,
      liveItem: owner.liveItem,
      value,
    };
  } catch (error) {
    try {
      dispose();
    } catch (_error) {
      // The factory error is the operation that failed; cleanup still ran.
    }
    throw error;
  }
};

globalThis.__rt.keyed_key_type = (key) => {
  const type = typeof key;
  if (type === "number") {
    if (!Number.isSafeInteger(key)) {
      throw new TypeError("a keyed-list number key must be a safe integer");
    }
    return type;
  }
  if (type === "string" || type === "bigint" || type === "boolean") return type;
  throw new TypeError(`unsupported keyed-list key type: ${type}`);
};

// One materialization for the initial array/slice source lane. A Rust slice is
// the backend's `{ buf, off, len }` view; native arrays and iterables use the
// ordinary Array.from path.
globalThis.__rt.keyed_values = (values) => {
  if (values !== null && typeof values === "object"
      && "buf" in values && "off" in values && "len" in values) {
    const { buf, off, len } = values;
    if (!buf || !Number.isSafeInteger(buf.length) || !Number.isSafeInteger(off)
        || !Number.isSafeInteger(len) || off < 0 || len < 0 || off + len > buf.length) {
      throw new TypeError("invalid keyed-list slice source");
    }
    return Array.prototype.slice.call(buf, off, off + len);
  }
  return Array.from(values);
};

// The shared array/slice lane implements Rust's `IntoIterator for &[T]`: each
// item is a reference to one source slot, not the value copied out of that slot.
globalThis.__rt.keyed_references = (values) => {
  let buf;
  let off;
  let len;
  if (Array.isArray(values)) {
    buf = values;
    off = 0;
    len = values.length;
  } else if (values !== null && typeof values === "object"
      && "buf" in values && "off" in values && "len" in values) {
    ({ buf, off, len } = values);
  }
  if (!buf || !Number.isSafeInteger(buf.length) || !Number.isSafeInteger(off)
      || !Number.isSafeInteger(len) || off < 0 || len < 0 || off + len > buf.length) {
    throw new TypeError("invalid keyed-list borrowed source");
  }
  return Array.from({ length: len }, (_unused, index) => ({ buf, off: off + index }));
};

// One compiler-known keyed selection. `slotOf` is shared by every row and
// returns the row's boolean signal slot, so changing selection touches only the
// previous and current keys and creates no per-row selection closures.
globalThis.__rt.keyed_selector = (state, key, slotOf) => {
  if (!state || !(state.rows instanceof Map) || typeof slotOf !== "function") {
    throw new TypeError("a keyed selector needs a list state and row slot accessor");
  }
  return {
    disposed: false,
    is: candidate => candidate === key,
    get key() {
      return key;
    },
    select: next => {
      if (state.disposed) throw new Error("cannot select in a disposed keyed list");
      if (next !== null) {
        const type = globalThis.__rt.keyed_key_type(next);
        if (state.keyType !== null && type !== state.keyType) {
          throw new TypeError(`keyed selection type changed from ${state.keyType} to ${type}`);
        }
      }
      if (next === key) return;
      const before = key;
      const previousRow = state.rows.get(before);
      const nextRow = state.rows.get(next);
      const changes = [];
      for (const [row, value] of [[previousRow, false], [nextRow, true]]) {
        if (row === undefined) continue;
        const slot = slotOf(row);
        if (!slot || typeof slot !== "object" || !("value" in slot)) {
          throw new TypeError("a keyed selector row has no boolean slot");
        }
        if (slot.value !== value) changes.push([slot, slot.value, value]);
      }
      let ran = false;
      const publish = () => {
        if (ran) throw new Error("a keyed batch ran selection more than once");
        ran = true;
        for (const [slot, _old, value] of changes) slot.value = value;
        key = next;
      };
      try {
        if (state.batch === undefined) publish();
        else state.batch(publish);
        if (!ran) throw new Error("a keyed batch did not run selection synchronously");
      } catch (error) {
        key = before;
        for (let at = changes.length - 1; at >= 0; --at) {
          try {
            changes[at][0].value = changes[at][1];
          } catch (_error) {
            // Preserve the selection error after attempting every slot rollback.
          }
        }
        throw error;
      }
      globalThis.__rt.keyed_metric("selectionEffects", changes.length);
    },
  };
};

globalThis.__rt.keyed_dispose_row = (row) => {
  if (typeof row.dispose === "function") row.dispose();
};

// A static keyed row persists as one Node or a nonempty Node array. DocumentFragment
// is deliberately not a carrier: inserting it would empty the value that owns identity.
globalThis.__rt.keyed_dom_owner = (factory, item, index, key, hooks) => {
  const row = globalThis.__rt.keyed_owner(factory, item, index, key, hooks);
  let nodes;
  const seen = new Set();
  try {
    nodes = Array.isArray(row.value) ? row.value.slice() : [row.value];
    if (nodes.length === 0) throw new TypeError("a keyed DOM row must contain at least one node");
    for (const node of nodes) {
      if (!node || !Number.isSafeInteger(node.nodeType) || node.nodeType === 11 || seen.has(node)) {
        throw new TypeError("a keyed DOM row must contain distinct DOM nodes");
      }
      seen.add(node);
    }
  } catch (error) {
    try {
      row.dispose();
    } catch (_error) {
      // Preserve the invalid-carrier error after attempting owner cleanup.
    }
    throw error;
  }
  const disposeOwner = row.dispose;
  row.nodes = nodes;
  row.dispose = () => {
    let firstError = null;
    for (const node of nodes) {
      try {
        if (node.parentNode !== null) node.parentNode.removeChild(node);
      } catch (error) {
        if (firstError === null) firstError = { error };
      }
    }
    try {
      disposeOwner();
    } catch (error) {
      if (firstError === null) firstError = { error };
    }
    if (firstError !== null) throw firstError.error;
  };
  return row;
};

// One DOM list instance and its permanent end boundary.
globalThis.__rt.keyed_dom_state = (parent, end, hooks) => {
  if (!parent || typeof parent.insertBefore !== "function" || typeof parent.removeChild !== "function") {
    throw new TypeError("a keyed DOM list needs a parent node");
  }
  if (end === undefined) {
    const document = parent.ownerDocument;
    if (!document || typeof document.createComment !== "function") {
      throw new TypeError("a keyed DOM list needs an explicit end boundary");
    }
    end = document.createComment("");
  }
  if (!end || !Number.isSafeInteger(end.nodeType)) {
    throw new TypeError("a keyed DOM list end boundary must be a node");
  }
  if (end.parentNode === null) parent.insertBefore(end, null);
  if (end.parentNode !== parent) {
    throw new Error("a keyed DOM list end boundary belongs to another parent");
  }
  const state = globalThis.__rt.keyed_state(hooks);
  state.dom = { end, hydrated: false, ownsEnd: true, parent };
  return state;
};

// Adopt a server-rendered row range after ordinary template hydration has
// resolved every row root through `data-hk`. The closing Solid marker is a
// boundary the surrounding view owns, so the keyed list borrows rather than
// removes it. An explicit `tc:k` marker is accepted for server emitters that do
// not otherwise need a dynamic-child marker pair.
const keyedDomAdopt = (state, nodes) => {
  if (nodes.length === 0 || nodes.every(node => node.parentNode === null)) return false;
  const parent = nodes[0].parentNode;
  if (parent === null || nodes.some(node => node.parentNode !== parent)) {
    return false;
  }
  for (let at = 1; at < nodes.length; ++at) {
    if (nodes[at - 1].nextSibling !== nodes[at]) {
      return false;
    }
  }
  const end = nodes[nodes.length - 1].nextSibling;
  const marker = end && (end.data === null || end.data === undefined ? end.nodeValue : end.data);
  if (!end || end.nodeType !== 8 || (marker !== "/" && marker !== "tc:k")) {
    return false;
  }
  state.dom = { end, hydrated: true, ownsEnd: false, parent };
  globalThis.__rt.keyed_metric("hydratedRows", state.order.length);
  return true;
};

const keyedDomContains = (row, target) => {
  if (target === null || target === undefined) return false;
  return row.nodes.some(node => node === target
    || (typeof node.contains === "function" && node.contains(target)));
};

const keyedDomContiguousBefore = (row, parent, reference) => {
  const nodes = row.nodes;
  if (nodes[nodes.length - 1].nextSibling !== reference) return false;
  for (let at = 0; at < nodes.length; ++at) {
    if (nodes[at].parentNode !== parent) return false;
    if (at + 1 < nodes.length && nodes[at].nextSibling !== nodes[at + 1]) return false;
  }
  return true;
};

const keyedDomMoveRow = (parent, row, reference, atomic) => {
  if (keyedDomContiguousBefore(row, parent, reference)) return false;
  for (const node of row.nodes) {
    if (atomic && node.parentNode === parent) parent.moveBefore(node, reference);
    else parent.insertBefore(node, reference);
  }
  return true;
};

// The no-focus path is the same two-ended reconciliation used by the existing
// DOM runtime. Running it over the row nodes makes a distant two-row swap two
// moves instead of walking every row between them backwards.
const keyedDomOrderNodes = (parent, end, before, next, atomic) => {
  let nextLength = next.length;
  let beforeEnd = before.length;
  let nextEnd = nextLength;
  let beforeStart = 0;
  let nextStart = 0;
  let map = null;
  let moves = 0;
  const move = (node, reference) => {
    if (atomic && node.parentNode === parent) parent.moveBefore(node, reference);
    else parent.insertBefore(node, reference);
    moves += 1;
  };
  const remove = node => {
    if (node.parentNode === parent) parent.removeChild(node);
  };
  const replace = (node, replaced) => {
    if (typeof parent.replaceChild === "function") parent.replaceChild(node, replaced);
    else {
      parent.insertBefore(node, replaced);
      parent.removeChild(replaced);
    }
    moves += 1;
  };

  while (beforeStart < beforeEnd || nextStart < nextEnd) {
    if (before[beforeStart] === next[nextStart]) {
      beforeStart += 1;
      nextStart += 1;
      continue;
    }
    while (before[beforeEnd - 1] === next[nextEnd - 1]) {
      beforeEnd -= 1;
      nextEnd -= 1;
    }
    if (beforeEnd === beforeStart) {
      const reference = nextEnd < nextLength
        ? (nextStart ? next[nextStart - 1].nextSibling : next[nextEnd - nextStart])
        : end;
      while (nextStart < nextEnd) move(next[nextStart++], reference);
    } else if (nextEnd === nextStart) {
      while (beforeStart < beforeEnd) {
        if (!map || !map.has(before[beforeStart])) remove(before[beforeStart]);
        beforeStart += 1;
      }
    } else if (before[beforeStart] === next[nextEnd - 1]
        && next[nextStart] === before[beforeEnd - 1]) {
      const reference = before[--beforeEnd].nextSibling;
      move(next[nextStart++], before[beforeStart++].nextSibling);
      move(next[--nextEnd], reference);
      before[beforeEnd] = next[nextEnd];
    } else {
      if (!map) {
        map = new Map();
        let at = nextStart;
        while (at < nextEnd) map.set(next[at], at++);
      }
      const index = map.get(before[beforeStart]);
      if (index !== undefined) {
        if (nextStart < index && index < nextEnd) {
          let at = beforeStart;
          let sequence = 1;
          let mapped;
          while (++at < beforeEnd && at < nextEnd) {
            mapped = map.get(before[at]);
            if (mapped === undefined || mapped !== index + sequence) break;
            sequence += 1;
          }
          if (sequence > index - nextStart) {
            const reference = before[beforeStart];
            while (nextStart < index) move(next[nextStart++], reference);
          } else replace(next[nextStart++], before[beforeStart++]);
        } else beforeStart += 1;
      } else {
        remove(before[beforeStart++]);
      }
    }
  }
  return moves;
};

// Orders rows around the retained active row. The common insertBefore path never
// touches that row, so focus and selection survive without observable blur/focus.
const keyedDomOrder = (parent, end, rows) => {
  const atomic = typeof parent.moveBefore === "function";
  const document = parent.ownerDocument;
  const active = document ? document.activeElement : null;
  const anchor = atomic ? -1 : rows.findIndex(row => keyedDomContains(row, active));
  let moves = 0;
  if (anchor >= 0) {
    let reference = rows[anchor].nodes[0];
    for (let at = anchor - 1; at >= 0; --at) {
      if (keyedDomMoveRow(parent, rows[at], reference, false)) moves += 1;
      reference = rows[at].nodes[0];
    }
    reference = end;
    for (let at = rows.length - 1; at > anchor; --at) {
      if (keyedDomMoveRow(parent, rows[at], reference, false)) moves += 1;
      reference = rows[at].nodes[0];
    }
    return moves;
  }
  const next = rows.flatMap(row => row.nodes);
  const wanted = new Set(next);
  const children = parent.childNodes === undefined ? parent.children : parent.childNodes;
  const before = Array.from(children).filter(node => wanted.has(node));
  return keyedDomOrderNodes(parent, end, before, next, atomic);
};

// `insert` is the one place that knows the exact hydration range: the parent,
// its closing marker (or the end of a sole-child parent), and the server nodes
// between them. The DOM bundle calls this hook only while `hydrate` is running.
// A matching range is merely associated with the keyed state. A mismatch is
// reconciled inside that range before the ordinary insert observes it, so no
// sibling outside the list can be replaced.
globalThis.__rt.keyed_dom_insert = (value, parent, end, current) => {
  const state = globalThis.__rt._keyed_dom_values.get(value);
  if (state === undefined) return false;
  if (state.disposed) throw new Error("cannot hydrate a disposed keyed list");
  if (!parent || typeof parent.insertBefore !== "function"
      || typeof parent.removeChild !== "function") {
    throw new TypeError("a keyed hydration range needs a parent node");
  }

  let boundary = end === undefined ? null : end;
  const expected = state.order.flatMap(row => row.nodes);
  const expectedSet = new Set(expected);
  if (current !== undefined && !Array.isArray(current)) {
    throw new TypeError("a keyed hydration range must be an array of server nodes");
  }
  let server = current === undefined ? Array.from(parent.childNodes) : current.slice();
  let boundaryMismatch = false;

  // `getNextMarker` returns a null boundary and every following sibling when
  // the expected closing comment was damaged. Recover only when an unexpected
  // comment still provides an exact stop point; otherwise there is no safe way
  // to distinguish stale list rows from static siblings outside the list.
  if (boundary === null && current !== undefined) {
    const at = server.findIndex(node => node.nodeType === 8 && !expectedSet.has(node));
    if (at < 0) throw new Error("keyed hydration boundary is unavailable");
    boundary = server[at];
    server = server.slice(0, at);
    boundaryMismatch = true;
  }
  if (boundary !== null
      && (!Number.isSafeInteger(boundary.nodeType) || boundary.parentNode !== parent)) {
    throw new Error("keyed hydration boundary does not belong to its parent");
  }
  let reference = boundary;
  for (let at = server.length - 1; at >= 0; --at) {
    const node = server[at];
    if (!node || node.parentNode !== parent || node.nextSibling !== reference) {
      throw new Error("keyed hydration server range is not contiguous");
    }
    reference = node;
  }

  const serverSet = new Set(server);
  for (const node of expected) {
    if (node.parentNode !== null
        && (node.parentNode !== parent || !serverSet.has(node))) {
      throw new Error("a keyed hydration row escaped its declared list range");
    }
  }
  const matching = !boundaryMismatch && expected.length === server.length
    && expected.every((node, index) => node === server[index]);
  const wasHydrated = state.dom && state.dom.hydrated === true;
  state.dom = {
    end: boundary,
    hydrated: matching,
    ownsEnd: false,
    parent,
  };
  if (matching) {
    if (!wasHydrated) globalThis.__rt.keyed_metric("hydratedRows", state.order.length);
    return true;
  }

  if (boundaryMismatch && boundary.data !== "/") boundary.data = "/";
  for (const node of server) {
    if (!expectedSet.has(node) && node.parentNode === parent) parent.removeChild(node);
  }
  const moves = keyedDomOrder(parent, boundary, state.order);
  globalThis.__rt.keyed_metric("domMoves", moves);
  globalThis.__rt.keyed_metric("hydrationFallbacks");
  globalThis.__rt.keyed_metric("rangeReplacements");
  return true;
};

const keyedDomValidate = (state, rows) => {
  if (!state.dom || !state.dom.parent
      || (state.dom.end !== null && state.dom.end.parentNode !== state.dom.parent)) {
    throw new Error("invalid keyed DOM list boundary");
  }
  const parent = state.dom.parent;
  const seen = new Set();
  for (const row of rows) {
    if (!row || !Array.isArray(row.nodes) || row.nodes.length === 0) {
      throw new Error("invalid keyed DOM row range");
    }
    const attached = row.nodes[0].parentNode;
    for (let at = 0; at < row.nodes.length; ++at) {
      const node = row.nodes[at];
      if (seen.has(node) || (node.parentNode !== null && node.parentNode !== parent)) {
        throw new Error("invalid keyed DOM row range");
      }
      seen.add(node);
      if (node.parentNode !== attached) throw new Error("invalid keyed DOM row range");
      if (attached === parent && at > 0 && row.nodes[at - 1].nextSibling !== node) {
        throw new Error("invalid keyed DOM row range");
      }
    }
  }
};

const keyedDomCommit = (state, rows, journal) => {
  keyedDomValidate(state, rows);
  const { end, parent } = state.dom;
  const before = state.order.slice();
  const beforeSet = new Set(before);
  const nextSet = new Set(rows);
  journal(() => {
    for (const row of rows) {
      if (beforeSet.has(row)) continue;
      for (const node of row.nodes) {
        if (node.parentNode === parent) parent.removeChild(node);
      }
    }
    keyedDomOrder(parent, end, before);
  });
  for (const row of before) {
    if (nextSet.has(row)) continue;
    for (const node of row.nodes) {
      if (node.parentNode === parent) parent.removeChild(node);
    }
  }
  return keyedDomOrder(parent, end, rows);
};

globalThis.__rt.keyed_dom_reconcile = (state, values, keyOf, factory) => {
  if (!state.dom) throw new TypeError("keyed DOM reconciliation needs a DOM state");
  return globalThis.__rt.keyed_reconcile(
    state,
    values,
    keyOf,
    (item, index, key) => globalThis.__rt.keyed_dom_owner(
      owner => factory(owner, item, index, key),
      item,
      index,
      key,
      state.hooks,
    ),
    (_values, rows, journal) => keyedDomCommit(state, rows, journal),
  );
};

globalThis.__rt.keyed_dom_dispose = (state) => {
  if (state.running) {
    globalThis.__rt.keyed_dispose(state);
    return;
  }
  const dom = state.dom;
  const parent = dom && (dom.parent || (dom.end && dom.end.parentNode));
  let failure = null;
  try {
    globalThis.__rt.keyed_dispose(state);
  } catch (error) {
    failure = { error };
  }
  if (parent && dom.end && dom.ownsEnd !== false && dom.end.parentNode === parent) {
    try {
      parent.removeChild(dom.end);
    } catch (error) {
      if (failure === null) failure = { error };
    }
  }
  if (failure !== null) throw failure.error;
};

// The compiler-facing static-row adapter. Its first run builds detached row
// carriers and returns them with a permanent end boundary for `insert`. Later
// collection-effect runs find that boundary's parent and commit ranges directly.
globalThis.__rt.keyed_dom_view = (collection, keyOf, factory, hooks) => {
  if (typeof collection !== "function" || typeof keyOf !== "function"
      || typeof factory !== "function") {
    throw new TypeError("a keyed DOM view needs collection, key, and row callables");
  }
  for (const name of ["batch", "cleanup", "effect", "root", "signal", "untrack"]) {
    if (!hooks || typeof hooks[name] !== "function") {
      throw new TypeError(`a keyed DOM view needs a ${name} hook`);
    }
  }
  const document = globalThis.document;
  if (!document || typeof document.createComment !== "function") {
    throw new TypeError("a keyed DOM view needs a document");
  }

  let end = document.createComment("");
  const state = globalThis.__rt.keyed_state(hooks);
  state.dom = { end, hydrated: false, ownsEnd: true, parent: null };
  let initial = null;
  let first = true;
  const buildRow = (owner, item, index) => factory(
    hooks.live ? owner.liveItem : item,
    hooks.liveIndex ? owner.liveIndex : index,
  );
  const createRow = (item, index, key) => globalThis.__rt.keyed_dom_owner(
    owner => buildRow(owner, item, index, key),
    item,
    index,
    key,
    hooks,
  );
  const detached = values => {
    globalThis.__rt.keyed_reconcile(state, values, keyOf, createRow);
    const nodes = state.order.flatMap(row => row.nodes);
    if (initial === null && keyedDomAdopt(state, nodes)) end = state.dom.end;
    if (initial === null) {
      initial = [...nodes, end];
      globalThis.__rt._keyed_dom_values.set(initial, state);
    }
    else initial.splice(0, initial.length, ...nodes, end);
    return initial;
  };
  const update = () => {
    const collectionValue = collection();
    const values = hooks.items === "shared-slot"
      ? globalThis.__rt.keyed_references(collectionValue)
      : collectionValue;
    return hooks.untrack(() => {
      if (first) {
        first = false;
        return detached(values);
      }
      const dom = state.dom;
      const parent = dom.parent || (dom.end && dom.end.parentNode);
      if (parent === null) return detached(values);
      if (dom.parent === null) dom.parent = parent;
      if (dom.parent !== parent) {
        throw new Error("a keyed DOM view boundary moved to another parent");
      }
      // The insertion carrier is needed only until mounting. Keeping it in
      // this effect would retain the first rows even after the list is cleared.
      initial = null;
      return globalThis.__rt.keyed_dom_reconcile(state, values, keyOf, buildRow);
    });
  };

  try {
    let initialRuns = 0;
    hooks.effect(() => {
      initialRuns += 1;
      return update();
    });
    if (initialRuns !== 1 || initial === null) {
      throw new Error("a keyed DOM effect must run exactly once synchronously when created");
    }
    return initial;
  } catch (error) {
    try {
      globalThis.__rt.keyed_dom_dispose(state);
    } catch (_error) {
      // Preserve the construction failure after attempting every row cleanup.
    }
    throw error;
  }
};

// Run one already-materialized keyed transaction. The public wrapper below
// serializes reentrant requests and keeps only the newest pending request.
const keyedReconcileOnce = (state, source, keyOf, createRow, commit) => {
  const plan = [];
  const keys = new Map();
  let keyType = state.keyType;
  for (let index = 0; index < source.length; ++index) {
    globalThis.__rt.keyed_metric("collectionScans");
    const item = source[index];
    const key = keyOf(item, index);
    const currentType = globalThis.__rt.keyed_key_type(key);
    if (keyType === null) keyType = currentType;
    if (currentType !== keyType) {
      throw new TypeError(`keyed-list key type changed from ${keyType} to ${currentType}`);
    }
    if (keys.has(key)) throw new Error(`duplicate keyed-list key: ${String(key)}`);
    keys.set(key, true);
    plan.push({ index, item, key, row: state.rows.get(key) });
  }

  const created = [];
  try {
    for (const entry of plan) {
      if (entry.row !== undefined) continue;
      globalThis.__rt.keyed_metric("rowFactories");
      entry.row = createRow(entry.item, entry.index, entry.key);
      if (!entry.row || typeof entry.row !== "object") {
        throw new TypeError("a keyed row factory must return a row record");
      }
      created.push(entry.row);
    }
  } catch (error) {
    for (let at = created.length - 1; at >= 0; --at) {
      try {
        globalThis.__rt.keyed_dispose_row(created[at]);
      } catch (_error) {
        // Preserve the factory failure after attempting every temporary cleanup.
      }
    }
    throw error;
  }

  const ordered = plan.map(entry => entry.row);
  const previousKeyType = state.keyType;
  const previousOrder = state.order;
  const previousRows = state.rows;
  const nextRows = new Map();
  let retained = 0;
  for (const entry of plan) {
    if (previousRows.has(entry.key)) retained += 1;
    nextRows.set(entry.key, entry.row);
  }
  const removed = [...previousRows.entries()]
    .filter(([key]) => !nextRows.has(key))
    .map(([, row]) => row);
  const rollbacks = [];
  const journal = rollback => {
    if (typeof rollback !== "function") {
      throw new TypeError("a keyed commit rollback must be a function");
    }
    rollbacks.push(rollback);
  };
  let moves = 0;
  let published = false;
  try {
    const publish = () => {
      if (published) throw new Error("a keyed batch ran its transaction more than once");
      published = true;
      journal(() => {
        state.keyType = previousKeyType;
        state.order = previousOrder;
        state.rows = previousRows;
      });
      for (const entry of plan) {
        const row = entry.row;
        if (row.item && typeof row.item === "object" && row.item.value !== entry.item) {
          const before = row.item.value;
          journal(() => {
            row.item.value = before;
          });
          row.item.value = entry.item;
        }
        if (row.index && typeof row.index === "object" && row.index.value !== entry.index) {
          const before = row.index.value;
          journal(() => {
            row.index.value = before;
          });
          row.index.value = entry.index;
        }
      }
      moves = commit === undefined
        ? 0
        : commit(ordered.map(row => row.value), ordered, journal);
      if (!Number.isSafeInteger(moves) || moves < 0) {
        throw new TypeError("a keyed commit must return a non-negative DOM-move count");
      }
      state.keyType = keyType;
      state.order = ordered;
      state.rows = nextRows;
    };
    if (state.batch === undefined) publish();
    else state.batch(publish);
    if (!published) throw new Error("a keyed batch did not run its transaction synchronously");
  } catch (error) {
    const rollbackErrors = [];
    for (let at = rollbacks.length - 1; at >= 0; --at) {
      try {
        rollbacks[at]();
      } catch (rollbackError) {
        rollbackErrors.push(rollbackError);
      }
    }
    for (let at = created.length - 1; at >= 0; --at) {
      try {
        globalThis.__rt.keyed_dispose_row(created[at]);
      } catch (_error) {
        // Preserve the commit failure after attempting every temporary cleanup.
      }
    }
    if (rollbackErrors.length > 0 && error !== null && typeof error === "object") {
      try {
        error.keyedRollbackErrors = rollbackErrors;
      } catch (_error) {
        // A frozen or host-owned exception still remains the operation's error.
      }
    }
    throw error;
  }
  globalThis.__rt.keyed_metric("domMoves", moves);
  globalThis.__rt.keyed_metric("createdRows", created.length);
  globalThis.__rt.keyed_metric("retainedRows", retained);
  globalThis.__rt.keyed_metric("removedRows", removed.length);
  globalThis.__rt.keyed_metric("liveRowRecords", created.length);
  globalThis.__rt._keyed_metrics.liveRowRecords -= removed.length;

  const cleanupErrors = [];
  for (const row of removed) {
    try {
      globalThis.__rt.keyed_dispose_row(row);
    } catch (error) {
      cleanupErrors.push(error);
    }
  }
  if (cleanupErrors.length > 0) {
    const error = new Error("keyed row cleanup failed");
    error.keyedCommitted = true;
    error.errors = cleanupErrors;
    throw error;
  }
  return ordered.map(row => row.value);
};

// keyed_reconcile(state, values, keyOf, createRow, commit) -> row values.
//
// Phase A evaluates every key, validates one typed key lane, and rejects all
// duplicates. Phase B creates new rows under temporary owners. Only then may
// `commit` publish DOM order. A commit registers each inverse operation through
// its third `journal` argument before mutating the DOM. A reentrant call queues
// the newest request; cleanup failures report a committed state after every
// removed row has been attempted. Owner disposal takes precedence over pending
// reconciliations and runs after the current transaction finishes or rolls back.
globalThis.__rt.keyed_reconcile = (state, values, keyOf, createRow, commit) => {
  if (state.disposed || state.disposePending) throw new Error("cannot reconcile a disposed keyed list");
  if (typeof keyOf !== "function" || typeof createRow !== "function") {
    throw new TypeError("keyed reconciliation requires key and row factories");
  }
  if (commit !== undefined && typeof commit !== "function") {
    throw new TypeError("a keyed commit must be a function");
  }
  const outer = !state.running;
  const previousPending = state.pending;
  const request = { commit, createRow, keyOf, source: null };
  state.running = true;
  state.pending = request;
  let failure = null;
  let result = [];
  try {
    // Reserve this request before iterating. A source may queue a newer request,
    // whose snapshot must not be overwritten when the older iterator returns.
    request.source = globalThis.__rt.keyed_values(values);
    if (state.disposed || state.disposePending) throw new Error("cannot reconcile a disposed keyed list");
    if (!outer) return state.order.map(row => row.value);
    if (state.pending === request) state.pending = null;
    let current = request;
    while (current !== null && !state.disposePending) {
      try {
        result = keyedReconcileOnce(
          state,
          current.source,
          current.keyOf,
          current.createRow,
          current.commit,
        );
      } catch (error) {
        if (failure === null) failure = { error };
        if (error === null || typeof error !== "object" || error.keyedCommitted !== true) {
          state.pending = null;
          break;
        }
      }
      current = state.pending;
      state.pending = null;
    }
  } catch (error) {
    if (!outer) {
      if (state.pending === request) state.pending = previousPending;
      throw error;
    }
    failure = { error };
    state.pending = null;
  } finally {
    if (outer) {
      state.running = false;
      if (state.disposePending) {
        state.disposePending = false;
        try {
          if (state.dom) globalThis.__rt.keyed_dom_dispose(state);
          else globalThis.__rt.keyed_dispose(state);
        } catch (error) {
          if (failure === null) failure = { error };
        }
        result = [];
      }
    }
  }
  if (failure !== null) throw failure.error;
  return result;
};

globalThis.__rt.keyed_dispose = (state) => {
  if (state.disposed) return;
  if (state.running) {
    state.disposePending = true;
    state.pending = null;
    return;
  }
  state.disposed = true;
  const rows = [...state.rows.values()];
  state.rows.clear();
  state.order = [];
  state.pending = null;
  globalThis.__rt._keyed_states.delete(state);
  globalThis.__rt._keyed_metrics.transactionStates -= 1;
  globalThis.__rt._keyed_metrics.liveRowRecords -= rows.length;
  globalThis.__rt.keyed_metric("removedRows", rows.length);
  let firstError = null;
  for (const row of rows) {
    try {
      globalThis.__rt.keyed_dispose_row(row);
    } catch (error) {
      if (firstError === null) firstError = { error };
    }
  }
  if (firstError !== null) throw firstError.error;
};

// ---------------------------------------------------------------------------
// Legacy keyed rows
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
  if (seen !== undefined) return seen;
  rows.set(key, node);
  return node;
};

// ---------------------------------------------------------------------------
// The executor (view-abi's `microtask` and `on_settled`)
// ---------------------------------------------------------------------------
//
// Baseline builds keep the poll loop and owner policy in safe Rust. Optimized
// builds hand one concrete future's poll and drop callbacks to the host drivers
// below. The shared primitives here are scheduling and Promise settlement.

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
  const then = value !== null && value !== undefined ? value.then : undefined;
  if (typeof then === "function") {
    Reflect.apply(then, value, [
      (x) => f(x, true),
      (e) => f(e, false),
    ]);
    return;
  }
  queueMicrotask(() => f(value, true));
};

// The general host boundary used by mires-js. These helpers exist only where JavaScript syntax
// has no expression-level spelling the MIR backend can use directly.
globalThis.__rt.host_try = (f, drop) => {
  try {
    return { TAG: "Ok", _0: f() };
  } catch (error) {
    return { TAG: "Err", _0: error };
  } finally {
    if (drop !== undefined) drop();
  }
};

// Primitive conversion can call user code and throw even when the result is unused. Keep the
// catch boundary so bundlers do not erase or inline this call as a pure arithmetic expression.
globalThis.__rt.host_add = (left, right) => {
  try {
    return left + right;
  } catch (error) {
    throw error;
  }
};

globalThis.__rt.host_is_error = (value) => value instanceof Error;

// A typed Uint8Array operation with complete capability and fallback accounting. The native path
// emits unpadded Base64URL directly. The compatibility path chunks the variadic conversion to
// avoid argument limits, then normalizes ordinary Base64 to the same result.
globalThis.__rt.host_base64url = (bytes) => {
  if (typeof bytes.toBase64 === "function") {
    return bytes.toBase64({ alphabet: "base64url", omitPadding: true });
  }
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 32768) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};

globalThis.__rt.host_local_waker = (f) => ({
  data: f,
  vtable: ((target) => {
    const current = target.vtable;
    return current === null || current === undefined
      ? (target.vtable = {
          clone: globalThis.__rt.host_local_waker,
          wake: (callback) => callback(),
          wake_by_ref: (callback) => callback(),
          drop: () => {},
        })
      : current;
  })(globalThis.__rt.host_local_waker),
});

// Drives one concrete Rust Future without a Rust task allocation. `poll` and `drop` are
// monomorphic callbacks emitted by the backend. `run` optionally restores a captured owner.
globalThis.__rt.host_spawn = (future, poll, drop, defer, owner, run, cleanup) => {
  const state = { future, scheduled: false, polling: false, done: false, cancelled: false };
  const finish = () => {
    if (state.done) return;
    state.done = true;
    try {
      if (drop !== undefined) drop(state);
    } finally {
      state.future = undefined;
    }
  };
  const schedule = () => {
    if (state.done || state.cancelled || state.scheduled) return;
    state.scheduled = true;
    queueMicrotask(step);
  };
  const waker = globalThis.__rt.host_local_waker(schedule);
  const step = () => {
    if (state.done || state.polling) return;
    state.scheduled = false;
    state.polling = true;
    try {
      const pollAndFinish = () => {
        const ready = poll(state, waker);
        if (ready || state.cancelled) finish();
      };
      if (run === undefined) pollAndFinish();
      else run(owner, pollAndFinish);
    } catch (error) {
      // A thrown poll is terminal too. Release the pinned future before propagating the host
      // error, under the same owner that ran the poll. `finish` is idempotent, so a throwing drop
      // reached from the ready path is not run twice here.
      if (run === undefined) finish();
      else run(owner, finish);
      throw error;
    } finally {
      state.polling = false;
    }
  };
  if (cleanup !== undefined && owner != null) {
    run(owner, () => cleanup(() => {
      if (state.done || state.cancelled) return;
      state.cancelled = true;
      // A pinned future cannot be dropped while its poll holds a mutable borrow.
      if (!state.polling) run(owner, finish);
    }));
  }
  if (defer) schedule();
  else step();
};

// The Promise terminal uses the same scheduler shape, but owns the resolver pair and settles only
// after the concrete future has been dropped. Promise resolution adopts returned thenables.
globalThis.__rt.host_future_promise = (future, poll, drop) =>
  new Promise((resolve, reject) => {
    const state = { future, scheduled: false, polling: false, done: false };
    const finish = () => {
      if (state.done) return;
      state.done = true;
      try {
        if (drop !== undefined) drop(state);
      } finally {
        state.future = undefined;
      }
    };
    const schedule = () => {
      if (state.done || state.scheduled) return;
      state.scheduled = true;
      queueMicrotask(step);
    };
    const waker = globalThis.__rt.host_local_waker(schedule);
    const step = () => {
      if (state.done || state.polling) return;
      state.scheduled = false;
      state.polling = true;
      try {
        const settled = poll(state, waker);
        if (settled.TAG !== "Ready") return;
        const result = settled._0;
        finish();
        (result.TAG === "Ok" ? resolve : reject)(result._0);
      } catch (error) {
        // Promise executors turn throws into rejections, but first release the Rust future. This
        // keeps captured values from leaking and preserves drop-before-settle ordering.
        try {
          finish();
        } catch (dropError) {
          reject(dropError);
          return;
        }
        reject(error);
      } finally {
        state.polling = false;
      }
    };
    step();
  });

// A proven nonsuspending generated coroutine cannot observe this waker. Keep the original poll
// and drop callbacks, including exception priority and Promise adoption, without scheduler state.
globalThis.__rt.host_future_ready = (future, poll, drop) =>
  new Promise((resolve, reject) => {
    const state = { future };
    let result;
    try {
      result = poll(state, globalThis.__rt.host_local_waker(() => {}))._0;
    } finally {
      try {
        if (drop !== undefined) drop(state);
      } finally {
        state.future = undefined;
      }
    }
    (result.TAG === "Ok" ? resolve : reject)(result._0);
  });

// Opt-in continuation trial: keep the concrete Rust state machine and exact poll/drop callbacks,
// but let one native async function own suspension and resumption. Synchronous and repeated wakes
// are coalesced exactly like host_future_promise; a late wake after completion is ignored.
globalThis.__rt.host_future_async = async (future, poll, drop) => {
  const state = { future, done: false, notified: false, resume: undefined };
  const finish = () => {
    if (state.done) return;
    state.done = true;
    state.resume = undefined;
    try {
      if (drop !== undefined) drop(state);
    } finally {
      state.future = undefined;
    }
  };
  const schedule = () => {
    if (state.done || state.notified) return;
    state.notified = true;
    const resume = state.resume;
    state.resume = undefined;
    if (resume !== undefined) resume();
  };
  const waker = globalThis.__rt.host_local_waker(schedule);
  try {
    while (true) {
      state.notified = false;
      const settled = poll(state, waker);
      if (settled.TAG === "Ready") {
        const result = settled._0;
        if (result.TAG === "Ok") return result._0;
        throw result._0;
      }
      if (state.notified) {
        await undefined;
      } else {
        await new Promise((resume) => {
          state.resume = resume;
          if (state.notified) {
            state.resume = undefined;
            resume();
          }
        });
      }
    }
  } finally {
    finish();
  }
};

globalThis.__rt.host_task = (f) => {
  const channel = new MessageChannel();
  channel.port1.onmessage = () => {
    channel.port1.close();
    channel.port2.close();
    f();
  };
  channel.port2.postMessage(undefined);
};

// convert_host_sb(s): a host string entering the mutable native String carrier
// `{ text, cap }` (Phase 4 Convert adapter). An immutable host string's capacity
// is its UTF-8 byte length, which the carrier records exactly.
globalThis.__rt.convert_host_sb = (s) => ({
  text: s,
  cap: globalThis.__rt.str_len(s),
});

// convert_str_block(s): the UTF-8 bytes of a host string as a fresh `{ buf, off,
// len }` block the caller owns (Phase 4 Convert adapter, host string back to the
// baseline `Vec<u8>` String). `str_bytes` may return a memoized record whose
// buffer other `as_bytes` calls alias, and an owned baseline String must not
// share its allocation, so the bytes are copied into a new plain array.
globalThis.__rt.convert_str_block = (s) => {
  const shared = globalThis.__rt.str_bytes(s);
  const copy = shared.buf.slice();
  // The baseline String's byte block is a heap block: reads reach it through
  // `unscale`, which answers from the block registry, so the fresh copy is
  // registered exactly as `alloc` would have (already element-typed at 1 byte).
  const p = globalThis.__rt._register(copy, copy.length, false);
  globalThis.__rt._blocks.get(copy).fresh = false;
  // A host string is immutable and exact-fit: its capacity is its byte length.
  return { buf: p.buf, off: 0, len: shared.len, cap: shared.len };
};

// convert_sb_block(sb): a mutable native `{ text, cap }` String's bytes as a
// fresh registered `{ buf, off, len, cap }` block (Phase 4 Convert adapter,
// builder back to the baseline `Vec<u8>` String). The backing array is capacity
// length with `[0, len)` initialized, and the builder's Rust-visible capacity
// rides along for the baseline cap leaf — a moved String keeps its capacity.
globalThis.__rt.convert_sb_block = (sb) => {
  const shared = globalThis.__rt.str_bytes(sb.text);
  const cap = Math.max(sb.cap, shared.len);
  const copy = shared.buf.slice(0, shared.len);
  copy.length = cap;
  const p = globalThis.__rt._register(copy, cap, false);
  globalThis.__rt._blocks.get(copy).fresh = false;
  return { buf: p.buf, off: 0, len: shared.len, cap };
};

// convert_nv_block(nv, size): a native `{ buf, len, cap }` Vec's elements as a
// registered baseline heap block of `size`-byte elements (Phase 4 Convert
// adapter, native Vec back to the baseline raw parts). The block keeps the
// carrier's capacity and is already element-typed, exactly as an `alloc` +
// first-cast retype would have left it.
globalThis.__rt.convert_nv_block = (nv, size) => {
  const copy = nv.buf.slice(0, nv.len);
  copy.length = Math.max(nv.cap, nv.len);
  const p = globalThis.__rt._register(copy, copy.length * size, false);
  const block = globalThis.__rt._blocks.get(copy);
  block.es = size;
  block.fresh = false;
  return p;
};

// convert_block_nv(buf, off, len, cap): a baseline Vec's raw parts entering the
// native `{ buf, len, cap }` carrier (Phase 4 Convert adapter). The backing
// array is capacity length with `[0, len)` initialized, exactly the carrier's
// contract; the copy keeps the baseline block unshared.
globalThis.__rt.convert_block_nv = (buf, off, len, cap) => {
  const copy = new Array(cap);
  for (let i = 0; i < len; i += 1) copy[i] = buf[off + i];
  return { buf: copy, len, cap };
};

// convert_sv_nv(s): a `{ buf, off, len }` slice view entering the native Vec
// carrier as an owned exact-fit value (Phase 4 Convert adapter; the planner
// only converts a dead, unaliased source this way).
globalThis.__rt.convert_sv_nv = (s) => ({
  buf: s.buf.slice(s.off, s.off + s.len),
  len: s.len,
  cap: s.len,
});
