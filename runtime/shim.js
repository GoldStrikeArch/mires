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
  if (error === null || (typeof error !== "object" && typeof error !== "function")) {
    error = new Error(String(error));
  }
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
    const ev = globalThis.__rt._element_view_of.get(target);
    if (ev !== undefined) {
      for (let i = 0; i < ev.n; i++) ev.buf[ev.off + i] = value[i];
      return target;
    }
    target.length = 0;
    return Object.assign(target, value);
  }
  for (const key in target) {
    if (!(key in value)) delete target[key];
  }
  for (const key in value) target[key] = value[key];
  return target;
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

// Optional lazy byte-view dispatch table installed by `_bv_enable` when a
// program may observe wider storage as bytes. Stays `null` (and is constant-
// folded away in lexical ESM mode) for programs that never form wide byte views.
globalThis.__rt._bv = null;

// Raw slice metadata can describe an allocator's uninitialized byte block, or a
// byte slice over wider storage (`byte` is 1 for u8, -1 for i8).
globalThis.__rt.slice_from_parts = (p, len, byte) => {
  if (typeof p === "number") {
    if (p > 0 && p < 4096) p = globalThis.__rt._dangling(p);
    else return { buf: p, off: 0, len };
  } else if (p !== null && typeof p === "object" && typeof p.buf === "number" && p.pr === undefined) {
    const a = (p.buf + (typeof p.off === "number" ? p.off * (p.sc !== undefined ? 1 : p.es || 1) : 0)) >>> 0;
    if (a > 0 && a < 4096) p = globalThis.__rt._dangling(a);
    else return { buf: a, off: 0, len };
  }
  if (p.sc !== undefined) {
    if (len > 0) throw __rt.abort_error(new Error("rustc_codegen_js: cannot view " + p.sc + "-byte non-scalar elements as a byte slice"));
    return { buf: p.buf, off: p.off, len: 0, sc: p.sc };
  }
  if (byte !== undefined && globalThis.__rt._bv !== null) return globalThis.__rt._bv.slice(p, len, byte);
  return { buf: p.buf, off: p.off, len };
};

// A u8/i8 slice reference establishes initialized byte storage. Word reads must
// alias these bytes. Raw slices and MaybeUninit slices do not establish this fact.
globalThis.__rt.byte_slice_ref = (p, signed) => {
  if (globalThis.__rt._bv !== null) return globalThis.__rt._bv.byte_slice_ref(p, !!signed);
  const block = globalThis.__rt._heap_block(p.buf);
  if (block !== undefined && block.es === 1) block.fresh = false;
  return p;
};

// `p as usize`: the synthetic address of a pointer, where `size` is the size of
// the pointee in bytes. Every buffer is given a base the first time it is asked
// about (`_addr_mint`, below), kept in a WeakMap so that it never moves, and an
// element at offset `off` sits at `base + off * size`. A scaled slot's offset
// is already in bytes and is added as it is; a number is an address already and
// comes back unchanged.
//
// The four properties this scheme exists for: a base is never zero, so
// `is_null()` is false for every real pointer and true only for `null()`; a
// base is at least 4096, so `NonNull::dangling()` (an alignment, always
// smaller) never collides with one; a base is aligned to the buffer's byte size
// rounded up to a power of two (at least 16, at most 4096), so an address is a
// multiple of the element alignment and alignment predicates answer correctly;
// and the low bits are therefore determined by the element size, which is what
// `fmt::Arguments` tests when it looks at `bits & 1`.
//
// `undefined` as a pointer is the reference form of a zero-sized value: every
// pointer to a zero-sized local, to an element of a zero-sized array, and into a
// slice unsized from one (CONTRACT.md, "Pointers"). Rust promises such a pointer
// is non-null and aligned and nothing more, so all of them share one address,
// `_zst_addr`: 2^29, the largest alignment a type can ask for. No base is ever
// given that address, so it is never mistaken for an element of a real buffer.
globalThis.__rt._addrs = new WeakMap();
globalThis.__rt._addrNext = 4096;
globalThis.__rt._zst_addr = 536870912;

globalThis.__rt.addr = (p, size) => {
  if (typeof p === "number") return p;
  if (p === null) return 0;
  if (p === undefined) return globalThis.__rt._zst_addr;
  if (p.pr !== undefined) return globalThis.__rt.addr(p.pr, p.ps);
  let buf, off, inBytes, step;
  if (globalThis.__rt._bv !== null) {
    const u = globalThis.__rt._bv.addr_unwrap(p, size);
    if (typeof u === "number") return u;
    buf = u.buf;
    off = u.off;
    inBytes = u.inBytes;
    step = u.es !== undefined
      ? u.es
      : u.evEs !== undefined
      ? u.evEs
      : (u.w !== undefined && u.w > 0 ? size / u.w : size);
  } else {
    if (typeof p === "object" && !("buf" in p)) {
      const ev = globalThis.__rt._element_view_of.get(p);
      if (ev !== undefined) {
        const elemSize = ev.es !== undefined ? ev.es : (ev.n > 0 ? size / ev.n : size);
        return globalThis.__rt.addr({ buf: ev.buf, off: ev.off }, elemSize);
      }
      if (globalThis.__rt._boxes !== null) {
        const slot = globalThis.__rt._boxes.get(p);
        if (slot !== undefined && slot.buf[slot.off] === p) {
          const block = globalThis.__rt._blk_get(slot.buf);
          return globalThis.__rt.addr(slot, block !== undefined ? block.es : size);
        }
      }
    }
    globalThis.__rt._named(p, "addr");
    buf = p.buf;
    off = typeof p.off === "number" ? p.off : 0;
    const key = typeof p.off === "number" ? undefined : p.off;
    inBytes = p.sc !== undefined;
    let es = p.sc !== undefined ? p.sc : p.es;
    let w = p.w;
    let evEs;
    while (typeof buf === "object" && buf !== null) {
      const ev = globalThis.__rt._element_view_of.get(buf);
      if (ev !== undefined) {
        const elemEs = ev.es !== undefined ? ev.es : es;
        off = inBytes && elemEs !== undefined ? off + ev.off * elemEs : off + ev.off;
        if (ev.es !== undefined) evEs = ev.es;
        buf = ev.buf;
        continue;
      }
      if (key === undefined && globalThis.__rt._boxed.has(buf)) {
        const held = buf[0];
        const bev = held !== null && typeof held === "object"
          ? globalThis.__rt._element_view_of.get(held)
          : undefined;
        if (bev !== undefined) {
          const elemEs = bev.es !== undefined
            ? bev.es
            : (es !== undefined && bev.n > 0 ? es / bev.n : es);
          off = inBytes && elemEs !== undefined ? off + bev.off * elemEs : bev.off + off * bev.n;
          if (elemEs !== undefined) {
            evEs = elemEs;
            if (!inBytes) es = elemEs;
          }
          w = bev.n;
          buf = bev.buf;
          continue;
        }
      }
      const scalar = globalThis.__rt._scalar_view_of.get(buf);
      if (scalar !== undefined) {
        off = off * scalar.size + scalar.phase;
        inBytes = true;
        if (es === undefined) es = 1;
        buf = scalar.buf;
        continue;
      }
      const blk = globalThis.__rt._blk_get(buf);
      if (blk !== undefined && blk.typed !== undefined && buf !== blk.typed && blk.hdr === undefined) {
        inBytes = true;
        es = blk.es;
        buf = blk.typed;
        continue;
      }
      break;
    }
    step = es !== undefined
      ? es
      : evEs !== undefined
      ? evEs
      : (w !== undefined && w > 0 ? size / w : size);
  }
  if (typeof buf === "number") return (buf + (inBytes ? off : off * step)) >>> 0;
  const key = typeof buf === "object" && buf !== null ? buf : p;
  const align = globalThis.__rt._dangling_of.get(key);
  if (align !== undefined) return align + (inBytes ? off : off * step);
  let base = globalThis.__rt._addrs.get(key);
  if (base === undefined) {
    base = globalThis.__rt._addr_mint(key, step);
  }
  return base + (inBytes ? off : off * step);
};

// The synthetic address space. A usize is 32 bits, so every base has to fit in
// 2^32 -- and a base that was never given back would exhaust it: one per
// `NonNull::new` on a fresh allocation is a million allocations, which a
// long-running page reaches. So bases are carved like a buddy allocator.
//
// A buffer of `n` bytes takes a granule of `2^k` bytes, the smallest power of
// two that holds it, at least 16 and at most 4096, and its base is a multiple of
// the granule: every type's alignment divides its size, so a buffer is aligned
// for anything it can hold (a buffer larger than 4096 bytes is aligned to 4096,
// as before). Granules come from a per-size free list first and from a bump
// pointer otherwise; the alignment gap the bump leaves is split into smaller
// free granules rather than lost. A base is handed back:
//
//   * by `dealloc`, for a heap block (`_addr_release`). The freed buffer keeps
//     its entry, so a dangling pointer still answers the address it had, and a
//     later allocation may reuse it -- exactly as a native allocator does;
//   * by a FinalizationRegistry, for any other buffer (a stack array, a box, a
//     string's byte view) once the engine has collected it, which is the moment
//     nothing can ask for that address again.
//
// A base is therefore unique among the buffers that are alive, and stable for a
// buffer's whole lifetime. Only when the bump pointer reaches 2^32 with no free
// granule of the size asked is there no base to give, and that is a refusal that
// says so rather than an address that wraps to null.
globalThis.__rt._addrFree = Array.from({ length: 13 }, () => []);
globalThis.__rt._addrHeap = new WeakMap();
let _has_minted_addrs = false;
globalThis.__rt._addrGone = typeof FinalizationRegistry === "function"
  ? new FinalizationRegistry((held) => {
    // `held` is the base plus its granule's exponent, which is below 16.
    globalThis.__rt._addrFree[held & 15].push(held - (held & 15));
  })
  : undefined;

// _addr_mint(key, es): a base for `key`, whose elements are `es` bytes each:
// a free granule of its size, or a fresh one from the bump pointer. The top is
// reached only when about 2^32 bytes of granules have been live at once (a
// million live 4 KiB buffers), which no engine's heap holds, so free granules
// of other sizes are not split -- the top is a refusal.
globalThis.__rt._addr_mint = (key, es) => {
  const free = globalThis.__rt._addrFree;
  const block = globalThis.__rt._blk_get(key);
  const bytes = block !== undefined ? block.bytes
    : (typeof key.length === "number" ? key.length : 1) * (es > 0 ? es : 1);
  let k = 4;
  while (k < 12 && 2 ** k < bytes) k++;
  let base = free[k].pop();
  if (base === undefined) {
    const size = 2 ** k;
    base = globalThis.__rt._addrNext;
    while (base % size !== 0) {
      let j = 4;
      while (j + 1 < k && base % 2 ** (j + 1) === 0) j++;
      free[j].push(base);
      base += 2 ** j;
    }
    // Never hand out the shared zero-sized address (see `addr`).
    if (base === globalThis.__rt._zst_addr) base += 4096;
    if (base + size > 4294967296) {
      throw __rt.abort_error(new Error("rustc_codegen_js: the 32-bit synthetic address space is exhausted"));
    }
    globalThis.__rt._addrNext = base + size;
  }
  globalThis.__rt._addrs.set(key, base);
  if (block !== undefined) {
    _has_minted_addrs = true;
    globalThis.__rt._addrHeap.set(key, k);
  } else if (globalThis.__rt._addrGone !== undefined) {
    globalThis.__rt._addrGone.register(key, base + k);
  }
  return base;
};

// _addr_release(buf, bytes): gives a heap block's granule back. `dealloc` calls
// it without `bytes`; `realloc` calls it with the new size, and it acts only if
// the block grew past its granule (below 4096): the block then takes a base at
// its new size the next time it is asked, because otherwise its addresses would
// run into its neighbours'. Rust lets a reallocation move, so the old address
// ending with it is no change of meaning.
globalThis.__rt._addr_release = (buf, bytes) => {
  if (!_has_minted_addrs) return;
  const k = globalThis.__rt._addrHeap.get(buf);
  if (k === undefined || (bytes !== undefined && (k === 12 || bytes <= 2 ** k))) return;
  globalThis.__rt._addrHeap.delete(buf);
  globalThis.__rt._addrFree[k].push(globalThis.__rt._addrs.get(buf));
  if (bytes !== undefined) globalThis.__rt._addrs.delete(buf);
};

// ptr_is_null(ptr, size): exact boolean `(addr(ptr, size) >>> 0) === 0`, without
// minting a synthetic address for live `{ buf: Array, off }` pointers on the fast path.
globalThis.__rt.ptr_is_null = (ptr, size = 1) => {
  while (ptr !== null && ptr !== undefined && typeof ptr === "object" && ptr.pr !== undefined) {
    size = ptr.ps;
    ptr = ptr.pr;
  }
  if (typeof ptr === "number") return (ptr >>> 0) === 0;
  if (ptr === null) return true;
  if (ptr === undefined) return false;
  if (typeof ptr === "object" && typeof ptr.buf === "object" && ptr.buf !== null) {
    if (ptr.off === 0 || ptr.off === undefined) return false;
    const step = ptr.sc !== undefined ? 1 : ptr.es === undefined ? size : ptr.es;
    if (
      typeof ptr.off === "number" &&
      ptr.off >= 0 &&
      ptr.off <= 65536 &&
      globalThis.__rt._addrNext < 2147483648 &&
      typeof step === "number" &&
      step >= 0 &&
      step <= 1024
    ) {
      return false;
    }
  }
  return (globalThis.__rt.addr(ptr, size) >>> 0) === 0;
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
  if (a.buf === b.buf && a.w === undefined && b.w === undefined) return a.off - b.off;
  const unwrap = (p) => {
    const w = p.w !== undefined ? p.w : 1;
    if (p.buf !== null && typeof p.buf === "object") {
      const ev = globalThis.__rt._element_view_of.get(p.buf);
      if (ev !== undefined) {
        return {
          buf: ev.buf,
          off: p.sc !== undefined ? ev.off * p.sc + p.off : ev.off + p.off,
          w,
        };
      }
      if (p.w === undefined && globalThis.__rt._boxed.has(p.buf)) {
        const held = p.buf[0];
        if (held !== null && typeof held === "object") {
          const bev = globalThis.__rt._element_view_of.get(held);
          if (bev !== undefined) {
            return { buf: bev.buf, off: bev.off + p.off * bev.n, w: bev.n };
          }
        }
      }
    }
    return { buf: p.buf, off: p.off, w };
  };
  let ua = unwrap(a);
  let ub = unwrap(b);
  if (ua.buf !== ub.buf) {
    if (a.w === undefined && globalThis.__rt._boxed.has(a.buf) && a.buf[0] === ub.buf && Array.isArray(ub.buf)) {
      ua = { buf: ub.buf, off: a.off * ub.buf.length, w: ub.w };
    } else if (b.w === undefined && globalThis.__rt._boxed.has(b.buf) && b.buf[0] === ua.buf && Array.isArray(ua.buf)) {
      ub = { buf: ua.buf, off: b.off * ua.buf.length, w: ua.w };
    }
  }
  if (ua.buf === ub.buf) {
    const w = ua.w > 1 ? ua.w : ub.w;
    return w > 1 ? Math.trunc((ua.off - ub.off) / w) : ua.off - ub.off;
  }
  const home = (p, other) => {
    if (!globalThis.__rt._boxed.has(p.buf) || !Array.isArray(other.buf)) return -1;
    return other.buf.indexOf(p.buf[p.off]);
  };
  const inb = home(ua, ub);
  if (inb >= 0) return inb - ub.off;
  const ina = home(ub, ua);
  if (ina >= 0) return ua.off - ina;
  if (!globalThis.__rt._boxed.has(a.buf) && !globalThis.__rt._boxed.has(b.buf)) {
    return a.off - b.off;
  }
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
  if (a == null || b == null) return false;
  const aNum = typeof a === "number";
  const bNum = typeof b === "number";
  if ((aNum && bNum) || (!aNum && typeof a !== "object") || (!bNum && typeof b !== "object")) return false;
  if (!aNum) globalThis.__rt._named(a, "ptr_eq");
  if (!bNum) globalThis.__rt._named(b, "ptr_eq");
  if (globalThis.__rt._bv !== null) return globalThis.__rt._bv.ptr_eq(a, b);
  if (aNum || bNum) {
    let p = aNum ? b : a;
    if (!("buf" in p)) return false;
    while (p.pr !== undefined) p = p.pr;
    let pb = p.buf, po = typeof p.off === "number" ? p.off : 0;
    let ps = p.sc !== undefined ? 1 : p.es !== undefined ? p.es : 1;
    const psv = typeof pb === "object" && pb !== null ? globalThis.__rt._scalar_view_of.get(pb) : undefined;
    if (psv !== undefined) { po = po * psv.size + psv.phase; ps = 1; pb = psv.buf; }
    const base = typeof pb === "number" ? pb : globalThis.__rt._dangling_of.get(pb);
    if (base === undefined) return false;
    return ((base + po * ps) >>> 0) === ((aNum ? a : b) >>> 0);
  }
  let l = a, r = b;
  const aSlot = "buf" in l;
  const bSlot = "buf" in r;
  if (!aSlot && !bSlot) {
    const aEv = globalThis.__rt._element_view_of.get(l);
    const bEv = globalThis.__rt._element_view_of.get(r);
    if (aEv === undefined && bEv === undefined) return false;
    if (aEv !== undefined && bEv !== undefined) {
      return aEv.buf === bEv.buf && aEv.off === bEv.off;
    }
    const ev = aEv !== undefined ? aEv : bEv;
    const other = aEv !== undefined ? r : l;
    return ev.buf === other && ev.off === 0;
  }
  if (!aSlot) {
    const ev = globalThis.__rt._element_view_of.get(l);
    if (ev !== undefined) {
      l = ev.es !== undefined && ev.es > 1
        ? { buf: ev.buf, off: ev.off, es: ev.es }
        : { buf: ev.buf, off: ev.off };
    } else if (globalThis.__rt._boxes !== null) {
      const slot = globalThis.__rt._boxes.get(l);
      if (slot !== undefined && slot.buf[slot.off] === l) {
        const block = globalThis.__rt._blk_get(slot.buf);
        l = block !== undefined && block.es > 1 ? { buf: slot.buf, off: slot.off, es: block.es } : slot;
      }
    }
  }
  if (!bSlot) {
    const ev = globalThis.__rt._element_view_of.get(r);
    if (ev !== undefined) {
      r = ev.es !== undefined && ev.es > 1
        ? { buf: ev.buf, off: ev.off, es: ev.es }
        : { buf: ev.buf, off: ev.off };
    } else if (globalThis.__rt._boxes !== null) {
      const slot = globalThis.__rt._boxes.get(r);
      if (slot !== undefined && slot.buf[slot.off] === r) {
        const block = globalThis.__rt._blk_get(slot.buf);
        r = block !== undefined && block.es > 1 ? { buf: slot.buf, off: slot.off, es: block.es } : slot;
      }
    }
  }
  if (!("buf" in l) || !("buf" in r)) return false;
  while (l.pr !== undefined) l = l.pr;
  while (r.pr !== undefined) r = r.pr;
  let lb = l.buf, lo = typeof l.off === "number" ? l.off : 0, lKey = typeof l.off === "number" ? undefined : l.off;
  let lInB = l.sc !== undefined, lEs = l.sc !== undefined ? l.sc : l.es;
  let rb = r.buf, ro = typeof r.off === "number" ? r.off : 0, rKey = typeof r.off === "number" ? undefined : r.off;
  let rInB = r.sc !== undefined, rEs = r.sc !== undefined ? r.sc : r.es;
  while (typeof lb === "object" && lb !== null) {
    const ev = globalThis.__rt._element_view_of.get(lb);
    if (ev !== undefined) {
      const elemEs = ev.es !== undefined ? ev.es : lEs;
      lo = lInB && elemEs !== undefined ? lo + ev.off * elemEs : lo + ev.off;
      if (!lInB && ev.es !== undefined && lEs === undefined) lEs = ev.es;
      lb = ev.buf;
      continue;
    }
    if (lKey === undefined && globalThis.__rt._boxed.has(lb)) {
      const held = lb[0];
      const bev = held !== null && typeof held === "object"
        ? globalThis.__rt._element_view_of.get(held)
        : undefined;
      if (bev !== undefined) {
        const elemEs = bev.es !== undefined
          ? bev.es
          : (lEs !== undefined && bev.n > 0 ? lEs / bev.n : lEs);
        lo = lInB && elemEs !== undefined ? lo + bev.off * elemEs : bev.off + lo * bev.n;
        if (!lInB && elemEs !== undefined) lEs = elemEs;
        lb = bev.buf;
        continue;
      }
    }
    const sv = globalThis.__rt._scalar_view_of.get(lb);
    if (sv !== undefined) {
      lo = lo * sv.size + sv.phase;
      lInB = true;
      if (lEs === undefined) lEs = 1;
      lb = sv.buf;
      continue;
    }
    const blk = globalThis.__rt._blk_get(lb);
    if (blk !== undefined && blk.typed !== undefined && lb !== blk.typed && blk.hdr === undefined) {
      lInB = true;
      lEs = blk.es;
      lb = blk.typed;
      continue;
    }
    break;
  }
  while (typeof rb === "object" && rb !== null) {
    const ev = globalThis.__rt._element_view_of.get(rb);
    if (ev !== undefined) {
      const elemEs = ev.es !== undefined ? ev.es : rEs;
      ro = rInB && elemEs !== undefined ? ro + ev.off * elemEs : ro + ev.off;
      if (!rInB && ev.es !== undefined && rEs === undefined) rEs = ev.es;
      rb = ev.buf;
      continue;
    }
    if (rKey === undefined && globalThis.__rt._boxed.has(rb)) {
      const held = rb[0];
      const bev = held !== null && typeof held === "object"
        ? globalThis.__rt._element_view_of.get(held)
        : undefined;
      if (bev !== undefined) {
        const elemEs = bev.es !== undefined
          ? bev.es
          : (rEs !== undefined && bev.n > 0 ? rEs / bev.n : rEs);
        ro = rInB && elemEs !== undefined ? ro + bev.off * elemEs : bev.off + ro * bev.n;
        if (!rInB && elemEs !== undefined) rEs = elemEs;
        rb = bev.buf;
        continue;
      }
    }
    const sv = globalThis.__rt._scalar_view_of.get(rb);
    if (sv !== undefined) {
      ro = ro * sv.size + sv.phase;
      rInB = true;
      if (rEs === undefined) rEs = 1;
      rb = sv.buf;
      continue;
    }
    const blk = globalThis.__rt._blk_get(rb);
    if (blk !== undefined && blk.typed !== undefined && rb !== blk.typed && blk.hdr === undefined) {
      rInB = true;
      rEs = blk.es;
      rb = blk.typed;
      continue;
    }
    break;
  }
  if (lb === rb && lKey === rKey && typeof lb !== "number") {
    if (lInB === rInB) return lo === ro;
    const es = lEs !== undefined && lEs > 1 ? lEs : (rEs !== undefined && rEs > 1 ? rEs : 1);
    return (lInB ? lo : lo * es) === (rInB ? ro : ro * es);
  }
  if (typeof lb === "number" || typeof rb === "number") {
    const la = typeof lb === "number" ? lb : globalThis.__rt._dangling_of.get(lb);
    const ra = typeof rb === "number" ? rb : globalThis.__rt._dangling_of.get(rb);
    if (la !== undefined && ra !== undefined) {
      return ((la + lo * (lInB ? 1 : lEs || 1)) >>> 0) ===
        ((ra + ro * (rInB ? 1 : rEs || 1)) >>> 0);
    }
  }
  return false;
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
  if (a === b) {
    globalThis.__rt._named(a, "ptr_cmp");
    return 0;
  }
  let aSize = size;
  let bSize = size;
  while (a !== null && typeof a === "object" && a.pr !== undefined) {
    aSize = a.ps;
    a = a.pr;
  }
  while (b !== null && typeof b === "object" && b.pr !== undefined) {
    bSize = b.ps;
    b = b.pr;
  }
  if (
    a !== null && b !== null && typeof a === "object" && typeof b === "object" &&
    a.buf !== undefined && a.buf === b.buf && typeof a.buf !== "number" &&
    a.sc === b.sc && a.es === b.es &&
    (globalThis.__rt._bv === null || !globalThis.__rt._bv.is_bv(a.buf))
  ) {
    globalThis.__rt._named(a, "ptr_cmp");
    return a.off < b.off ? -1 : a.off > b.off ? 1 : 0;
  }
  const x = globalThis.__rt.addr(a, aSize);
  const y = globalThis.__rt.addr(b, bSize);
  return x < y ? -1 : x > y ? 1 : 0;
};

