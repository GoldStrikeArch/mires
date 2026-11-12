include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_c::<f64, 1>(0);
}
