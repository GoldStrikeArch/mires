// In-place collect from wider rows to narrower rows (safe Rust): Vec<[u32; 4]> -> Vec<[u32; 2]>.
use alloc::vec::Vec;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 4]> = (0..3u32).map(|i| [i, i + 10, i + 20, i + 30]).collect();
    let f: Vec<[u32; 2]> = v.into_iter().map(|r| [r[0], r[3]]).collect();
    print_usize(f.len());
    for r in &f { print_u32(r[0]); print_u32(r[1]); }
}