// Metadata participates in ordering of two pointers to the same unsized type:
// data addresses compare first, and equal data addresses tie-break by metadata
// (`len` for slices/str, vtable or tail metadata for dyn/unsized-tail pointers).
globalThis.__rt.slice_ptr_cmp = (a, b, size) => {
  const d = globalThis.__rt.ptr_cmp(a, b, size);
  return d !== 0 ? d : a.len < b.len ? -1 : a.len > b.len ? 1 : 0;
};
globalThis.__rt.fat_ptr_cmp = (a, b, size) => {
  const d = globalThis.__rt.ptr_cmp(a.ptr, b.ptr, size);
  if (d !== 0) return d;
  if (a.meta === b.meta) return 0;
  const x = globalThis.__rt.addr(a.meta, 1);
  const y = globalThis.__rt.addr(b.meta, 1);
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
  // The reference form of a zero-sized value is the address `_zst_addr` (see
  // `addr`): a zero-sized step leaves it as it is, any other one is arithmetic on
  // that address.
  if (p === undefined) {
    return count === 0 || size === 0 ? p : globalThis.__rt._zst_addr + count * size;
  }
  if (p !== null && typeof p === "object" && typeof p.buf === "number" && p.pr === undefined) {
    const base = p.sc !== undefined ? p.buf + p.off : p.buf + p.off * (p.es !== undefined ? p.es : size);
    return (base + count * (p.sc !== undefined ? 1 : size)) >>> 0;
  }
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
  let buf = p.buf;
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
    if (globalThis.__rt._bv !== null) {
      buf = globalThis.__rt._bv.unwrap_buf(buf);
    }
  }
  if (typeof buf === "object" && buf !== null) {
    const ev = globalThis.__rt._element_view_of.get(buf);
    if (ev !== undefined) {
      buf = ev.buf;
      off += ev.off;
    }
  }
  if (Array.isArray(buf) && off + n > buf.length) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: `" + op + "` over " + n + " elements at " + off +
        " reaches past the end of a buffer of " + buf.length,
    ));
  }
  return { buf: buf, off: off, n: n };
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
// when an element is an aggregate held by value: a byte for byte copy produces
// an independent value, and assigning a JavaScript object would produce an
// alias.
//
// Inside one buffer with nothing to clone, `copyWithin` is the whole operation
// and handles the overlap itself. Otherwise the loop runs away from the
// overlap: upwards when the destination is below the source, downwards when it
// is above. Which store each element gets is `_rebinds`.
globalThis.__rt.copy = (dst, src, count, clone) => {
  if (count === 0) return;
  let effCl = clone, fromNs = false;
  if (clone === undefined && globalThis.__rt._bv !== null) {
    const r = globalThis.__rt._bv.prep_copy_pair(dst, src, count);
    dst = r.dst; src = r.src; fromNs = r.fromNs; effCl = r.cl;
  }
  globalThis.__rt._scales(dst, src, "copy");
  const d = globalThis.__rt._run(dst, count, "copy");
  const s = globalThis.__rt._run(src, count, "copy");
  if (effCl === undefined && d.buf === s.buf && Array.isArray(d.buf)
      && (globalThis.__rt._bv === null || !globalThis.__rt._bv.is_bv(d.buf))) {
    d.buf.copyWithin(d.off, s.off, s.off + s.n);
    return;
  }
  const bv = globalThis.__rt._bv;
  const down = (bv !== null ? bv.unwrap_buf(d.buf) : d.buf) === (bv !== null ? bv.unwrap_buf(s.buf) : s.buf) && d.off > s.off;
  if (effCl === undefined && (fromNs || globalThis.__rt._rebinds(dst, d.buf, s))) {
    if (down) { for (let i = s.n - 1; i >= 0; i--) d.buf[d.off + i] = s.buf[s.off + i]; }
    else { for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i]; }
    return;
  }
  const each = typeof effCl === "function" ? (fromNs ? (v) => (v === null || typeof v !== "object" ? v : effCl(v)) : effCl) : (v) => v;
  const put = globalThis.__rt._put;
  const dIsHeap = globalThis.__rt._blk_get(d.buf) !== undefined;
  if (down) { for (let i = s.n - 1; i >= 0; i--) put(d.buf, d.off + i, each(s.buf[s.off + i]), dIsHeap); }
  else { for (let i = 0; i < s.n; i++) put(d.buf, d.off + i, each(s.buf[s.off + i]), dIsHeap); }
};

// Whether an element move with no `clone` into `buf` (the buffer of the
// destination `dst`, the source run being `s`) rebinds each place, an
// assignment, rather than writing into the object already there (`_put`).
//
// Only an aggregate held BY VALUE is written into its place, and the compiler
// says when the element is one: it hands over `clone`. Every other element is a
// value that names something else -- a reference to an aggregate is that
// aggregate's own object, a raw pointer or a `Box` is a slot record, a `&[T]`
// or a `&dyn Trait` is a fat one -- and writing it into the object in the place
// would overwrite the *pointee*: `[&P; 2].swap(0, 1)` would copy one `P` over
// the other instead of exchanging the two references. Two destinations keep
// `_put` whatever the element is:
//
//   * a `__rt.box` buffer is not a buffer but the one place standing for a
//     local, which still names the object in it (what makes
//     `ptr::copy(.., &mut local, 1)` reach the local). It holds one element, so
//     only a move of one object can be written into it;
//   * a byte view (`sc`) counts bytes of elements that the pointee type -- a
//     `u8` -- does not describe, so the absence of `clone` says nothing about
//     them.
globalThis.__rt._rebinds = (dst, buf, s) => {
  if (dst.sc !== undefined) return false;
  const first = s.buf[s.off];
  return s.n !== 1 || first === null || typeof first !== "object" ||
    !globalThis.__rt._boxed.has(buf);
};

// The one element store the copies share for an aggregate held by value (see
// `_rebinds`), and the one place where storing is not an assignment.
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
globalThis.__rt._put = (buf, off, value, heap) => {
  const old = buf[off];
  if (old !== null && typeof old === "object" && value !== null && typeof value === "object") {
    globalThis.__rt.overwrite(old, value);
    return;
  }
  buf[off] = value;
  // An aggregate stored into a **heap block** remembers where it lives, so that
  // `box` hands back that place rather than inventing a buffer of one.
  //
  // `Box<dyn Trait>` is why. Its data half is the *reference* to the concrete
  // value, which for an aggregate is the object itself, so the slot naming the
  // allocation is not in the fat pointer any more -- and dropping the box has to
  // find it again to free the block. Only a heap block is registered: for every
  // other buffer, a raw pointer made from a bare object is not expected to name
  // the place the object happens to sit in.
  if (
    value !== null && typeof value === "object" &&
    (heap !== undefined ? heap : globalThis.__rt._blk_get(buf) !== undefined) &&
    !globalThis.__rt._boxes.has(value)
  ) {
    globalThis.__rt._boxes.set(value, { buf, off });
  }
};

// copy_nonoverlapping(dst, src, count): the same move, without the overlap, so
// the direction never matters.
globalThis.__rt.copy_nonoverlapping = (dst, src, count, clone) => {
  if (count === 0) return;
  let effCl = clone, fromNs = false;
  if (clone === undefined && globalThis.__rt._bv !== null) {
    const r = globalThis.__rt._bv.prep_copy_pair(dst, src, count);
    dst = r.dst; src = r.src; fromNs = r.fromNs; effCl = r.cl;
  }
  globalThis.__rt._scales(dst, src, "copy_nonoverlapping");
  const d = globalThis.__rt._run(dst, count, "copy_nonoverlapping");
  const s = globalThis.__rt._run(src, count, "copy_nonoverlapping");
  if (effCl === undefined && (fromNs || globalThis.__rt._rebinds(dst, d.buf, s))) {
    for (let i = 0; i < s.n; i++) d.buf[d.off + i] = s.buf[s.off + i];
    return;
  }
  const each = typeof effCl === "function" ? (fromNs ? (v) => (v === null || typeof v !== "object" ? v : effCl(v)) : effCl) : (v) => v;
  const put = globalThis.__rt._put;
  const dIsHeap = globalThis.__rt._blk_get(d.buf) !== undefined;
  for (let i = 0; i < s.n; i++) put(d.buf, d.off + i, each(s.buf[s.off + i]), dIsHeap);
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
  } else if (dst !== null && typeof dst === "object") {
    if (dst.sc !== undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: `write_bytes` of the byte " + byte + " over the byte view of " +
          dst.sc + "-byte non-scalar elements",
      ));
    }
    if (globalThis.__rt._bv !== null) {
      dst = globalThis.__rt._bv.write_bytes(dst, byte, count);
    }
  }
  const d = globalThis.__rt._run(dst, count, "write_bytes");
  if (Array.isArray(d.buf)) {
    d.buf.fill(value, d.off, d.off + d.n);
    return;
  }
  for (let i = 0; i < d.n; i++) d.buf[d.off + i] = value;
};

// _bytes(p): a byte pointer as the byte view of its buffer. Usually it is one
// already: an unscaled slot over bytes, or the scaled slot an `as *const u8`
// cast made of an element pointer. The exception is the pointer a heap block
// was allocated as, which `RawVecInner` keeps as its `*mut u8`: once rustc's GVN
// folds `NonNull<u8> -> *const T -> *const u8` into one cast, `[u16] == [u16]`
// hands that stored pointer to `compare_bytes`. Into a block retyped to `es`-byte
// elements an unscaled pointer is keyed in elements (`_retype`), so its bytes
// are the scaled view `scale` would have made of it.
globalThis.__rt._bytes = (p) => {
  if (p === null || typeof p !== "object" || p.sc !== undefined) return p;
  if (globalThis.__rt._bv !== null) return globalThis.__rt._bv.bytes(p);
  const block = globalThis.__rt._heap_block(p.buf);
  if (block === undefined || block.typed === undefined || p.buf === block.typed || block.hdr !== undefined) {
    return p;
  }
  return { buf: block.typed, off: p.off, sc: block.es };
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
//
// Both pointers are byte pointers, and one may be the pointer a heap block was
// allocated as rather than a byte view of it (`_bytes`).
globalThis.__rt.compare_bytes = (a, b, count) => {
  if (count === 0) return 0;
  a = globalThis.__rt._bytes(a);
  b = globalThis.__rt._bytes(b);
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
  if (p.w === undefined) {
    const held = p.buf[p.off];
    if (held !== null && typeof held === "object") {
      const ev = globalThis.__rt._element_view_of.get(held);
      if (ev !== undefined) return Array.prototype.slice.call(ev.buf, ev.off, ev.off + n);
    }
    return held;
  }
  const buf = globalThis.__rt._bv !== null ? globalThis.__rt._bv.raw_buf(p.buf) : p.buf;
  return Array.prototype.slice.call(buf, p.off, p.off + n);
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
globalThis.__rt.array_ref = (p, n, es) => {
  if (p === null || typeof p !== "object") {
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer does not name an array"));
  }
  if (p.w === undefined) {
    const held = p.buf === undefined ? undefined : p.buf[p.off];
    if (held === undefined || (Array.isArray(held) && held.length === n)) {
      if (es !== undefined && held !== null && typeof held === "object") {
        const ev = globalThis.__rt._element_view_of.get(held);
        if (ev !== undefined && ev.es === undefined) ev.es = es;
      }
      return held;
    }
  }
  const buf = globalThis.__rt._bv !== null ? globalThis.__rt._bv.raw_buf(p.buf) : p.buf;
  return globalThis.__rt._element_view(buf, p.off, n, es);
};

// The element views built so far, keyed by buffer and then by offset and length,
// so that two references to the same `n` elements are the same object and pointer
// identity holds across them.
globalThis.__rt._views = new WeakMap();
globalThis.__rt._element_view_of = new WeakMap();

// _element_view(buf, off, n, es): an array of `n` accessors onto `buf[off .. off + n]`.
globalThis.__rt._element_view = (buf, off, n, es) => {
  if (buf === null || typeof buf !== "object") {
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer names no buffer to take an array out of"));
  }
  const outer = globalThis.__rt._element_view_of.get(buf);
  if (outer !== undefined) {
    buf = outer.buf;
    off += outer.off;
    if (es === undefined) es = outer.es;
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
  if (found !== undefined) {
    if (es !== undefined) {
      const foundEv = globalThis.__rt._element_view_of.get(found);
      if (foundEv !== undefined && foundEv.es === undefined) foundEv.es = es;
    }
    return found;
  }

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
  globalThis.__rt._element_view_of.set(view, { buf, off, n, es });
  views.set(key, view);
  return view;
};

// write_array(p, values): stores `values` element by element at `p`, the write
// half of read_array. Element by element, never a rebind: the elements live in
// somebody else's buffer and every alias of it has to see the new contents.
globalThis.__rt.write_array = (p, values, indirect) => {
  if (p === null || typeof p !== "object") {
    throw __rt.abort_error(new Error("rustc_codegen_js: cannot write through this pointer"));
  }
  if (p.w === undefined) {
    // `_put` rather than `overwrite`: the one place a plain slot names may hold
    // no array yet, which is what a fresh heap block is (`Box::new([1, 2, 3])`).
    globalThis.__rt._put(p.buf, p.off, values);
    return;
  }
  if (p.w === 0 || values === undefined) return;
  const buf = globalThis.__rt._bv !== null ? globalThis.__rt._bv.raw_buf(p.buf) : p.buf;
  if (indirect) {
    const put = globalThis.__rt._put;
    const heap = globalThis.__rt._blk_get(buf) !== undefined;
    for (let i = 0; i < values.length; i++) put(buf, p.off + i, values[i], heap);
    return;
  }
  for (let i = 0; i < values.length; i++) buf[p.off + i] = values[i];
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
  if (typeof p === "number") return globalThis.__rt.slice_from_parts(p, len);
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
globalThis.__rt.erase = (p, size, kind) => {
  if (p === null || typeof p !== "object") return p;
  if (p.sc !== undefined || p.es !== undefined || p.buf === undefined) return p;
  const out = kind === undefined
    ? { buf: p.buf, off: p.off, es: size }
    : { buf: p.buf, off: p.off, es: size, sk: kind };
  if (p.pr !== undefined) {
    out.pr = p.pr;
    out.ps = p.ps;
  }
  if (p.pv !== undefined) {
    out.pv = p.pv;
    out.po = p.po;
  }
  return out;
};

// scale(p, size): the byte view of a pointer to `size`-wide elements, which is
// what `as *const u8` produces -- `{ buf, off * size, sc: size }`, a record that
// is not dereferenceable and exists to be offset in bytes and cast back.
//
// An *address* comes back unchanged, and that is the whole reason this is a
// helper rather than the record literal it used to be: `ptr::null::<i32>()` is
// the number `0`, `is_null` is `self.cast::<u8>().addr() == 0`, and a record
// built out of `(0).buf` names nothing at all.
globalThis.__rt.scale = (p, size, cl) => {
  if (typeof p === "number") return p;
  if (p === null || p === undefined) return 0;
  if (typeof p.buf === "number") return (p.buf + (p.sc !== undefined ? p.off : p.off * size)) >>> 0;
  if (p.buf !== null && typeof p.buf === "object" && typeof p.off === "number" && p.pr === undefined) {
    const ev = globalThis.__rt._element_view_of.get(p.buf);
    if (ev !== undefined) p = { buf: ev.buf, off: ev.off + p.off };
  }
  if (p.pr !== undefined) {
    if (typeof p.pr.off !== "number") {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a nested zero-offset aggregate field has no byte view",
      ));
    }
    if (globalThis.__rt._bv !== null) return globalThis.__rt._bv.prefix_ns(p, size);
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
    return { buf: scalar.buf, off: p.off * size + scalar.phase };
  }
  if (globalThis.__rt._bv !== null) {
    globalThis.__rt._bv.set_cl(p.buf, cl === undefined ? false : cl);
  }
  return { buf: p.buf, off: p.off * size, sc: size };
};

// Aliasing byte views over wider scalar buffers (`u16/i16/u32/i32/u64/i64/u128/i128/f32/f64/char`).
globalThis.__rt._byte_views = new WeakMap();
globalThis.__rt._prefix_byte_views = new WeakMap();
globalThis.__rt._byte_view_of = new WeakMap();
let _has_byte_views = false;
globalThis.__rt._cl_of = new WeakMap();
globalThis.__rt._prefix_of = new WeakMap();

globalThis.__rt._byte_load = (buf, elem, lane, parts, signed) => {
  const v = buf[elem];
  if (v === undefined) return undefined;
  let b;
  if (parts.kind === "f") {
    const dv = _fbuf || (_fbuf = new DataView(new ArrayBuffer(8)));
    if (parts.bits === 32) dv.setFloat32(0, v, true);
    else dv.setFloat64(0, v, true);
    b = dv.getUint8(lane);
  } else if (parts.bits <= 32) {
    b = (v >>> (lane * 8)) & 255;
  } else {
    b = Number((BigInt.asUintN(parts.bits, v) >> BigInt(lane * 8)) & BigInt("255"));
  }
  return signed && b > 127 ? b - 256 : b;
};
let _fbuf = null;

globalThis.__rt._byte_store = (buf, elem, lane, parts, byte) => {
  const b = byte & 255;
  const cur = buf[elem];
  if (parts.kind === "f") {
    const dv = _fbuf || (_fbuf = new DataView(new ArrayBuffer(8)));
    if (parts.bits === 32) {
      dv.setFloat32(0, cur === undefined ? 0 : cur, true);
      dv.setUint8(lane, b);
      buf[elem] = dv.getFloat32(0, true);
    } else {
      dv.setFloat64(0, cur === undefined ? 0 : cur, true);
      dv.setUint8(lane, b);
      buf[elem] = dv.getFloat64(0, true);
    }
    return;
  }
  if (parts.bits <= 32) {
    const shift = lane * 8;
    const raw = (((cur === undefined ? 0 : cur) >>> 0) & ~(255 << shift)) | (b << shift);
    if (parts.bits === 16) {
      buf[elem] = parts.kind === "i" ? (raw << 16) >> 16 : raw & 65535;
    } else {
      buf[elem] = parts.kind === "i" ? raw | 0 : raw >>> 0;
    }
    return;
  }
  const shift = BigInt(lane * 8);
  const curBig = cur === undefined ? BigInt("0") : BigInt.asUintN(parts.bits, cur);
  const next = (curBig & ~(BigInt("255") << shift)) | (BigInt(b) << shift);
  buf[elem] = parts.kind === "i" ? BigInt.asIntN(parts.bits, next) : BigInt.asUintN(parts.bits, next);
};

globalThis.__rt._byte_view = (buf, size, kind, signed, key, pr, ps) => {
  if (_has_byte_views && buf !== null && typeof buf === "object") {
    const inner = globalThis.__rt._byte_view_of.get(buf);
    if (inner !== undefined) buf = inner.buf;
  }
  const parts = globalThis.__rt._scalar_parts(kind);
  if (parts === null || parts.size !== size || buf === null || typeof buf !== "object") {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: cannot form a byte view of " + kind + " storage",
    ));
  }
  const tag = (signed ? "i:" : "u:") + kind + (key === undefined ? "" : ":k:" + key);
  let views;
  let cacheKey = tag;
  if (pr === undefined) {
    views = globalThis.__rt._byte_views.get(buf);
    if (views === undefined) {
      views = new Map();
      globalThis.__rt._byte_views.set(buf, views);
    }
  } else {
    let pmap = globalThis.__rt._prefix_byte_views.get(buf);
    if (pmap === undefined) {
      pmap = new WeakMap();
      globalThis.__rt._prefix_byte_views.set(buf, pmap);
    }
    let top = pr;
    while (top.pr !== undefined) top = top.pr;
    const anchor = typeof top.buf === "object" && top.buf !== null ? top.buf : top;
    views = pmap.get(anchor);
    if (views === undefined) {
      views = new Map();
      pmap.set(anchor, views);
    }
    cacheKey = tag + ":p:" + top.off + ":" + ps;
  }
  let view = views.get(cacheKey);
  if (view !== undefined) return view;
  const load = globalThis.__rt._byte_load;
  const store = globalThis.__rt._byte_store;
  const single = key !== undefined;
  view = new Proxy([], {
    get(target, prop, receiver) {
      if (typeof prop === "string") {
        if (prop === "length") return single ? size : buf.length * size;
        const idx = Number(prop);
        if (Number.isInteger(idx) && idx >= 0 && String(idx) === prop) {
          if (!single && buf[globalThis.__rt._freed] === true) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: this pointer names a heap block that has been freed",
            ));
          }
          const cap = single ? size : buf.length * size;
          if (idx >= cap) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte index " + idx + " is out of bounds of " + cap + "-byte scalar storage",
            ));
          }
          if (single) return load(buf, key, idx, parts, signed);
          const elem = Math.floor(idx / size);
          return load(buf, elem, idx - elem * size, parts, signed);
        }
      }
      return Reflect.get(target, prop, receiver);
    },
    set(target, prop, value, receiver) {
      if (typeof prop === "string") {
        const idx = Number(prop);
        if (Number.isInteger(idx) && idx >= 0 && String(idx) === prop) {
          if (!single && buf[globalThis.__rt._freed] === true) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: this pointer names a heap block that has been freed",
            ));
          }
          const cap = single ? size : buf.length * size;
          if (idx >= cap) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte index " + idx + " is out of bounds of " + cap + "-byte scalar storage",
            ));
          }
          if (single) {
            store(buf, key, idx, parts, value);
            return true;
          }
          const elem = Math.floor(idx / size);
          store(buf, elem, idx - elem * size, parts, value);
          return true;
        }
      }
      return Reflect.set(target, prop, value, receiver);
    },
    has(target, prop) {
      if (typeof prop === "string") {
        const idx = Number(prop);
        if (Number.isInteger(idx) && idx >= 0 && String(idx) === prop) {
          return idx < (single ? size : buf.length * size);
        }
      }
      return Reflect.has(target, prop);
    },
  });
  views.set(cacheKey, view);
  const info = pr === undefined
    ? { buf, size, kind, signed, key }
    : { buf, size, kind, signed, key, pr, ps };
  globalThis.__rt._byte_view_of.set(view, info);
  _has_byte_views = true;
  return view;
};

