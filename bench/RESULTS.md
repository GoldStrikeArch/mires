# krausest js-framework-benchmark: Topcoat's Rust->JS compiler

Two entries compiled from Rust by the spike's `rustc_codegen_mires` backend, measured by the
real upstream harness on this machine against vanillajs (keyed and non-keyed), solid and
leptos. Both entries are submitted as **non-keyed**; the harness's own `isKeyed` detector
gates that before anything is measured.

## Duration

Median of the harness's iterations, in milliseconds, with the slowdown against
`keyed/vanillajs` in brackets.
Lower is better.

| benchmark                              | vanillajs keyed | solid keyed    | leptos keyed   | vanillajs non-keyed | mires-vanilla | mires-island |
|:-------------------------------------- | ---------------:| --------------:| --------------:| -------------------:| ---------------:| --------------:|
| create 1,000 rows                      | 20.2 (1.00)     | 21.4 (1.06)    | 25.7 (1.27)    | 20.2 (1.00)         | 24.5 (1.21)     | 20.8 (1.03)    |
| replace all 1,000 rows                 | 22.4 (1.00)     | 24.2 (1.08)    | 29.7 (1.33)    | 10.4 (0.46)         | 13.4 (0.60)     | 23.4 (1.04)    |
| partial update (every 10th, x16) [^t4] | 9.9 (1.00)      | 13.0 (1.31)    | 12.4 (1.25)    | 20.5 (2.07)         | 24.7 (2.49)     | 116.0 (11.72)  |
| select row [^t4]                       | 4.3 (1.00)      | 5.8 (1.35)     | 5.8 (1.35)     | 2.4 (0.56)          | 2.4 (0.56)      | 118.1 (27.47)  |
| swap rows [^t4]                        | 11.9 (1.00)     | 12.7 (1.07)    | 12.4 (1.04)    | 7.6 (0.64)          | 8.4 (0.71)      | 86.7 (7.29)    |
| remove row [^t2]                       | 15.4 (1.00)     | 15.9 (1.03)    | 10.2 (0.66)    | 17.6 (1.14)         | 20.4 (1.32)     | 45.1 (2.93)    |
| create 10,000 rows                     | 251.2 (1.00)    | 269.6 (1.07)   | 267.6 (1.07)   | 223.3 (0.89)        | 253.0 (1.01)    | 226.0 (0.90)   |
| append 1,000 to 10,000                 | 29.4 (1.00)     | 30.8 (1.05)    | 28.2 (0.96)    | 23.2 (0.79)         | 27.7 (0.94)     | 43.4 (1.48)    |
| clear 1,000 rows [^t4]                 | 9.9 (1.00)      | 14.1 (1.42)    | 18.8 (1.90)    | 8.5 (0.86)          | 9.1 (0.92)      | 9.5 (0.96)     |
| **geometric mean of slowdowns** [^geo] | **1.00** (n=9)  | **1.15** (n=9) | **1.16** (n=9) | **0.85** (n=9)      | **0.98** (n=9)  | **2.76** (n=9) |

[^t2]: run with a 2x CPU slowdown applied by the harness.
[^t4]: run with a 4x CPU slowdown applied by the harness.

[^geo]: **This is not the ranking number from the published table.** Upstream's is a
    weighted geometric mean over values normalised against the p90 of the entire
    published population, measured on the maintainer's machine; that population is not
    reproducible locally and a locally invented weighting would be worse than none. This
    is the plain geometric mean of the per-benchmark slowdowns against `keyed/vanillajs`,
    over the benchmarks that ran (`n`). Comparing it to a published geomean is comparing
    two different statistics.

## Memory

| benchmark (MB)                   | vanillajs keyed | solid keyed | leptos keyed | vanillajs non-keyed | mires-vanilla | mires-island |
|:-------------------------------- | ---------------:| -----------:| ------------:| -------------------:| ---------------:| --------------:|
| ready memory                     | 0.57            | 0.54        | 1.71         | 0.49                | 0.61            | 0.74           |
| memory after adding 1,000 rows   | 1.87            | 2.68        | 5.38         | 1.84                | 2.35            | 2.29           |
| memory after clearing 1,000 rows | 0.63            | 0.77        | 4.51         | 0.63                | 2.24            | 0.97           |

