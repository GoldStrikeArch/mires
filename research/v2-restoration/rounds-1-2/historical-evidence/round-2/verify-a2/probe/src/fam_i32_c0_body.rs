include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_c::<i32, 0>(0);
}
