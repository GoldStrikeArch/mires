include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_a::<(u8, u32), 2>(0);
}
