extern crate alloc;
fn print_u32(x:u32) {println!("{}",x);}
fn main() { const F: fn(u32) -> u32 = |x| x + 1; let f = core::hint::black_box(F); print_u32(f(41)); }