// Unwraps arbitrary layers of prefix (`pr`), scalar views (`_scalar_view_of`),
// byte views (`_byte_view_of`), prefix markers (`_prefix_of`), and pre-retype
// raw allocation arrays (`block.typed`) down to the underlying backing buffer.
globalThis.__rt._unwrap_view = (p) => {
  let outerPs;
  while (p.pr !== undefined) {
    outerPs = p.ps;
    p = p.pr;
  }
  let buf = p.buf;
  let off = typeof p.off === "number" ? p.off : 0;
  let key = typeof p.off === "number" ? undefined : p.off;
  let inBytes = p.sc !== undefined;
  let es = p.sc !== undefined ? p.sc : (p.es !== undefined ? p.es : outerPs);
  let w = p.w;
  let evEs;
  while (typeof buf === "object" && buf !== null) {
    const ev = globalThis.__rt._element_view_of.get(buf);
    if (ev !== undefined) {
      const elemEs = ev.es !== undefined ? ev.es : es;
      off = inBytes && elemEs !== undefined ? off + ev.off * elemEs : off + ev.off;
      if (ev.es !== undefined) evEs = ev.es;
      buf = ev.buf;
      continue;
    }
    if (key === undefined && globalThis.__rt._boxed.has(buf)) {
      const held = buf[0];
      const bev = held !== null && typeof held === "object"
        ? globalThis.__rt._element_view_of.get(held)
        : undefined;
      if (bev !== undefined) {
        const elemEs = bev.es !== undefined
          ? bev.es
          : (es !== undefined && bev.n > 0 ? es / bev.n : es);
        off = inBytes && elemEs !== undefined ? off + bev.off * elemEs : bev.off + off * bev.n;
        if (elemEs !== undefined) {
          evEs = elemEs;
          if (!inBytes) es = elemEs;
        }
        w = bev.n;
        buf = bev.buf;
        continue;
      }
    }
    const scalar = globalThis.__rt._scalar_view_of.get(buf);
    if (scalar !== undefined) {
      off = off * scalar.size + scalar.phase;
      inBytes = true;
      if (es === undefined) es = 1;
      buf = scalar.buf;
      continue;
    }
    const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(buf) : undefined;
    if (bv !== undefined) {
      if (bv.pr !== undefined) {
        const base = globalThis.__rt._unwrap_view(bv.pr);
        const baseEs = base.es !== undefined ? base.es : bv.ps;
        const baseOff = base.inBytes ? base.off : base.off * baseEs;
        return {
          buf: base.buf,
          off: baseOff + off,
          key: base.key,
          inBytes: true,
          es: baseEs,
        };
      }
      inBytes = true;
      es = bv.size;
      if (bv.key !== undefined) key = bv.key;
      buf = bv.buf;
      continue;
    }
    const pref = globalThis.__rt._prefix_of.get(buf);
    if (pref !== undefined) {
      inBytes = true;
      es = pref.ps;
      buf = pref.buf;
      continue;
    }
    const blk = globalThis.__rt._blk_get(buf);
    if (blk !== undefined && blk.typed !== undefined && buf !== blk.typed && blk.hdr === undefined) {
      inBytes = true;
      es = blk.es;
      buf = blk.typed;
      continue;
    }
    break;
  }
  return { buf, off, key, inBytes, es, w, evEs };
};

// Normalizes byte-granular pointers (scaled non-scalar slots, `_byte_view`,
// pre-retype `raw` heap arrays, and `_uninit_blocks` arrays) for
// `copy`/`copy_nonoverlapping` and extracts the compiler-typed clone policy.
globalThis.__rt._prep_copy = (p, otherSc, otherCl, count) => {
  if (p === null || typeof p !== "object") return { p, cl: undefined, ns: false };
  if (p.sc !== undefined) {
    if (p.pv !== undefined || globalThis.__rt._prefix_of.has(p.buf)) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: cannot byte-copy through a zero-offset aggregate prefix pointer",
      ));
    }
    const clEntry = globalThis.__rt._cl_of.get(p.buf);
    if (clEntry !== undefined) {
      return { p, cl: clEntry === false ? undefined : clEntry, ns: true };
    }
    const blk = globalThis.__rt._blk_get(p.buf);
    if (blk !== undefined) return { p, cl: blk.cl, ns: true };
    return { p, cl: undefined, ns: false };
  }
  const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(p.buf) : undefined;
  if (bv !== undefined) {
    if (bv.buf.length === 0) globalThis.__rt._heap_block(bv.buf);
    return { p, cl: undefined, ns: false };
  }
  if (p.buf !== undefined && p.buf !== null) {
    const block = globalThis.__rt._heap_block(p.buf);
    if (block !== undefined) {
      if (block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined) {
        if (block.sk !== undefined) {
          return {
            p: { buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, false), off: p.off },
            cl: undefined,
            ns: false,
          };
        }
        return { p: { buf: block.typed, off: p.off, sc: block.es }, cl: block.cl, ns: true };
      }
      if (
        block.fresh &&
        otherSc !== undefined &&
        (!block.zeroed || (p.off === 0 && count === block.bytes && block.bytes % otherSc === 0)) &&
        p.off % otherSc === 0 &&
        count % otherSc === 0
      ) {
        const typed = new Array(Math.floor(block.bytes / otherSc));
        p.buf.length = 0;
        block.raw = p.buf;
        block.typed = typed;
        block.es = otherSc;
        block.fresh = false;
        block.zeroed = false;
        if (otherCl !== undefined) block.cl = otherCl;
        globalThis.__rt._blocks.set(typed, block);
        if (_has_minted_addrs) {
          const base = globalThis.__rt._addrs.get(p.buf);
          if (base !== undefined) {
            globalThis.__rt._addrs.set(typed, base);
            const k = globalThis.__rt._addrHeap.get(p.buf);
            if (k !== undefined) {
              globalThis.__rt._addrHeap.set(typed, k);
              globalThis.__rt._addrHeap.delete(p.buf);
            }
          }
        }
        return { p: { buf: typed, off: p.off, sc: otherSc }, cl: otherCl, ns: true };
      }
    } else if (otherSc !== undefined) {
      const ub = globalThis.__rt._uninit_blocks.get(p.buf);
      if (ub !== undefined && ub.typed !== undefined && p.buf !== ub.typed && ub.es === otherSc && ub.sk === undefined) {
        return { p: { buf: ub.typed, off: p.off, sc: ub.es }, cl: ub.cl, ns: true };
      }
    }
  }
  return { p, cl: undefined, ns: false };
};

// Slow-path raw byte read/write over retyped `raw` arrays, scaled non-scalar
// pointers, and freed heap blocks (no eager Proxy required).
globalThis.__rt._bv_read = (buf, off, sc, signed) => {
  if (sc !== undefined) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: cannot view " + sc + "-byte non-scalar elements as a byte slice",
    ));
  }
  const block = buf !== null && buf !== undefined ? globalThis.__rt._heap_block(buf) : undefined;
  if (block !== undefined && block.typed !== undefined && buf !== block.typed && block.hdr === undefined) {
    if (block.sk === undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: cannot view " + block.es + "-byte non-scalar elements as a byte slice",
      ));
    }
    const cap = block.typed.length * block.es;
    if (off < 0 || off >= cap) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: byte index " + off + " is out of bounds of " + cap + "-byte scalar storage",
      ));
    }
    const parts = globalThis.__rt._scalar_parts(block.sk);
    const elem = Math.floor(off / block.es);
    return globalThis.__rt._byte_load(block.typed, elem, off - elem * block.es, parts, signed);
  }
  return buf[off];
};

globalThis.__rt._bv_write = (buf, off, sc, value) => {
  if (sc !== undefined) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: cannot view " + sc + "-byte non-scalar elements as a byte slice",
    ));
  }
  const block = buf !== null && buf !== undefined ? globalThis.__rt._heap_block(buf) : undefined;
  if (block !== undefined && block.typed !== undefined && buf !== block.typed && block.hdr === undefined) {
    if (block.sk === undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: cannot view " + block.es + "-byte non-scalar elements as a byte slice",
      ));
    }
    const cap = block.typed.length * block.es;
    if (off < 0 || off >= cap) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: byte index " + off + " is out of bounds of " + cap + "-byte scalar storage",
      ));
    }
    const parts = globalThis.__rt._scalar_parts(block.sk);
    const elem = Math.floor(off / block.es);
    globalThis.__rt._byte_store(block.typed, elem, off - elem * block.es, parts, value);
    return;
  }
  buf[off] = value;
};

// Installs the lazy wide-byte-view dispatch table into `__rt._bv`. Included in
// extracted shims only when a program reaches wide byte views.
globalThis.__rt._bv_enable = (() => {
  const bareAddr = (u) => {
    const base = typeof u.buf === "number" ? u.buf : globalThis.__rt._dangling_of.get(u.buf);
    if (base === undefined) return undefined;
    if (u.off === 0) return base >>> 0;
    if (u.inBytes) return (base + u.off) >>> 0;
    if (u.es !== undefined) return (base + u.off * u.es) >>> 0;
    return undefined;
  };
  const hooks = {
    is_bv(buf) {
      return _has_byte_views && globalThis.__rt._byte_view_of.has(buf);
    },
    unwrap_buf(buf) {
      if (_has_byte_views) {
        const bv = globalThis.__rt._byte_view_of.get(buf);
        if (bv !== undefined) return bv.buf;
      }
      const pref = globalThis.__rt._prefix_of.get(buf);
      if (pref !== undefined) return pref.buf;
      return buf;
    },
    unwrap_slot(buf, off) {
      if (_has_byte_views) {
        const bv = globalThis.__rt._byte_view_of.get(buf);
        if (bv !== undefined) return { buf: bv.buf, off: off / bv.size };
      }
      return { buf, off };
    },
    set_cl(buf, cl) {
      if (buf !== null && typeof buf === "object") globalThis.__rt._cl_of.set(buf, cl);
    },
    prefix_ns(p, size) {
      const off = p.pr.off * p.ps;
      const marker = [];
      globalThis.__rt._prefix_of.set(marker, { buf: p.pr.buf, ps: p.ps });
      return { buf: marker, off, sc: size, pv: p, po: off };
    },
    slice(p, len, byte) {
      const signed = byte < 0;
      const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(p.buf) : undefined;
      if (bv !== undefined) {
        if (len > 0) {
          const cap = bv.key !== undefined ? bv.size : bv.buf.length * bv.size;
          if (p.off < 0 || p.off + len > cap) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte slice of " + len + " bytes at offset " + p.off +
                " exceeds " + cap + "-byte scalar storage",
            ));
          }
        }
        let buf = p.buf;
        if (bv.signed !== signed) {
          buf = globalThis.__rt._byte_view(bv.buf, bv.size, bv.kind, signed, bv.key, bv.pr, bv.ps);
        }
        return { buf, off: p.off, len };
      }
      if (p.es !== undefined && p.es > 1) {
        if (p.buf !== null && typeof p.buf === "object" && typeof p.off === "number" && p.pr === undefined) {
          const ev = globalThis.__rt._element_view_of.get(p.buf);
          if (ev !== undefined) {
            p = p.sk !== undefined
              ? { buf: ev.buf, off: ev.off + p.off, es: p.es, sk: p.sk }
              : { buf: ev.buf, off: ev.off + p.off, es: p.es };
          }
        }
        if (p.sk === undefined) {
          if (len > 0) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: cannot view " + p.es + "-byte non-scalar elements as a byte slice",
            ));
          }
          return { buf: p.buf, off: typeof p.off === "number" ? p.off * p.es : 0, len: 0 };
        }
        const single = typeof p.off !== "number" || p.pr !== undefined;
        const boff = single ? 0 : p.off * p.es;
        if (len > 0) {
          const cap = single ? p.es : p.buf.length * p.es;
          if (boff < 0 || boff + len > cap) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte slice of " + len + " bytes at offset " + boff +
                " exceeds " + cap + "-byte scalar storage",
            ));
          }
        }
        const view = globalThis.__rt._byte_view(
          p.buf, p.es, p.sk, signed, single ? p.off : undefined, p.pr, p.ps,
        );
        return { buf: view, off: boff, len };
      }
      return { buf: p.buf, off: p.off, len };
    },
    byte_slice_ref(p, signed) {
      const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(p.buf) : undefined;
      if (bv !== undefined) {
        if (p.len > 0) {
          const cap = bv.key !== undefined ? bv.size : bv.buf.length * bv.size;
          if (p.off < 0 || p.off + p.len > cap) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte slice of " + p.len + " bytes at offset " + p.off +
                " exceeds " + cap + "-byte scalar storage",
            ));
          }
        }
        if (bv.signed !== signed) {
          return {
            buf: globalThis.__rt._byte_view(bv.buf, bv.size, bv.kind, signed, bv.key, bv.pr, bv.ps),
            off: p.off,
            len: p.len,
          };
        }
        return p;
      }
      if (p.sc !== undefined) {
        if (p.len > 0) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: cannot view " + p.sc + "-byte non-scalar elements as a byte slice",
          ));
        }
        return p;
      }
      const block = globalThis.__rt._heap_block(p.buf);
      if (block !== undefined) {
        if (block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined) {
          if (block.sk === undefined) {
            if (p.len > 0) {
              throw __rt.abort_error(new Error(
                "rustc_codegen_js: cannot view " + block.es + "-byte non-scalar elements as a byte slice",
              ));
            }
            return p;
          }
          if (p.len > 0) {
            const cap = block.typed.length * block.es;
            if (p.off < 0 || p.off + p.len > cap) {
              throw __rt.abort_error(new Error(
                "rustc_codegen_js: byte slice of " + p.len + " bytes at offset " + p.off +
                  " exceeds " + cap + "-byte scalar storage",
              ));
            }
          }
          return {
            buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, signed),
            off: p.off,
            len: p.len,
          };
        }
        if (block.es === 1) block.fresh = false;
      }
      return p;
    },
    addr_unwrap(p, size) {
      if (typeof p === "object" && !("buf" in p)) {
        const ev = globalThis.__rt._element_view_of.get(p);
        if (ev !== undefined) {
          const elemSize = ev.es !== undefined ? ev.es : (ev.n > 0 ? size / ev.n : size);
          return globalThis.__rt.addr({ buf: ev.buf, off: ev.off }, elemSize);
        }
        if (globalThis.__rt._boxes !== null) {
          const slot = globalThis.__rt._boxes.get(p);
          if (slot !== undefined && slot.buf[slot.off] === p) {
            const block = globalThis.__rt._blk_get(slot.buf);
            return globalThis.__rt.addr(slot, block !== undefined ? block.es : size);
          }
        }
      }
      globalThis.__rt._named(p, "addr");
      return globalThis.__rt._unwrap_view(p);
    },
    ptr_eq(a, b) {
      const aNum = typeof a === "number";
      const bNum = typeof b === "number";
      if (!aNum && !bNum && "buf" in a && "buf" in b) {
        const left = a.pr === undefined ? a : a.pr;
        const right = b.pr === undefined ? b : b.pr;
        if (
          left.buf === right.buf &&
          left.pr === undefined &&
          right.pr === undefined &&
          typeof left.buf !== "number" &&
          a.sc === b.sc &&
          !(_has_byte_views && globalThis.__rt._byte_view_of.has(left.buf))
        ) {
          return left.off === right.off;
        }
      }
      if (aNum || bNum) {
        const num = (aNum ? a : b) >>> 0;
        const obj = aNum ? b : a;
        if (!("buf" in obj)) return false;
        return bareAddr(globalThis.__rt._unwrap_view(obj)) === num;
      }
      const aSlot = "buf" in a;
      const bSlot = "buf" in b;
      if (!aSlot && !bSlot) {
        const aEv = globalThis.__rt._element_view_of.get(a);
        const bEv = globalThis.__rt._element_view_of.get(b);
        if (aEv === undefined && bEv === undefined) return false;
        if (aEv !== undefined && bEv !== undefined) {
          return aEv.buf === bEv.buf && aEv.off === bEv.off;
        }
        const ev = aEv !== undefined ? aEv : bEv;
        const other = aEv !== undefined ? b : a;
        return ev.buf === other && ev.off === 0;
      }
      if (!aSlot) {
        const ev = globalThis.__rt._element_view_of.get(a);
        if (ev !== undefined) {
          a = ev.es !== undefined && ev.es > 1
            ? { buf: ev.buf, off: ev.off, es: ev.es }
            : { buf: ev.buf, off: ev.off };
        } else {
          const slot = globalThis.__rt._boxes !== null ? globalThis.__rt._boxes.get(a) : undefined;
          if (slot === undefined || slot.buf[slot.off] !== a) return false;
          const block = globalThis.__rt._blk_get(slot.buf);
          a = block !== undefined && block.es > 1 ? { buf: slot.buf, off: slot.off, es: block.es } : slot;
        }
      }
      if (!bSlot) {
        const ev = globalThis.__rt._element_view_of.get(b);
        if (ev !== undefined) {
          b = ev.es !== undefined && ev.es > 1
            ? { buf: ev.buf, off: ev.off, es: ev.es }
            : { buf: ev.buf, off: ev.off };
        } else {
          const slot = globalThis.__rt._boxes !== null ? globalThis.__rt._boxes.get(b) : undefined;
          if (slot === undefined || slot.buf[slot.off] !== b) return false;
          const block = globalThis.__rt._blk_get(slot.buf);
          b = block !== undefined && block.es > 1 ? { buf: slot.buf, off: slot.off, es: block.es } : slot;
        }
      }
      const lu = globalThis.__rt._unwrap_view(a);
      const ru = globalThis.__rt._unwrap_view(b);
      const la = bareAddr(lu);
      const ra = bareAddr(ru);
      if (la !== undefined && ra !== undefined) return la === ra;
      if (lu.buf !== ru.buf || lu.key !== ru.key) return false;
      if (lu.inBytes === ru.inBytes) return lu.off === ru.off;
      const es = lu.inBytes
        ? (ru.es !== undefined ? ru.es : ru.evEs !== undefined ? ru.evEs : lu.es)
        : (lu.es !== undefined ? lu.es : lu.evEs !== undefined ? lu.evEs : ru.es);
      if (es === undefined) return lu.off === ru.off;
      const loff = lu.inBytes ? lu.off : lu.off * es;
      const roff = ru.inBytes ? ru.off : ru.off * es;
      return loff === roff;
    },
    prep_copy_pair(dst, src, count) {
      let sp = globalThis.__rt._prep_copy(src, undefined, undefined, count);
      const dp = globalThis.__rt._prep_copy(dst, sp.ns ? sp.p.sc : undefined, sp.cl, count);
      if (!sp.ns && dp.ns) sp = globalThis.__rt._prep_copy(src, dp.p.sc, dp.cl, count);
      return {
        dst: dp.p,
        src: sp.p,
        fromNs: sp.ns || dp.ns,
        cl: sp.cl !== undefined ? sp.cl : dp.cl,
      };
    },
    write_bytes(dst, byte, count) {
      const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(dst.buf) : undefined;
      if (bv === undefined && dst.buf !== undefined && dst.buf !== null) {
        const block = globalThis.__rt._heap_block(dst.buf);
        if (block !== undefined && block.typed !== undefined && dst.buf !== block.typed && block.hdr === undefined) {
          if (block.sk === undefined) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: `write_bytes` of the byte " + byte + " over the byte view of " +
                block.es + "-byte non-scalar elements",
            ));
          }
          return {
            buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, false),
            off: dst.off,
          };
        }
      }
      return dst;
    },
    bytes(p) {
      if (_has_byte_views && globalThis.__rt._byte_view_of.has(p.buf)) return p;
      const block = globalThis.__rt._heap_block(p.buf);
      if (block === undefined || block.typed === undefined || p.buf === block.typed || block.hdr !== undefined) {
        return p;
      }
      if (block.sk !== undefined) {
        return {
          buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, false),
          off: p.off,
        };
      }
      return { buf: block.typed, off: p.off, sc: block.es };
    },
    raw_buf(buf) {
      if (buf !== null && typeof buf === "object" && buf.length === 0) {
        const blk = globalThis.__rt._heap_block(buf);
        if (blk !== undefined && blk.typed !== undefined && buf !== blk.typed && blk.hdr === undefined) {
          if (blk.sk === undefined) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: cannot view " + blk.es + "-byte non-scalar elements as a byte array",
            ));
          }
          return globalThis.__rt._byte_view(blk.typed, blk.es, blk.sk, false);
        }
      }
      return buf;
    },
    unscale_scalar(p, size, mk, kind) {
      const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(p.buf) : undefined;
      if (bv !== undefined) {
        if (bv.buf.length === 0) globalThis.__rt._heap_block(bv.buf);
        if (bv.kind === kind) {
          if (p.pv !== undefined) {
            if (p.off !== p.po) {
              throw __rt.abort_error(new Error(
                "rustc_codegen_js: a byte offset moved outside its zero-offset aggregate field",
              ));
            }
            return p.pv;
          }
          if (bv.key !== undefined && p.off === 0) {
            return bv.pr !== undefined
              ? { buf: bv.buf, off: bv.key, pr: bv.pr, ps: bv.ps }
              : { buf: bv.buf, off: bv.key };
          }
          if (bv.key === undefined && p.off % size === 0) return { buf: bv.buf, off: p.off / size };
        }
        return globalThis.__rt._scalar_view(p, size, kind);
      }
      const block = globalThis.__rt._heap_block(p.buf);
      if (block !== undefined && block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined) {
        if (block.es !== size) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: this heap block already holds " + block.es +
              "-byte elements, so it cannot be read as " + size + "-byte ones",
          ));
        }
        if (block.sk === undefined) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: cannot view " + block.es + "-byte non-scalar elements as " + kind,
          ));
        }
        if (block.sk !== kind || p.off % size !== 0) {
          return globalThis.__rt._scalar_view(
            { buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, false), off: p.off },
            size,
            kind,
          );
        }
        return { buf: block.typed, off: p.off / size };
      }
      return undefined;
    },
    thin(p, size, signed) {
      const bv = _has_byte_views ? globalThis.__rt._byte_view_of.get(p.buf) : undefined;
      if (bv !== undefined) {
        if (size === 1) {
          if (bv.signed !== signed) {
            const view = globalThis.__rt._byte_view(bv.buf, bv.size, bv.kind, signed, bv.key, bv.pr, bv.ps);
            return p.pv !== undefined ? { buf: view, off: p.off, pv: p.pv, po: p.po } : { buf: view, off: p.off };
          }
          return p;
        }
        if (bv.buf.length === 0) globalThis.__rt._heap_block(bv.buf);
        if (bv.size !== size) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: this pointer is a byte view of " + bv.size +
              "-byte elements, not of " + size + "-byte ones",
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
        if (bv.key !== undefined) {
          if (p.off !== 0) {
            throw __rt.abort_error(new Error(
              "rustc_codegen_js: byte offset " + p.off + " leaves its scalar field",
            ));
          }
          return bv.pr !== undefined
            ? { buf: bv.buf, off: bv.key, pr: bv.pr, ps: bv.ps }
            : { buf: bv.buf, off: bv.key };
        }
        if (p.off % size !== 0) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: byte offset " + p.off + " is not a multiple of " +
              size + ", so it does not name an element",
          ));
        }
        return { buf: bv.buf, off: p.off / size };
      }
      if (size === 1 && p.es !== undefined && p.es > 1) {
        if (p.buf !== null && typeof p.buf === "object" && typeof p.off === "number" && p.pr === undefined) {
          const ev = globalThis.__rt._element_view_of.get(p.buf);
          if (ev !== undefined) {
            p = p.sk !== undefined
              ? { buf: ev.buf, off: ev.off + p.off, es: p.es, sk: p.sk }
              : { buf: ev.buf, off: ev.off + p.off, es: p.es };
          }
        }
        const scalar = globalThis.__rt._scalar_view_of.get(p.buf);
        if (scalar !== undefined) {
          const off = p.off * scalar.size + scalar.phase;
          const ibv = _has_byte_views ? globalThis.__rt._byte_view_of.get(scalar.buf) : undefined;
          if (ibv !== undefined && ibv.signed !== signed) {
            return { buf: globalThis.__rt._byte_view(ibv.buf, ibv.size, ibv.kind, signed, ibv.key, ibv.pr, ibv.ps), off };
          }
          return { buf: scalar.buf, off };
        }
        if (p.sk !== undefined) {
          const single = typeof p.off !== "number" || p.pr !== undefined;
          const view = globalThis.__rt._byte_view(
            p.buf, p.es, p.sk, signed, single ? p.off : undefined, p.pr, p.ps,
          );
          if (p.pr !== undefined) return { buf: view, off: 0, pv: p, po: 0 };
          return { buf: view, off: single ? 0 : p.off * p.es };
        }
        return { buf: p.buf, off: typeof p.off === "number" ? p.off * p.es : 0, sc: p.es };
      }
      if ("buf" in p && p.buf !== undefined && p.buf !== null) {
        const block = globalThis.__rt._blk_get(p.buf);
        if (block !== undefined && block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined) {
          if (size === 1) {
            if (block.sk !== undefined) {
              return { buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, signed), off: p.off };
            }
            return { buf: block.typed, off: p.off, sc: block.es };
          }
          if (p.es === undefined) {
            if (block.es !== size) {
              throw __rt.abort_error(new Error(
                "rustc_codegen_js: this heap block already holds " + block.es +
                  "-byte elements, so it cannot be read as " + size + "-byte ones",
              ));
            }
            if (p.off % size !== 0) {
              throw __rt.abort_error(new Error(
                "rustc_codegen_js: byte offset " + p.off + " is not a multiple of " +
                  size + ", so it does not name an element",
              ));
            }
            return { buf: block.typed, off: p.off / size };
          }
        }
      }
      return undefined;
    },
  };
  globalThis.__rt._bv = hooks;
  return hooks;
})();