## Transfer size and first paint

Measured by the harness, which counts every response the page pulls (excluding `/css`,
which is served from the harness root and shared by every implementation) and brotli-
compresses the total itself.

| measure                         | vanillajs keyed | solid keyed | leptos keyed | vanillajs non-keyed | mires-vanilla | mires-island |
|:------------------------------- | ---------------:| -----------:| ------------:| -------------------:| ---------------:| --------------:|
| transferred (brotli) (KB)       | 2.5             | 4.5         | 48.8         | 2.4                 | 9.8             | 21.2           |
| transferred (uncompressed) (KB) | 11.3            | 11.5        | 189.6        | 12.0                | 44.1            | 71.2           |
| first paint (ms)                | 53.7            | 58.9        | 241.1        | 58.3                | 81.6            | 47.9           |

## What our payload is made of

The table above gives one brotli number per implementation, which is the right number for
comparing them and no help at all in understanding one. These are the same two entries
broken down per delivered file, measured with the spike's own convention: **gzip at a pinned
level 9, summed per artifact** (`contract/parity/lib/budget.mjs`), because that is what the
rest of the spike's budgets are recorded in.

**These numbers are not comparable with the brotli column above** -- different compressor,
different level, and this counts files on disk rather than responses the browser actually
fetched.

### `non-keyed/mires-vanilla`

Entry B: standalone crate, DOM ops written by hand in Rust over `#[js_extern]`.

The 2 artifacts the page loads, per `bench-artifacts.json`:

| artifact       | raw        | gzip -9    |
|:-------------- | ----------:| ----------:|
| `index.html`   | 1,924      | 507        |
| `dist/main.js` | 43,187     | 10,989     |
| **total**      | **45,111** | **11,496** |

Also in the directory and **not** counted above (1 files: `bench-artifacts.json`). These are served but never fetched by the benchmark page, so they are not part of
its payload -- and the harness's brotli column does not count them either.

### `non-keyed/mires-island`

Entry A: one island, `view!` + signals, the idiomatic framework path.

The 6 artifacts the page loads, per `bench-artifacts.json`:

| artifact                | raw        | gzip -9    |
|:----------------------- | ----------:| ----------:|
| `index.html`            | 2,938      | 908        |
| `demo/island-loader.js` | 3,320      | 1,483      |
| `demo/chunks/bench.js`  | 12,161     | 3,026      |
| `demo/chunks/shared.js` | 9,452      | 2,230      |
| `demo/mires-dom.js`   | 24,080     | 9,140      |
| `demo/island-rt.js`     | 20,949     | 7,952      |
| **total**               | **72,900** | **24,739** |

Also in the directory and **not** counted above (12 files: `_topcoat/assets/demo-2af030514e537829.css`, `_topcoat/assets/topcoat-d47cb250aeed1b76.js`, `bench-artifacts.json`, `demo/chart-lib.js`, `demo/chunks/counter.js`, `demo/chunks/dashboard.js`, `demo/chunks/life.js`, `demo/chunks/mines.js`, ...). These are served but never fetched by the benchmark page, so they are not part of
its payload -- and the harness's brotli column does not count them either.

## Melange comparison

# Melange comparison: the krausest store, twice

The non-keyed js-framework-benchmark store, written once in OCaml and once in Rust and
compiled to JavaScript by Melange v7.0.1 and by `rustc_codegen_mires`. No DOM on either
side: this measures the part of a benchmark implementation that is actually written in
the source language, with the framework and the renderer taken out of the picture.

Generated by `bench/melange/compare.mjs`. Sources: `bench/melange/store.ml`,
`bench/melange/store.rs`.

## How the two halves were made comparable

**Both sides are the same program, and it is checked rather than asserted.** Before
anything is timed, all three modules are driven through every operation against a seeded
`Math.random`, and their entire state -- every id, every label, the selection, across
nine steps including a `run` after a `clear` to prove the ids never reset -- is
compared string for string. This report does not exist if they diverge.

