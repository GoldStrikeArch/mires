include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_b::<D, 1>(0);
    out(40000, drops() as u64);
    out(40001, live() as i64 as u64);
}
