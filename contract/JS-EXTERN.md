# JS-EXTERN: the foreign call shapes

What a `#[js_extern]` declaration means at the JavaScript level, stated once so that the backend's lowering and the dashboard's chart bindings cannot disagree about it while both looking correct.

The machine-readable half is `fixtures/js-extern/vectors.json`. This page is the prose: what each call shape is, what makes it different from the shape next to it, and what is not settled.

**Status.** `#[js_extern]` landed in wave 4: the macro crate, the descriptor codec and the backend's lowering all exist. This page and its vectors were written first, so the lowering was written against a stated target rather than against a chart that happened to appear. The attribute encoding was left to the backend to design and is now described in [The attribute encoding](#the-attribute-encoding) below, read out of the one file that defines it.

## Why this page is a contract artifact and not backend documentation

Three consumers have to agree, and two of them do not exist yet:

1. the backend, which lowers a descriptor to JavaScript;
2. the dashboard, whose chart bindings are written against the descriptor;
3. this harness, which asserts that the compiled output performs the operations the descriptor named.

The failure this prevents is specific and quiet. Every shape below has a neighbour it can be confused with, and against a real chart library the confusion is invisible: `new Chart(el, cfg)` and `Chart(el, cfg)` both draw a chart if the library tolerates both, `chart.resize(800)` and `chart.resize(800, undefined)` both resize, and a return wrapper that only handles `undefined` looks identical to one that handles `null` until the day the library returns the other one. So the shapes are pinned here, against a recorder, where the distinction is the thing being measured.

## The fake library

`fixtures/js-extern/chart-lib.mjs` is a checked-in fake of a chart library, written as a recorder rather than as an implementation. It is the `#[js_extern]` counterpart of `harness/trace.mjs`: a module that runs to completion without a DOM and leaves behind a normalized, order-sensitive trace of exactly which JavaScript operations were performed on it.

Its surface is modelled on the way Chart.js is actually used, because the dashboard is what this has to carry. Nothing in it is derived from Chart.js and nothing here should be read as a claim about Chart.js.

| binding | shape | notes |
|---|---|---|
| `Chart` | named export, constructor | `new Chart(target, config)`. Calling it without `new` records the attempt and throws. |
| `default` | default export, constructor | The same constructor under the other binding. A distinct function object, so which binding was imported is observable. |
| `Chart.register(plugin)` | static method | A call reached through a scope path rather than through a bare export. |
| `Chart.version` | static property | A read at the same scope. |
| `chart.update(data)` | instance method | One argument, no return value. |
| `chart.resize(width, height)` | instance method | The second argument is optional and the library branches on `arguments.length`. |
| `chart.destroy()` | instance method | No arguments. |
| `chart.getPoint(index)` | instance method | Returns a point in range and `null` out of range. |
| `chart.data` | instance property | Returns a further view, so a scope path of more than one step is expressible. |
| `chart.data.datasets` | indexed collection | Numeric index reads and `length`. Out of range reads return `undefined`. |
| `chart.title` | instance property | Readable and writable. |

Two design choices in the fake are load-bearing and worth stating, because both make something observable that would otherwise not be.

**The constructor is exported twice as two distinct function objects.** A library shipping both an ESM default and a named export is ordinary, and a descriptor has to say which binding it imports. If both exports were the same object that choice would be unobservable and the trace would agree with either answer. Every record made through a binding carries `via`, so the answer is in the vector.

**The two out of range reads disagree on purpose.** `getPoint(99)` returns `null` and `datasets[9]` returns `undefined`. Both are real library behaviour, they sit next to each other, and a return wrapper written to test one of them fails on the other. Treating them as interchangeable is the bug those two vectors exist to catch.

Labels in a trace are computed when a record is written, never stored when an object is built. Instances are numbered in first appearance order, and a property view is labelled by its path from its instance, so `chart.data.datasets` records as `chart#0.data.datasets` and a walk is legible in a diff instead of being three anonymous numbers.

## The reference emissions

`fixtures/js-extern/drivers.mjs` holds one driver per vector: the JavaScript that a correct lowering has to be equivalent to. Each driver is executed against the fake and the trace it produced is recorded into `vectors.json`, so a vector's expected trace is not a transcription of what somebody believed the shape does. It is what that expression actually did.

