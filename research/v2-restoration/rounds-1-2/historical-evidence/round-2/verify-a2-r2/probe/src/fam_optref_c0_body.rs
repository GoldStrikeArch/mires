include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_c::<Option<&'static u32>, 0>(0);
}