**Neither half seeds its own generator.** The OCaml declares
`external random : unit -> float = "random" [@@mel.scope "Math"]` and the Rust declares
`#[js(call = "Math.random")]`; both emit `Math.random()` inline, so installing one
seeded function makes both do bit-identical work. Injecting a random function as a
parameter instead was considered and rejected: it would put a hand written PRNG inside
each measurement and make "are these the same generator" a thing to be proven rather
than a thing that is true by construction.

**Melange's output imports nothing, so its runtime is zero.** That is not a courtesy --
it is the constraint the OCaml was written under. Melange's real runtime
(`melange.js/caml_array.js` and friends) is an opam package with no npm form, so an
output that reached for it could neither be run here nor honestly weighed. Every idiom
in `store.ml` was chosen against the playground to emit no import, `extract.mjs`
fails the build if one appears, and the report you are reading is only produced from a
clean import report. No stub modules were needed.

**Our runtime is counted, and it is the minimal subset.** The compiled module imports
`__rt`. Counting all 61 KB of `runtime/shim.js` would be counting the spike's test
infrastructure; counting zero would be dishonest. `min-shim.mjs` reads the
`__rt.<name>` references out of the compiled program, closes them over the references
those members make to each other, and emits only the statements that install them: 47 members, 18647 bytes. It fails loudly on a member that is referenced and absent. Member documentation is not carried
over -- the comments are for a reader of the spike, not for a browser -- but every member
body is `runtime/shim.js`'s own text, byte for byte. No JavaScript minifier is applied
to the shim in either column: `js-minify` is a backend option and only touches emitted
Rust.

**Compression levels are pinned at the extreme**, gzip 9 (through
`contract/parity/lib/budget.mjs`, whose header explains why 9 and not a realistic 6)
and brotli 11, so that a number that moves means the content moved.
Totals are summed per artifact, not compressed as one concatenated stream, because a
browser fetches and decompresses each module separately.

### The idiom choices, and which of them are asymmetric

Every one of these was verified against the v7.0.1 playground rather than assumed.

| decision | OCaml | Rust | fair? |
| --- | --- | --- | --- |
| the container | `Js.Array` externals (`push`, `concat`, `splice`, `[i]`) -- a real JS array | `Vec<Row>` -- a heap block in the model's own heap | asymmetric BY DESIGN; this is the comparison |
| the label | OCaml `string`, which IS a JS string; `^` is `+` | `String`, which is a `Vec<u8>` of real bytes | asymmetric BY DESIGN; same reason |
| `Stdlib.Array` / `a.(i)` | rejected: pulls `melange.js/caml_array.js` | n/a | -- |
| the RNG | `external ... [@@mel.scope "Math"]` | `#[js_extern] #[js(call = "Math.random")]` | identical emission, same generator |
| `Js.Math.random` | rejected: pulls `melange.js/js_math.js` | n/a | -- |
| the three word tables | `let adjectives = [| .. |]`, one module binding | `static`, one module binding. `const` would have inlined the whole literal into the loop -- three thousand throwaway arrays per `run` | equal after the fix; the keyword is the emission |
| `_random`'s remainder | `mod_float`, because integer `mod` by a variable divisor pulls `melange.js/caml_int32.js` for its zero check | integer `%`, whose zero check is one inline branch | slightly against Rust: 3 extra branches per row |
| `_random`'s truncation | `int_of_float` -> `| 0` | `as usize`, which Rust defines as saturating -> `__rt.f2i(x, 0, 4294967295)` | slightly against Rust: 3 shim calls per row |
| argument evaluation order | the three draws are `let`-bound, because OCaml evaluates application arguments right to left | naturally left to right | equal after the fix |
| `add` | `t.rows.concat(build_data t 1000)`, the reference's own line | `Vec::extend`, appending in place | the one non-transliteration; both grow-and-copy once |

## Sizes

