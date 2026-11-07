#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// A destructor that both mutates through `&mut self` and prints, so the observable
// order is the drop order and the observable value proves the mutation happened.
struct Noisy {
    id: i32,
}

impl Drop for Noisy {
    fn drop(&mut self) {
        self.id += 100;
        print_i32(self.id);
    }
}

// Droppable fields inside a type that has its own `Drop`: the outer body runs first,
// then the fields in declaration order.
struct Pair {
    first: Noisy,
    second: Noisy,
}

impl Drop for Pair {
    fn drop(&mut self) {
        print_i32(-1);
    }
}

// No `Drop` impl of its own -- only the field glue runs.
struct Wrapper {
    tag: i32,
    inner: Noisy,
}

// Takes ownership and drops at the end of its own body.
fn consume<T>(_v: T) {}

fn early_return(n: i32) -> i32 {
    let _guard = Noisy { id: n };
    if n > 0 {
        return 1;
    }
    0
}

#[no_mangle]
fn rust_entry() {
    // Drop order inside a block: reverse of declaration.
    print_str("block");
    {
        let _a = Noisy { id: 1 };
        let _b = Noisy { id: 2 };
    }

    // Outer destructor, then fields in declaration order.
    print_str("pair");
    {
        let _p = Pair { first: Noisy { id: 3 }, second: Noisy { id: 4 } };
    }

    // Field glue with no outer `Drop` impl.
    print_str("wrapper");
    {
        let _w = Wrapper { tag: 9, inner: Noisy { id: 5 } };
    }

    // The guard drops on the early-return path before the caller sees the value.
    print_str("early");
    print_i32(early_return(6));
    print_i32(early_return(0));

    // Ownership passed to a generic function: the drop happens in the callee.
    print_str("consume");
    consume(Noisy { id: 7 });

    print_str("move");
    {
        let a = Noisy { id: 8 };
        consume(a);
        print_str("after");
    }

    // A value that is moved out of a local is not dropped again at scope end.
    print_str("shadow");
    {
        let c = Noisy { id: 10 };
        let d = c;
        print_i32(d.id);
    }

    print_str("09_drop ok");
}
