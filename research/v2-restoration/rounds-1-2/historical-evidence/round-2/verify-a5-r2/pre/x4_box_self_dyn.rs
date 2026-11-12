// Pre-existing: a `self: Box<Self>` method called on `Box<dyn Trait>` throws.
use alloc::boxed::Box;
trait Shape {
    fn area(&self) -> u32;
    fn boxed_consume(self: Box<Self>) -> u32 {
        self.area() + 2
    }
}
struct Sq(u32);
impl Shape for Sq {
    fn area(&self) -> u32 {
        self.0 * self.0
    }
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let s: Box<dyn Shape> = Box::new(Sq(3));
    print_u32(s.boxed_consume());
}
