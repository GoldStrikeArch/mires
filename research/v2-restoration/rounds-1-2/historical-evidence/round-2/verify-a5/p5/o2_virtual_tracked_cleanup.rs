use alloc::boxed::Box;
use alloc::string::String;
use core::hint::black_box;
use core::panic::Location;
trait Tr {
    #[track_caller]
    fn tm(&self) -> u32;
    fn um(&self) -> u32;
}
struct A(u32);
struct B(u32);
impl Tr for A {
    fn tm(&self) -> u32 {
        Location::caller().line() * 1000 + Location::caller().column() + self.0
    }
    fn um(&self) -> u32 {
        self.0
    }
}
impl Tr for B {
    fn tm(&self) -> u32 {
        Location::caller().line() * 1000 + Location::caller().column() + self.0 * 2
    }
    #[track_caller]
    fn um(&self) -> u32 {
        Location::caller().line() + self.0
    }
}
fn pick(k: u32) -> Box<dyn Tr> {
    if k == 0 { Box::new(A(1)) } else { Box::new(B(2)) }
}
#[track_caller]
fn work(x: &dyn Tr) -> u32 {
    let s = String::from("abc");
    x.tm() + s.len() as u32
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = pick(black_box(0));
    let b = pick(black_box(1));
    print_u32(work(&*a));
    print_u32(work(&*b));
    let p: fn(&dyn Tr) -> u32 = work;
    print_u32(p(&*a));
}
