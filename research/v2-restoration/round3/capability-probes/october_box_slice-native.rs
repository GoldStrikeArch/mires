extern crate alloc;
fn print_u32(x:u32) {println!("{}",x);}
fn main() { let source = core::hint::black_box([1u32, 2, 3]); let mut owned = alloc::boxed::Box::<[u32]>::from(&source[..]); print_u32(owned.len() as u32); print_u32(owned.iter().copied().sum()); owned[1] = 9; print_u32(source[1]); print_u32(owned[1]); }
