//! The krausest js-framework-benchmark non-keyed store, in Rust, compiled to JavaScript by
//! rustc_codegen_mires.
//!
//! This is the Rust half of the core-logic comparison; `store.ml` is the OCaml half. Same data,
//! same operations, same order of side effects, no DOM on either side. What is being compared is
//! the store -- the part of a benchmark implementation that is actually written in the source
//! language rather than delegated to a framework.
//!
//! # What this crate is allowed to use
//!
//! `#![no_std]` with `extern crate alloc`, so `Vec` and `String` are the real ones out of the
//! sysroot and every allocation lands in the heap the shim keeps (CONTRACT.md, "Allocation").
//! That is deliberate and it is the expensive choice: an OCaml `string` is a JavaScript string
//! and a Rust `String` is a heap block of bytes, so the labels this builds are the part of the
//! comparison where the two models differ most. Measuring the cheap version instead -- keeping
//! labels as `&'static str` triples, say -- would be measuring a different program.
//!
//! The only foreign surface is two `#[js_extern]` declarations:
//!
//! ```text
//! #[js(call = "Math.random")] fn random() -> f64;
//! #[js(call = "Math.round")]  fn round(x: f64) -> f64;
//! ```
//!
//! Both are rooted at the global scope, so they emit `Math.random()` and `Math.round(x)` inline
//! and add no import to the module -- which is exactly what the OCaml half's
//! `external ... [@@mel.scope "Math"]` declarations emit. The two halves therefore draw from the
//! SAME generator, and a harness that installs one seeded function as `Math.random` gets
//! bit-identical work, and bit-identical labels, out of both. That is why the RNG is an extern
//! rather than a seeded PRNG written twice: a PRNG written twice is two pieces of code being
//! benchmarked in addition to the store, and it would have to be proven identical before any of
//! the numbers meant anything.
//!
//! # The store is a value, not a global
//!
//! There is no `static mut` here. `bench_create` returns a `Store`, and every operation takes it
//! back as `&mut Store`. In this backend an aggregate reference IS the object, so the value the
//! host holds between calls is the store itself and nothing has to be unwrapped -- the same shape
//! the OCaml half has, where `create ()` returns the record and every function takes it as its
//! first argument.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use js_extern_macro::js_extern;

// The shim's abort, for the panic handler. A `loop {}` handler would hang node on a bug instead
// of reporting it, and this crate is meant to be run, not only weighed.
unsafe extern "C" {
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { js_abort("bench/melange store.rs panicked") }
}

/// The two globals the reference implementation's `_random` is built out of. No module, so
/// nothing is imported: `#[js(call = "Math.random")]` is a path rooted at the global scope and
/// emits the call site verbatim.
#[js_extern]
unsafe extern "C" {
    /// `Math.random()`.
    #[js(call = "Math.random")]
    fn random() -> f64;

    /// `Math.round(x)`.
    #[js(call = "Math.round")]
    fn round(x: f64) -> f64;
}

// --------------------------------------------------------------------------------------- data

/// One row: an id that is never reused and a label that `update` appends to in place.
pub struct Row {
    id: u32,
    label: String,
}

/// The store.
pub struct Store {
    rows: Vec<Row>,
    /// Ids start at 1 and never reset, across every run in the process.
    next_id: u32,
    /// The index of the selected row, or -1. The reference uses `undefined`; an out-of-range
    /// integer is the same information without an `Option`, and the OCaml half spells it the
    /// same way.
    selected: i32,
}

// `static`, NOT `const`, and the difference is worth 3000 array allocations per `run`.
//
// A Rust `const` is inlined at every use, and this backend takes that literally: with `const`,
// each of the three lookups below emits the whole array literal INSIDE the loop, so a thousand
// rows build three thousand throwaway JavaScript arrays. A `static` is one module level binding
// named at each use, which is what the OCaml half's `let adjectives = [| .. |]` is and what any
// hand written JavaScript would be. Hoisting is not automatic here: the backend hoists
// `#[track_caller]` locations and long string literals and nothing else (CONTRACT.md, "Hoisted
// constants"), so the choice of keyword IS the choice of emission.
static ADJECTIVES: [&str; 25] = [
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
    "fancy",
];

/// "brown" twice, in the reference and therefore here.
static COLOURS: [&str; 11] = [
    "red", "yellow", "blue", "green", "pink", "brown", "purple", "brown", "white", "black",
    "orange",
];

static NOUNS: [&str; 13] = [
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
    "keyboard",
];

// ------------------------------------------------------------------------------------- random

