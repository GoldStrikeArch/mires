include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_b::<i8, 3>(0);
}