A recording cannot disagree with the code that produced it, so recording alone proves nothing about intent. Each driver therefore also carries hand-written assertions about the shape it is demonstrating, and those run on every check whether or not the fixture is being rewritten. Two further proofs run over the whole set: every required shape has at least one vector, and no vector's assertions hold for any other vector's recording. That last one is the real guard. If `send-one-arg`'s claims held for `index-length`'s trace, they were never claims about `send`. 272 pairs are tested and none is confusable.

The backend does not have to emit the driver's text. It has to be indistinguishable from it at the trace.

## The shapes

### new

Construct an instance. Vectors `new-named`, `new-default`, `new-without-new`.

The negative arm is the one worth reading. A lowering that emits a plain call where the descriptor said `new` does not quietly produce a slightly wrong chart. The library throws a `TypeError` naming the missing `new`, the way a real class does, and the vector pins that so the failure mode is known rather than assumed.

The two positive arms differ only in `via`. That field is the entire record of which import binding the descriptor named.

### send

Call a method on an instance. Vectors `send-one-arg`, `send-zero-args`, `send-trailing-omitted`, `send-trailing-undefined`.

Argument count is observable and is recorded as `argc` separately from the argument list, because the two spellings of an omitted optional argument are not equivalent to a library that branches on `arguments.length`. `send-trailing-omitted` and `send-trailing-undefined` are the same call written both ways and their traces differ. This is Melange's trailing `undefined` stripping made into a measurement: the descriptor has to pick a rule, and this is where a lowering that pads optional arguments shows up.

The zero argument case is its own vector because an empty argument list has to be empty, not a list containing one `undefined`.

### get

Read a property. Vectors `get-property`, `get-scoped`, `get-static`, and `set-property` for the assignment counterpart.

A `get` is not a zero argument `send`. There is no call, and a lowering that emitted `chart.data()` finds a non function. The scope axis is what makes `chart.data.datasets` one declaration rather than two, and the trace shows both steps, because each step is a real property read that the library sees. `get-static` is the same shape rooted at the module binding instead of at an instance.

`set` is not required this wave. The four shapes the dashboard needs are new, send, get and index. It is pinned anyway because it shares the getter's scope machinery and costs nothing now.

### index

Read an element out of an indexed collection. Vectors `index-in-range`, `index-length`, `index-out-of-range`.

This is a separate shape from `get` because the index is a runtime value rather than a constant path. The index is recorded as a number and not as the string a property read would carry.

`length` is a `get` on the collection rather than an `index`, which matters more than it looks: anything iterating the collection from Rust needs it, so a descriptor that models a collection as opaque cannot express the loop at all.

### call

Call a function reached through a scope path, as in `Chart.register(plugin)`. Vector `call-static`.

Not one of this wave's four, but it is how the real library is initialised, so the dashboard will need it. The vector is here to be met rather than discovered.

## Return wrappers

`nullable-some` and `nullable-none` are the two arms of a nullable return, the `null_to_opt` of Melange's descriptor set. In range the value arrives present; out of range the library answers `null`.

The neighbouring hazard is `index-out-of-range`, which answers `undefined` for the same class of mistake on the same object graph. A wrapper that tests only one of the two passes every other vector on this page.

## The attribute encoding

Landed in wave 4. The section below is the contract's reading of it, written against the one file that defines it, and it answers the six questions this stub used to ask.

The encoding's source of truth is `js-extern-macro/src/descriptor.rs`. That file is compiled twice: the macro crate builds it to encode, and the backend includes the same file with `#[path = "../../js-extern-macro/src/descriptor.rs"]` to decode. This is worth stating here because it is the reason this page can describe one format rather than two. A format written down in two places is a format that drifts, and a drifted descriptor is not an error, it is a wrong emission that still runs.

### The grammar

A declaration travels as a `link_section` on the marker function the macro writes, which is how `codegen_fn_attrs` carries it across a crate boundary without new machinery.

```text
rcgjs.ext.<version>.<shape><flags>.<module-len>,<name-len>.<module><name>
```

