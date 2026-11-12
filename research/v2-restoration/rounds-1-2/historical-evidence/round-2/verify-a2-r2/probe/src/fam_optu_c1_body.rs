include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_c::<Option<u32>, 1>(0);
}