globalThis.__rt.read_byte = (p, signed) => {
  globalThis.__rt._bv_enable;
  if (p.sc === undefined) {
    const v = p.buf[p.off];
    if (v !== undefined) return v;
  }
  return globalThis.__rt._bv_read(p.buf, p.off, p.sc, !!signed);
};

globalThis.__rt.write_byte = (p, value) => {
  globalThis.__rt._bv_enable;
  if (p.sc === undefined && p.buf.length !== 0) {
    p.buf[p.off] = value;
    return;
  }
  globalThis.__rt._bv_write(p.buf, p.off, p.sc, value);
};

globalThis.__rt.read_byte_at = (buf, off, signed) => {
  globalThis.__rt._bv_enable;
  const v = buf[off];
  if (v !== undefined) return v;
  return globalThis.__rt._bv_read(buf, off, undefined, !!signed);
};

globalThis.__rt.write_byte_at = (buf, off, value) => {
  globalThis.__rt._bv_enable;
  if (buf.length !== 0) {
    buf[off] = value;
    return;
  }
  globalThis.__rt._bv_write(buf, off, undefined, value);
};

globalThis.__rt.scale_scalar = (p, size, kind, signed) => {
  globalThis.__rt._bv_enable;
  if (typeof p === "number") return p;
  if (p === null || p === undefined) return 0;
  if (typeof p.buf === "number") return (p.buf + (p.sc !== undefined ? p.off : p.off * size)) >>> 0;
  if (p.buf !== null && typeof p.buf === "object" && typeof p.off === "number" && p.pr === undefined) {
    const ev = globalThis.__rt._element_view_of.get(p.buf);
    if (ev !== undefined) p = { buf: ev.buf, off: ev.off + p.off };
  }
  if (p.pr !== undefined) {
    const view = globalThis.__rt._byte_view(p.buf, size, kind, !!signed, p.off, p.pr, p.ps);
    return { buf: view, off: 0, pv: p, po: 0 };
  }
  const scalar = globalThis.__rt._scalar_view_of.get(p.buf);
  if (scalar !== undefined) {
    if (scalar.size !== size) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: a " + scalar.size + "-byte scalar view cannot be scaled as " +
          size + "-byte elements",
      ));
    }
    return { buf: scalar.buf, off: p.off * size + scalar.phase };
  }
  if (typeof p.off !== "number") {
    return { buf: globalThis.__rt._byte_view(p.buf, size, kind, !!signed, p.off), off: 0 };
  }
  return { buf: globalThis.__rt._byte_view(p.buf, size, kind, !!signed), off: p.off * size };
};

// Switches signedness (`u8` <-> `i8`) on a wider-scalar byte view (`_byte_view`
// or retyped allocation raw pointer), leaving ordinary byte buffers unchanged.
globalThis.__rt.byte_sign = (p, signed) => {
  globalThis.__rt._bv_enable;
  if (p !== null && typeof p === "object") {
    if (_has_byte_views) {
      const bv = globalThis.__rt._byte_view_of.get(p.buf);
      if (bv !== undefined && bv.signed !== signed) {
        const view = globalThis.__rt._byte_view(bv.buf, bv.size, bv.kind, signed, bv.key, bv.pr, bv.ps);
        return p.pv !== undefined ? { buf: view, off: p.off, pv: p.pv, po: p.po } : { buf: view, off: p.off };
      }
    }
    const block = p.buf !== undefined && p.buf !== null ? globalThis.__rt._blk_get(p.buf) : undefined;
    if (block !== undefined && block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined && block.sk !== undefined) {
      return { buf: globalThis.__rt._byte_view(block.typed, block.es, block.sk, signed), off: p.off };
    }
  }
  return p;
};

// Aliasing scalar views over buffers whose elements really are bytes. Numeric properties on the
// view are accessors, so the compiler's ordinary `p.buf[p.off]` load/store spelling continues to
// work while every read and write observes the original byte buffer. Views are cached per source
// buffer, scalar kind and phase (the byte offset modulo the scalar's width), preserving pointer
// identity for repeated casts.
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

// A byte offset that is not a multiple of the width is as good a place for a scalar as any other:
// `read_unaligned`, and the first and last words of `<[u8]>::is_ascii`, read one there. Its view
// is the one whose element `i` starts at byte `i * size + phase`, so an unaligned pointer is an
// element too, and the strict offset (`off + n`) moves it by whole scalars in the same view.
globalThis.__rt._scalar_view = (p, size, kind) => {
  const parts = globalThis.__rt._scalar_parts(kind);
  if (parts === null || parts.size !== size || !Number.isInteger(p.off) || p.off < 0) {
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
  const phase = p.off % size;
  const key = phase === 0 ? kind : kind + "@" + phase;
  let view = views.get(key);
  if (view === undefined) {
    view = [];
    views.set(key, view);
    globalThis.__rt._scalar_view_of.set(view, { buf: p.buf, size, kind, phase });
  }
  // Reallocation preserves the byte buffer's identity and can add new words.
  for (let i = view.length; i < Math.floor((p.buf.length - phase) / size); i++) {
    const at = i * size + phase;
    Object.defineProperty(view, i, {
      get: () => globalThis.__rt._scalar_load(p.buf, at, parts),
      set: (value) => globalThis.__rt._scalar_store(p.buf, at, parts, value),
      enumerable: true,
      configurable: false,
    });
  }
  return { buf: view, off: (p.off - phase) / size };
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
globalThis.__rt.unscale = (p, size, mk, cl) => {
  // The way back for an address, which [`scale`] let through unchanged --
  // except for the one address that is not an address. See `_dangling`.
  if (typeof p === "number") {
    return p > 0 && p < 4096 ? globalThis.__rt._dangling(p) : p;
  }
  if (p !== null && typeof p === "object" && typeof p.buf === "number" && p.pr === undefined) {
    const addr = (p.buf + (typeof p.off === "number" ? p.off : 0)) >>> 0;
    return addr > 0 && addr < 4096 ? globalThis.__rt._dangling(addr) : addr;
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
    const buf = globalThis.__rt._bv !== null ? globalThis.__rt._bv.unwrap_buf(p.buf) : p.buf;
    return { buf, off: p.off / size };
  }
  const rawBuf = globalThis.__rt._bv !== null ? globalThis.__rt._bv.unwrap_buf(p.buf) : p.buf;
  const block = globalThis.__rt._heap_block(rawBuf);
  if (block !== undefined) return globalThis.__rt._retype(p, block, size, mk, undefined, cl);
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
  if (p !== null && typeof p === "object" && p.sc === undefined && typeof p.buf !== "number") {
    if (globalThis.__rt._bv !== null) {
      const hit = globalThis.__rt._bv.unscale_scalar(p, size, mk, kind);
      if (hit !== undefined) return hit;
    }
    const block = globalThis.__rt._heap_block(p.buf);
    if (block === undefined || (block.es === 1 && !block.fresh)) {
      return globalThis.__rt._scalar_view(p, size, kind);
    }
    return globalThis.__rt._retype(p, block, size, mk, kind);
  }
  return globalThis.__rt.unscale(p, size, mk);
};

// thin(p, size, signed): the pointer `from_raw_parts` answers with when the
// metadata is `()`, which is to say no rebuild at all -- a thin pointer *is*
// its data pointer, over `size`-wide elements.
//
// All of the work is in the erasure that data pointer arrived through.
// `from_raw_parts` takes a `*const impl Thin`, and `with_metadata_of` -- how
// `byte_offset`, `byte_add` and `byte_sub` are written -- hands it
// `self as *const ()`. So the value can be the scaled byte slot that
// `cast::<u8>()` produced, which has to be keyed back into elements, or a plain
// slot that is already one, or the reference form of an aggregate, or an address.
// Only the record knows which, so it is asked rather than guessed from the type,
// the same way `unwindow` is.
globalThis.__rt.thin = (p, size, signed) => {
  if (p === null || typeof p !== "object") return p;
  if (typeof p.buf === "number" && p.pr === undefined) {
    const step = p.sc !== undefined ? 1 : p.es !== undefined ? p.es : size;
    return (p.buf + (typeof p.off === "number" ? p.off * step : 0)) >>> 0;
  }
  // A byte view: key it back, which throws unless the offset lands on an element.
  // No zero factory: a byte view is never a fresh block, so no retype can happen
  // here and there is nothing to fill.
  if (p.sc !== undefined) return size === 1 ? p : globalThis.__rt.unscale(p, size, null);
  if (globalThis.__rt._bv !== null) {
    const hit = globalThis.__rt._bv.thin(p, size, !!signed);
    if (hit !== undefined) return hit;
  }
  // Already a slot -- including the provenance-less record `_named` reports on,
  // which stays as it is so that the operation that cannot answer says so.
  if ("buf" in p) {
    if (p.es !== undefined && size > 1 && p.es === size && p.pr === undefined && p.pv === undefined) {
      return p.w !== undefined ? { buf: p.buf, off: p.off, w: p.w } : { buf: p.buf, off: p.off };
    }
    if (p.buf !== undefined && p.buf !== null && p.es === undefined) {
      const block = globalThis.__rt._blk_get(p.buf);
      if (block !== undefined && block.typed !== undefined && p.buf !== block.typed && block.hdr === undefined) {
        if (block.es !== size) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: this heap block already holds " + block.es +
              "-byte elements, so it cannot be read as " + size + "-byte ones",
          ));
        }
        if (p.off % size !== 0) {
          throw __rt.abort_error(new Error(
            "rustc_codegen_js: byte offset " + p.off + " is not a multiple of " +
              size + ", so it does not name an element",
          ));
        }
        return { buf: block.typed, off: p.off / size };
      }
    }
    return p;
  }
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
  if (Array.isArray(p)) {
    const ev = globalThis.__rt._element_view_of.get(p);
    if (ev !== undefined) return { buf: ev.buf, off: ev.off };
    return { buf: p, off: 0 };
  }
  if (p.w === undefined) {
    const held = p.buf[p.off];
    if (held !== null && typeof held === "object") {
      const ev = globalThis.__rt._element_view_of.get(held);
      if (ev !== undefined) return { buf: ev.buf, off: ev.off };
    }
    return { buf: held, off: 0 };
  }
  return { buf: p.buf, off: p.off };
};

