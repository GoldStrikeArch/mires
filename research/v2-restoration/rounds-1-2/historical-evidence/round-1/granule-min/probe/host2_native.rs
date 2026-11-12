extern crate alloc;
#[allow(dead_code)] const ROUNDS: u32 = 3100000;
#[allow(dead_code)] fn print_bool(x: bool) { println!("{}", x) }
#[allow(dead_code)] fn print_usize(x: usize) { println!("{}", x) }
#[allow(dead_code)] fn print_u32(x: u32) { println!("{}", x) }
#[allow(dead_code)] fn print_str(x: &str) { println!("{}", x) }
include!("host2_body.rs");
fn main() { rust_entry() }