| artifact                                                  |    kind |        raw |   gzip -9 | brotli -11 |
| --------------------------------------------------------- | ------: | ---------: | --------: | ---------: |
| Melange v7.0.1 / store.melange.js                         | program |       2467 |       931 |        831 |
| **Melange v7.0.1 TOTAL**                                  |         |   **2467** |   **931** |    **831** |
| rustc_codegen_mires, readable / store.readable.js            | program |      95120 |     17787 |      14630 |
| rustc_codegen_mires, readable / shim.js (minimal subset)     | runtime |      18647 |      5097 |       4491 |
| **rustc_codegen_mires, readable TOTAL**                      |         | **113767** | **22884** |  **19121** |
| rustc_codegen_mires, js-minify=on / store.minified.js        | program |      43150 |     10225 |       8866 |
| rustc_codegen_mires, js-minify=on / shim.js (minimal subset) | runtime |      18647 |      5097 |       4491 |
| **rustc_codegen_mires, js-minify=on TOTAL**                  |         |  **61797** | **15322** |  **13357** |

| total                          |    raw |    vs | gzip -9 |    vs | brotli -11 |    vs |
| ------------------------------ | -----: | ----: | ------: | ----: | ---------: | ----: |
| Melange v7.0.1                 |   2467 |  1.0x |     931 |  1.0x |        831 |  1.0x |
| rustc_codegen_mires, readable     | 113767 | 46.1x |   22884 | 24.6x |      19121 | 23.0x |
| rustc_codegen_mires, js-minify=on |  61797 | 25.0x |   15322 | 16.5x |      13357 | 16.1x |

Line counts, for scale: 173 for `store.melange.js`, 2182 for `store.readable.js`, 190 for `store.minified.js`, 521 for the minimal shim.

## Node micro-benchmark

One iteration is the krausest operation set end to end, over a store of its own:

```
create -> run(1000) -> update -> select -> swapRows -> delete x10 -> add -> clear
```

200 untimed iterations per side first, so that no round is measuring V8 still
tiering up; then 5 warmup iterations and 25 measured ones, timed individually with
`process.hrtime.bigint()`; the whole thing 3 times. The seed is reset at the
start of every iteration, so all 25 of them build exactly the same two thousand
labels. Medians, because one iteration in a batch catches the collector and a mean does
not survive that; and a full collection is forced before each side's batch, because
one iteration of the Rust side allocates about two thousand JavaScript arrays and
whether a round happens to contain a major collection otherwise moves its median by
about as much as the gate allows.

| implementation                 | median ms | IQR ms | min ms | max ms | vs Melange | round spread |
| ------------------------------ | --------: | -----: | -----: | -----: | ---------: | -----------: |
| Melange v7.0.1                 |     0.077 |  0.001 |  0.075 |  0.079 |       1.0x |         3.6% |
| rustc_codegen_mires, readable     |     4.627 |  0.145 |  4.381 |  8.816 |      60.2x |         3.3% |
| rustc_codegen_mires, js-minify=on |     4.903 |  0.411 |  4.696 |  9.601 |      63.8x |         0.9% |

Per-round medians (the gate is that these agree within 10%):

| implementation                 | round 1 | round 2 | round 3 | spread |
| ------------------------------ | ------: | ------: | ------: | -----: |
| Melange v7.0.1                 |   0.077 |   0.080 |   0.077 |   3.6% |
| rustc_codegen_mires, readable     |   4.778 |   4.694 |   4.627 |   3.3% |
| rustc_codegen_mires, js-minify=on |   4.946 |   4.924 |   4.903 |   0.9% |

Worst spread across rounds: **3.6%**, inside the 10% gate.

`js-minify` renames and reformats; it does not change the program, and the two Rust
rows agreeing is the check on that.

## Readability

### Melange, in full (173 lines)