// The `str` hybrid. A `&str` is a JavaScript string, and its byte view is
// produced on demand by the helpers below. The wrappers construct their native
// codec on first use, so importing a package never needs browser codecs.
globalThis.__rt._encoder = {
  value: null,
  encode(value) {
    if (typeof TextEncoder === "function") {
      return (this.value || (this.value = new TextEncoder())).encode(value);
    }
    const buf = new Uint8Array(3 * value.length);
    const len = this.into(value, buf);
    return buf.subarray(0, len);
  },
  // encodeInto: the bytes written into `target`, which the caller sized to hold
  // all of them (three bytes per UTF-16 code unit is always enough). An engine
  // whose encoder predates `encodeInto` encodes and copies.
  into(value, target) {
    if (typeof TextEncoder === "function") {
      const encoder = this.value || (this.value = new TextEncoder());
      if (typeof encoder.encodeInto === "function") return encoder.encodeInto(value, target).written;
      const bytes = encoder.encode(value);
      target.set(bytes);
      return bytes.length;
    }
    let out = 0;
    const n = value.length;
    for (let i = 0; i < n; i++) {
      const c = value.charCodeAt(i);
      if (c < 0x80) {
        target[out++] = c;
      } else if (c < 0x800) {
        target[out++] = 0xc0 | (c >> 6);
        target[out++] = 0x80 | (c & 0x3f);
      } else if ((c & 0xfc00) === 0xd800 && i + 1 < n && (value.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
        const cp = 0x10000 + (((c & 0x3ff) << 10) | (value.charCodeAt(++i) & 0x3ff));
        target[out++] = 0xf0 | (cp >> 18);
        target[out++] = 0x80 | ((cp >> 12) & 0x3f);
        target[out++] = 0x80 | ((cp >> 6) & 0x3f);
        target[out++] = 0x80 | (cp & 0x3f);
      } else if ((c & 0xf800) === 0xd800) {
        target[out++] = 0xef;
        target[out++] = 0xbf;
        target[out++] = 0xbd;
      } else {
        target[out++] = 0xe0 | (c >> 12);
        target[out++] = 0x80 | ((c >> 6) & 0x3f);
        target[out++] = 0x80 | (c & 0x3f);
      }
    }
    return out;
  },
};
globalThis.__rt._decoder = {
  value: null,
  decode(value) {
    if (typeof TextDecoder === "function") {
      return (this.value || (this.value = new TextDecoder("utf-8", { ignoreBOM: true }))).decode(value);
    }
    let out = "";
    const codes = [];
    for (let i = 0, n = value.length; i < n;) {
      const b0 = value[i++];
      if (b0 < 0x80) {
        codes.push(b0);
      } else {
        let cp = 0xfffd;
        const b1 = i < n ? value[i] : -1;
        if (b0 >= 0xc2 && b0 <= 0xdf && (b1 & 0xc0) === 0x80) {
          i++;
          cp = ((b0 & 0x1f) << 6) | (b1 & 0x3f);
        } else if (
          b0 >= 0xe0 && b0 <= 0xef &&
          b1 >= (b0 === 0xe0 ? 0xa0 : 0x80) && b1 <= (b0 === 0xed ? 0x9f : 0xbf)
        ) {
          i++;
          const b2 = i < n ? value[i] : -1;
          if ((b2 & 0xc0) === 0x80) {
            i++;
            cp = ((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f);
          }
        } else if (
          b0 >= 0xf0 && b0 <= 0xf4 &&
          b1 >= (b0 === 0xf0 ? 0x90 : 0x80) && b1 <= (b0 === 0xf4 ? 0x8f : 0xbf)
        ) {
          i++;
          const b2 = i < n ? value[i] : -1;
          if ((b2 & 0xc0) === 0x80) {
            i++;
            const b3 = i < n ? value[i] : -1;
            if ((b3 & 0xc0) === 0x80) {
              i++;
              cp = (((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f)) - 0x10000;
              codes.push(0xd800 | (cp >> 10));
              cp = 0xdc00 | (cp & 0x3ff);
            }
          }
        }
        codes.push(cp);
      }
      if (codes.length >= 4096) {
        out += String.fromCharCode.apply(null, codes);
        codes.length = 0;
      }
    }
    if (codes.length > 0) out += String.fromCharCode.apply(null, codes);
    return out;
  },
};

// _utf8(s): the UTF-8 bytes of a JavaScript string as a fresh plain array,
// exactly the bytes `TextEncoder` produces -- a lone surrogate is U+FFFD.
//
// A short ASCII string is built straight from its code units: a call into the
// native encoder costs more than the whole loop. Anything else is encoded
// natively and copied element by element into an array allocated at its final
// length, which is an order of magnitude cheaper than handing the typed array to
// `Array.prototype.slice` or `Array.from`. Strings of up to 4096 code units are
// encoded into one reused 12 KiB scratch buffer, so they allocate only the
// result; a longer one allocates a transient `Uint8Array`.
globalThis.__rt._utf8_scratch = null;
globalThis.__rt._utf8 = (s) => {
  const n = s.length;
  if (n <= 48) {
    const bytes = new Array(n);
    let i = 0;
    for (; i < n; i++) {
      const c = s.charCodeAt(i);
      if (c > 0x7f) break;
      bytes[i] = c;
    }
    if (i === n) return bytes;
  }
  let source;
  let len;
  if (n <= 4096) {
    source = globalThis.__rt._utf8_scratch ||
      (globalThis.__rt._utf8_scratch = new Uint8Array(3 * 4096));
    len = globalThis.__rt._encoder.into(s, source);
  } else {
    source = globalThis.__rt._encoder.encode(s);
    len = source.length;
  }
  const bytes = new Array(len);
  for (let i = 0; i < len; i++) bytes[i] = source[i];
  return bytes;
};

// The byte-view memo: string -> the `{ buf, off: 0, len }` record `str_bytes`
// returned for it.
//
// A `&str` sliced out of a longer one is encoded again on every slice, because
// `&s[a..b]` is `s.as_bytes()` plus a decode: a parser walking a document asks
// for the document's bytes once per token. The memo makes that one encode per
// operation instead of one per token, which is the difference between linear
// and quadratic. Its lifetime is the operation and its size is bounded:
//
//   * it is emptied when the JavaScript stack unwinds -- the first insertion
//     queues a microtask that clears it, so no entry outlives the synchronous
//     run of Rust code (an export, a callback, one poll of a future) that made
//     it. Without `queueMicrotask` only the bound below applies;
//   * it holds at most `budget` (4 MiB) of encoded bytes; the insertion that
//     would exceed it empties it first;
//   * a string longer than `budget / 4` is held apart from that budget, and at
//     most the two most recent such strings are, so one large document that
//     every slice re-reads is not evicted by its own slices.
//
// Strings are immutable, so an entry can never be stale: a `String` that is
// mutated derefs to a different JavaScript string. Two equal strings share one
// record, which is as good as two everywhere but pointer identity -- and
// CONTRACT.md documents identity across independent calls as not guaranteed.
// The record and its buffer are never handed to an owner that mutates them:
// the converters that build an owned `String` from a host string copy.
globalThis.__rt._str_memo = { map: new Map(), bytes: 0, huge: [], armed: false, budget: 4194304 };

globalThis.__rt._str_memo_clear = () => {
  const memo = globalThis.__rt._str_memo;
  memo.map.clear();
  memo.bytes = 0;
  memo.huge.length = 0;
  memo.armed = false;
  _sub_reset();
};

function _arm_str_memo() {
  const memo = globalThis.__rt._str_memo;
  if (!memo.armed) {
    memo.armed = true;
    if (typeof queueMicrotask === "function") queueMicrotask(globalThis.__rt._str_memo_clear);
  }
}

// str_bytes(s): the UTF-8 bytes of a JavaScript string as `{ buf, off, len }`,
// which is `str::as_bytes` and `str::as_ptr`.
//
// `buf` is a plain JavaScript array, not the `Uint8Array` the encoder returns,
// because that is what every other `[u8]` in the emitted program is: a slice
// built from a Rust array literal is a plain array, and the two have to be the
// same kind of thing for `__rt.copy`, `__rt.overwrite` and a bare `p.buf[p.off]`
// write to work on both.
//
// A repeated call within one operation returns the *same* record (see the memo
// above), so `s.as_bytes().as_ptr() == s.as_bytes().as_ptr()` holds in the
// shapes a program actually writes and a byte slice compared with itself takes
// the fast path. Correctness never depends on it.
globalThis.__rt.str_bytes = (s) => {
  if (s !== "") {
    if (s === _sub_k0) return _sub_v0;
    if (s === _sub_k1) return _sub_v1;
    if (s === _sub_k2) return _sub_v2;
    if (s === _sub_k3) return _sub_v3;
  }
  const memo = globalThis.__rt._str_memo;
  let record = memo.map.get(s);
  if (record !== undefined) return record;
  const bytes = globalThis.__rt._utf8(s);
  record = { buf: bytes, off: 0, len: bytes.length };
  (globalThis.__rt._str_memo_bufs || (globalThis.__rt._str_memo_bufs = new WeakSet())).add(bytes);
  if (bytes.length === s.length) {
    (_ascii_src || (_ascii_src = new WeakMap())).set(bytes, s);
  }
  if (_sub_arm === undefined) _sub_arm = _arm_str_memo;
  _arm_str_memo();
  if (bytes.length > memo.budget / 4) {
    if (memo.huge.length === 2) {
      memo.map.delete(memo.huge.shift());
      _sub_reset();
    }
    memo.huge.push(s);
  } else {
    if (memo.bytes + bytes.length > memo.budget) {
      const huge = memo.huge.map((key) => memo.map.get(key));
      memo.map.clear();
      memo.huge.forEach((key, i) => memo.map.set(key, huge[i]));
      memo.bytes = 0;
      _sub_reset();
    }
    memo.bytes += bytes.length;
  }
  memo.map.set(s, record);
  return record;
};

// str_bytes_mut(s): `str::as_bytes_mut` and `as_mut_ptr` -- the same bytes as
// `str_bytes`, in a fresh record that is never memoized. A `&mut str` has no
// mutable JavaScript form (a write through it is lost; CONTRACT.md), and the
// `&mut str` a `String` derefs to is a decoded copy, so the one thing this owes
// is containment: the write must not land in the view an equal string shares.
globalThis.__rt.str_bytes_mut = (s) => {
  const bytes = globalThis.__rt._utf8(s);
  return { buf: bytes, off: 0, len: bytes.length };
};

globalThis.__rt._str_memo_bufs = null;

// detach_str_memo(p): when a `*const str` backed by a memoized `&str` byte view
// (`str_bytes`, `bstr_bytes`, `bstr_utf8_bytes`) crosses a const->mut boundary
// (`*const str as *mut str`, `&mut *p_const`, `NonNull::from(&str).as_mut()`),
// detach a fresh copy so writes cannot corrupt `_str_memo` / `_bstr_memo` for
// unrelated uses of the same string. Live heap blocks (`Box<str>`, `String`),
// stack arrays, and already-detached buffers are never in `_str_memo_bufs` and
// pass through unchanged.
globalThis.__rt.detach_str_memo = (p) => {
  if (
    globalThis.__rt._str_memo_bufs !== null &&
    typeof p === "object" &&
    p !== null &&
    globalThis.__rt._str_memo_bufs.has(p.buf)
  ) {
    return {
      buf: p.buf.slice(p.off, p.off + p.len),
      off: 0,
      len: p.len,
    };
  }
  return p;
};

// str_len(s): the UTF-8 byte length of a JavaScript string, which is the
// metadata of a `&str` and `str::len`.
//
// Nothing is encoded: a code unit below 0x80 is one byte, below 0x800 two, a
// surrogate pair four, and any other unit three -- including a lone surrogate,
// which the encoder replaces with the three-byte U+FFFD. A string long enough
// for the scan to cost more than a lookup is answered from the byte-view memo
// when an earlier `str_bytes` already has it, which keeps a length asked once
// per token of a long document from costing a scan each time.
globalThis.__rt.str_len = (s) => {
  const n = s.length;
  if (n > 16) {
    if (s === _sub_k0) return _sub_v0.len;
    if (s === _sub_k1) return _sub_v1.len;
    if (s === _sub_k2) return _sub_v2.len;
    if (s === _sub_k3) return _sub_v3.len;
  }
  const map = globalThis.__rt._str_memo.map;
  if (n > 64 && map.size !== 0) {
    const record = map.get(s);
    if (record !== undefined) return record.len;
  }
  let len = n;
  for (let i = 0; i < n; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) continue;
    if (c < 0x800) {
      len += 1;
    } else if ((c & 0xfc00) === 0xd800 && i + 1 < n && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
      len += 2;
      i++;
    } else {
      len += 2;
    }
  }
  return len;
};

// bytes_str(buf, off, len): decodes the `len` UTF-8 bytes at `buf[off]` back
// into a JavaScript string, which is `str::from_utf8_unchecked` and a raw
// pointer whose pointee is `str`.
//
// The bytes are a plain array (see str_bytes), which the decoder does not take,
// so they are copied into a `Uint8Array` first; a typed array that arrived from
// somewhere else is viewed directly. The copy is also what reads an `i8`
// element as its byte: a store into a `Uint8Array` wraps a negative number.
//
// The ASCII fast path is what a `String` built out of `alloc` actually hits:
// every byte below 0x80 is its own code point, so the string is assembled
// directly and neither the copy nor the decoder runs. `String.fromCharCode`
// takes a bounded run of arguments, so the loop chunks; above the chunk size,
// or at the first byte that is not ASCII (a negative `i8` included), the
// decoder takes over.
globalThis.__rt._ASCII_CHUNK = 4096;
let _ascii_src;
let _sub_k0 = "";
let _sub_v0;
let _sub_k1 = "";
let _sub_v1;
let _sub_k2 = "";
let _sub_v2;
let _sub_k3 = "";
let _sub_v3;
let _sub_i = 0;
let _sub_arm;
function _sub_reset() {
  _sub_k0 = _sub_k1 = _sub_k2 = _sub_k3 = "";
  _sub_v0 = _sub_v1 = _sub_v2 = _sub_v3 = undefined;
}
globalThis.__rt.bytes_str = (buf, off, len) => {
  // A zero length `str` reads nothing, and the buffer it names may not exist at
  // all: `let none: [u8; 0] = []` is a zero sized place, whose JavaScript value
  // is `undefined`.
  if (len === 0) return "";
  if (_ascii_src !== undefined && typeof buf === "object" && buf !== null) {
    const src = _ascii_src.get(buf);
    if (src !== undefined && off >= 0 && len >= 0 && off + len <= src.length) {
      if (off === 0 && len === src.length) return src;
      const sub = src.slice(off, off + len);
      if (_sub_arm !== undefined) _sub_arm();
      const slot = { buf, off, len };
      if (_sub_i === 0) {
        _sub_k0 = sub;
        _sub_v0 = slot;
      } else if (_sub_i === 1) {
        _sub_k1 = sub;
        _sub_v1 = slot;
      } else if (_sub_i === 2) {
        _sub_k2 = sub;
        _sub_v2 = slot;
      } else {
        _sub_k3 = sub;
        _sub_v3 = slot;
      }
      _sub_i = (_sub_i + 1) & 3;
      return sub;
    }
  }
  const typed = buf instanceof Uint8Array;
  if (len <= globalThis.__rt._ASCII_CHUNK) {
    let i = 0;
    while (i < len && (buf[off + i] & 0x80) === 0) i++;
    if (i === len) {
      return String.fromCharCode.apply(
        null,
        typed ? buf.subarray(off, off + len)
          : (off === 0 && len === buf.length && Array.isArray(buf)
            ? buf : Array.prototype.slice.call(buf, off, off + len)),
      );
    }
  }
  let bytes;
  if (typed) {
    bytes = buf.subarray(off, off + len);
  } else {
    bytes = new Uint8Array(len);
    for (let i = 0; i < len; i++) bytes[i] = buf[off + i];
  }
  return globalThis.__rt._decoder.decode(bytes);
};

// ---------------------------------------------------------------------------
// Byte strings (`-Cllvm-args=js-str=bytes`, experimental)
// ---------------------------------------------------------------------------
//
// Under the link-time switch `js-str=bytes` a Rust `str` is a *byte string*: a
// JavaScript string whose code unit `i` is byte `i` of the UTF-8 text, so every
// unit is below 256, `length` is the Rust length and a byte offset is an index
// (js_of_ocaml holds OCaml strings the same way). A program linked that way
// never calls the UTF-16 helpers above; it calls these. Only two of them are
// codecs, and they run where Rust text meets the host: `bstr_in` (host text
// becoming a Rust `str`) and `bstr_out` (Rust text reaching the host). Every
// other conversion is a copy of code units.

// _bstr_units(buf, off, len): the byte string of `len` bytes at `buf[off]`, a
// plain array or a typed array. An `i8` element is read as its byte.
// `String.fromCharCode` takes a bounded run of arguments, so long runs chunk.
globalThis.__rt._bstr_units = (buf, off, len) => {
  if (len <= 0) return "";
  let out = "";
  for (let at = 0; at < len; at += 4096) {
    const end = Math.min(len, at + 4096);
    const codes = new Array(end - at);
    for (let i = at; i < end; i++) codes[i - at] = buf[off + i] & 0xff;
    out += String.fromCharCode.apply(null, codes);
  }
  return out;
};

// bstr_in(s): host text as the byte string of its UTF-8 bytes -- exactly the
// bytes `TextEncoder` writes, so a lone surrogate is U+FFFD (EF BF BD). ASCII
// text is its own byte string and comes back as is.
globalThis.__rt.bstr_in = (s) => {
  if (globalThis.__rt._bstr_is_ascii(s)) return s;
  const bytes = globalThis.__rt._encoder.encode(s);
  return globalThis.__rt._bstr_units(bytes, 0, bytes.length);
};

// _bstr_is_ascii(s): fast ASCII predicate. Short strings (< 24 code units) run
// a charCodeAt loop to avoid JIT->RegExp C++ transitions; longer strings use the
// engine's SIMD regexp scan.
globalThis.__rt._bstr_is_ascii = (s) => {
  const n = s.length;
  if (n < 24) {
    for (let i = 0; i < n; i++) if (s.charCodeAt(i) > 127) return false;
    return true;
  }
  return !_bstr_wide.test(s);
};
const _bstr_wide = /[^\x00-\x7f]/;

// bytes_is_ascii(s): `<[u8]>::is_ascii` on a `{ buf, off, len }` byte slice, one byte at a time
// (`js-str=bytes`), where `core` reads a JavaScript array a word at a time through scalar views.
// A byte is tested the way `core`'s own byte loop tests it, `b < 128`.
globalThis.__rt.bytes_is_ascii = (s) => {
  const buf = s.buf;
  for (let i = s.off, end = s.off + s.len; i < end; i++) if (!(buf[i] < 128)) return false;
  return true;
};

// bstr_out(s): a byte string as the host text it encodes. A byte string is
// always well-formed UTF-8 (Rust's invariant), so ASCII text comes back as is;
// non-ASCII strings <= 48 bytes decode in pure JS (zero Uint8Array /
// TextDecoder C++ call), 49..=1024 reuse a shared scratch Uint8Array, and larger
// strings allocate a fresh Uint8Array.
globalThis.__rt.bstr_out = (s) => {
  if (globalThis.__rt._bstr_is_ascii(s)) return s;
  const n = s.length;
  if (n <= 48) {
    let out = "";
    let i = 0;
    while (i < n) {
      const start = i;
      while (i < n && s.charCodeAt(i) < 0x80) i++;
      if (i > start) out += s.slice(start, i);
      if (i >= n) break;
      const b0 = s.charCodeAt(i);
      if (b0 < 0xe0) {
        out += String.fromCharCode(((b0 & 0x1f) << 6) | (s.charCodeAt(i + 1) & 0x3f));
        i += 2;
      } else if (b0 < 0xf0) {
        out += String.fromCharCode(
          ((b0 & 0x0f) << 12) | ((s.charCodeAt(i + 1) & 0x3f) << 6) | (s.charCodeAt(i + 2) & 0x3f),
        );
        i += 3;
      } else {
        const cp =
          (((b0 & 0x07) << 18) |
            ((s.charCodeAt(i + 1) & 0x3f) << 12) |
            ((s.charCodeAt(i + 2) & 0x3f) << 6) |
            (s.charCodeAt(i + 3) & 0x3f)) -
          0x10000;
        out += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
        i += 4;
      }
    }
    return out;
  }
  const bytes =
    n <= 1024
      ? (_bstr_out_buf || (_bstr_out_buf = new Uint8Array(1024))).subarray(0, n)
      : new Uint8Array(n);
  for (let j = 0; j < n; j++) bytes[j] = s.charCodeAt(j);
  return globalThis.__rt._decoder.decode(bytes);
};
let _bstr_out_buf;

// bstr_char(point): the byte string of one Rust `char`, its UTF-8 bytes.
globalThis.__rt.bstr_char = (c) => {
  if (c < 0x80) return String.fromCharCode(c);
  if (c < 0x800) return String.fromCharCode(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
  if (c < 0x10000) {
    return String.fromCharCode(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
  }
  return String.fromCharCode(
    0xf0 | (c >> 18),
    0x80 | ((c >> 12) & 0x3f),
    0x80 | ((c >> 6) & 0x3f),
    0x80 | (c & 0x3f),
  );
};

// The byte-view memo of `bstr_bytes`: the same contract and bounds as the
// UTF-16 memo above (`_str_memo`), over byte strings. A view's buffer is the
// string's code units, so `_bstr_src` maps every memoized buffer back to its
// string and `bstr_of` over any range of it is a `slice`; the four-entry
// `_bsub_*` cache maps such a slice back to its view, so asking for the bytes of
// a slice just made copies nothing.
globalThis.__rt._bstr_memo = { map: new Map(), utf8: new Map(), bytes: 0, huge: [], armed: false, budget: 4194304 };
let _bstr_src;
let _bsub_arm;
let _bsub_clear;
function _arm_bstr_memo() {
  const memo = globalThis.__rt._bstr_memo;
  if (!memo.armed) {
    memo.armed = true;
    if (typeof queueMicrotask === "function") queueMicrotask(globalThis.__rt._bstr_memo_clear);
  }
}
function _bstr_memo_put(map, key, record, n) {
  const memo = globalThis.__rt._bstr_memo;
  if (_bsub_arm === undefined) _bsub_arm = _arm_bstr_memo;
  _arm_bstr_memo();
  if (n > memo.budget / 4) {
    if (memo.huge.length === 2) {
      const evicted = memo.huge.shift();
      evicted.map.delete(evicted.key);
      if (_bsub_clear !== undefined) _bsub_clear();
    }
    memo.huge.push({ map, key });
  } else {
    if (memo.bytes + n > memo.budget) {
      const kept = memo.huge.map((e) => ({ map: e.map, key: e.key, val: e.map.get(e.key) }));
      memo.map.clear();
      memo.utf8.clear();
      for (let i = 0; i < kept.length; i++) kept[i].map.set(kept[i].key, kept[i].val);
      memo.bytes = 0;
      if (_bsub_clear !== undefined) _bsub_clear();
    }
    memo.bytes += n;
  }
  map.set(key, record);
}

globalThis.__rt._bstr_memo_clear = () => {
  const memo = globalThis.__rt._bstr_memo;
  memo.map.clear();
  memo.utf8.clear();
  memo.bytes = 0;
  memo.huge.length = 0;
  memo.armed = false;
  if (_bsub_clear !== undefined) _bsub_clear();
};

// _bstr_sub_reset / _bsub_*: isolate the 4-entry subslice cache outside `_bstr_memo`
// so bundles using only `bstr_utf8_bytes` (without `bstr_of` or `bstr_bytes`) tree-shake it.
globalThis.__rt._bstr_sub_reset = () => {
  _bsub_k0 = _bsub_k1 = _bsub_k2 = _bsub_k3 = "";
  _bsub_v0 = _bsub_v1 = _bsub_v2 = _bsub_v3 = undefined;
};
let _bsub_k0 = "";
let _bsub_v0;
let _bsub_k1 = "";
let _bsub_v1;
let _bsub_k2 = "";
let _bsub_v2;
let _bsub_k3 = "";
let _bsub_v3;
let _bsub_i = 0;

// bstr_bytes(s): `str::as_bytes` of a byte string, `{ buf, off, len }` over a
// plain array of its code units, memoized for the operation like `str_bytes`.
//
// A string of at most 64 units is copied and never memoized: copying it costs
// less than hashing it into the memo, and the memo exists for the long text a
// parser re-reads per token, not for the short pieces it appends.
globalThis.__rt.bstr_bytes = (s) => {
  const n = s.length;
  if (n <= 64) {
    const bytes = new Array(n);
    for (let i = 0; i < n; i++) bytes[i] = s.charCodeAt(i);
    return { buf: bytes, off: 0, len: n };
  }
  if (globalThis.__rt._bsub_prop === null) globalThis.__rt._bsub_prop = _bstr_sub_propagate;
  if (s === _bsub_k0) return _bsub_v0;
  if (s === _bsub_k1) return _bsub_v1;
  if (s === _bsub_k2) return _bsub_v2;
  if (s === _bsub_k3) return _bsub_v3;
  const memo = globalThis.__rt._bstr_memo;
  let record = memo.map.get(s);
  if (record !== undefined) return record;
  const bytes = new Array(n);
  for (let i = 0; i < n; i++) bytes[i] = s.charCodeAt(i);
  record = { buf: bytes, off: 0, len: n };
  (globalThis.__rt._str_memo_bufs || (globalThis.__rt._str_memo_bufs = new WeakSet())).add(bytes);
  (_bstr_src || (_bstr_src = new WeakMap())).set(bytes, s);
  _bstr_memo_put(memo.map, s, record, n);
  return record;
};

function _bstr_sub_propagate(s, start, sub) {
  let parent;
  if (s === _bsub_k0) parent = _bsub_v0;
  else if (s === _bsub_k1) parent = _bsub_v1;
  else if (s === _bsub_k2) parent = _bsub_v2;
  else if (s === _bsub_k3) parent = _bsub_v3;
  else parent = globalThis.__rt._bstr_memo.map.get(s);
  if (parent === undefined) return;
  if (_bsub_arm !== undefined) _bsub_arm();
  if (_bsub_clear === undefined) _bsub_clear = globalThis.__rt._bstr_sub_reset;
  const slot = { buf: parent.buf, off: parent.off + start, len: sub.length };
  if (_bsub_i === 0) {
    _bsub_k0 = sub;
    _bsub_v0 = slot;
  } else if (_bsub_i === 1) {
    _bsub_k1 = sub;
    _bsub_v1 = slot;
  } else if (_bsub_i === 2) {
    _bsub_k2 = sub;
    _bsub_v2 = slot;
  } else {
    _bsub_k3 = sub;
    _bsub_v3 = slot;
  }
  _bsub_i = (_bsub_i + 1) & 3;
}

// bstr_utf8_bytes(s): `{ buf, off, len }` of host UTF-16 string `s` encoded as
// UTF-8 in a single pass, fusing `bstr_bytes(bstr_in(s))`. ASCII fills a plain
// array in one charCodeAt scan; on the first non-ASCII code unit it switches to
// `TextEncoder`. Only pure-ASCII arrays are registered in `_bstr_src`, and keys
// live in `memo.utf8` so host strings never collide with Rust byte strings.
globalThis.__rt.bstr_utf8_bytes = (s) => {
  if (typeof s !== "string") {
    throw new TypeError("expected a string for str, got " + typeof s);
  }
  const n = s.length;
  if (n <= 64) {
    const bytes = new Array(n);
    for (let i = 0; i < n; i++) {
      const code = s.charCodeAt(i);
      if (code > 127) {
        const enc = globalThis.__rt._encoder.encode(s);
        const arr = new Array(enc.length);
        for (let j = 0; j < enc.length; j++) arr[j] = enc[j];
        return { buf: arr, off: 0, len: arr.length };
      }
      bytes[i] = code;
    }
    return { buf: bytes, off: 0, len: n };
  }
  const memo = globalThis.__rt._bstr_memo;
  let record = memo.utf8.get(s);
  if (record !== undefined) return record;
  let bytes = new Array(n);
  let ascii = true;
  for (let i = 0; i < n; i++) {
    const code = s.charCodeAt(i);
    if (code > 127) {
      ascii = false;
      const enc = globalThis.__rt._encoder.encode(s);
      bytes = new Array(enc.length);
      for (let j = 0; j < enc.length; j++) bytes[j] = enc[j];
      break;
    }
    bytes[i] = code;
  }
  record = { buf: bytes, off: 0, len: bytes.length };
  (globalThis.__rt._str_memo_bufs || (globalThis.__rt._str_memo_bufs = new WeakSet())).add(bytes);
  if (ascii) (_bstr_src || (_bstr_src = new WeakMap())).set(bytes, s);
  _bstr_memo_put(memo.utf8, s, record, bytes.length);
  return record;
};

// bstr_bytes_mut(s): `as_bytes_mut`/`as_mut_ptr` -- the same bytes in a fresh,
// unmemoized record, for the containment `str_bytes_mut` documents.
globalThis.__rt.bstr_bytes_mut = (s) => {
  const n = s.length;
  const bytes = new Array(n);
  for (let i = 0; i < n; i++) bytes[i] = s.charCodeAt(i);
  return { buf: bytes, off: 0, len: n };
};

// bstr_of(buf, off, len): `len` bytes at `buf[off]` as a byte string -- what
// `from_utf8_unchecked` and a raw `str` pointer are. No decoding: a byte is a
// code unit. A range of a memoized view is a slice of the string it came from.
globalThis.__rt.bstr_of = (buf, off, len) => {
  // A zero length `str` reads nothing; its buffer may not exist at all.
  if (len === 0) return "";
  if (_bstr_src !== undefined && typeof buf === "object" && buf !== null) {
    const src = _bstr_src.get(buf);
    if (src !== undefined && off >= 0 && len >= 0 && off + len <= src.length) {
      if (off === 0 && len === src.length) return src;
      const sub = src.slice(off, off + len);
      if (_bsub_arm !== undefined) _bsub_arm();
      if (_bsub_clear === undefined) _bsub_clear = globalThis.__rt._bstr_sub_reset;
      const slot = { buf, off, len };
      if (_bsub_i === 0) {
        _bsub_k0 = sub;
        _bsub_v0 = slot;
      } else if (_bsub_i === 1) {
        _bsub_k1 = sub;
        _bsub_v1 = slot;
      } else if (_bsub_i === 2) {
        _bsub_k2 = sub;
        _bsub_v2 = slot;
      } else {
        _bsub_k3 = sub;
        _bsub_v3 = slot;
      }
      _bsub_i = (_bsub_i + 1) & 3;
      return sub;
    }
  }
  return globalThis.__rt._bstr_units(buf, off, len);
};

// bstr_boundary(text, index): whether `index` is a char boundary of the byte
// string `text`: 0..=len, and not the index of a continuation byte.
globalThis.__rt.bstr_boundary = (text, index) =>
  Number.isInteger(index) && index >= 0 && index <= text.length &&
  (index === text.length || (text.charCodeAt(index) & 0xc0) !== 0x80);

// Set lazily by `bstr_bytes` so `bstr_get*` does not pull `_bstr_memo` or
// `_bstr_sub_propagate` when a bundle never calls `bstr_bytes`.
globalThis.__rt._bsub_prop = null;

// bstr_get*, shared `str::get` / `&s[a..b]` slice-and-boundary helpers: return
// the substring when valid, or `undefined` otherwise.
globalThis.__rt.bstr_get = (s, start, end) => {
  const len = s.length;
  if (
    start >= 0 &&
    start <= end &&
    end <= len &&
    (start === len || (s.charCodeAt(start) & 0xc0) !== 0x80) &&
    (end === len || (s.charCodeAt(end) & 0xc0) !== 0x80)
  ) {
    const sub = s.slice(start, end);
    if (globalThis.__rt._bsub_prop !== null && sub.length > 64) globalThis.__rt._bsub_prop(s, start, sub);
    return sub;
  }
  return undefined;
};

globalThis.__rt.bstr_get_from = (s, start) => {
  const len = s.length;
  if (start >= 0 && start <= len && (start === len || (s.charCodeAt(start) & 0xc0) !== 0x80)) {
    const sub = s.slice(start);
    if (globalThis.__rt._bsub_prop !== null && sub.length > 64) globalThis.__rt._bsub_prop(s, start, sub);
    return sub;
  }
  return undefined;
};

globalThis.__rt.bstr_get_to = (s, end) => {
  const len = s.length;
  if (end >= 0 && end <= len && (end === len || (s.charCodeAt(end) & 0xc0) !== 0x80)) {
    const sub = s.slice(0, end);
    if (globalThis.__rt._bsub_prop !== null && sub.length > 64) globalThis.__rt._bsub_prop(s, 0, sub);
    return sub;
  }
  return undefined;
};

// bstr_sub*, outlined `&s[a..b]` / `&s[a..]` / `&s[..b]` helpers: preserve
// exact char-boundary checks and delegate invalid slices either to linked
// `core::str::slice_error_fail` (4 args with caller location) or to a fallback
// thunk when `loc === undefined`.
globalThis.__rt.bstr_sub = (s, start, end, fail, loc) => {
  const r = globalThis.__rt.bstr_get(s, start, end);
  return r !== undefined ? r : loc === undefined ? fail() : fail(s, start, end, loc);
};

globalThis.__rt.bstr_sub_from = (s, start, fail, loc) => {
  const r = globalThis.__rt.bstr_get_from(s, start);
  return r !== undefined ? r : loc === undefined ? fail() : fail(s, start, s.length, loc);
};

globalThis.__rt.bstr_sub_to = (s, end, fail, loc) => {
  const r = globalThis.__rt.bstr_get_to(s, end);
  return r !== undefined ? r : loc === undefined ? fail() : fail(s, 0, end, loc);
};

// bstr_trim_start(s), bstr_trim_end(s), bstr_trim(s): exact Rust `str::trim*`
// matching Unicode `White_Space` over a byte string.
globalThis.__rt.bstr_trim_start = (s) => s.slice(_bstr_ws_start(s));
function _bstr_ws_start(s) {
  const n = s.length;
  let i = 0;
  while (i < n) {
    const b0 = s.charCodeAt(i);
    if (b0 === 0x20 || (b0 >= 0x09 && b0 <= 0x0d)) {
      i += 1;
    } else if (b0 === 0xc2) {
      const b1 = s.charCodeAt(i + 1);
      if (b1 === 0x85 || b1 === 0xa0) i += 2;
      else break;
    } else if (b0 === 0xe1) {
      if (s.charCodeAt(i + 1) === 0x9a && s.charCodeAt(i + 2) === 0x80) i += 3;
      else break;
    } else if (b0 === 0xe2) {
      const b1 = s.charCodeAt(i + 1);
      const b2 = s.charCodeAt(i + 2);
      if (
        (b1 === 0x80 && ((b2 >= 0x80 && b2 <= 0x8a) || b2 === 0xa8 || b2 === 0xa9 || b2 === 0xaf)) ||
        (b1 === 0x81 && b2 === 0x9f)
      ) {
        i += 3;
      } else {
        break;
      }
    } else if (b0 === 0xe3) {
      if (s.charCodeAt(i + 1) === 0x80 && s.charCodeAt(i + 2) === 0x80) i += 3;
      else break;
    } else {
      break;
    }
  }
  return i;
}

globalThis.__rt.bstr_trim_end = (s) => s.slice(0, _bstr_ws_end(s, 0));
function _bstr_ws_end(s, min) {
  let j = s.length;
  while (j > min) {
    const b = s.charCodeAt(j - 1);
    if (b === 0x20 || (b >= 0x09 && b <= 0x0d)) {
      j -= 1;
    } else if ((b & 0xc0) === 0x80) {
      if (j - 2 >= min && s.charCodeAt(j - 2) === 0xc2 && (b === 0x85 || b === 0xa0)) {
        j -= 2;
      } else if (j - 3 >= min) {
        const b0 = s.charCodeAt(j - 3);
        const b1 = s.charCodeAt(j - 2);
        if (b0 === 0xe1 && b1 === 0x9a && b === 0x80) {
          j -= 3;
        } else if (
          b0 === 0xe2 &&
          ((b1 === 0x80 && ((b >= 0x80 && b <= 0x8a) || b === 0xa8 || b === 0xa9 || b === 0xaf)) ||
            (b1 === 0x81 && b === 0x9f))
        ) {
          j -= 3;
        } else if (b0 === 0xe3 && b1 === 0x80 && b === 0x80) {
          j -= 3;
        } else {
          break;
        }
      } else {
        break;
      }
    } else {
      break;
    }
  }
  return j;
}

globalThis.__rt.bstr_trim = (s) => {
  const start = _bstr_ws_start(s);
  return s.slice(start, _bstr_ws_end(s, start));
};

// bstr_push(value, point): `native_str_push` over a byte-string builder
// `{ text, cap, len }`, whose `len` is always `text.length`.
globalThis.__rt.bstr_push = (value, point) => {
  value.text += globalThis.__rt.bstr_char(point);
  value.len = value.text.length;
};

// bstr_pop(value): `native_str_pop` over a byte-string builder: -1 for `None`,
// otherwise the last scalar, decoded from its UTF-8 bytes. Capacity is kept.
globalThis.__rt.bstr_pop = (value) => {
  const text = value.text;
  if (text.length === 0) return -1;
  let at = text.length - 1;
  while (at > 0 && (text.charCodeAt(at) & 0xc0) === 0x80) at -= 1;
  const b0 = text.charCodeAt(at);
  const n = text.length - at;
  let point = n === 1 ? b0 : n === 2 ? b0 & 0x1f : n === 3 ? b0 & 0x0f : b0 & 0x07;
  for (let i = at + 1; i < text.length; i++) point = (point << 6) | (text.charCodeAt(i) & 0x3f);
  value.text = text.slice(0, at);
  value.len = at;
  return point;
};

// bstr_truncate(value, byteIndex): `native_str_truncate` over a byte-string
// builder: a no-op at or past the length, a refusal off a char boundary.
globalThis.__rt.bstr_truncate = (value, byteIndex) => {
  if (byteIndex >= value.len) return;
  if (!globalThis.__rt.bstr_boundary(value.text, byteIndex)) {
    throw __rt.abort_error(new RangeError("native String truncate index is not a UTF-8 boundary"));
  }
  value.text = value.text.slice(0, byteIndex);
  value.len = byteIndex;
};

// bstr_host_sb(s): `convert_host_sb` for a byte string: an exact-fit builder.
globalThis.__rt.bstr_host_sb = (s) => ({ text: s, cap: s.length, len: s.length });

// bstr_block(s): `convert_str_block` for a byte string: its bytes as a fresh,
// registered, exact-fit `{ buf, off, len, cap }` baseline block.
globalThis.__rt.bstr_block = (s) => {
  const n = s.length;
  const copy = new Array(n);
  for (let i = 0; i < n; i++) copy[i] = s.charCodeAt(i);
  const p = globalThis.__rt._register(copy, n, false);
  globalThis.__rt._blk_get(copy).fresh = false;
  return { buf: p.buf, off: 0, len: n, cap: n };
};

// bstr_sb_block(sb): `convert_sb_block` for a byte-string builder: a fresh
// registered block of capacity `max(cap, len)`, `[0, len)` initialized.
globalThis.__rt.bstr_sb_block = (sb) => {
  const text = sb.text;
  const n = text.length;
  const cap = Math.max(sb.cap, n);
  const copy = new Array(n);
  for (let i = 0; i < n; i++) copy[i] = text.charCodeAt(i);
  copy.length = cap;
  const p = globalThis.__rt._register(copy, cap, false);
  globalThis.__rt._blk_get(copy).fresh = false;
  return { buf: p.buf, off: 0, len: n, cap };
};

// bstr_sb_check(value): `_sb_check` (`RCGJS_SB_VERIFY=1`) for a byte-string
// builder: `len` is `text.length`, `cap` at least that, and the text is a
// byte string holding well-formed UTF-8.
globalThis.__rt.bstr_sb_check = (value) => {
  const text = value.text;
  const bytes = new Uint8Array(text.length);
  let valid = true;
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    if (unit > 0xff) valid = false;
    bytes[i] = unit;
  }
  if (valid && typeof TextDecoder === "function") {
    try {
      new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch (_) {
      valid = false;
    }
  }
  if (value.len !== text.length || !(value.cap >= text.length) || !valid) {
    const detail = "len " + value.len + ", cap " + value.cap + ", text measures " + text.length +
      (valid ? "" : ", text is not a UTF-8 byte string");
    throw __rt.abort_error(new RangeError("native String length drift: " + detail));
  }
  return value;
};

globalThis.__rt._bstr_out_ref = new WeakMap();
globalThis.__rt._bstr_in_ref = new WeakMap();
globalThis.__rt._bstr_unwrap_rust = new WeakMap();
globalThis.__rt._bstr_unwrap_js = new WeakMap();

globalThis.__rt._bstr_spec_key = (() => {
  const fnIds = new WeakMap();
  let nextId = 1;
  const childKey = (c) => {
    if (c === "s" || c === "u") return c;
    if (c === null) return "n";
    if (c === undefined) return "_";
    if (Array.isArray(c)) {
      let s = "f(";
      for (let i = 0; i < c.length; i++) {
        const fn = c[i];
        let id = 0;
        if (typeof fn === "function" || (typeof fn === "object" && fn !== null)) {
          id = fnIds.get(fn);
          if (id === undefined) {
            id = nextId++;
            fnIds.set(fn, id);
          }
        }
        if (i > 0) s += ",";
        s += id;
      }
      return s + ")";
    }
    return specKey(c);
  };
  const mapKey = (obj) => {
    const keys = Object.keys(obj).sort();
    let s = "{";
    for (let i = 0; i < keys.length; i++) {
      if (i > 0) s += ",";
      const k = keys[i];
      s += JSON.stringify(k) + ":" + childKey(obj[k]);
    }
    return s + "}";
  };
  const specKey = (spec) => {
    const k = spec.k;
    if (k === "o") return "o" + mapKey(spec.f);
    if (k === "e") {
      const tags = Object.keys(spec.v).sort();
      let s = "e{";
      for (let i = 0; i < tags.length; i++) {
        if (i > 0) s += ",";
        const t = tags[i];
        s += JSON.stringify(t) + ":" + mapKey(spec.v[t]);
      }
      return s + "}";
    }
    if (k === "a" || k === "slice" || k === "slot") return k + "(" + childKey(spec.c) + ")";
    return k + "(" + childKey(spec.h) + "," + childKey(spec.c) + ")";
  };
  return specKey;
})();

globalThis.__rt.bstr_str_in = (s) => {
  if (typeof s !== "string") {
    throw new TypeError(`expected a string for \`str\`, got ${typeof s}`);
  }
  return globalThis.__rt.bstr_in(s);
};

globalThis.__rt._bstr_has = Object.prototype.hasOwnProperty;

globalThis.__rt._bstr_is_idx = (p) => {
  if (typeof p !== "string" || p.length === 0) return false;
  const c = p.charCodeAt(0);
  if (c < 48 || c > 57) return false;
  const n = p >>> 0;
  return n < 4294967295 && String(n) === p;
};

globalThis.__rt._bstr_conv = (spec, val, out, allowUndef) => {
  if (spec === "s" || spec === "u") {
    if (val === undefined && (out || allowUndef || spec === "u")) return undefined;
    return out ? globalThis.__rt.bstr_out(val) : globalThis.__rt.bstr_str_in(val);
  }
  if (val === undefined) return undefined;
  return (out ? spec[0] : spec[1])(val);
};

globalThis.__rt._bstr_wrap = (value, spec, out) => {
  if (value === null || typeof value !== "object") return value;
  const unwrap = out ? globalThis.__rt._bstr_unwrap_js : globalThis.__rt._bstr_unwrap_rust;
  if (unwrap.has(value)) return unwrap.get(value);
  const cache = out ? globalThis.__rt._bstr_out_ref : globalThis.__rt._bstr_in_ref;
  const specKey = globalThis.__rt._bstr_spec_key(spec);
  let perObj = cache.get(value);
  if (perObj !== undefined) {
    const hit = perObj.get(specKey);
    if (hit !== undefined) return hit;
  }
  const rev = out ? globalThis.__rt._bstr_unwrap_rust : globalThis.__rt._bstr_unwrap_js;
  const has = globalThis.__rt._bstr_has;
  const isIdx = globalThis.__rt._bstr_is_idx;
  const conv = globalThis.__rt._bstr_conv;
  const makeMembrane = (real, readVal, writeVal) => {
    const isArr = Array.isArray(real);
    const shadow = isArr ? [] : Object.create(Object.getPrototypeOf(real));
    const findPropDesc = (obj, prop) => {
      let cur = obj;
      while (cur !== null && cur !== undefined) {
        const d = Reflect.getOwnPropertyDescriptor(cur, prop);
        if (d !== undefined) return { owner: cur, desc: d };
        cur = Object.getPrototypeOf(cur);
      }
      return undefined;
    };
    const wrapAccDesc = (prop, desc) => {
      const outDesc = { ...desc };
      if ("get" in desc) {
        const g = desc.get;
        outDesc.get = g ? function () { return writeVal(real, prop, g.call(proxy)); } : undefined;
      }
      if ("set" in desc) {
        const s = desc.set;
        outDesc.set = s ? function (raw) { s.call(proxy, readVal(real, prop, raw)); } : undefined;
      }
      return outDesc;
    };
    const proxy = new Proxy(shadow, {
      get(st, prop, recv) {
        const stDesc = Reflect.getOwnPropertyDescriptor(st, prop);
        if (stDesc !== undefined) {
          if (!("value" in stDesc)) {
            return stDesc.get ? stDesc.get.call(recv) : undefined;
          }
          if (!stDesc.configurable && !stDesc.writable) {
            return stDesc.value;
          }
        }
        const raw = Reflect.get(real, prop, real);
        return readVal(real, prop, raw);
      },
      set(st, prop, val, recv) {
        const stDesc = Reflect.getOwnPropertyDescriptor(st, prop);
        if (stDesc !== undefined) {
          if ("value" in stDesc) {
            if (!stDesc.writable) return false;
          } else {
            if (!stDesc.set) return false;
            stDesc.set.call(recv, val);
            return true;
          }
        } else {
          const found = findPropDesc(real, prop);
          if (found !== undefined) {
            if ("value" in found.desc) {
              if (!found.desc.writable) return false;
            } else {
              if (!found.desc.set) return false;
              const isOwnReal = found.owner === real;
              const setterArg = isOwnReal ? writeVal(real, prop, val) : val;
              found.desc.set.call(recv === proxy && isOwnReal ? real : recv, setterArg);
              return true;
            }
          } else if (recv === proxy && !Reflect.isExtensible(st)) {
            return false;
          }
        }
        if (recv !== proxy) {
          if (recv === null || (typeof recv !== "object" && typeof recv !== "function")) return false;
          const rDesc = Reflect.getOwnPropertyDescriptor(recv, prop);
          if (rDesc !== undefined) {
            if (!("value" in rDesc) || !rDesc.writable) return false;
            return Reflect.defineProperty(recv, prop, { value: val });
          }
          if (!Reflect.isExtensible(recv)) return false;
          return Reflect.defineProperty(recv, prop, {
            value: val,
            writable: true,
            enumerable: true,
            configurable: true,
          });
        }
        const next = writeVal(real, prop, val);
        if (!Reflect.set(real, prop, next, real)) return false;
        if (isArr && prop === "length") {
          const lenDesc = Reflect.getOwnPropertyDescriptor(st, "length");
          if (lenDesc && lenDesc.writable) Reflect.defineProperty(st, "length", { value: real.length });
        } else if (stDesc !== undefined && "value" in stDesc) {
          Reflect.defineProperty(st, prop, { value: val });
        }
        return true;
      },
      has(st, prop) {
        return Reflect.has(st, prop) || Reflect.has(real, prop);
      },
      ownKeys(st) {
        if (!Reflect.isExtensible(st)) return Reflect.ownKeys(st);
        if (isArr) {
          const lenDesc = Reflect.getOwnPropertyDescriptor(st, "length");
          if (lenDesc && lenDesc.writable && st.length !== real.length) {
            Reflect.defineProperty(st, "length", { value: real.length });
          }
        }
        const keys = Reflect.ownKeys(real);
        const stKeys = Reflect.ownKeys(st);
        for (let i = 0; i < stKeys.length; i++) {
          if (!Reflect.getOwnPropertyDescriptor(real, stKeys[i])) keys.push(stKeys[i]);
        }
        return keys;
      },
      getOwnPropertyDescriptor(st, prop) {
        const stDesc = Reflect.getOwnPropertyDescriptor(st, prop);
        if (stDesc !== undefined) {
          if (!stDesc.configurable && "value" in stDesc && !stDesc.writable) return stDesc;
          if (isArr && prop === "length") {
            const realLen = real.length;
            if (stDesc.writable && stDesc.value !== realLen) Reflect.defineProperty(st, "length", { value: realLen });
            return Reflect.getOwnPropertyDescriptor(st, "length");
          }
          const liveDesc = Reflect.getOwnPropertyDescriptor(real, prop);
          if (liveDesc === undefined) return stDesc;
          if ("value" in stDesc) {
            return {
              value: "value" in liveDesc ? readVal(real, prop, liveDesc.value) : stDesc.value,
              writable: stDesc.writable,
              enumerable: stDesc.enumerable,
              configurable: stDesc.configurable,
            };
          }
          return stDesc;
        }
        if (!Reflect.isExtensible(st)) return undefined;
        let desc = Reflect.getOwnPropertyDescriptor(real, prop);
        if (desc === undefined) return undefined;
        if ("value" in desc) {
          desc = { ...desc, value: readVal(real, prop, desc.value) };
        }
        if (!desc.configurable) {
          if (!desc.writable && "value" in desc && Reflect.isExtensible(st)) {
            Reflect.defineProperty(st, prop, desc);
            return Reflect.getOwnPropertyDescriptor(st, prop);
          }
          return { ...desc, configurable: true };
        }
        return desc;
      },
      defineProperty(st, prop, desc) {
        const stDesc = Reflect.getOwnPropertyDescriptor(st, prop);
        if (!Reflect.isExtensible(st) && stDesc === undefined) return false;
        if (isArr && prop === "length") {
          if (stDesc && !stDesc.writable) return Reflect.defineProperty(st, "length", desc);
          if (stDesc && stDesc.writable) Reflect.defineProperty(st, "length", { value: real.length });
          if (!Reflect.defineProperty(real, "length", desc)) return false;
          return Reflect.defineProperty(st, "length", desc);
        }
        const hasVal = "value" in desc;
        const hasAcc = "get" in desc || "set" in desc;
        if (hasVal && hasAcc) {
          return Reflect.defineProperty(st, prop, desc);
        }
        if (stDesc !== undefined && !stDesc.configurable) {
          if ("value" in stDesc && !stDesc.writable) {
            return Reflect.defineProperty(st, prop, desc);
          }
          if (desc.configurable === true) return false;
          if ("enumerable" in desc && desc.enumerable !== stDesc.enumerable) return false;
          if ("value" in stDesc ? hasAcc : hasVal) return false;
          if (!("value" in stDesc)) {
            return Reflect.defineProperty(st, prop, desc);
          }
          const realDesc = Reflect.getOwnPropertyDescriptor(real, prop);
          if (hasVal) {
            const next = writeVal(real, prop, desc.value);
            const realArg = { ...desc, value: next };
            if (realArg.configurable === false && (!realDesc || realDesc.configurable !== false)) {
              delete realArg.configurable;
            }
            if (realArg.writable === false && (!realDesc || realDesc.writable !== false)) {
              delete realArg.writable;
            }
            if (!Reflect.defineProperty(real, prop, realArg)) return false;
          }
          if (!hasVal && !hasAcc && realDesc !== undefined && "value" in realDesc) {
            Reflect.defineProperty(st, prop, { value: readVal(real, prop, realDesc.value) });
          }
          return Reflect.defineProperty(st, prop, desc);
        }
        const rawVal = hasVal ? writeVal(real, prop, desc.value) : undefined;
        const realDesc = Reflect.getOwnPropertyDescriptor(real, prop);
        const realArg = hasVal
          ? { ...desc, value: rawVal }
          : (hasAcc ? wrapAccDesc(prop, desc) : { ...desc });
        if (realDesc === undefined) {
          if (realArg.configurable === undefined || realArg.configurable === false) realArg.configurable = true;
          if (hasVal && (realArg.writable === undefined || realArg.writable === false)) realArg.writable = true;
          if (realArg.enumerable === undefined) realArg.enumerable = false;
        } else {
          if (realArg.configurable === false && realDesc.configurable !== false) delete realArg.configurable;
          if (realArg.writable === false && realDesc.writable !== false) delete realArg.writable;
        }
        if (!Reflect.defineProperty(real, prop, realArg)) return false;
        const locksSt =
          hasAcc ||
          desc.configurable === false ||
          desc.writable === false ||
          (realDesc === undefined && desc.configurable !== true && hasVal);
        if (stDesc !== undefined || !Reflect.isExtensible(st) || locksSt) {
          if (stDesc === undefined) {
            if (realDesc !== undefined) {
              const base = "value" in realDesc
                ? { ...realDesc, value: readVal(real, prop, realDesc.value) }
                : { ...realDesc };
              Reflect.defineProperty(st, prop, base);
            } else if (!hasVal && !hasAcc) {
              Reflect.defineProperty(st, prop, { enumerable: false, writable: true, configurable: true });
            }
          } else if ("value" in stDesc && !hasVal && !hasAcc && realDesc !== undefined && "value" in realDesc) {
            Reflect.defineProperty(st, prop, { value: readVal(real, prop, realDesc.value) });
          }
          return Reflect.defineProperty(st, prop, desc);
        }
        return true;
      },
      deleteProperty(st, prop) {
        const stDesc = Reflect.getOwnPropertyDescriptor(st, prop);
        if (stDesc !== undefined && !stDesc.configurable) return false;
        if (!Reflect.deleteProperty(real, prop)) return false;
        if (stDesc !== undefined) Reflect.deleteProperty(st, prop);
        return true;
      },
      preventExtensions(st) {
        if (isArr) {
          const lenDesc = Reflect.getOwnPropertyDescriptor(st, "length");
          if (lenDesc && lenDesc.writable) Reflect.defineProperty(st, "length", { value: real.length });
        }
        const keys = Reflect.ownKeys(real);
        for (let i = 0; i < keys.length; i++) {
          const key = keys[i];
          if (isArr && key === "length") continue;
          const stDesc = Reflect.getOwnPropertyDescriptor(st, key);
          if (stDesc !== undefined) {
            if (!stDesc.configurable || !("value" in stDesc)) continue;
            const liveDesc = Reflect.getOwnPropertyDescriptor(real, key);
            if (liveDesc !== undefined && "value" in liveDesc) {
              Reflect.defineProperty(st, key, { value: readVal(real, key, liveDesc.value) });
            }
            continue;
          }
          let desc = Reflect.getOwnPropertyDescriptor(real, key);
          if (desc === undefined) continue;
          if ("value" in desc) {
            desc = { ...desc, value: readVal(real, key, desc.value) };
          }
          Reflect.defineProperty(st, key, desc);
        }
        return Reflect.preventExtensions(st);
      },
      isExtensible(st) {
        return Reflect.isExtensible(st);
      },
    });
    return proxy;
  };
  const k = spec.k;
  let proxy;
  if (k === "tbuf") {
    const hdr = spec.h;
    const elem = spec.c;
    proxy = makeMembrane(
      value,
      (_t, prop, raw) => {
        if (!isIdx(prop)) return raw;
        const c = prop === "0" ? hdr : elem;
        return c ? conv(c, raw, out, true) : raw;
      },
      (_t, prop, val) => {
        if (!isIdx(prop)) return val;
        const c = prop === "0" ? hdr : elem;
        return c ? conv(c, val, !out, false) : val;
      },
    );
  } else if (k === "tail") {
    const tailPtrSpec = { k: "tailptr", h: spec.h, c: spec.c };
    proxy = makeMembrane(
      value,
      (_t, prop, raw) => (prop === "ptr" && raw !== null && typeof raw === "object"
        ? globalThis.__rt._bstr_wrap(raw, tailPtrSpec, out)
        : raw),
      (_t, _prop, val) => val,
    );
  } else if (k === "tailptr") {
    const tbufSpec = { k: "tbuf", h: spec.h, c: spec.c };
    proxy = makeMembrane(
      value,
      (_t, prop, raw) => {
        if (prop === "buf" && raw !== null && typeof raw === "object") {
          if (globalThis.__rt._bv !== null) raw = globalThis.__rt._bv.unwrap_buf(raw);
          if (Array.isArray(raw)) {
            if (Object.isFrozen(raw) && raw.length === 0) return raw;
            return globalThis.__rt._bstr_wrap(raw, tbufSpec, out);
          }
        }
        return raw;
      },
      (_t, _prop, val) => val,
    );
  } else if (k === "slice") {
    const elem = spec.c;
    proxy = makeMembrane(
      value,
      (_t, prop, raw) => {
        if (prop === "buf" && raw !== null && typeof raw === "object") {
          if (globalThis.__rt._bv !== null) raw = globalThis.__rt._bv.unwrap_buf(raw);
          if (Array.isArray(raw) && Object.isFrozen(raw) && raw.length === 0) return raw;
          return globalThis.__rt._bstr_wrap(raw, { k: "a", c: elem }, out);
        }
        return raw;
      },
      (_t, _prop, val) => val,
    );
  } else if (k === "slot") {
    const elem = spec.c;
    const arrSpecKey = globalThis.__rt._bstr_spec_key({ k: "a", c: elem });
    let partialCache;
    proxy = makeMembrane(
      value,
      (t, prop, raw) => {
        if (prop === "buf" && raw !== null && typeof raw === "object") {
          if (globalThis.__rt._bv !== null) raw = globalThis.__rt._bv.unwrap_buf(raw);
          if (unwrap.has(raw)) return unwrap.get(raw);
          if (Array.isArray(raw) && Object.isFrozen(raw) && raw.length === 0) return raw;
          const rawMap = cache.get(raw);
          const existing = rawMap !== undefined ? rawMap.get(arrSpecKey) : undefined;
          if (existing !== undefined) return existing;
          const key = t.off;
          if (partialCache !== undefined && partialCache[0] === raw && partialCache[1] === key) {
            return partialCache[2];
          }
          const slotKey = String(key);
          const p = makeMembrane(
            raw,
            (_bt, bprop, braw) => (bprop === slotKey && elem ? conv(elem, braw, out, out) : braw),
            (_bt, bprop, bval) => (bprop === slotKey && elem ? conv(elem, bval, !out, false) : bval),
          );
          rev.set(p, raw);
          partialCache = [raw, key, p];
          return p;
        }
        return raw;
      },
      (_t, _prop, val) => val,
    );
  } else {
    const fieldsFor = (t) => {
      if (k === "e") {
        const tag = t.TAG;
        return typeof tag === "string" && has.call(spec.v, tag) ? spec.v[tag] : null;
      }
      return k === "o" ? spec.f : null;
    };
    proxy = makeMembrane(
      value,
      (t, prop, raw) => {
        if (k === "a") {
          return isIdx(prop) && spec.c ? conv(spec.c, raw, out, true) : raw;
        }
        if (typeof prop === "string") {
          const fields = fieldsFor(t);
          if (fields !== null && has.call(fields, prop)) {
            return conv(fields[prop], raw, out, out);
          }
        }
        return raw;
      },
      (t, prop, val) => {
        if (k === "a") {
          return isIdx(prop) && spec.c ? conv(spec.c, val, !out, false) : val;
        }
        if (typeof prop === "string") {
          const fields = fieldsFor(t);
          if (fields !== null && has.call(fields, prop)) {
            return conv(fields[prop], val, !out, false);
          }
        }
        return val;
      },
    );
  }
  if (perObj === undefined) {
    perObj = new Map();
    cache.set(value, perObj);
  }
  perObj.set(specKey, proxy);
  rev.set(proxy, value);
  return proxy;
};

globalThis.__rt.bstr_ref = (value, spec) => globalThis.__rt._bstr_wrap(value, spec, true);
globalThis.__rt.bstr_ref_in = (value, spec) => globalThis.__rt._bstr_wrap(value, spec, false);

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

// native_str_wf(s): the text a `&str` is once Rust holds it as bytes. A lone
// surrogate (only a host can hand Rust one) is the U+FFFD that `str_bytes`
// encodes for it, so this is exactly what a baseline `Vec<u8>` String decodes
// back after the same append. A native String's text is only ever built from
// such text: it stays well formed, its UTF-8 length is the sum of its pieces'
// (`str_len`), and two appended halves can never pair into one scalar the
// baseline never saw. A well-formed string -- every string Rust itself makes --
// is returned as is, without allocating.
globalThis.__rt.native_str_wf = (s) => {
  let out = null;
  let from = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if ((c & 0xf800) !== 0xd800) continue;
    if (c < 0xdc00 && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
      i++;
      continue;
    }
    out = (out === null ? "" : out) + s.slice(from, i) + String.fromCharCode(0xfffd);
    from = i + 1;
  }
  return out === null ? s : out + s.slice(from);
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

globalThis.__rt._num_dv = new DataView(new ArrayBuffer(8));

globalThis.__rt._f64_decomp = (mag) => {
  const dv = globalThis.__rt._num_dv;
  dv.setFloat64(0, mag, false);
  const hi = dv.getUint32(0, false);
  const lo = dv.getUint32(4, false);
  const expBits = (hi >>> 20) & 0x7ff;
  const frac = (BigInt(hi & 0xfffff) << BigInt("32")) | BigInt(lo);
  if (expBits === 0) return [frac, -1074];
  return [(BigInt("1") << BigInt("52")) | frac, expBits - 1075];
};

globalThis.__rt._f64_shortest_dk = (magnitude) => {
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
    const parts = globalThis.__rt._f64_decomp(magnitude);
    const mantissaBits = parts[0];
    const binaryPower = parts[1];
    let binaryNumerator;
    let binaryDenominator;
    if (binaryPower >= 0) {
      binaryNumerator = mantissaBits << BigInt(binaryPower);
      binaryDenominator = BigInt("1");
    } else {
      binaryNumerator = mantissaBits;
      binaryDenominator = BigInt("1") << BigInt(-binaryPower);
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
  return [digits, digits.length + power];
};

globalThis.__rt._f32_shortest_dk = (mag) => {
  const dv = globalThis.__rt._num_dv;
  dv.setFloat32(0, mag, false);
  const bits = dv.getUint32(0, false);
  const expBits = (bits >>> 23) & 0xff;
  const frac = bits & 0x7fffff;
  const ONE = BigInt("1");
  const TWO = BigInt("2");
  const TEN = BigInt("10");
  let m;
  let e;
  let lowerHalf;
  if (expBits === 0) {
    m = BigInt(frac);
    e = -149;
    lowerHalf = false;
  } else {
    m = BigInt(0x800000 | frac);
    e = expBits - 150;
    lowerHalf = frac === 0 && expBits > 1;
  }
  const acceptBounds = (m & ONE) === BigInt("0");
  let r = m << ONE;
  let s = TWO;
  let mPlus = ONE;
  let mMinus = ONE;
  if (lowerHalf) {
    r <<= ONE;
    s <<= ONE;
    mPlus <<= ONE;
  }
  if (e >= 0) {
    const eb = BigInt(e);
    r <<= eb;
    mPlus <<= eb;
    mMinus <<= eb;
  } else {
    s <<= BigInt(-e);
  }
  let k = Math.ceil(Math.log10(mag));
  if (k > 0) s *= TEN ** BigInt(k);
  else if (k < 0) {
    const p = TEN ** BigInt(-k);
    r *= p;
    mPlus *= p;
    mMinus *= p;
  }
  while (acceptBounds ? (r + mPlus >= s) : (r + mPlus > s)) {
    s *= TEN;
    k += 1;
  }
  while (acceptBounds ? ((r + mPlus) * TEN < s) : ((r + mPlus) * TEN <= s)) {
    r *= TEN;
    mPlus *= TEN;
    mMinus *= TEN;
    k -= 1;
  }
  let digits = "";
  for (;;) {
    r *= TEN;
    mPlus *= TEN;
    mMinus *= TEN;
    const d = r / s;
    r = r % s;
    const low = acceptBounds ? (r <= mMinus) : (r < mMinus);
    const high = acceptBounds ? (r + mPlus >= s) : (r + mPlus > s);
    if (!low && !high) digits += d.toString();
    else if (low && !high) {
      digits += d.toString();
      break;
    } else if (high && !low) {
      digits += (d + ONE).toString();
      break;
    } else {
      digits += (r * TWO < s ? d : d + ONE).toString();
      break;
    }
  }
  return [digits, k];
};

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
  else if (value === 0) text = "0";
  else {
    const negative = value < 0;
    const dk = globalThis.__rt._f64_shortest_dk(negative ? -value : value);
    const digits = dk[0];
    const decimalAt = dk[1];
    if (decimalAt <= 0) text = "0." + "0".repeat(-decimalAt) + digits;
    else if (decimalAt >= digits.length) text = digits + "0".repeat(decimalAt - digits.length);
    else text = digits.slice(0, decimalAt) + "." + digits.slice(decimalAt);
    if (negative) text = "-" + text;
  }
  // Every character of the rendering is ASCII: its UTF-16 length is its UTF-8
  // length, and an owned `String` built from it is exact-fit.
  return { text: text, cap: text.length, len: text.length };
};

// Rust's default f32 Display: exact shortest round-tripping binary32 decimal in
// fixed notation (matching core::num::imp::flt2dec).
globalThis.__rt.native_f32_to_string = (value) => {
  let text;
  if (Number.isNaN(value)) text = "NaN";
  else if (value === Infinity) text = "inf";
  else if (value === -Infinity) text = "-inf";
  else if (Object.is(value, -0)) text = "-0";
  else if (value === 0) text = "0";
  else {
    const negative = value < 0;
    const dk = globalThis.__rt._f32_shortest_dk(negative ? -value : value);
    const digits = dk[0];
    const decimalAt = dk[1];
    if (decimalAt <= 0) text = "0." + "0".repeat(-decimalAt) + digits;
    else if (decimalAt >= digits.length) text = digits + "0".repeat(decimalAt - digits.length);
    else text = digits.slice(0, decimalAt) + "." + digits.slice(decimalAt);
    if (negative) text = "-" + text;
  }
  return { text: text, cap: text.length, len: text.length };
};

// Exact IEEE-754 round-half-to-even fixed-precision float formatter for both f64
// and f32 (every finite f32 has the exact same rational value in binary64).
globalThis.__rt.fmt_float_fixed = (val, prec, signPlus) => {
  if (Number.isNaN(val)) return "NaN";
  if (val === Infinity) return signPlus ? "+inf" : "inf";
  if (val === -Infinity) return "-inf";
  const neg = val < 0 || Object.is(val, -0);
  const sign = neg ? "-" : (signPlus ? "+" : "");
  const mag = neg ? -val : val;
  if (mag === 0) return prec === 0 ? sign + "0" : sign + "0." + "0".repeat(prec);
  const parts = globalThis.__rt._f64_decomp(mag);
  const mant = parts[0];
  const exp2 = parts[1];
  if (exp2 >= 0) {
    const intStr = (mant << BigInt(exp2)).toString();
    return prec === 0 ? sign + intStr : sign + intStr + "." + "0".repeat(prec);
  }
  const shift = -exp2;
  const FIVE = BigInt("5");
  const ONE = BigInt("1");
  const ZERO = BigInt("0");
  if (prec >= shift) {
    const exact = (mant * (FIVE ** BigInt(shift))).toString();
    const trail = "0".repeat(prec - shift);
    if (exact.length <= shift) {
      return sign + "0." + "0".repeat(shift - exact.length) + exact + trail;
    }
    const split = exact.length - shift;
    return sign + exact.slice(0, split) + "." + exact.slice(split) + trail;
  }
  const k = shift - prec;
  const num = mant * (FIVE ** BigInt(prec));
  const kb = BigInt(k);
  let q = num >> kb;
  const rem = num & ((ONE << kb) - ONE);
  const half = ONE << BigInt(k - 1);
  if (rem > half || (rem === half && (q & ONE) !== ZERO)) q += ONE;
  const s = q.toString();
  if (prec === 0) return sign + s;
  if (s.length <= prec) return sign + "0." + "0".repeat(prec - s.length) + s;
  const split = s.length - prec;
  return sign + s.slice(0, split) + "." + s.slice(split);
};

// Integer hex formatting matching Rust `{:x}`/`{:X}`/`{:0Wx}`/`{:#0Wx}` for all
// signed and unsigned integer widths (8..=128 bits). Note Rust `{:#X}` uses
// lowercase `0x` prefix with uppercase hex digits.
globalThis.__rt.fmt_int_hex = (val, bits, upper, width, zeroPad, alt) => {
  let u;
  if (bits <= 32) {
    u = bits === 32 ? (val >>> 0) : (val & ((1 << bits) - 1));
  } else {
    u = BigInt.asUintN(bits, val);
  }
  let digits = u.toString(16);
  if (upper) digits = digits.toUpperCase();
  const prefix = alt ? "0x" : "";
  if (width > 0) {
    if (zeroPad) {
      const pad = width - prefix.length;
      return prefix + (pad > digits.length ? digits.padStart(pad, "0") : digits);
    }
    const full = prefix + digits;
    return full.length < width ? full.padStart(width, " ") : full;
  }
  return prefix + digits;
};

// Integer decimal formatting matching Rust `{:0W}`/`{:+0W}`/`{:W}`/`{:+}` for all
// signed and unsigned integer widths (8..=128 bits).
globalThis.__rt.fmt_int_dec = (val, width, zeroPad, signPlus) => {
  const neg = val < 0;
  const sign = neg ? "-" : (signPlus ? "+" : "");
  const digits = neg ? String(-val) : String(val);
  if (width > 0) {
    if (zeroPad) {
      const pad = width - sign.length;
      return sign + (pad > digits.length ? digits.padStart(pad, "0") : digits);
    }
    const full = sign + digits;
    return full.length < width ? full.padStart(width, " ") : full;
  }
  return sign + digits;
};

// Host-delegated core::fmt::float parts builder for Formatter::pad_formatted_parts.
// Modes: 0=fixed(prec), 1=shortest(minPrec), 2=exp_exact_lo(prec), 3=exp_exact_hi(prec),
// 4=exp_short_lo, 5=exp_short_hi. Writes ASCII bytes into outPtr (len >= 800) and part
// descriptors into metaPtr (len >= 10). Never throws.
globalThis.__rt.rcgjs_fmt_float_parts = (val, isF32, mode, param, forceSign, outPtr, metaPtr) => {
  const obuf = outPtr.buf;
  const ooff = outPtr.off;
  const mbuf = metaPtr.buf;
  const moff = metaPtr.off;
  let pos = 0;
  let np = 0;
  const pushCopy = (str) => {
    const start = pos;
    for (let i = 0; i < str.length; i++) obuf[ooff + pos++] = str.charCodeAt(i);
    mbuf[moff + 2 + np * 2] = 0;
    mbuf[moff + 3 + np * 2] = (start << 16) | str.length;
    np += 1;
  };
  const pushZero = (count) => {
    if (count <= 0) return;
    mbuf[moff + 2 + np * 2] = 1;
    mbuf[moff + 3 + np * 2] = count;
    np += 1;
  };
  if (Number.isNaN(val)) {
    mbuf[moff] = 0;
    pushCopy("NaN");
    mbuf[moff + 1] = np;
    return;
  }
  if (val === Infinity) {
    mbuf[moff] = forceSign ? 2 : 0;
    pushCopy("inf");
    mbuf[moff + 1] = np;
    return;
  }
  if (val === -Infinity) {
    mbuf[moff] = 1;
    pushCopy("inf");
    mbuf[moff + 1] = np;
    return;
  }
  const neg = val < 0 || Object.is(val, -0);
  mbuf[moff] = neg ? 1 : (forceSign ? 2 : 0);
  const mag = neg ? -val : val;
  const ONE = BigInt("1");
  const TWO = BigInt("2");
  const FIVE = BigInt("5");
  const TEN = BigInt("10");
  const ZERO = BigInt("0");
  if (mode === 0) {
    const prec = param;
    if (mag === 0) {
      if (prec === 0) pushCopy("0");
      else {
        pushCopy("0.");
        pushZero(prec);
      }
    } else {
      const decomp = globalThis.__rt._f64_decomp(mag);
      const mant = decomp[0];
      const exp2 = decomp[1];
      if (exp2 >= 0) {
        const intStr = (mant << BigInt(exp2)).toString();
        if (prec === 0) pushCopy(intStr);
        else {
          pushCopy(intStr + ".");
          pushZero(prec);
        }
      } else {
        const shift = -exp2;
        if (prec >= shift) {
          const exact = (mant * (FIVE ** BigInt(shift))).toString();
          if (exact.length <= shift) {
            pushCopy("0.");
            pushZero(shift - exact.length);
            pushCopy(exact);
            pushZero(prec - shift);
          } else {
            const split = exact.length - shift;
            pushCopy(exact.slice(0, split) + "." + exact.slice(split));
            pushZero(prec - shift);
          }
        } else {
          const k = shift - prec;
          const num = mant * (FIVE ** BigInt(prec));
          const kb = BigInt(k);
          let q = num >> kb;
          const rem = num & ((ONE << kb) - ONE);
          const half = ONE << BigInt(k - 1);
          if (rem > half || (rem === half && (q & ONE) !== ZERO)) q += ONE;
          const s = q.toString();
          if (prec === 0) pushCopy(s);
          else if (s.length <= prec) {
            pushCopy("0.");
            pushZero(prec - s.length);
            pushCopy(s);
          } else {
            const split = s.length - prec;
            pushCopy(s.slice(0, split) + "." + s.slice(split));
          }
        }
      }
    }
  } else if (mode === 1) {
    const minPrec = param;
    const dk = mag === 0
      ? ["0", 1]
      : (isF32 ? globalThis.__rt._f32_shortest_dk(mag) : globalThis.__rt._f64_shortest_dk(mag));
    const digits = dk[0];
    const k = dk[1];
    if (k <= 0) {
      pushCopy("0.");
      pushZero(-k);
      pushCopy(digits);
    } else if (k >= digits.length) {
      pushCopy(digits);
      pushZero(k - digits.length);
      if (minPrec > 0) {
        pushCopy(".");
        pushZero(minPrec);
      }
    } else {
      pushCopy(digits.slice(0, k) + "." + digits.slice(k));
    }
  } else if (mode === 4 || mode === 5) {
    const eChar = mode === 5 ? "E" : "e";
    const dk = mag === 0
      ? ["0", 1]
      : (isF32 ? globalThis.__rt._f32_shortest_dk(mag) : globalThis.__rt._f64_shortest_dk(mag));
    const digits = dk[0];
    const k = dk[1];
    const mantStr = digits.length === 1 ? digits : digits[0] + "." + digits.slice(1);
    pushCopy(mantStr + eChar + (k - 1));
  } else {
    const eChar = mode === 3 ? "E" : "e";
    const prec = param;
    if (mag === 0) {
      if (prec === 0) pushCopy("0" + eChar + "0");
      else {
        pushCopy("0.");
        pushZero(prec);
        pushCopy(eChar + "0");
      }
    } else {
      const decomp = globalThis.__rt._f64_decomp(mag);
      const mant = decomp[0];
      const exp2 = decomp[1];
      let e10 = Math.floor(Math.log10(mag));
      const cmpPow10 = (p) => {
        let lhs = mant;
        let rhs = ONE;
        if (exp2 >= 0) lhs <<= BigInt(exp2);
        else rhs <<= BigInt(-exp2);
        if (p >= 0) rhs *= TEN ** BigInt(p);
        else lhs *= TEN ** BigInt(-p);
        return lhs < rhs ? -1 : (lhs > rhs ? 1 : 0);
      };
      while (cmpPow10(e10) < 0) e10 -= 1;
      while (cmpPow10(e10 + 1) >= 0) e10 += 1;
      const p = prec - e10;
      const shift = exp2 < 0 ? -exp2 : 0;
      let extraZeros = 0;
      let effP = p;
      if (effP > shift) {
        extraZeros = effP - shift;
        effP = shift;
      }
      let num = mant;
      let den = ONE;
      if (effP >= 0) num *= TEN ** BigInt(effP);
      else den *= TEN ** BigInt(-effP);
      if (exp2 >= 0) num <<= BigInt(exp2);
      else den <<= BigInt(-exp2);
      let q = num / den;
      const rem = num % den;
      const cmp = rem * TWO;
      if (cmp > den || (cmp === den && (q & ONE) !== ZERO)) q += ONE;
      let s = q.toString();
      if (s.length + extraZeros > prec + 1) {
        e10 += 1;
        if (extraZeros > 0) extraZeros -= 1;
        else s = s.slice(0, -1);
      }
      if (prec === 0) {
        pushCopy(s + eChar + e10);
      } else {
        pushCopy(s.length === 1 ? s + "." : s[0] + "." + s.slice(1));
        pushZero(extraZeros);
        pushCopy(eChar + e10);
      }
    }
  }
  mbuf[moff + 1] = np;
};

// Host-delegated Rust dec2flt (`<f64 as FromStr>::from_str` & `<f32 as FromStr>::from_str`).
// Strict Rust Float EBNF grammar (ASCII-only, no whitespace, no underscores, no 0x).
// Returns `undefined` on invalid/empty input; otherwise the exact IEEE-754 float.
globalThis.__rt._flt_re = /^[+-]?(?:(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?|[iI][nN][fF](?:[iI][nN][iI][tT][yY])?|[nN][aA][nN])$/;

globalThis.__rt._parse_flt_special = (s) => {
  let i = 0;
  let neg = false;
  const c0 = s.charCodeAt(0);
  if (c0 === 45) {
    neg = true;
    i = 1;
  } else if (c0 === 43) {
    i = 1;
  }
  const c = s.charCodeAt(i);
  if (c === 105 || c === 73) return neg ? -Infinity : Infinity;
  if (c === 110 || c === 78) {
    const dv = globalThis.__rt._num_dv;
    dv.setUint32(0, neg ? 0xfff80000 : 0x7ff80000, false);
    dv.setUint32(4, 0, false);
    return dv.getFloat64(0, false);
  }
  return null;
};

globalThis.__rt.parse_f64 = (s) => {
  if (!globalThis.__rt._flt_re.test(s)) return undefined;
  const sp = globalThis.__rt._parse_flt_special(s);
  if (sp !== null) return sp;
  return Number(s);
};

globalThis.__rt._cmp_dec_to_pow2 = (s, mant, exp2) => {
  const TEN = BigInt("10");
  let i = 0;
  const n = s.length;
  if (s.charCodeAt(0) === 43 || s.charCodeAt(0) === 45) i = 1;
  let expAt = s.indexOf("e", i);
  if (expAt < 0) expAt = s.indexOf("E", i);
  let exp10 = 0;
  if (expAt >= 0) {
    const parsedExp = Number(s.slice(expAt + 1));
    if (!Number.isFinite(parsedExp) || parsedExp > 1000000) exp10 = 1000000;
    else if (parsedExp < -1000000) exp10 = -1000000;
    else exp10 = parsedExp;
  } else {
    expAt = n;
  }
  let sig = "";
  let seenDot = false;
  let fracCount = 0;
  let droppedRight = 0;
  let sticky = false;
  for (let j = i; j < expAt; j++) {
    const ch = s.charCodeAt(j);
    if (ch === 46) {
      seenDot = true;
      continue;
    }
    if (sig.length === 0 && ch === 48) {
      if (seenDot) fracCount += 1;
      continue;
    }
    if (sig.length < 200) {
      sig += s[j];
      if (seenDot) fracCount += 1;
    } else {
      if (!seenDot) droppedRight += 1;
      if (ch !== 48) sticky = true;
    }
  }
  if (sig.length === 0) return -1;
  let p10 = exp10 - fracCount + droppedRight;
  while (sig.length > 1 && sig.charCodeAt(sig.length - 1) === 48) {
    sig = sig.slice(0, -1);
    p10 += 1;
  }
  const mag10 = sig.length + p10;
  if (mag10 > 50) return 1;
  if (mag10 < -60) return -1;
  let lhs = BigInt(sig);
  let rhs = mant;
  if (p10 >= 0) lhs *= TEN ** BigInt(p10);
  else rhs *= TEN ** BigInt(-p10);
  if (exp2 >= 0) rhs <<= BigInt(exp2);
  else lhs <<= BigInt(-exp2);
  if (lhs < rhs) return -1;
  if (lhs > rhs) return 1;
  return sticky ? 1 : 0;
};

globalThis.__rt.parse_f32 = (s) => {
  if (!globalThis.__rt._flt_re.test(s)) return undefined;
  const sp = globalThis.__rt._parse_flt_special(s);
  if (sp !== null) return sp;
  const d = Number(s);
  const f = Math.fround(d);
  if (d === 0 || !Number.isFinite(d)) return f;
  const neg = d < 0;
  const mag = neg ? -d : d;
  const dv = globalThis.__rt._num_dv;
  dv.setFloat64(0, mag, false);
  const hi = dv.getUint32(0, false);
  const lo = dv.getUint32(4, false);
  const exp64 = (hi >>> 20) & 0x7ff;
  if (exp64 < 873 || exp64 > 1150) return f;
  let mant;
  if (exp64 >= 897) {
    if ((lo & 0x1fffffff) !== 0x10000000) return f;
    mant = (BigInt(0x100000 | (hi & 0xfffff)) << BigInt("32")) | BigInt(lo);
  } else {
    const bit = 925 - exp64;
    const ONE = BigInt("1");
    mant = (BigInt(0x100000 | (hi & 0xfffff)) << BigInt("32")) | BigInt(lo);
    if ((mant & ((ONE << BigInt(bit + 1)) - ONE)) !== (ONE << BigInt(bit))) return f;
  }
  const cmp = globalThis.__rt._cmp_dec_to_pow2(s, mant, exp64 - 1075);
  if (cmp === 0) return f;
  const fmag = neg ? -f : f;
  if (cmp < 0 && fmag <= mag) return f;
  if (cmp > 0 && fmag >= mag) return f;
  dv.setFloat32(0, fmag, false);
  const u32 = dv.getUint32(0, false);
  dv.setUint32(0, cmp < 0 ? u32 - 1 : u32 + 1, false);
  const adj = dv.getFloat32(0, false);
  return neg ? -adj : adj;
};

// A mutable native String is `{ text, cap, len }`: well-formed text (see
// native_str_wf), its Rust-visible capacity and its exact UTF-8 byte length,
// both in bytes. Every operation keeps `len` in step with the text, so nothing
// rescans the text to learn its length -- which is what would make building a
// string quadratic. The compiled reserve path has already established enough
// capacity, so pushing a Rust `char` appends it and advances the length by its
// width.
globalThis.__rt.native_str_push = (value, point) => {
  value.text += String.fromCodePoint(point);
  value.len += globalThis.__rt._utf8_width(point);
};

// Returns -1 for `None`; every Rust `char` is otherwise a non-negative Unicode
// scalar value. Capacity is retained, as it is by `String::pop`; the length
// drops by the popped scalar's width.
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
  value.len -= globalThis.__rt._utf8_width(point);
  return point;
};

// Rust makes an index at or past the current byte length a no-op and rejects an
// in-range index that is not a char boundary. Capacity is retained; the length
// becomes the index.
globalThis.__rt.native_str_truncate = (value, byteIndex) => {
  if (byteIndex >= value.len) return;
  const text = value.text;
  const utf16 = globalThis.__rt.native_str_utf16_index(text, byteIndex);
  if (utf16 < 0) {
    throw __rt.abort_error(new RangeError("native String truncate index is not a UTF-8 boundary"));
  }
  value.text = text.slice(0, utf16);
  value.len = byteIndex;
};

// _sb_check(value): the `RCGJS_SB_VERIFY=1` drift check the compiler emits after
// every mutation of a native String and around every read of its length. The
// text must be well formed (a lone surrogate would decode differently from the
// baseline's bytes), the cached `len` must be its UTF-8 byte length and `cap`
// at least that; anything else is a compiler bug, reported before the length is
// used. Returns the value, so a checked length read is
// `__rt._sb_check(value).len`.
globalThis.__rt._sb_check = (value) => {
  const measured = globalThis.__rt.str_len(value.text);
  const wellFormed = globalThis.__rt.native_str_wf(value.text) === value.text;
  if (value.len !== measured || !(value.cap >= measured) || !wellFormed) {
    const detail = "len " + value.len + ", cap " + value.cap + ", text measures " + measured +
      (wellFormed ? "" : ", text has a lone surrogate");
    throw __rt.abort_error(new RangeError("native String length drift: " + detail));
  }
  return value;
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
// entry; `_freed` is the key of a mark `dealloc` writes onto the emptied array
// of a block it frees, kept only so that a use after free is a refusal naming
// itself rather than a read of an emptied array. The mark lives on the array,
// not in a side table: a `WeakSet` insert per free cost an allocation-heavy
// program a third of its run time.
globalThis.__rt._blocks = new WeakMap();
let _blkK = null;
let _blkV;
globalThis.__rt._freed = Symbol("rustc_codegen_js.freed");
// Stack arrays reached through `MaybeUninit<u8>` need the same one-time scale
// bookkeeping without becoming heap blocks. In particular, `_put` must not
// attach heap ownership metadata to values stored in sort scratch space.
globalThis.__rt._uninit_blocks = new WeakMap();

globalThis.__rt._blk_get = (buf) => {
  if (buf === _blkK) return _blkV;
  const meta = globalThis.__rt._blocks.get(buf);
  if (meta !== undefined) {
    _blkK = buf;
    _blkV = meta;
  }
  return meta;
};

// _heap_block(buf): the block record for a buffer, or undefined if it is not
// one. A freed block is a refusal rather than a miss: the mistake is worth
// naming.
globalThis.__rt._heap_block = (buf) => {
  const block = globalThis.__rt._blk_get(buf);
  if (block !== undefined) return block;
  if (buf != null && buf[globalThis.__rt._freed] === true) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this pointer names a heap block that has been freed",
    ));
  }
  return undefined;
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
globalThis.__rt._retype = (p, block, size, mk, kind, cl) => {
  const raw = globalThis.__rt._bv !== null ? globalThis.__rt._bv.unwrap_buf(p.buf) : p.buf;
  const fromBv = raw !== p.buf;
  const isRaw = block.typed !== undefined && raw !== block.typed;
  const active = block.typed !== undefined ? block.typed : raw;
  const same = block.es === size;
  if (same && !(block.fresh && block.zeroed)) {
    if (cl !== undefined && block.cl === undefined) block.cl = cl;
    if (isRaw || fromBv) {
      if (size > 1 && p.off !== 0) {
        if (p.off % size !== 0) throw __rt.abort_error(new Error("rustc_codegen_js: byte offset " + p.off + " is not a multiple of " + size + ", so it does not name an element"));
        return { buf: active, off: p.off / size };
      }
      return { buf: active, off: p.off };
    }
    return p;
  }
  if (!same) {
    if (!block.fresh) throw __rt.abort_error(new Error("rustc_codegen_js: this heap block already holds " + block.es + "-byte elements, so it cannot be read as " + size + "-byte ones"));
    if (block.zeroed && block.bytes % size !== 0) throw __rt.abort_error(new Error("rustc_codegen_js: a heap block of " + block.bytes + " bytes is not a whole number of " + size + "-byte elements"));
  }
  const byte_off = isRaw || fromBv ? p.off : p.off * block.es;
  if (byte_off % size !== 0) throw __rt.abort_error(new Error("rustc_codegen_js: byte offset " + byte_off + " is not a multiple of " + size + ", so it does not name an element"));
  const length = Math.floor(block.bytes / size);
  let target = active;
  if (!same) {
    const typed = new Array(length);
    raw.length = 0;
    block.raw = raw; block.typed = typed; block.es = size;
    if (kind !== undefined) block.sk = kind;
    if (cl !== undefined) block.cl = cl;
    if (block.uninit) {
      globalThis.__rt._uninit_blocks.set(typed, block);
    } else {
      globalThis.__rt._blocks.set(typed, block);
      _blkK = typed; _blkV = block;
      if (_has_minted_addrs) {
        const base = globalThis.__rt._addrs.get(raw);
        if (base !== undefined) {
          globalThis.__rt._addrs.set(typed, base);
          const k = globalThis.__rt._addrHeap.get(raw);
          if (k !== undefined) { globalThis.__rt._addrHeap.set(typed, k); globalThis.__rt._addrHeap.delete(raw); }
        }
      }
    }
    target = typed;
    if (!block.zeroed && block.bytes === size && typeof mk === "function") {
      const shell = mk();
      if (shell !== null && typeof shell === "object") globalThis.__rt._put(typed, 0, shell);
    }
  } else if (cl !== undefined && block.cl === undefined) block.cl = cl;
  block.fresh = false;
  if (block.zeroed) {
    if (mk == null) throw __rt.abort_error(new Error("rustc_codegen_js: a zeroed heap block cannot be read as " + size + "-byte elements, because this backend cannot spell a zero one"));
    for (let i = 0; i < length; i++) {
      const v = mk();
      target[i] = v;
      if (v !== null && typeof v === "object" && !block.uninit && globalThis.__rt._boxes !== null) globalThis.__rt._boxes.set(v, { buf: target, off: i });
    }
  }
  return same && !fromBv && !isRaw ? p : { buf: target, off: byte_off / size };
};

// unscale_uninit(p, size, mk): the `MaybeUninit<u8>` storage form of
// `unscale`. Rust also uses aligned stack byte arrays as scratch storage and
// casts them to `MaybeUninit<T>`. The union at the source proves that no value
// is observed through the old element shape, so the array may acquire an
// element scale exactly once.
globalThis.__rt.unscale_uninit = (p, size, mk, cl) => {
  if (typeof p === "number" || p === null || typeof p !== "object" || p.sc !== undefined) {
    return globalThis.__rt.unscale(p, size, mk, cl);
  }
  const raw = globalThis.__rt._bv !== null && p.buf !== null && p.buf !== undefined
    ? globalThis.__rt._bv.unwrap_buf(p.buf)
    : p.buf;
  let block = globalThis.__rt._heap_block(raw);
  if (block === undefined) {
    if (!Array.isArray(raw)) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: uninitialized storage does not have an array backing buffer",
      ));
    }
    block = globalThis.__rt._uninit_blocks.get(raw);
    if (block === undefined) {
      block = {
        bytes: raw.length,
        es: 1,
        fresh: true,
        zeroed: false,
        uninit: true,
      };
      globalThis.__rt._uninit_blocks.set(raw, block);
    }
  }
  return globalThis.__rt._retype(p, block, size, mk, undefined, cl);
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
  const raw = globalThis.__rt._bv !== null && p.buf !== null && p.buf !== undefined
    ? globalThis.__rt._bv.unwrap_buf(p.buf)
    : p.buf;
  const block = globalThis.__rt._heap_block(raw);
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
  let buf = p === null || typeof p !== "object" ? null : p.buf;
  if (globalThis.__rt._bv !== null && buf !== null && buf !== undefined) {
    buf = globalThis.__rt._bv.unwrap_buf(buf);
  }
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
  const active = block.typed !== undefined ? block.typed : buf;
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
    return { ptr: { buf: active, off: 0 }, meta: len };
  }
  if (header_mk === null || header_mk === undefined) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: this backend cannot spell the header of a struct with " +
        "an unsized tail, so its block cannot be reshaped",
    ));
  }
  for (let i = 0; i < active.length; i++) {
    if (active[i] !== undefined) {
      throw __rt.abort_error(new Error(
        "rustc_codegen_js: this heap block has already been written, so it " +
          "cannot be reshaped into a header plus an unsized tail",
      ));
    }
  }
  active.length = 1 + len;
  active[0] = header_mk();
  block.es = tail_es;
  block.hdr = 1;
  block.fresh = false;
  return { ptr: { buf: active, off: 0 }, meta: len };
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
  let sbuf = slot === undefined ? undefined : slot.buf;
  if (globalThis.__rt._bv !== null && sbuf !== undefined) {
    sbuf = globalThis.__rt._bv.unwrap_buf(sbuf);
  }
  const block = sbuf === undefined ? undefined : globalThis.__rt._heap_block(sbuf);
  if (block !== undefined && block.hdr !== undefined) {
    return { buf: slot.buf, off: slot.off + block.hdr, len };
  }
  return { buf: record[key], off: 0, len };
};

