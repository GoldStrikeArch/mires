#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../examples/mini_core.rs"]
mod mini_core;
use mini_core::*;

// Compile with:
//
//   scripts/compile.sh demo/counter.rs demo/counter.js
//
// The counter's state lives in JavaScript; these are pure functions over it.

// The counter shows ten distinct values, 0..=9, then wraps back to zero. The
// bound is spelled as a literal rather than a `const` so the patterns below
// stay literal patterns, which need nothing from mini_core.

struct Counter {
    count: i32,
}

impl Counter {
    fn increment(&mut self) {
        // Wrapping exercises a branch and a field write in the compiled output
        // rather than a single add.
        if self.count >= 9 {
            self.count = 0;
        } else {
            self.count = self.count + 1;
        }
    }
}

/// Click handler: takes the current count and returns the next one.
#[no_mangle]
fn counter_clicked(n: i32) -> i32 {
    let mut counter = Counter { count: n };
    counter.increment();
    counter.count
}

/// Classifies a count so the page can label it without string formatting:
/// 0 = start, 1 = counting, 2 = about to wrap.
#[no_mangle]
fn counter_label_kind(n: i32) -> i32 {
    match n {
        0 => 0,
        9 => 2,
        _ => 1,
    }
}

/// Logs a count through the print shim (`__rt.js_log_i32`), so the demo also
/// demonstrates a call out to the runtime.
#[no_mangle]
fn counter_log(n: i32) {
    print_i32(n);
}