/// The reference's `_random`:
///
/// ```js
/// _random(max) { return Math.round(Math.random() * 1000) % max; }
/// ```
///
/// `round` already returns a whole number, so the `as usize` truncation is exact and the
/// remainder is the same one the reference takes.
///
/// Two things this spelling costs that the OCaml half's does not, both left in on purpose because
/// each is what the idiomatic line in that language compiles to, and both are noted in the report:
///
/// * `as usize` is a SATURATING cast in Rust, so it is `__rt.f2i(x, 0, 4294967295)` -- a call into
///   the shim -- where `int_of_float` was a bare `| 0`.
/// * `%` on an integer with a non-constant divisor emits a divide-by-zero guard. OCaml's integer
///   `mod` has the same guard, and there it costs a whole runtime module rather than one branch,
///   which is exactly why the OCaml half went through `mod_float` instead.
fn random_below(max: usize) -> usize {
    (round(random() * 1000.0) as usize) % max
}

// ---------------------------------------------------------------------------------- the store

impl Store {
    fn new() -> Store {
        Store { rows: Vec::new(), next_id: 1, selected: -1 }
    }

    /// `count` fresh rows, taking ids from the store's counter. Built into a new `Vec` rather
    /// than into `self.rows`, because `add` concatenates and `run` replaces.
    fn build_data(&mut self, count: usize) -> Vec<Row> {
        let mut data: Vec<Row> = Vec::with_capacity(count);
        let mut made = 0;
        while made < count {
            // Left to right, the order the reference draws them in and the order the OCaml half
            // is pinned to.
            let adjective = ADJECTIVES[random_below(ADJECTIVES.len())];
            let colour = COLOURS[random_below(COLOURS.len())];
            let noun = NOUNS[random_below(NOUNS.len())];

            // `with_capacity` + `push_str`, never `format!`: the formatting machinery walks a
            // byte-packed template through raw pointers and is the single largest thing a small
            // program can accidentally pull in.
            let mut label = String::with_capacity(adjective.len() + colour.len() + noun.len() + 2);
            label.push_str(adjective);
            label.push(' ');
            label.push_str(colour);
            label.push(' ');
            label.push_str(noun);

            data.push(Row { id: self.next_id, label });
            self.next_id += 1;
            made += 1;
        }
        data
    }
}

/// A new, empty store. The host holds what comes back and passes it to every operation below.
#[unsafe(no_mangle)]
pub fn bench_create() -> Store {
    Store::new()
}

#[unsafe(no_mangle)]
pub fn bench_run(store: &mut Store) {
    let data = store.build_data(1000);
    store.rows = data;
    store.selected = -1;
}

#[unsafe(no_mangle)]
pub fn bench_run_lots(store: &mut Store) {
    let data = store.build_data(10000);
    store.rows = data;
    store.selected = -1;
}

/// The reference's `add` is `this.data = this.data.concat(this.buildData(1000))`, and the OCaml
/// half says that literally. `extend` appends in place instead of building a third array; both
/// end with one array of the old rows followed by the new ones, and both pay one grow-and-copy
/// of the existing elements to get there. It is the idiomatic spelling in each language of the
/// same operation, and it is the one place in the store where the two are not a transliteration.
#[unsafe(no_mangle)]
pub fn bench_add(store: &mut Store) {
    let more = store.build_data(1000);
    store.rows.extend(more);
}

/// Every tenth row from index 0, appending " !!!" to the label in place.
#[unsafe(no_mangle)]
pub fn bench_update(store: &mut Store) {
    let length = store.rows.len();
    let mut index = 0;
    while index < length {
        store.rows[index].label.push_str(" !!!");
        index += 10;
    }
}

#[unsafe(no_mangle)]
pub fn bench_select(store: &mut Store, index: i32) {
    store.selected = index;
}

#[unsafe(no_mangle)]
pub fn bench_delete(store: &mut Store, index: usize) {
    // The removed row's `String` is freed here. The OCaml half's `splice` drops a reference and
    // lets the collector have it; that difference is the model's, not the program's, and it is
    // one of the things the micro-benchmark is measuring.
    let _ = store.rows.remove(index);
}

/// The reference swaps 1 and 998, and only when there are enough rows to have both.
#[unsafe(no_mangle)]
pub fn bench_swap_rows(store: &mut Store) {
    if store.rows.len() > 998 {
        store.rows.swap(1, 998);
    }
}

#[unsafe(no_mangle)]
pub fn bench_clear(store: &mut Store) {
    store.rows = Vec::new();
    store.selected = -1;
}

// ------------------------------------------------------------------------------- observations

// Read-only accessors, so the harness can compare the two stores element by element without
// knowing either representation. The OCaml half exports the same four.

#[unsafe(no_mangle)]
pub fn bench_length(store: &Store) -> usize {
    store.rows.len()
}

#[unsafe(no_mangle)]
pub fn bench_row_id(store: &Store, index: usize) -> u32 {
    store.rows[index].id
}

/// A `&str` crosses to the host as a JavaScript string (CONTRACT.md, "`str` is a hybrid"), so
/// this is the one place the byte representation of a label is paid for on the way out.
#[unsafe(no_mangle)]
pub fn bench_row_label(store: &Store, index: usize) -> &str {
    &store.rows[index].label
}

#[unsafe(no_mangle)]
pub fn bench_selected(store: &Store) -> i32 {
    store.selected
}
