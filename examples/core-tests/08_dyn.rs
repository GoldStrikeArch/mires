//! Trait objects against real `core`: vtables, default methods, supertraits, `dyn` in a struct,
//! and the `core` traits that are themselves object safe.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

trait Shape {
    fn area(&self) -> i32;

    /// A default method, which the vtable has to carry even though nobody overrides it here.
    fn describe(&self) -> &'static str {
        "shape"
    }

    fn scaled(&self, factor: i32) -> i32 {
        self.area() * factor
    }
}

struct Square(i32);
struct Rect {
    w: i32,
    h: i32,
}

impl Shape for Square {
    fn area(&self) -> i32 {
        self.0 * self.0
    }
    fn describe(&self) -> &'static str {
        "square"
    }
}

impl Shape for Rect {
    fn area(&self) -> i32 {
        self.w * self.h
    }
}

/// A supertrait, so the vtable has to carry the parent's methods too.
trait Named {
    fn name(&self) -> &'static str;
}

trait Greeter: Named {
    fn greeting(&self) -> &'static str {
        self.name()
    }
}

struct Person;

impl Named for Person {
    fn name(&self) -> &'static str {
        "person"
    }
}

impl Greeter for Person {}

/// A `&mut dyn` receiver, which mutates through the vtable.
trait Counter {
    fn bump(&mut self);
    fn value(&self) -> i32;
}

struct Tally(i32);

impl Counter for Tally {
    fn bump(&mut self) {
        self.0 += 1;
    }
    fn value(&self) -> i32 {
        self.0
    }
}

struct Holder<'a> {
    inner: &'a dyn Shape,
}

fn total(shapes: &Holder) -> i32 {
    shapes.inner.area()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let square = Square(4);
    let rect = Rect { w: 3, h: 5 };

    // Dynamic dispatch, overridden and defaulted.
    let shape: &dyn Shape = &square;
    print_i32(shape.area());
    print_str(shape.describe());
    print_i32(shape.scaled(2));

    let shape: &dyn Shape = &rect;
    print_i32(shape.area());
    print_str(shape.describe());
    print_i32(shape.scaled(3));

    // The same call site, two vtables.
    let shapes: [&dyn Shape; 2] = [&square, &rect];
    let mut sum = 0;
    let mut i = 0;
    while i < shapes.len() {
        sum += shapes[i].area();
        i += 1;
    }
    print_i32(sum);

    // A `dyn` behind a struct field.
    print_i32(total(&Holder { inner: &square }));

    // A supertrait's method through the subtrait's object.
    let greeter: &dyn Greeter = &Person;
    print_str(greeter.greeting());
    print_str(greeter.name());

    // `&mut dyn`.
    let mut tally = Tally(0);
    {
        let counter: &mut dyn Counter = &mut tally;
        counter.bump();
        counter.bump();
        print_i32(counter.value());
    }
    print_i32(tally.value());

    // `core`'s own object safe traits. `FnMut` is one of them.
    let mut seen = 0i32;
    {
        let f: &mut dyn FnMut(i32) = &mut |x| seen += x;
        f(3);
        f(4);
    }
    print_i32(seen);

    // A generic function monomorphized against `dyn`, so the receiver is fat at every step.
    fn twice(shape: &dyn Shape) -> i32 {
        shape.area() + shape.area()
    }
    print_i32(twice(&square));
    print_i32(twice(&rect));
}