// _register(buf, bytes, zeroed): a new block, and the pointer to its start.
globalThis.__rt._register = (buf, bytes, zeroed) => {
  const meta = { bytes, es: 1, fresh: true, zeroed };
  globalThis.__rt._blocks.set(buf, meta);
  _blkK = buf;
  _blkV = meta;
  return { buf, off: 0 };
};

// alloc(size, align): `__rust_alloc`. Returns a plain byte-granular array;
// `_retype` allocates a distinct typed array on first wider cast so pre-retype
// `*mut u8` and post-retype `*mut T` pointers stay unambiguous with zero Proxy.
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
// `undefined` rather than the value it used to hold, and the `_freed` mark turns
// a later cast through that pointer into a refusal that says so. Its synthetic
// address, if it was ever asked for, goes back to the free list (`addr`).
globalThis.__rt.dealloc = (p, size, align) => {
  let buf = p === null || typeof p !== "object" ? null : p.buf;
  if (globalThis.__rt._bv !== null && buf != null) buf = globalThis.__rt._bv.unwrap_buf(buf);
  const block = buf === null ? undefined : globalThis.__rt._blk_get(buf);
  if (block === undefined) {
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer does not name a live heap block, so it cannot be deallocated"));
  }
  const active = block.typed !== undefined ? block.typed : buf;
  globalThis.__rt._blocks.delete(buf);
  if (block.raw !== undefined) {
    globalThis.__rt._blocks.delete(block.raw);
    block.raw.length = 0;
    block.raw[globalThis.__rt._freed] = true;
  }
  if (block.typed !== undefined) globalThis.__rt._blocks.delete(block.typed);
  if (_blkK === buf || _blkK === block.raw || _blkK === block.typed) { _blkK = null; _blkV = undefined; }
  if (_has_minted_addrs) globalThis.__rt._addr_release(active);
  active.length = 0;
  active[globalThis.__rt._freed] = true;
};

