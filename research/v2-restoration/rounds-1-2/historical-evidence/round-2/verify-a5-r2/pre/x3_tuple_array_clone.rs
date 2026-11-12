// Pre-existing: cloning a tuple that holds a String through the tuple CloneShim throws.
use alloc::string::String;
fn dup<T: Clone>(t: &T) -> (T, T) {
    (t.clone(), t.clone())
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let t = (1u32, String::from("x"));
    let (t1, t2) = dup(&t);
    print_u32(t1.0 + t2.1.len() as u32);
}