`rcgjs.ext.2.n-.8,5.chart.jsChart` is the worked example, and it is `new Chart(..)` on the `Chart` export of `chart.js`.

| field | what it is |
|---|---|
| `rcgjs.ext.` | the prefix the backend matches on. `#[js_extern]` sections live outside the `rcgjs.tc.` namespace the view ABI's markers use, so the two never collide. |
| `<version>` | currently `2`, and the first thing decoded. A descriptor written by another version is named as such rather than mis-read. |
| `<shape>` | one letter: `c` call, `n` new, `s` method (this page's "send"), `g` get, `t` set, `i` index, `j` index-set. |
| `<flags>` | a run of letters, or `-` for none. `n` is a nullable return, `g` is rooted at the global scope. `encode` writes them in that order and `decode` accepts them in any. |
| `<module-len>,<name-len>` | byte counts, decimal. |
| `<module><name>` | the module specifier and the JavaScript name, concatenated, split by the first length. |

`rcgjs.ext.2.gn.0,5.width` is the other end of the range: a nullable get of `width`, rooted at an argument because the module is empty and the `g` flag is absent. Note that the `g` there is the SHAPE letter for a get, in the first position; the flags are `n` alone. The two `g`s are in different fields and mean different things.

An empty name carries meaning by being empty: it is an index straight off its receiver, with no path walked first.

The version is 2 rather than 1 because wave 5 added the `g` flag. The grammar did not move, since the flags field was already a run of letters, but the vocabulary did, and the module doc's rule is that a change to the grammar bumps the version. Bumping was the better call anyway: a version-1 decoder meeting a `g` refuses it as "not a flag", which is safe, but the version check refuses it by naming the version, which is the more useful message. Nothing persists a descriptor across a build, so nothing had to migrate.

### The three roots

Every shape is rooted somewhere, and there are exactly three places. This is the wave-5 addition and it is worth stating separately, because the bug it fixes was silent.

| root | how it is spelled | what it emits |
|---|---|---|
| an argument | a member shape with no module and no `g` | the operation acts on the call's FIRST argument, which is therefore not passed on |
| a module binding | a module, from the block or the declaration | an `import { <first segment> as <local> }`, and the path walks from there |
| the global scope | the `g` flag, `#[js(global, ..)]` | the identifier itself, importing nothing |

Before wave 5 the rooting was a predicate, `module.is_empty()`, and it had no way to say "global" for a shape that has a receiver. `new EventSource(url)` worked, because a construction has no receiver and so was already global with no module. But `#[js(get = "dash.status")]` with no module read a property of ARGUMENT ZERO: a declaration taking no arguments produced `undefined.status`, and one taking an unrelated argument read a property of that. Neither is a refusal, which is what made it the dangerous half. The flag makes the third root explicit, and the backend now switches on a `Root` enum rather than re-reading `module.is_empty()`, so one answer is not computed in two places that can disagree.

Three rules come with it, each with a reason rather than a convention.

1. **`call` and `new` need no flag.** They have no receiver, so with no module they are already global. Not requiring it keeps every wave-4 declaration compiling and keeps `Math.max` a one-liner.
2. **`global` CLEARS the block's module** rather than sitting beside it, and `#[js(global, module = "x")]` is a spanned error naming both roots. This is exactly the dashboard's shape: a browser global declared inside a `#[js_extern(module = "chart.js")]` block. Without the clearing rule that declaration would inherit `chart.js` and emit an import for a name the host already has.
3. **An index is the one shape a module does not move.** `#[js(index = "data.datasets")]` inside a block naming a package is the ordinary declaration, and it is the chart fixture's own. Re-rooting it at the module binding would change what `index-in-range` and `index-out-of-range` emit. So an index roots at its argument unless the flag says otherwise. The asymmetry is deliberate and is documented on `Descriptor::root`.

### Why the lengths and not a delimiter

Because no character can be safely excluded from both halves.

A module specifier holds `.`, `/`, `-`, `@` and `:` in ordinary use: `chart.js`, `@scope/pkg`, `./local.js`, `node:fs`. A JavaScript name holds `$` and `_`, and the name here is a dotted path, so it holds `.` too. Any delimiter picked out of that set needs an escape, and an escaping bug is the quiet kind: the descriptor still decodes, it just decodes to a different interface than the one declared, and the program calls the wrong thing successfully. The test that pins this is `a_specifier_full_of_delimiters_survives` in `descriptor.rs`, which round-trips `a.b,c-d/e` for exactly this reason.

Lengths need no escape and make decoding total. Every failure path returns an error naming the descriptor text, and none of them guesses: a short payload, a length that is not a number, a lengths field with no comma, an unknown flag, an unknown shape letter, and a length that lands inside a multi-byte character are six separate refusals with six separate messages. `every_malformed_descriptor_is_an_error_and_not_a_guess` and `a_length_that_splits_a_character_is_refused` hold that shut.

The version being first is the same argument one level up. A macro and a backend that disagree report each other by name instead of emitting something.

A binary codec was considered and rejected. Postcard plus base64 would need the same codec crate on both sides, which is version skew waiting to happen, and its failure mode is a mis-decoded descriptor rather than a refusal. The text form is diffable in a fixture, readable in an error message, and version checked before anything else is read.

### The six answers

**1. The module and the import binding.** The block carries `#[js_extern(module = "chart.js")]`, or nothing, and a single declaration overrides it with `module = ".."` inside its own `#[js(..)]`. The import binding is the path's FIRST segment and nothing else: `#[js(call = "Chart.register")]` imports `Chart` and reaches `register` through it, because `import { Chart.register }` is not something anyone can write. The binding is interned by module and name together, so two modules exporting `Chart` bind two locals and two crates importing the same `Chart` bind one.

The default export is named `default`. Imports are emitted as named specifiers, and `import { default as x }` is exactly a default import, so `new-named` is `#[js(new = "Chart")]` and `new-default` is `#[js(new = "default")]`. Both are measured rather than asserted: `examples/extern-tests/01_chart.rs:49-55` declares the pair, the emitted import is visible in `01_chart.js.expected:5`, and `scripts/extern-check.mjs:115` drives the compiled `new-default` entry point against this page's `new-default` vector. That matters because `via` is the only thing the two vectors differ on, so an unmeasured default binding would leave half of the `new` shape unproven.

**2. The scope path.** The name is a dotted path and the shape only decides where the path is rooted. `chart.data.datasets` is one declaration. The walk is emitted one member at a time rather than collapsed, because each step is a property read the library actually sees, and a library whose `data` getter has an effect would notice the difference. This is what makes `instance.data` and `Chart.version` the same shape written twice: a member shape acts on its first argument unless it names a module, in which case it is a static of that module's export.

**3. The optional trailing argument.** It is not spelled, and that is the answer rather than an omission. Arity comes from the Rust signature: the shape consumes its receiver, its key and its written value, and every remaining argument passes through in order. Nothing is padded and nothing is stripped. So a declaration written with two parameters lowers to `send-trailing-omitted` and there is no way for it to produce `send-trailing-undefined` by accident. Reaching `send-trailing-undefined` requires declaring the third parameter and passing something that lowers to `undefined`, which is a different declaration and reads as one. Melange strips trailing `undefined`; this does not, because it never creates one.

**4. The nullable return.** The `n` flag, written `#[js(nullable)]`, and the wrapper accepts BOTH. The test is `== null`, not `=== null`, which is what makes `undefined` a `None` as well, and that is the case that matters: a missing property reads as `undefined`, not as `null`. So `nullable-none`, where `getPoint(99)` returns `null`, and `index-out-of-range`, where `datasets[9]` returns `undefined`, both become `None`, which is the disagreement the fake was built to expose. Two further constraints hold the flag honest. It is rejected at declaration time on a shape that returns nothing, since `nullable` on a setter is meaningless, and it is a zombie at lowering time if the destination is not an `Option`, so a flag that does not match the return type is a named refusal rather than a wrong value. The value is held in a temporary before the test, because both arms read it and a declared interface may be called for its effect as well as its answer.

**5. The indexed collection.** An accessor, not a type. `index` is its own shape because the key is a runtime value rather than a constant path, and it is recorded as a number rather than as the string a property read carries. `length` is reached as a plain `get` on the same collection. Modelling the collection as an opaque type would put `length` out of reach and make the loop inexpressible, which is why this is a decision and not a detail.

**6. A declaration the library cannot satisfy.** It splits three ways, and the split is deliberate.

- Wrong on its face: a compile error, spanned on the declaration. Too few parameters for the shape, `nullable` on a setter, a name that is not a JavaScript path, a shape rooted at a binding with no name, more than one `#[js(..)]`, no shape at all. The arity check is duplicated in the macro and the backend so that the common mistake is a spanned error on the declaration instead of a zombie at some call site.
- Undecodable or unlowerable: a zombie, deferred and reachability-gated, carrying the descriptor text in its message. A malformed descriptor, a call with fewer arguments than the shape acts on, a `set` with no property to write, a `nullable` whose destination is not an `Option`. Unreached code compiles; reaching it fails by name.
- Wrong about the library: not detected here at all. A declaration that says `new` where the library exports a plain function, or names an export that does not exist, is a JavaScript `TypeError` at run time. This is the boundary of what a declared interface can promise, and it is why the vectors record a `new-without-new` trace: the failure is pinned as a known shape rather than assumed to be impossible.

**7. A question the stub did not ask, and the dashboard does: the global scope is a root of its own.** Covered in [The three roots](#the-three-roots) above. It belongs in this list because it is the one answer that changed after the vectors on this page were written, and it changed because of a silent wrong emission rather than a refusal.

The vectors do not move for any of this. A descriptor encoding that cannot produce these traces is the wrong encoding, whatever it looks like in Rust.

### What the global root is measured against

The vectors on this page drive the chart library, which is a module, so none of them exercises a global. That is a hole in this page rather than in the implementation: the backend drives eight global shapes against a recorder of its own, in `examples/extern-tests/02_globals.rs` with `scripts/globals-check.mjs` (17 checks), and one of those checks reads the emitted TEXT to assert that a global imported nothing, which running the program cannot tell you because an import-bound global resolves just as well.

The proposed vector ids, for when a global recorder joins the chart library in `fixtures/js-extern/`, are `new-global`, `call-global`, `call-global-path`, `get-global`, `set-global`, `index-global`, `nullable-global`, and the negative `new-global-without-new`. `globals-check.mjs` already drives all eight and becomes their driver unchanged. Until then the global root is proven, but it is proven next door rather than here, and this page should not be read as having pinned it.

## Running it

```sh
node --import ./harness/register-loader.mjs harness/check-js-extern.mjs
```

Compares the committed vectors against what the drivers produce now and runs every assertion. Add `--record` to rewrite the fixture; that is what `run-all.mjs` does, so a change to a driver shows up as fixture drift.

To check a compiled module against the vectors:

1. Compile a module whose `#[js_extern]` declarations describe `chart-lib.mjs`.
2. Resolve its import specifier for the library to `fixtures/js-extern/chart-lib.mjs`, the way `run-trace.mjs` resolves `r-dom` to the recording stub. Do not rewrite the compiled text.
3. Drive it so it performs the operation named by a vector's `id`.
4. Compare `__trace()` with that vector's `trace`, using `compareTrace` exported from `harness/check-js-extern.mjs`.

`compareTrace` compares position by position rather than by the longest common subsequence alignment `compare-trace.mjs` uses. That comparator exists because two emitters can order independent runtime calls differently and both be correct. Here the trace is the sequence of operations one expression performed on one object, and reordering it changes what happened.

## Open questions

- **The import specifier is decided, its resolution is not.** The encoding settled the first half: the module string is carried verbatim and emitted verbatim as the `from` of a named import, so a declaration saying `chart.js` produces `from "chart.js"` and one saying `./local.js` produces that. What still is not pinned is who resolves it. The vectors record what was done to the library, not how it was reached, and the harness continues to resolve by mapping, which accepts any answer. A build option, an import map or a bundler are all still open.
- **`Chart.register` returns nothing useful, and no vector covers a static call that does.** A static factory returning an instance would combine `call` and `new` in a way none of these vectors exercises. Add it when something needs it, rather than guessing at the shape now.
- **Nothing here covers callbacks.** A chart library that takes an event handler hands JavaScript a function that has to be a compiled Rust closure, and that is a different problem with a different owner. It is the largest hole in this page and it is deliberate.