globalThis.__rt.realloc = (p, size, align, new_size) => {
  let buf = p === null || typeof p !== "object" ? null : p.buf;
  if (globalThis.__rt._bv !== null && buf != null) buf = globalThis.__rt._bv.unwrap_buf(buf);
  const block = buf === null ? undefined : globalThis.__rt._heap_block(buf);
  if (block === undefined) {
    throw __rt.abort_error(new Error("rustc_codegen_js: this pointer does not name a live heap block, so it cannot be reallocated"));
  }
  if (new_size % block.es !== 0) {
    throw __rt.abort_error(new Error("rustc_codegen_js: a heap block of " + new_size + " bytes is not a whole number of " + block.es + "-byte elements"));
  }
  const active = block.typed !== undefined ? block.typed : buf;
  block.bytes = new_size;
  active.length = new_size / block.es;
  globalThis.__rt._addr_release(active, new_size);
  if (p.sc !== undefined) {
    let raw = block.raw;
    if (raw === undefined) {
      raw = [];
      block.raw = raw;
      block.typed = active;
      globalThis.__rt._blocks.set(raw, block);
    }
    return { buf: raw, off: p.off };
  }
  return { buf: p.buf, off: p.off };
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
// `{ text, cap, len }` (Phase 4 Convert adapter). The text is taken as Rust
// holds it (native_str_wf), and an immutable host string's capacity is its
// UTF-8 byte length, measured once: the builder's length and its exact-fit
// capacity both.
globalThis.__rt.convert_host_sb = (s) => {
  const text = globalThis.__rt.native_str_wf(s);
  const len = globalThis.__rt.str_len(text);
  return { text: text, cap: len, len: len };
};

// convert_str_block(s): the UTF-8 bytes of a host string as a fresh `{ buf, off,
// len }` block the caller owns (Phase 4 Convert adapter, host string back to the
// baseline `Vec<u8>` String). `str_bytes` may return a memoized record whose
// buffer other `as_bytes` calls alias, and an owned baseline String must not
// share its allocation, so the bytes are copied into a new plain array.
globalThis.__rt.convert_str_block = (s) => {
  const shared = globalThis.__rt.str_bytes(s);
  const copy = shared.buf.slice(shared.off, shared.off + shared.len);
  // The baseline String's byte block is a heap block: reads reach it through
  // `unscale`, which answers from the block registry, so the fresh copy is
  // registered exactly as `alloc` would have (already element-typed at 1 byte).
  const p = globalThis.__rt._register(copy, copy.length, false);
  globalThis.__rt._blk_get(copy).fresh = false;
  // A host string is immutable and exact-fit: its capacity is its byte length.
  return { buf: p.buf, off: 0, len: shared.len, cap: shared.len };
};

// convert_sb_block(sb): a mutable native `{ text, cap, len }` String's bytes as
// a fresh registered `{ buf, off, len, cap }` block (Phase 4 Convert adapter,
// builder back to the baseline `Vec<u8>` String). The backing array is capacity
// length with `[0, len)` initialized, and the builder's Rust-visible capacity
// rides along for the baseline cap leaf — a moved String keeps its capacity.
// The bytes have to be encoded here anyway, so the block's length is theirs.
globalThis.__rt.convert_sb_block = (sb) => {
  const shared = globalThis.__rt.str_bytes(sb.text);
  const cap = Math.max(sb.cap, shared.len);
  const copy = shared.buf.slice(shared.off, shared.off + shared.len);
  copy.length = cap;
  const p = globalThis.__rt._register(copy, cap, false);
  globalThis.__rt._blk_get(copy).fresh = false;
  return { buf: p.buf, off: 0, len: shared.len, cap };
};

// convert_nv_block(nv, size): a native `{ buf, len, cap }` Vec's elements as a
// registered baseline heap block of `size`-byte elements (Phase 4 Convert
// adapter, native Vec back to the baseline raw parts). The block keeps the
// carrier's capacity and is already element-typed, exactly as an `alloc` +
// first-cast retype would have left it.
globalThis.__rt.convert_nv_block = (nv, size, kind) => {
  const copy = nv.buf.slice(0, nv.len);
  copy.length = Math.max(nv.cap, nv.len);
  const p = globalThis.__rt._register(copy, copy.length * size, false);
  const block = globalThis.__rt._blk_get(copy);
  block.es = size;
  const sk = typeof kind === "string" ? kind : undefined;
  if (sk !== undefined) block.sk = sk;
  const cl = typeof kind === "function" || kind === true ? kind : undefined;
  if (cl !== undefined) {
    block.cl = cl;
    if (globalThis.__rt._boxes !== null) {
      for (let i = 0; i < nv.len; i++) {
        const v = copy[i];
        if (v !== null && typeof v === "object" && !globalThis.__rt._boxes.has(v)) {
          globalThis.__rt._boxes.set(v, { buf: copy, off: i });
        }
      }
    }
  }
  block.fresh = false;
  if (size > 1) {
    const raw = [];
    block.raw = raw;
    block.typed = copy;
    globalThis.__rt._blocks.set(raw, block);
    return { buf: raw, off: 0 };
  }
  return p;
};

// convert_block_nv(buf, off, len, cap): a baseline Vec's raw parts entering the
// native `{ buf, len, cap }` carrier (Phase 4 Convert adapter). The backing
// array is capacity length with `[0, len)` initialized, exactly the carrier's
// contract; the copy keeps the baseline block unshared.
globalThis.__rt.convert_block_nv = (buf, off, len, cap) => {
  if (globalThis.__rt._bv !== null) {
    const u = globalThis.__rt._bv.unwrap_slot(buf, off);
    buf = u.buf;
    off = u.off;
  }
  const blk = globalThis.__rt._blk_get(buf);
  if (blk !== undefined && blk.typed !== undefined && buf !== blk.typed) {
    buf = blk.typed;
    off /= blk.es;
  }
  const copy = new Array(cap);
  for (let i = 0; i < len; i += 1) copy[i] = buf[off + i];
  return { buf: copy, len, cap };
};

// convert_block_sv(p, len): a baseline Vec's raw pointer entering SliceView,
// unwrapping any aliasing byte view or pre-retype raw array.
globalThis.__rt.convert_block_sv = (p, len) => {
  if (typeof p === "number" && p > 0 && p < 4096) p = globalThis.__rt._dangling(p);
  let buf = p.buf;
  let off = p.off;
  if (p.sc !== undefined) {
    off /= p.sc;
  } else {
    if (globalThis.__rt._bv !== null) {
      const u = globalThis.__rt._bv.unwrap_slot(buf, off);
      buf = u.buf;
      off = u.off;
    }
    const blk = globalThis.__rt._blk_get(buf);
    if (blk !== undefined && blk.typed !== undefined && buf !== blk.typed) {
      buf = blk.typed;
      off /= blk.es;
    }
  }
  return { buf, off, len };
};

// convert_sv_nv(s): a `{ buf, off, len }` slice view entering the native Vec
// carrier as an owned exact-fit value (Phase 4 Convert adapter; the planner
// only converts a dead, unaliased source this way).
globalThis.__rt.convert_sv_nv = (s) => ({
  buf: s.buf.slice(s.off, s.off + s.len),
  len: s.len,
  cap: s.len,
});

// The `%TypedArray%.prototype` accessors, captured when the runtime loads. They
// read a typed array's internal slots, so an own `length` or `subarray`
// property a host defines on one instance is never consulted, and the tag is
// `undefined` (never an exception, never a trap) for anything that is not a
// typed array, a Proxy included.
globalThis.__rt._typed_array = ((prototype) => ({
  length: Object.getOwnPropertyDescriptor(prototype, "length").get,
  tag: Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag).get,
  subarray: prototype.subarray,
}))(Object.getPrototypeOf(Uint8Array.prototype));

