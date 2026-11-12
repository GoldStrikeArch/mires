include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_a::<E, 3>(0);
}