```js
// Generated by Melange

const adjectives = [
  "pretty",
  "large",
  "big",
  "small",
  "tall",
  "short",
  "long",
  "handsome",
  "plain",
  "quaint",
  "clean",
  "elegant",
  "easy",
  "angry",
  "crazy",
  "helpful",
  "mushy",
  "odd",
  "unsightly",
  "adorable",
  "important",
  "inexpensive",
  "cheap",
  "expensive",
  "fancy"
];

const colours = [
  "red",
  "yellow",
  "blue",
  "green",
  "pink",
  "brown",
  "purple",
  "brown",
  "white",
  "black",
  "orange"
];

const nouns = [
  "table",
  "chair",
  "house",
  "bbq",
  "desk",
  "car",
  "pony",
  "cookie",
  "sandwich",
  "burger",
  "pizza",
  "mouse",
  "keyboard"
];

function random_below(max) {
  return Math.round(Math.random() * 1000) % max | 0;
}

function create(param) {
  return {
    rows: [],
    next_id: 1,
    selected: -1
  };
}

function build_data(t, count) {
  const data = [];
  for (let _for = 1; _for <= count; ++_for) {
    const adjective = adjectives[random_below(adjectives.length)];
    const colour = colours[random_below(colours.length)];
    const noun = nouns[random_below(nouns.length)];
    const row = {
      id: t.next_id,
      label: adjective + (" " + (colour + (" " + noun)))
    };
    t.next_id = t.next_id + 1 | 0;
    data.push(row);
  }
  return data;
}

function run(t) {
  t.rows = build_data(t, 1000);
  t.selected = -1;
}

function run_lots(t) {
  t.rows = build_data(t, 10000);
  t.selected = -1;
}

function add(t) {
  t.rows = t.rows.concat(build_data(t, 1000));
}

function update(t) {
  const length = t.rows.length;
  let index = 0;
  while (index < length) {
    const row = t.rows[index];
    row.label = row.label + " !!!";
    index = index + 10 | 0;
  };
}

function select(t, index) {
  t.selected = index;
}

function $$delete(t, index) {
  t.rows.splice(index, 1);
}

function swap_rows(t) {
  if (t.rows.length <= 998) {
    return;
  }
  const held = t.rows[1];
  t.rows[1] = t.rows[998];
  t.rows[998] = held;
}

function clear(t) {
  t.rows = [];
  t.selected = -1;
}

function length(t) {
  return t.rows.length;
}

function row_id(t, index) {
  return t.rows[index].id;
}

function row_label(t, index) {
  return t.rows[index].label;
}

function selected(t) {
  return t.selected;
}

export {
  adjectives,
  colours,
  nouns,
  random_below,
  create,
  build_data,
  run,
  run_lots,
  add,
  update,
  select,
  $$delete,
  swap_rows,
  clear,
  length,
  row_id,
  row_label,
  selected,
}
/* No side effect */
```

### rustc_codegen_mires, the two functions that matter

The readable emission is 2182 lines, so it is excerpted rather than
embedded. Most of what is not shown is `core` and `alloc`: `Vec`'s growth path,
`String`'s UTF-8 encoder, the allocator shims, the panic machinery, and the
17 `precondition_check`
stubs the collector proved unreachable and the emitter kept as self-describing throws.

`build_data` -- against Melange's `build_data` above:

```js
// Store::build_data
function store$impl_0_build_data$h9bc122e6a8d36c40(self, count) {
  let data, made, adjective, _8, _9, colour, _15, _16, noun, _22, _23, label, _31, _32;
  data = alloc$vec$impl_0_with_capacity$hb92abf1e34b8aade(count);
  made = 0;
  for (;;) {
    if (made < count) {
      _8 = store$ADJECTIVES$h03ca8ad7e87f5eda;
      _9 = store$random_below$h1499d2cc45e0f33a(25);
      if (!(_9 < 25)) {
        core$panicking$panic_bounds_check$h0d8d1e9ff46bdeb5(_9, 25, { filename: "bench/melange/store.rs", line: 193, col: 29 });
      }
      adjective = _8[_9];
      _15 = store$COLOURS$h4af6d6d1609a4539;
      _16 = store$random_below$h1499d2cc45e0f33a(11);
      if (!(_16 < 11)) {
        core$panicking$panic_bounds_check$h0d8d1e9ff46bdeb5(_16, 11, { filename: "bench/melange/store.rs", line: 194, col: 26 });
      }
      colour = _15[_16];
      _22 = store$NOUNS$h4b47d14d0c69836f;
      _23 = store$random_below$h1499d2cc45e0f33a(13);
      if (!(_23 < 13)) {
        core$panicking$panic_bounds_check$h0d8d1e9ff46bdeb5(_23, 13, { filename: "bench/melange/store.rs", line: 195, col: 24 });
      }
      noun = _22[_23];
      _32 = core$str$impl_0_len$h487e05cd7e931e51(adjective);
      _31 = _32 + core$str$impl_0_len$h487e05cd7e931e51(colour) >>> 0;
      label = alloc$string$impl_0_with_capacity$h5b3fbfd1e58ae63c((_31 + core$str$impl_0_len$h487e05cd7e931e51(noun) >>> 0) + 2 >>> 0);
      alloc$string$impl_0_push_str$h43eb738e8f8948e0(label, adjective);
      alloc$string$impl_0_push$h136346438a9ac781(label, 32);
      alloc$string$impl_0_push_str$h43eb738e8f8948e0(label, colour);
      alloc$string$impl_0_push$h136346438a9ac781(label, 32);
      alloc$string$impl_0_push_str$h43eb738e8f8948e0(label, noun);
      alloc$vec$impl_44_push$he5cdfc25e6b3ec7c(data, { id: self.next_id, label: label });
      self.next_id = self.next_id + 1 >>> 0;
      made = made + 1 >>> 0;
    } else {
      return data;
    }
  }
}
```

`random_below`, the one place both compilers had to decide what a remainder is:

```js
// random_below
function store$random_below$h1499d2cc45e0f33a(max) {
  let _2;
  _2 = __rt.f2i(Math.round(Math.random() * 1000), 0, 4294967295);
  if (max === 0) {
    core$panicking$panic_const$panic_const_rem_by_zero$hed0143a69c96b3da({ filename: "bench/melange/store.rs", line: 175, col: 5 });
  }
  return _2 % max >>> 0;
}
```

against Melange's:

```js
function random_below(max) {
  return Math.round(Math.random() * 1000) % max | 0;
}
```

And `update`, the smallest operation, where the difference is a field write against a
call through an index-and-bounds-check helper:

```js
// bench_update
function bench_update(store) {
  let length, index;
  index = 0;
  length = alloc$vec$impl_1_len$h53904c7d881eed5d(store.rows);
  for (;;) {
    if (index < length) {
      alloc$string$impl_0_push_str$h43eb738e8f8948e0(alloc$vec$impl_16_index_mut$h63f345e748d8193d(store.rows, index, { filename: "bench/melange/store.rs", line: 252, col: 19 }).label, " !!!");
      index = index + 10 >>> 0;
    } else {
      return;
    }
  }
}
```

against Melange's:

```js
function update(t) {
  const length = t.rows.length;
  let index = 0;
  while (index < length) {
    const row = t.rows[index];
    row.label = row.label + " !!!";
    index = index + 10 | 0;
  };
}
```

## Provenance

- node v24.5.0, Google Chrome 151.0.7922.72
- Melange playground v7.0.1 (`https://melange.re/v7.0.1/playground/`), driven by
  puppeteer-core; the compiler is client side, so nothing was uploaded
- spike commit `4f0de0a`
- generated 2026-07-31

## Provenance

- harness: github.com/krausest/js-framework-benchmark @ `247fafa22c1f2caeb4cad179aa64cf444398cbc7`
- browser: Google Chrome 151.0.7922.72 (not headless, 1280x800, `--js-flags=--expose-gc`)
- node: v24.5.0
- generated: 2026-07-31T10:58:13.374Z
- result files: 90 in `bench/results-raw/`, 6 of 6 implementations
- CPU iterations per benchmark: 15, 25

Medians here are recomputed from the raw `values` arrays in `bench/results-raw/`, not read off
the `median` field the harness writes beside them; the two agree on every value in this run.