// u8array_from_bytes(s): a Rust byte slice `{ buf, off, len }` copied into a
// fresh exact-length `Uint8Array` (mires-js `JsUint8Array::from_slice`). One
// host operation per conversion: the loop is here, not one boundary call per
// byte in the program. The result never aliases Rust storage. A store into a
// `Uint8Array` wraps, so an `i8` element is read as its byte, and typed
// backing storage is copied with one `set`. A zero length slice reads nothing;
// its buffer may be a frozen dangling array or absent.
//
// The operation raises nothing but a registered abort, which is what lets the
// compiler classify it as non-throwing (`HostOperation::may_throw`). The record
// is checked before anything is read through it -- an integer length and
// offset, a plain array or typed array buffer that holds the range -- and
// whatever else goes wrong (an allocation the engine refuses, storage that is
// not bytes) becomes the same abort.
globalThis.__rt.u8array_from_bytes = (s) => {
  try {
    const len = s.len;
    if (Number.isInteger(len) && len >= 0) {
      const out = new Uint8Array(len);
      if (len === 0) return out;
      const buf = s.buf;
      const off = s.off;
      if (Number.isInteger(off) && off >= 0) {
        if (Array.isArray(buf)) {
          if (off + len <= buf.length) {
            for (let i = 0; i < len; i += 1) out[i] = buf[off + i];
            return out;
          }
        } else if (ArrayBuffer.isView(buf)) {
          const typed = globalThis.__rt._typed_array;
          if (off + len <= Reflect.apply(typed.length, buf, [])) {
            out.set(Reflect.apply(typed.subarray, buf, [off, off + len]));
            return out;
          }
        }
      }
    }
  } catch (error) {
    // Reported below, without reading the value that was thrown.
  }
  throw __rt.abort_error(new Error(
    "rustc_codegen_js: this value is not a Rust byte slice, so it cannot be copied into a Uint8Array",
  ));
};

// u8array_length(view): the length of a `Uint8Array` from its internal slots
// (mires-js `JsUint8Array::len`), so an own `length` property cannot shadow
// it. A detached view has length zero. No host code runs: anything that is not
// a `Uint8Array` -- a Proxy, another typed array, a host value a declaration
// mistyped -- is refused with the same `TypeError` a mismatched host read
// raises (`rcgjs.host.badread`), so cleanup still runs on the way out.
globalThis.__rt.u8array_length = (view) => {
  const typed = globalThis.__rt._typed_array;
  if (Reflect.apply(typed.tag, view, []) !== "Uint8Array") {
    throw new TypeError("host value does not match its Rust representation");
  }
  return Reflect.apply(typed.length, view, []);
};

// u8array_into_bytes(src, dst): a `Uint8Array`'s bytes written into the Rust
// byte slice `dst` (mires-js `JsUint8Array::copy_to_slice` and `to_vec`, which
// size `dst` from `u8array_length`). The lengths are checked again here, from
// the internal slots, so the helper is fail-closed on its own: a mismatch
// aborts before anything is written. The writes are the element stores Rust
// itself would make, so the storage keeps its element type, and nothing
// aliases `src` afterwards.
globalThis.__rt.u8array_into_bytes = (src, dst) => {
  const len = dst.len;
  if (globalThis.__rt.u8array_length(src) !== len) {
    throw __rt.abort_error(new Error(
      "rustc_codegen_js: a Uint8Array does not hold as many bytes as its Rust destination",
    ));
  }
  const buf = dst.buf;
  const off = dst.off;
  for (let i = 0; i < len; i += 1) buf[off + i] = src[i];
};

// Exact BigInt -> f32 conversion without intermediate f64 double-rounding when
// |x| > 2^53. Collapses bits below the top 53 bits of |x| into a single sticky
// LSB before scaling by the exact power of 2 and rounding once with Math.fround.
globalThis.__rt.i2f32_big = (x) => {
  const neg = x < BigInt("0");
  const n = neg ? -x : x;
  const bits = n.toString(2).length;
  if (bits <= 53) return Math.fround(Number(x));
  const shift = BigInt(bits - 53);
  const q = n >> shift;
  const sticky = (q << shift) !== n ? BigInt("1") : BigInt("0");
  const scaled = Number(q | sticky) * Math.pow(2, bits - 53);
  return Math.fround(neg ? -scaled : scaled);
};

// slice_sort(buf, off, len, isLess, cmpFn, isIndirect): host-delegated
// `slice::sort*` and `slice::sort_unstable*`. Uses a spec-stable iterative
// bottom-up merge sort over an index array so ES2018 stability is guaranteed
// and no element moves until every comparison returns (exact 1x drop &
// interior-mutability preservation if `cmpFn` throws/panics, and always a valid
// permutation even for inconsistent/NaN comparators).
globalThis.__rt.slice_sort = (buf, off, len, isLess, cmpFn, isIndirect) => {
  if (len < 2) return;
  let a = new Array(len);
  let b = new Array(len);
  for (let i = 0; i < len; i++) a[i] = i;
  const ref = isIndirect
    ? (i) => buf[off + i]
    : (i) => ({ buf: buf, off: off + i });
  for (let w = 1; w < len; w *= 2) {
    for (let lo = 0; lo < len; lo += 2 * w) {
      const mid = lo + w < len ? lo + w : len;
      const hi = lo + 2 * w < len ? lo + 2 * w : len;
      let i = lo;
      let j = mid;
      let k = lo;
      if (mid < hi && cmpFn(isLess, ref(a[mid]), ref(a[mid - 1]))) {
        while (i < mid && j < hi) {
          b[k++] = cmpFn(isLess, ref(a[j]), ref(a[i])) ? a[j++] : a[i++];
        }
      }
      while (i < mid) b[k++] = a[i++];
      while (j < hi) b[k++] = a[j++];
    }
    const t = a;
    a = b;
    b = t;
  }
  const tmp = new Array(len);
  for (let i = 0; i < len; i++) tmp[i] = buf[off + i];
  const boxes = globalThis.__rt._boxes;
  const track = boxes !== null && isIndirect && globalThis.__rt._blk_get(buf) !== undefined;
  for (let i = 0; i < len; i++) {
    const val = tmp[a[i]];
    buf[off + i] = val;
    if (track && val !== null && typeof val === "object") {
      boxes.set(val, { buf: buf, off: off + i });
    }
  }
};

// Host-delegated Unicode case mapping for `alloc::str::{to_lowercase,to_uppercase}`.
// `host_str_to_*` is called via `extern "C"` from `alloc/src/str.rs` (arguments and
// returns already cross `bstr_out`/`bstr_in` under `js-str=bytes`); `str_to_*` /
// `bstr_to_*` are used directly by native String lowering with an ASCII fast path.
globalThis.__rt.host_str_to_lowercase = (s) => s.toLowerCase();
globalThis.__rt.host_str_to_uppercase = (s) => s.toUpperCase();
globalThis.__rt.str_to_lowercase = (s) => globalThis.__rt.native_str_wf(s).toLowerCase();
globalThis.__rt.str_to_uppercase = (s) => globalThis.__rt.native_str_wf(s).toUpperCase();
globalThis.__rt.bstr_to_lowercase = (s) =>
  globalThis.__rt._bstr_is_ascii(s)
    ? s.toLowerCase()
    : globalThis.__rt.bstr_in(globalThis.__rt.bstr_out(s).toLowerCase());
globalThis.__rt.bstr_to_uppercase = (s) =>
  globalThis.__rt._bstr_is_ascii(s)
    ? s.toUpperCase()
    : globalThis.__rt.bstr_in(globalThis.__rt.bstr_out(s).toUpperCase());
